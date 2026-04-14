//! Generate variables phase.
//!
//! Generates semantic variable declarations for views that need them.
//!
//! This phase creates:
//! - Context variables for embedded views that need access to parent context
//! - Alias variables for @for loop context ($index, $first, etc.)
//! - Context read variables for @for loop context ($implicit, $index, $count, etc.)
//! - Context variable extractions inside listener handlers (from restored view)
//! - ContextLetReferenceExpr for @let declarations accessed from nested views
//!
//! Note: SavedView variables are created by the `save_restore_view` phase, not here.
//!
//! Variables are generated unconditionally and may be optimized away later
//! if their values are unused.
//!
//! Ported from Angular's `template/pipeline/src/phases/generate_variables.ts`.

use oxc_allocator::Box;
use oxc_str::Ident;

use crate::ir::enums::{SemanticVariableKind, VariableFlags};
use crate::ir::expression::{
    ContextExpr, ContextLetReferenceExpr, IrExpression, NextContextExpr, ReferenceExpr,
    ResolvedPropertyReadExpr, SlotHandle,
};
use crate::ir::ops::{CreateOp, SlotId, UpdateOp, UpdateVariableOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Information about a local reference collected from an element in a view.
#[derive(Debug, Clone)]
struct LocalRefInfo<'a> {
    /// Name of the local reference (e.g., "myDiv" from #myDiv).
    name: Ident<'a>,
    /// XrefId of the element this reference points to.
    target_id: XrefId,
    /// Slot of the target element.
    target_slot: Option<SlotId>,
    /// Offset of this reference among all refs on the element.
    offset: i32,
}

/// Information about a @let declaration collected from a view.
/// Corresponds to Angular's `LetDeclaration` interface.
#[derive(Debug, Clone)]
struct LetDeclarationInfo<'a> {
    /// XrefId of the @let declaration.
    target_id: XrefId,
    /// Slot of the @let declaration.
    target_slot: Option<SlotId>,
    /// Variable name.
    variable_name: Ident<'a>,
}

/// Lexical scope of a view, including a reference to its parent view's scope.
/// Corresponds to Angular's `Scope` interface.
#[derive(Debug)]
struct Scope<'a, 'b> {
    /// XrefId of the view this scope belongs to.
    view: XrefId,
    /// Context variables (name, value) from the view.
    context_variables: Vec<(Ident<'a>, Ident<'a>)>,
    /// Alias variables (name, expression) from the view.
    alias_variables: Vec<(Ident<'a>, IrExpression<'a>)>,
    /// Local references collected from elements within the view.
    references: Vec<LocalRefInfo<'a>>,
    /// @let declarations collected from the view.
    let_declarations: Vec<LetDeclarationInfo<'a>>,
    /// Parent scope, if any.
    parent: Option<&'b Scope<'a, 'b>>,
}

/// Generates variable declarations for template contexts.
///
/// This phase analyzes each view's needs and creates Variable operations:
/// - Context variables when embedded views need parent component context
/// - Alias variables for computed @for expressions ($first, $last, $even, $odd)
/// - Context read variables for @for loop context ($implicit, $index, $count, etc.)
/// - ContextLetReferenceExpr for @let declarations accessed from nested views or callbacks
///
/// Following Angular's generate_variables.ts, this phase recursively processes views
/// with a scope chain, so nested views can access @let declarations from parent views.
pub fn generate_variables(job: &mut ComponentCompilationJob<'_>) {
    // Process views recursively starting from root, building scope chains
    recursively_process_view(job, job.root.xref, None);
}

/// Information about an operation that needs processing during view traversal.
/// This is a minimal representation collected in the first pass to guide the second pass.
enum ProcessAction {
    /// Recurse into a child view.
    RecurseChildView(XrefId),
    /// Generate variables for a listener's handler_ops.
    /// The usize is the index of the listener among all listeners in the view.
    GenerateListenerVars(usize),
    /// Process a repeater: recurse into child views, then generate trackByOps variables.
    ProcessRepeater {
        body_view: XrefId,
        empty_view: Option<XrefId>,
        has_track_by_ops: bool,
        repeater_xref: XrefId,
    },
}

/// Process a view and its child views recursively, building scope chains.
///
/// This corresponds to Angular's `recursivelyProcessView` function.
///
/// IMPORTANT: The order of processing within this function matches Angular's TypeScript
/// implementation exactly. Angular processes create ops in order, handling:
/// 1. Child view recursion immediately when encountering Template/Conditional/Repeater ops
/// 2. Listener variable generation immediately when encountering Listener ops
/// 3. Repeater track_by_ops variable generation immediately after processing repeater children
/// This order affects XrefId allocation and must be preserved for output compatibility.
fn recursively_process_view<'a, 'b>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    parent_scope: Option<&'b Scope<'a, 'b>>,
) {
    // Build the scope for this view
    let scope = get_scope_for_view(job, view_xref, parent_scope);

    // First pass: collect minimal info about what actions to take.
    // This preserves iteration order while avoiding borrow conflicts.
    let actions = collect_process_actions(job, view_xref);

    // Second pass: process each action in order, matching Angular's exact sequence.
    // This is critical for XrefId allocation order.
    for action in actions {
        match action {
            ProcessAction::RecurseChildView(child_xref) => {
                recursively_process_view(job, child_xref, Some(&scope));
            }
            ProcessAction::GenerateListenerVars(listener_index) => {
                // Generate variables for listener handler with is_callback=true
                let listener_vars =
                    generate_variables_in_scope_for_view(job, view_xref, &scope, true);
                if !listener_vars.is_empty() {
                    prepend_listener_variables(job, view_xref, listener_index, listener_vars);
                }
            }
            ProcessAction::ProcessRepeater {
                body_view,
                empty_view,
                has_track_by_ops,
                repeater_xref,
            } => {
                // Body view first - matches Angular generate_variables.ts lines 56-58
                // (NOT naming.ts which has a different order!)
                recursively_process_view(job, body_view, Some(&scope));
                // Then empty view (if present) - matches Angular generate_variables.ts lines 59-61
                if let Some(empty) = empty_view {
                    recursively_process_view(job, empty, Some(&scope));
                }
                // Then handle track_by_ops if present (matches Angular lines 62-64)
                if has_track_by_ops {
                    let track_vars =
                        generate_variables_in_scope_for_view(job, view_xref, &scope, false);
                    if !track_vars.is_empty() {
                        prepend_track_by_variables(job, view_xref, repeater_xref, track_vars);
                    }
                }
            }
        }
    }

    // Generate variables for the view's update phase (matches Angular line 76)
    let variables = generate_variables_in_scope_for_view(job, view_xref, &scope, false);
    prepend_variables_to_update(job, view_xref, variables);

    // Generate variables for arrow functions (matches Angular lines 78-82)
    // Arrow functions need variables with is_callback=true like listeners
    prepend_variables_to_arrow_functions(job, view_xref, &scope, parent_scope);
}

/// Collect process actions from a view's create ops in iteration order.
/// This extracts the minimal information needed to guide the second pass
/// while preserving the exact iteration order from Angular's TypeScript implementation.
///
/// The key difference from the old collect_ops_info is that repeaters are ALWAYS
/// represented as ProcessRepeater (not split into separate child view actions),
/// ensuring the exact sequence: body recurse -> empty recurse -> trackByOps vars.
fn collect_process_actions(
    job: &ComponentCompilationJob<'_>,
    view_xref: XrefId,
) -> Vec<ProcessAction> {
    let is_root = view_xref == job.root.xref;
    let view =
        if is_root { Some(&job.root) } else { job.views.get(&view_xref).map(|v| v.as_ref()) };

    let Some(view) = view else {
        return Vec::new();
    };

    let mut actions = Vec::new();
    let mut listener_count = 0usize;

    for op in view.create.iter() {
        match op {
            // ConditionalCreate, ConditionalBranchCreate, Template: recurse into child view
            // (matches Angular lines 45-50)
            CreateOp::Conditional(cond) => {
                if job.views.contains_key(&cond.xref) || cond.xref == job.root.xref {
                    actions.push(ProcessAction::RecurseChildView(cond.xref));
                }
            }
            CreateOp::ConditionalBranch(branch) => {
                if job.views.contains_key(&branch.xref) || branch.xref == job.root.xref {
                    actions.push(ProcessAction::RecurseChildView(branch.xref));
                }
            }
            CreateOp::Template(tmpl) => {
                if job.views.contains_key(&tmpl.embedded_view)
                    || tmpl.embedded_view == job.root.xref
                {
                    actions.push(ProcessAction::RecurseChildView(tmpl.embedded_view));
                }
            }
            // Projection with fallback: recurse into fallback view (matches Angular lines 51-55)
            CreateOp::Projection(proj) => {
                if let Some(fallback) = proj.fallback {
                    if job.views.contains_key(&fallback) || fallback == job.root.xref {
                        actions.push(ProcessAction::RecurseChildView(fallback));
                    }
                }
            }
            // RepeaterCreate: ALWAYS use ProcessRepeater to preserve exact sequence
            // (matches Angular lines 56-64)
            CreateOp::RepeaterCreate(repeater) => {
                let body_valid = job.views.contains_key(&repeater.body_view)
                    || repeater.body_view == job.root.xref;

                if body_valid {
                    let empty_valid = repeater
                        .empty_view
                        .map(|e| job.views.contains_key(&e) || e == job.root.xref)
                        .unwrap_or(false);

                    actions.push(ProcessAction::ProcessRepeater {
                        body_view: repeater.body_view,
                        empty_view: if empty_valid { repeater.empty_view } else { None },
                        has_track_by_ops: repeater.track_by_ops.is_some(),
                        repeater_xref: repeater.xref,
                    });
                }
            }
            // Listener, TwoWayListener, AnimationListener, Animation: generate handler variables
            // (matches Angular lines 66-72)
            CreateOp::Listener(_)
            | CreateOp::TwoWayListener(_)
            | CreateOp::AnimationListener(_)
            | CreateOp::Animation(_) => {
                actions.push(ProcessAction::GenerateListenerVars(listener_count));
                listener_count += 1;
            }
            _ => {}
        }
    }

    actions
}

/// Prepend variables to a listener's handler_ops (identified by listener index).
fn prepend_listener_variables<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    listener_index: usize,
    variables: Vec<UpdateOp<'a>>,
) {
    let is_root = view_xref == job.root.xref;
    let view = if is_root {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    let allocator = job.allocator;

    // Find the nth listener op and prepend variables
    let mut current_listener = 0usize;
    for op in view.create.iter_mut() {
        let handler_ops = match op {
            CreateOp::Listener(listener) => Some(&mut listener.handler_ops),
            CreateOp::TwoWayListener(listener) => Some(&mut listener.handler_ops),
            CreateOp::AnimationListener(listener) => Some(&mut listener.handler_ops),
            CreateOp::Animation(animation) => Some(&mut animation.handler_ops),
            _ => None,
        };

        if let Some(handler_ops) = handler_ops {
            if current_listener == listener_index {
                for var in variables.into_iter().rev() {
                    handler_ops.insert(0, clone_update_op(allocator, &var));
                }
                return; // Found and processed, we're done
            }
            current_listener += 1;
        }
    }
}

/// Prepend variables to a repeater's track_by_ops (identified by xref).
fn prepend_track_by_variables<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    repeater_xref: XrefId,
    variables: Vec<UpdateOp<'a>>,
) {
    let is_root = view_xref == job.root.xref;
    let view = if is_root {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    let allocator = job.allocator;

    // Find the repeater op by xref and prepend variables
    for op in view.create.iter_mut() {
        if let CreateOp::RepeaterCreate(repeater) = op {
            if repeater.xref == repeater_xref {
                if let Some(track_by_ops) = &mut repeater.track_by_ops {
                    for var in variables.into_iter().rev() {
                        track_by_ops.insert(0, clone_update_op(allocator, &var));
                    }
                }
                return; // Found and processed, we're done
            }
        }
    }
}

/// Build a scope for the given view.
///
/// This corresponds to Angular's `getScopeForView` function.
fn get_scope_for_view<'a, 'b>(
    job: &ComponentCompilationJob<'a>,
    view_xref: XrefId,
    parent: Option<&'b Scope<'a, 'b>>,
) -> Scope<'a, 'b> {
    let allocator = job.allocator;
    let is_root = view_xref == job.root.xref;
    let view =
        if is_root { Some(&job.root) } else { job.views.get(&view_xref).map(|v| v.as_ref()) };

    let (context_variables, alias_variables, references, let_declarations) =
        if let Some(view) = view {
            // Collect context variables
            let ctx_vars: Vec<_> =
                view.context_variables.iter().map(|v| (v.name.clone(), v.value.clone())).collect();

            // Collect alias variables
            let alias_vars: Vec<_> = view
                .aliases
                .iter()
                .map(|a| (a.identifier.clone(), a.expression.clone_in(allocator)))
                .collect();

            // Collect local references from create ops
            let refs = collect_local_refs_from_view(view);

            // Collect @let declarations from create ops
            let let_decls = collect_let_declarations_from_view(view);

            (ctx_vars, alias_vars, refs, let_decls)
        } else {
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };

    Scope {
        view: view_xref,
        context_variables,
        alias_variables,
        references,
        let_declarations,
        parent,
    }
}

/// Collect local references from a view's create operations.
fn collect_local_refs_from_view<'a>(
    view: &crate::pipeline::compilation::ViewCompilationUnit<'a>,
) -> Vec<LocalRefInfo<'a>> {
    let mut refs = Vec::new();
    for op in view.create.iter() {
        match op {
            CreateOp::ElementStart(el) => {
                for (offset, local_ref) in el.local_refs.iter().enumerate() {
                    refs.push(LocalRefInfo {
                        name: local_ref.name.clone(),
                        target_id: el.xref,
                        target_slot: el.slot,
                        offset: offset as i32,
                    });
                }
            }
            CreateOp::Element(el) => {
                for (offset, local_ref) in el.local_refs.iter().enumerate() {
                    refs.push(LocalRefInfo {
                        name: local_ref.name.clone(),
                        target_id: el.xref,
                        target_slot: el.slot,
                        offset: offset as i32,
                    });
                }
            }
            CreateOp::Template(tmpl) => {
                for (offset, local_ref) in tmpl.local_refs.iter().enumerate() {
                    refs.push(LocalRefInfo {
                        name: local_ref.name.clone(),
                        target_id: tmpl.xref,
                        target_slot: tmpl.slot,
                        offset: offset as i32,
                    });
                }
            }
            CreateOp::Conditional(cond) => {
                for (offset, local_ref) in cond.local_refs.iter().enumerate() {
                    refs.push(LocalRefInfo {
                        name: local_ref.name.clone(),
                        target_id: cond.xref,
                        target_slot: cond.slot,
                        offset: offset as i32,
                    });
                }
            }
            CreateOp::ConditionalBranch(branch) => {
                for (offset, local_ref) in branch.local_refs.iter().enumerate() {
                    refs.push(LocalRefInfo {
                        name: local_ref.name.clone(),
                        target_id: branch.xref,
                        target_slot: branch.slot,
                        offset: offset as i32,
                    });
                }
            }
            _ => {}
        }
    }
    refs
}

/// Collect @let declarations from a view's create operations.
fn collect_let_declarations_from_view<'a>(
    view: &crate::pipeline::compilation::ViewCompilationUnit<'a>,
) -> Vec<LetDeclarationInfo<'a>> {
    let mut let_decls = Vec::new();
    for op in view.create.iter() {
        if let CreateOp::DeclareLet(decl) = op {
            let_decls.push(LetDeclarationInfo {
                target_id: decl.xref,
                target_slot: decl.slot,
                variable_name: decl.name.clone(),
            });
        }
    }
    let_decls
}

/// Generate variable ops for all variables in scope for a view.
///
/// This corresponds to Angular's `generateVariablesInScopeForView` function.
fn generate_variables_in_scope_for_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    scope: &Scope<'a, '_>,
    is_callback: bool,
) -> Vec<UpdateOp<'a>> {
    let allocator = job.allocator;
    let mut new_ops: Vec<UpdateOp<'a>> = Vec::new();

    // If scope.view !== view.xref, we need a NextContext to navigate to parent
    if scope.view != view_xref {
        let xref = job.allocate_xref_id();
        new_ops.push(create_next_context_variable(allocator, xref, scope.view));
    }

    // Add context variables from this scope's view
    for (name, value) in &scope.context_variables {
        let xref = job.allocate_xref_id();
        new_ops.push(create_context_read_variable(
            allocator,
            xref,
            scope.view,
            name.clone(),
            value.clone(),
        ));
    }

    // Add alias variables
    for (name, expr) in &scope.alias_variables {
        let xref = job.allocate_xref_id();
        new_ops.push(create_alias_variable(
            allocator,
            xref,
            scope.view,
            name.clone(),
            expr.clone_in(allocator),
        ));
    }

    // Add local reference variables
    for local_ref in &scope.references {
        let xref = job.allocate_xref_id();
        new_ops.push(create_reference_variable(allocator, xref, scope.view, local_ref.clone()));
    }

    // Per Angular's generate_variables.ts lines 293-304:
    // If scope.view !== view.xref (cross-view) OR is_callback (listener handler),
    // create ContextLetReferenceExpr for @let declarations
    if scope.view != view_xref || is_callback {
        for decl in &scope.let_declarations {
            let xref = job.allocate_xref_id();
            new_ops.push(create_context_let_reference_variable(
                allocator,
                xref,
                decl.variable_name.clone(),
                decl.target_id,
                decl.target_slot,
            ));
        }
    }

    // Recursively add variables from parent scope
    if let Some(parent) = scope.parent {
        new_ops.extend(generate_variables_in_scope_for_view(job, view_xref, parent, false));
    }

    new_ops
}

/// Clone an UpdateOp (needed because we insert into multiple handlers).
fn clone_update_op<'a>(allocator: &'a oxc_allocator::Allocator, op: &UpdateOp<'a>) -> UpdateOp<'a> {
    match op {
        UpdateOp::Variable(var) => UpdateOp::Variable(UpdateVariableOp {
            // Create a fresh base since UpdateOpBase doesn't implement Clone
            // (it contains linked list pointers that shouldn't be copied)
            base: Default::default(),
            xref: var.xref,
            kind: var.kind,
            name: var.name.clone(),
            initializer: Box::new_in(var.initializer.clone_in(allocator), allocator),
            flags: var.flags,
            view: var.view,
            local: var.local,
        }),
        // For now, we only clone Variable ops since that's what we generate here.
        // This branch should never be reached in practice.
        _ => UpdateOp::Variable(UpdateVariableOp {
            base: Default::default(),
            xref: XrefId(0),
            kind: SemanticVariableKind::Identifier,
            name: Ident::from(""),
            initializer: Box::new_in(
                IrExpression::NextContext(Box::new_in(
                    NextContextExpr { steps: 0, source_span: None },
                    allocator,
                )),
                allocator,
            ),
            flags: VariableFlags::NONE,
            view: None,
            local: false,
        }),
    }
}

/// Prepend variables to a view's update phase.
fn prepend_variables_to_update<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    variables: Vec<UpdateOp<'a>>,
) {
    if variables.is_empty() {
        return;
    }

    let is_root = view_xref == job.root.xref;
    let view = if is_root {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    // Insert in reverse order since we push_front
    for var in variables.into_iter().rev() {
        view.update.push_front(var);
    }
}

/// Prepend variables to arrow functions in a view.
/// This matches Angular's lines 78-82 in generate_variables.ts:
/// ```typescript
/// for (const expr of view.functions) {
///   expr.ops.prepend(
///     generateVariablesInScopeForView(view, getScopeForView(view, parentScope), true),
///   );
/// }
/// ```
fn prepend_variables_to_arrow_functions<'a, 'b>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    scope: &Scope<'a, 'b>,
    _parent_scope: Option<&'b Scope<'a, 'b>>,
) {
    // Check if there are functions in the view
    let has_functions = {
        let is_root = view_xref == job.root.xref;
        let view =
            if is_root { Some(&job.root) } else { job.views.get(&view_xref).map(|v| v.as_ref()) };
        view.map(|v| !v.functions.is_empty()).unwrap_or(false)
    };

    if !has_functions {
        return;
    }

    // Generate variables for the arrow function context with is_callback=true
    // Angular creates a fresh scope for each arrow function, but since we're
    // iterating through all functions in the same view, they share the same scope.
    let variables = generate_variables_in_scope_for_view(job, view_xref, scope, true);

    if variables.is_empty() {
        return;
    }

    // Get mutable access to the view
    let allocator = job.allocator;
    let is_root = view_xref == job.root.xref;
    let view = if is_root {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    // Prepend variables to each arrow function's ops
    // We need to clone the variables for each function since they're consumed
    for func_ptr in view.functions.iter() {
        // SAFETY: These pointers are valid as they point to ArrowFunctionExpr
        // allocated in the allocator and stored in the view's functions vec.
        let func = unsafe { &mut **func_ptr };

        // Clone variables for this function
        for var in variables.iter().rev() {
            let cloned = clone_update_op(allocator, var);
            func.ops.insert(0, cloned);
        }
    }
}

/// Creates a NextContext variable for navigating to a parent view's context.
///
/// Per TypeScript's generate_variables.ts, the variable is created with `name: null`
/// so that the naming phase will assign it a proper unique name like `ctx_r0`.
fn create_next_context_variable<'a>(
    allocator: &'a oxc_allocator::Allocator,
    xref: XrefId,
    context_view: XrefId,
) -> UpdateOp<'a> {
    let initializer = IrExpression::NextContext(Box::new_in(
        NextContextExpr { steps: 1, source_span: None },
        allocator,
    ));

    UpdateOp::Variable(UpdateVariableOp {
        base: Default::default(),
        xref,
        kind: SemanticVariableKind::Context,
        name: Ident::from(""), // Empty = naming phase will assign it
        initializer: Box::new_in(initializer, allocator),
        flags: VariableFlags::NONE,
        view: Some(context_view),
        local: false,
    })
}

/// Creates a variable that reads a context variable (e.g., $index from @for loop).
///
/// Per Angular's generate_variables.ts (lines 253-268):
/// - Creates a `ContextExpr(scope.view)` as the receiver
/// - The `ContextExpr` is later resolved by `resolve_contexts` to either:
///   - `ctx` if it's the current view's context
///   - `ReadVariableExpr(var_xref)` if it's a parent view's context (accessed via nextContext)
/// - If value equals `CTX_REF`, use the context directly (no property access)
fn create_context_read_variable<'a>(
    allocator: &'a oxc_allocator::Allocator,
    xref: XrefId,
    view_xref: XrefId,
    name: Ident<'a>,
    context_value: Ident<'a>,
) -> UpdateOp<'a> {
    use crate::pipeline::compilation::CTX_REF;

    // Create a ContextExpr for the scope's view (not the current view!)
    // This will be resolved by resolve_contexts to either `ctx` or `ReadVariable(nextContext_var)`
    let context_expr = IrExpression::Context(Box::new_in(
        ContextExpr { view: view_xref, source_span: None },
        allocator,
    ));

    // Per Angular's generate_variables.ts line 257-258:
    // "We either read the context, or, if the variable is CTX_REF, use the context directly."
    let initializer = if context_value.as_str() == CTX_REF {
        // Use the context directly (e.g., for conditional expression aliases like `@if (alias = expr)`)
        context_expr
    } else {
        // Create a property read from the context: ctx.$implicit, ctx.$index, etc.
        IrExpression::ResolvedPropertyRead(Box::new_in(
            ResolvedPropertyReadExpr {
                receiver: Box::new_in(context_expr, allocator),
                name: context_value,
                source_span: None,
            },
            allocator,
        ))
    };

    UpdateOp::Variable(UpdateVariableOp {
        base: Default::default(),
        xref,
        kind: SemanticVariableKind::Identifier,
        name,
        initializer: Box::new_in(initializer, allocator),
        flags: VariableFlags::NONE,
        view: Some(view_xref),
        local: false,
    })
}

/// Creates an alias variable for computed @for loop context values.
fn create_alias_variable<'a>(
    allocator: &'a oxc_allocator::Allocator,
    xref: XrefId,
    view_xref: XrefId,
    name: Ident<'a>,
    expression: IrExpression<'a>,
) -> UpdateOp<'a> {
    UpdateOp::Variable(UpdateVariableOp {
        base: Default::default(),
        xref,
        kind: SemanticVariableKind::Alias,
        name,
        initializer: Box::new_in(expression, allocator),
        flags: VariableFlags::ALWAYS_INLINE,
        view: Some(view_xref),
        local: false,
    })
}

/// Creates a variable for a local reference (#myRef).
fn create_reference_variable<'a>(
    allocator: &'a oxc_allocator::Allocator,
    xref: XrefId,
    view_xref: XrefId,
    local_ref: LocalRefInfo<'a>,
) -> UpdateOp<'a> {
    let target_slot = match local_ref.target_slot {
        Some(slot) => SlotHandle::with_slot(slot),
        None => SlotHandle::new(),
    };

    let initializer = IrExpression::Reference(Box::new_in(
        ReferenceExpr {
            target: local_ref.target_id,
            target_slot,
            offset: local_ref.offset,
            source_span: None,
        },
        allocator,
    ));

    UpdateOp::Variable(UpdateVariableOp {
        base: Default::default(),
        xref,
        kind: SemanticVariableKind::Identifier,
        name: local_ref.name,
        initializer: Box::new_in(initializer, allocator),
        flags: VariableFlags::NONE,
        view: Some(view_xref),
        local: false,
    })
}

/// Creates a variable for a @let declaration accessed from a different view or callback.
///
/// This creates a ContextLetReferenceExpr which will be reified to ɵɵreadContextLet(slot).
fn create_context_let_reference_variable<'a>(
    allocator: &'a oxc_allocator::Allocator,
    xref: XrefId,
    name: Ident<'a>,
    target_id: XrefId,
    target_slot: Option<SlotId>,
) -> UpdateOp<'a> {
    let slot_handle = match target_slot {
        Some(slot) => SlotHandle::with_slot(slot),
        None => SlotHandle::new(),
    };

    let initializer = IrExpression::ContextLetReference(Box::new_in(
        ContextLetReferenceExpr { target: target_id, target_slot: slot_handle, source_span: None },
        allocator,
    ));

    UpdateOp::Variable(UpdateVariableOp {
        base: Default::default(),
        xref,
        kind: SemanticVariableKind::Identifier,
        name,
        initializer: Box::new_in(initializer, allocator),
        flags: VariableFlags::NONE,
        view: None,
        local: false,
    })
}

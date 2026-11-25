//! Resolve names phase.
//!
//! Resolves lexical references in views to either:
//! - A target variable (from a Variable operation in scope)
//! - A property read on the component context (for component properties)
//!
//! This phase also matches `RestoreViewExpr` expressions with their
//! corresponding saved views from parent creation lists.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_names.ts`.

use std::cell::RefCell;

use oxc_allocator::Box;
use oxc_diagnostics::OxcDiagnostic;
use oxc_str::Ident;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast::expression::AngularExpression;
use crate::ir::enums::SemanticVariableKind;
use crate::ir::expression::{
    ContextExpr, IrExpression, LexicalReadExpr, ReadVariableExpr, ResolvedCallExpr,
    ResolvedKeyedReadExpr, ResolvedPropertyReadExpr, ResolvedSafePropertyReadExpr,
    RestoreViewTarget, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op, visit_expressions_in_create_op,
    visit_expressions_in_update_op,
};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use crate::pipeline::expression_store::ExpressionStore;

/// Information about a saved view variable.
#[derive(Debug, Clone)]
struct SavedView {
    /// The view XrefId that was saved.
    view: XrefId,
    /// The XrefId of the variable holding the saved view.
    variable: XrefId,
}

/// Resolves identifier names to declarations.
///
/// This phase transforms:
/// - `LexicalReadExpr` → `ReadVariableExpr` (if name is in scope)
/// - `LexicalReadExpr` → `PropertyRead(ContextExpr, name)` (if name is not in scope)
/// - `RestoreViewExpr` → resolved to the saved view variable
/// - `ExpressionRef` → `ReadVariable` if stored expression is PropertyRead(ImplicitReceiver, name) in scope
pub fn resolve_names(job: &mut ComponentCompilationJob<'_>) {
    super::generate_arrow_functions::collect_arrow_functions(job);
    let root_xref = job.root.xref;
    let allocator = job.allocator;

    // We need to borrow expression_store immutably while modifying views
    // Collect expression store data we need first
    let expression_store_ptr = &job.expressions as *const ExpressionStore<'_>;

    // Process each view's create and update operation lists.
    // Per Angular's TypeScript (resolve_names.ts lines 25-26), create and update ops are
    // processed as SEPARATE lexical scopes — each builds its own scope/localDefinitions
    // from its own Variable ops. This is important because variables from update ops
    // (like @for loop items: $implicit, $index) should NOT be visible when resolving
    // expressions in create ops (like RepeaterCreate.track). The generateVariables phase
    // already prepends all necessary variables to each op list.
    for view in job.all_views_mut() {
        // SAFETY: We're only reading from expression_store, not modifying it
        let expressions = unsafe { &*expression_store_ptr };

        // Hoisted arrow functions are their own lexical scope, built from the variables
        // prepended to their ops (resolve_names.ts: `processLexicalScope(unit, expr.ops, null)`).
        for fn_ptr in view.functions.iter() {
            // SAFETY: The pointer was collected from this view's operations at the start
            // of this phase, and the allocator keeps the data alive.
            let arrow_fn = unsafe { &mut **fn_ptr };
            let scope = build_scope_from_handler_ops(arrow_fn.ops.iter());
            let fresh = FreshSlots::default();
            for op in arrow_fn.ops.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, _flags| {
                        resolve_in_visitor(
                            expr,
                            &fresh,
                            &scope,
                            root_xref,
                            None,
                            &allocator,
                            expressions,
                        );
                    },
                    VisitorContextFlag::NONE,
                );
            }
            resolve_expression(
                arrow_fn.body.as_mut(),
                &scope,
                root_xref,
                None,
                &allocator,
                expressions,
            );
        }

        // Process create ops with their own scope (no update scope merged in)
        process_lexical_scope_create(root_xref, &mut view.create, None, &allocator, expressions);
        process_lexical_scope_update(root_xref, &mut view.update, None, &allocator, expressions);
    }

    // Verify no LexicalRead expressions remain after resolution.
    // Following Angular's TypeScript implementation, we check all expressions in all ops.
    verify_no_lexical_reads_remain(job);
}

/// Scope maps for variable resolution.
///
/// Following Angular's TypeScript implementation, we use two maps:
/// - `scope` - for all variable definitions
/// - `local_definitions` - specifically for variables with `local: true` (@let declarations)
///
/// During resolution, `local_definitions` is checked FIRST, meaning @let declarations
/// take precedence over other variables with the same name.
#[derive(Default, Clone)]
struct ScopeMaps<'a> {
    scope: FxHashMap<Ident<'a>, XrefId>,
    local_definitions: FxHashMap<Ident<'a>, XrefId>,
}

impl<'a> ScopeMaps<'a> {
    /// Look up a name, checking local definitions first (they take precedence).
    fn get(&self, name: &Ident<'a>) -> Option<&XrefId> {
        self.local_definitions.get(name).or_else(|| self.scope.get(name))
    }
}

/// Build scope maps from listener handler_ops.
///
/// Per Angular's resolve_names.ts (line 86), handler ops are processed as a
/// completely separate lexical scope via recursive `processLexicalScope(unit, op.handlerOps, savedView)`.
/// The generateVariables phase already prepends all necessary variables to handler_ops,
/// so no merging from the parent view's scope is needed. This matches Angular's
/// behavior where each `processLexicalScope` call starts with fresh scope/localDefinitions maps.
fn build_scope_from_handler_ops<'a, 'b>(
    ops: impl Iterator<Item = &'b UpdateOp<'a>>,
) -> ScopeMaps<'a>
where
    'a: 'b,
{
    let mut maps = ScopeMaps::default();
    for op in ops {
        if let UpdateOp::Variable(var_op) = op {
            match var_op.kind {
                SemanticVariableKind::Identifier => {
                    if var_op.local {
                        if !maps.local_definitions.contains_key(&var_op.name) {
                            maps.local_definitions.insert(var_op.name.clone(), var_op.xref);
                        }
                    } else if !maps.scope.contains_key(&var_op.name) {
                        maps.scope.insert(var_op.name.clone(), var_op.xref);
                    }
                    if !maps.scope.contains_key(&var_op.name) {
                        maps.scope.insert(var_op.name.clone(), var_op.xref);
                    }
                }
                SemanticVariableKind::Alias => {
                    if !maps.scope.contains_key(&var_op.name) {
                        maps.scope.insert(var_op.name.clone(), var_op.xref);
                    }
                }
                _ => {}
            }
        }
    }
    maps
}

/// Process create operations to build scope and resolve names.
fn process_lexical_scope_create<'a>(
    root_xref: XrefId,
    ops: &mut crate::ir::list::CreateOpList<'a>,
    saved_view: Option<SavedView>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    // Maps variable names to their XrefIds
    // Per Angular's TypeScript, each processLexicalScope call starts with a fresh scope.
    let mut scope: ScopeMaps<'a> = ScopeMaps::default();

    // Track saved view for RestoreView expressions
    let mut current_saved_view = saved_view;

    // First pass: Build scope from Variable operations and recurse into listeners
    for op in ops.iter() {
        match op {
            CreateOp::Variable(var_op) => {
                match var_op.kind {
                    SemanticVariableKind::Identifier => {
                        // Check if this is a local variable (@let declaration)
                        if var_op.local {
                            if !scope.local_definitions.contains_key(&var_op.name) {
                                scope.local_definitions.insert(var_op.name.clone(), var_op.xref);
                            }
                        }
                        // Also add to scope for all identifiers
                        if !scope.scope.contains_key(&var_op.name) {
                            scope.scope.insert(var_op.name.clone(), var_op.xref);
                        }
                    }
                    SemanticVariableKind::Alias => {
                        // Add to scope if not already present
                        if !scope.scope.contains_key(&var_op.name) {
                            scope.scope.insert(var_op.name.clone(), var_op.xref);
                        }
                    }
                    SemanticVariableKind::SavedView => {
                        // This variable holds a snapshot of the current view
                        if let Some(saved_view_xref) = var_op.view {
                            current_saved_view =
                                Some(SavedView { view: saved_view_xref, variable: var_op.xref });
                        }
                    }
                    SemanticVariableKind::Context | SemanticVariableKind::BoundaryState => {
                        // Context variables are handled differently
                    }
                }
            }
            CreateOp::Listener(_) => {
                // Listeners have their own lexical scope for handler_ops.
                // Handler name resolution is done separately in the handler phase.
            }
            _ => {}
        }
    }

    // Second pass: Transform expressions
    let fresh = FreshSlots::default();
    for op in ops.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                // Per Angular's resolve_names.ts (line 86):
                //   processLexicalScope(unit, op.handlerOps, savedView);
                // Handler ops are processed as a SEPARATE lexical scope — a fresh
                // scope/localDefinitions is built from handler_ops variables only,
                // with NO merging from the parent view's scope. The generateVariables
                // phase already prepends all necessary variables to handler_ops.
                let handler_scope = build_scope_from_handler_ops(listener.handler_ops.iter());

                // Process listener handler_ops with the handler scope
                for handler_op in listener.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            resolve_in_visitor(
                                expr,
                                &fresh,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                &allocator,
                                expressions,
                            );
                        },
                        VisitorContextFlag::NONE,
                    );
                }
                // Also resolve handler_expression if present
                if let Some(handler_expr) = &mut listener.handler_expression {
                    resolve_expression(
                        handler_expr.as_mut(),
                        &handler_scope,
                        root_xref,
                        current_saved_view.as_ref(),
                        &allocator,
                        expressions,
                    );
                }
            }
            CreateOp::TwoWayListener(listener) => {
                // Per Angular's resolve_names.ts: handler ops get their own scope
                let handler_scope = build_scope_from_handler_ops(listener.handler_ops.iter());

                for handler_op in listener.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            resolve_in_visitor(
                                expr,
                                &fresh,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                &allocator,
                                expressions,
                            );
                        },
                        VisitorContextFlag::NONE,
                    );
                }
            }
            CreateOp::AnimationListener(listener) => {
                // Per Angular's resolve_names.ts: handler ops get their own scope
                let handler_scope = build_scope_from_handler_ops(listener.handler_ops.iter());

                for handler_op in listener.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            resolve_in_visitor(
                                expr,
                                &fresh,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                &allocator,
                                expressions,
                            );
                        },
                        VisitorContextFlag::NONE,
                    );
                }
            }
            CreateOp::Animation(animation) => {
                // Per Angular's resolve_names.ts: handler ops get their own scope
                let handler_scope = build_scope_from_handler_ops(animation.handler_ops.iter());

                for handler_op in animation.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            resolve_in_visitor(
                                expr,
                                &fresh,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                &allocator,
                                expressions,
                            );
                        },
                        VisitorContextFlag::NONE,
                    );
                }
            }
            _ => {
                // Note: RepeaterCreate falls through here. Angular's resolve_names.ts first
                // pass (lines 88-92) processes trackByOps with its own scope, but trackByOps
                // is always None at this phase (phase 31) — it's created later by
                // track_fn_optimization (phase 34). The second pass processes all ops
                // (including RepeaterCreate) via transformExpressionsInOp with the parent
                // scope, which is what transform_expressions_in_create_op does here.
                transform_expressions_in_create_op(
                    op,
                    &|expr, _flags| {
                        resolve_in_visitor(
                            expr,
                            &fresh,
                            &scope,
                            root_xref,
                            current_saved_view.as_ref(),
                            &allocator,
                            expressions,
                        );
                    },
                    VisitorContextFlag::NONE,
                );
            }
        }
    }
}

/// Process update operations to resolve names.
fn process_lexical_scope_update<'a>(
    root_xref: XrefId,
    ops: &mut crate::ir::list::UpdateOpList<'a>,
    saved_view: Option<SavedView>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    // Maps variable names to their XrefIds
    let mut scope: ScopeMaps<'a> = ScopeMaps::default();
    let mut current_saved_view = saved_view;

    // First pass: Build scope from Variable operations
    for op in ops.iter() {
        if let UpdateOp::Variable(var_op) = op {
            match var_op.kind {
                SemanticVariableKind::Identifier => {
                    // Check if this is a local variable (@let declaration)
                    if var_op.local {
                        if !scope.local_definitions.contains_key(&var_op.name) {
                            scope.local_definitions.insert(var_op.name.clone(), var_op.xref);
                        }
                    }
                    // Also add to scope for all identifiers
                    if !scope.scope.contains_key(&var_op.name) {
                        scope.scope.insert(var_op.name.clone(), var_op.xref);
                    }
                }
                SemanticVariableKind::Alias => {
                    if !scope.scope.contains_key(&var_op.name) {
                        scope.scope.insert(var_op.name.clone(), var_op.xref);
                    }
                }
                SemanticVariableKind::SavedView => {
                    if let Some(saved_view_xref) = var_op.view {
                        current_saved_view =
                            Some(SavedView { view: saved_view_xref, variable: var_op.xref });
                    }
                }
                SemanticVariableKind::Context | SemanticVariableKind::BoundaryState => {}
            }
        }
    }

    // Second pass: Transform expressions
    let fresh = FreshSlots::default();
    for op in ops.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                resolve_in_visitor(
                    expr,
                    &fresh,
                    &scope,
                    root_xref,
                    current_saved_view.as_ref(),
                    &allocator,
                    expressions,
                );
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Slots holding subtrees that the visitor callback built and whose parent has
/// not been resolved yet. See [`resolve_in_visitor`].
///
/// Keyed by the address of the slot. Slots do not move while one op list is
/// transformed.
type FreshSlots<'a> = RefCell<FxHashSet<*const IrExpression<'a>>>;

/// Visitor callback for `transform_expressions_in_*_op`.
///
/// The visitor is post-order, so the children of `expr` were already passed to
/// this callback. Running the recursive [`resolve_expression`] on `expr` again
/// would walk the whole subtree once per level (nodes x depth). This does the
/// same work once per node:
///
/// - a leaf (`LexicalRead`, `RestoreView`, `Ast`, `ExpressionRef`) is resolved in place;
/// - an `Ast`/`ExpressionRef` leaf that became a new subtree is remembered in `fresh`.
///   Before, that subtree was resolved once more when its parent was resolved (this
///   second pass turns kept `Ast($event)` children into context reads). So the parent
///   does that one pass here, only for the child slots `resolve_expression` descends
///   into. Further passes from higher ancestors did not change anything.
fn resolve_in_visitor<'a>(
    expr: &mut IrExpression<'a>,
    fresh: &FreshSlots<'a>,
    scope: &ScopeMaps<'a>,
    root_xref: XrefId,
    saved_view: Option<&SavedView>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    match resolve_leaf(expr, scope, root_xref, saved_view, allocator, expressions) {
        Some(true) => {
            fresh.borrow_mut().insert(std::ptr::from_ref(expr));
        }
        Some(false) => {}
        None => {
            if fresh.borrow().is_empty() {
                return;
            }
            for_each_resolvable_child(expr, &mut |child| {
                let was_fresh = fresh.borrow_mut().remove(&std::ptr::from_ref(child));
                if was_fresh {
                    resolve_expression(child, scope, root_xref, saved_view, allocator, expressions);
                }
            });
        }
    }
}

/// Resolve a single expression and, recursively, the children listed in
/// [`for_each_resolvable_child`].
///
/// This handles:
/// - `LexicalRead(name)` → `ReadVariable(xref)` if in scope
/// - `LexicalRead(name)` → stays as-is with context property access for component properties
/// - `RestoreView(Static(xref))` → `RestoreView(Dynamic(ReadVariable(saved_view_var)))`
/// - `Ast(PropertyRead(ImplicitReceiver, name))` → `ReadVariable` or `Context` access
/// - `ExpressionRef` → `ReadVariable` if stored expression is PropertyRead(ImplicitReceiver, name) in scope
///
/// A subtree built from an `Ast`/`ExpressionRef` leaf is not descended into here.
fn resolve_expression<'a>(
    expr: &mut IrExpression<'a>,
    scope: &ScopeMaps<'a>,
    root_xref: XrefId,
    saved_view: Option<&SavedView>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    if resolve_leaf(expr, scope, root_xref, saved_view, allocator, expressions).is_none() {
        for_each_resolvable_child(expr, &mut |child| {
            resolve_expression(child, scope, root_xref, saved_view, allocator, expressions);
        });
    }
}

/// Resolve `expr` in place if it is a leaf this phase rewrites.
///
/// Returns `None` if `expr` is not such a leaf, `Some(true)` if an `Ast` or
/// `ExpressionRef` leaf was replaced by a subtree from [`resolve_angular_expression`],
/// and `Some(false)` otherwise.
fn resolve_leaf<'a>(
    expr: &mut IrExpression<'a>,
    scope: &ScopeMaps<'a>,
    root_xref: XrefId,
    saved_view: Option<&SavedView>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) -> Option<bool> {
    match expr {
        IrExpression::LexicalRead(lexical) => {
            // Check scope
            if let Some(&xref) = scope.get(&lexical.name) {
                // Found in scope, convert to ReadVariable
                // Leave name as None so the naming phase can assign the proper suffixed name
                *expr = IrExpression::ReadVariable(Box::new_in(
                    ReadVariableExpr { xref, name: None, source_span: lexical.source_span },
                    &allocator,
                ));
            } else {
                // Not in scope - access from component context (root view)
                // $event is special - it's the event handler parameter, not a component property
                let is_event_param = lexical.name.as_str() == "$event";

                if !is_event_param {
                    // Create PropertyRead on root context - resolve_contexts will handle
                    // converting this to the appropriate variable reference in embedded views
                    *expr = IrExpression::ResolvedPropertyRead(Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver: Box::new_in(
                                IrExpression::Context(Box::new_in(
                                    ContextExpr { view: root_xref, source_span: None },
                                    &allocator,
                                )),
                                &allocator,
                            ),
                            name: lexical.name.clone(),
                            optional: false,
                            source_span: lexical.source_span,
                        },
                        &allocator,
                    ));
                }
            }
            // If not in scope and in root view, it stays as LexicalRead for now
            // The reify phase will convert it to ctx.propertyName
            Some(false)
        }

        IrExpression::RestoreView(restore) => {
            if let RestoreViewTarget::Static(view_xref) = restore.view {
                if let Some(saved) = saved_view {
                    if saved.view == view_xref {
                        // Replace with dynamic reference to saved view variable
                        // Leave name as None so the naming phase can assign the proper suffixed name
                        restore.view = RestoreViewTarget::Dynamic(Box::new_in(
                            IrExpression::ReadVariable(Box::new_in(
                                ReadVariableExpr {
                                    xref: saved.variable,
                                    name: None,
                                    source_span: None,
                                },
                                &allocator,
                            )),
                            &allocator,
                        ));
                    }
                }
            }
            Some(false)
        }

        IrExpression::Ast(ast_expr) => {
            // Try to resolve the AST expression tree recursively using the same logic
            // as ExpressionRef. This handles nested property reads like `todo.done`
            // where the receiver `todo` needs to be resolved to a variable.
            if let Some(resolved) =
                resolve_angular_expression(ast_expr.as_ref(), scope, root_xref, &allocator)
            {
                *expr = resolved;
                return Some(true);
            }

            // Handle PropertyRead(ImplicitReceiver, name) pattern for remaining cases
            // This handles simple lexical reads that weren't resolved above
            if let AngularExpression::PropertyRead(prop_read) = ast_expr.as_ref() {
                if matches!(prop_read.receiver, AngularExpression::ImplicitReceiver(_)) {
                    let name = prop_read.name.clone();
                    let source_span = Some(prop_read.source_span.to_span());

                    // Context properties ($implicit, $index, etc.) may or may not be in scope:
                    // - In listener handlers: generate_variables adds Variables for them,
                    //   so they SHOULD be resolved to ReadVariable
                    // - In update expressions: they're accessed via ctx.$index directly,
                    //   so they should NOT be resolved
                    //
                    // Check scope first - if in scope, resolve it
                    if let Some(&xref) = scope.get(&name) {
                        // Leave name as None so the naming phase can assign the proper suffixed name
                        *expr = IrExpression::ReadVariable(Box::new_in(
                            ReadVariableExpr { xref, name: None, source_span },
                            &allocator,
                        ));
                        return Some(false);
                    }

                    // Not in scope - check if it's a context property that should stay as-is
                    let is_context_property = matches!(
                        name.as_str(),
                        "$implicit" | "$index" | "$count" | "$first" | "$last" | "$even" | "$odd"
                    ) || name.as_str().starts_with("ɵ$");

                    if is_context_property {
                        // Context property not in scope - keep as Ast for now
                        // reify will handle it properly (e.g., ctx.$index)
                        return Some(false);
                    }

                    // Not a context property and not in scope - access from component context
                    // resolve_contexts will handle converting this to the appropriate
                    // variable reference (ctx in root view, ctx_r in embedded views)
                    *expr = IrExpression::ResolvedPropertyRead(Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver: Box::new_in(
                                IrExpression::Context(Box::new_in(
                                    ContextExpr { view: root_xref, source_span: None },
                                    &allocator,
                                )),
                                &allocator,
                            ),
                            name: name.clone(),
                            optional: false,
                            source_span,
                        },
                        &allocator,
                    ));
                    return Some(false);
                }
            }

            // Handle ImplicitReceiver by itself (rare but possible)
            if matches!(ast_expr.as_ref(), AngularExpression::ImplicitReceiver(_)) {
                *expr = IrExpression::Context(Box::new_in(
                    ContextExpr { view: root_xref, source_span: None },
                    &allocator,
                ));
            }
            Some(false)
        }

        IrExpression::ExpressionRef(id) => {
            // Look up the expression in the store and recursively resolve it
            let stored_expr = expressions.get(*id);

            // Try to resolve the expression tree recursively
            if let Some(resolved) =
                resolve_angular_expression(stored_expr, scope, root_xref, &allocator)
            {
                *expr = resolved;
                return Some(true);
            }
            // Otherwise, keep the ExpressionRef - it will be converted during reify
            Some(false)
        }

        _ => None,
    }
}

/// Call `f` on each child of `expr` that [`resolve_expression`] descends into.
///
/// These IR expressions may hold nested expressions that need resolution (many
/// are created by convert_ast_to_ir during ingest and may contain LexicalRead).
/// Other expression types are not descended into.
fn for_each_resolvable_child<'a>(
    expr: &mut IrExpression<'a>,
    f: &mut dyn FnMut(&mut IrExpression<'a>),
) {
    match expr {
        // ResetView wraps an expression. This is important because save_restore_view
        // wraps handler_expression in ResetView before resolve_names runs.
        IrExpression::ResetView(reset) => f(reset.expr.as_mut()),

        IrExpression::ResolvedCall(call) => {
            f(call.receiver.as_mut());
            for arg in &mut call.args {
                f(arg);
            }
        }

        IrExpression::ResolvedPropertyRead(prop) => f(prop.receiver.as_mut()),

        IrExpression::ResolvedKeyedRead(keyed) => {
            f(keyed.receiver.as_mut());
            f(keyed.key.as_mut());
        }

        IrExpression::ResolvedBinary(binary) => {
            f(binary.left.as_mut());
            f(binary.right.as_mut());
        }

        // Binary expressions may contain LexicalRead operands that need resolution.
        // This is critical for handler expressions like `activeOption = Option.A`
        // which are converted to IrExpression::Binary during ingest.
        IrExpression::Binary(binary) => {
            f(binary.lhs.as_mut());
            f(binary.rhs.as_mut());
        }

        IrExpression::ResolvedSafePropertyRead(safe) => f(safe.receiver.as_mut()),

        // Ternary expressions (condition ? true : false) need all sub-expressions resolved.
        // This is critical for handler expressions like `condition ? a : b` where
        // a and b may contain variable references.
        IrExpression::Ternary(ternary) => {
            f(ternary.condition.as_mut());
            f(ternary.true_expr.as_mut());
            f(ternary.false_expr.as_mut());
        }

        // Safe property read (obj?.prop) - resolve the receiver
        IrExpression::SafePropertyRead(safe) => f(safe.receiver.as_mut()),

        // Safe keyed read (obj?.[key]) - resolve receiver and key
        IrExpression::SafeKeyedRead(safe) => {
            f(safe.receiver.as_mut());
            f(safe.index.as_mut());
        }

        // Safe function invocation (fn?.()) - resolve receiver and arguments
        IrExpression::SafeInvokeFunction(safe) => {
            f(safe.receiver.as_mut());
            for arg in &mut safe.args {
                f(arg);
            }
        }

        // Safe ternary (expanded safe navigation) - resolve all parts
        IrExpression::SafeTernary(safe) => {
            f(safe.guard.as_mut());
            f(safe.expr.as_mut());
        }

        // Literal map (object literal) - resolve all values
        IrExpression::LiteralMap(map) => {
            for value in &mut map.values {
                f(value);
            }
        }

        // Derived literal map (object literal with some resolved values) - resolve all values
        IrExpression::DerivedLiteralMap(map) => {
            for value in &mut map.values {
                f(value);
            }
        }

        // Literal array - resolve all entries
        IrExpression::LiteralArray(arr) => {
            for elem in &mut arr.elements {
                f(elem);
            }
        }

        // Derived literal array - resolve all entries
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in &mut arr.entries {
                f(entry);
            }
        }

        // Not (!expr), unary (+expr / -expr), typeof, void and parentheses - resolve the operand
        IrExpression::Not(not) => f(not.expr.as_mut()),
        IrExpression::Unary(unary) => f(unary.expr.as_mut()),
        IrExpression::Typeof(typeof_expr) => f(typeof_expr.expr.as_mut()),
        IrExpression::Void(void_expr) => f(void_expr.expr.as_mut()),
        IrExpression::Parenthesized(paren) => f(paren.expr.as_mut()),

        // `$safeNavigationMigration(...)` wrapper - resolve the wrapped expression. A listener's
        // `handler_expression` and hoisted arrow-function bodies reach `resolve_expression`
        // directly, not through the generic visitor, so the wrapper must be descended here.
        IrExpression::SafeNavigationMigration(migration) => f(migration.expr.as_mut()),

        // ResolvedTemplateLiteral (created by ingest for template literals with inner expressions)
        // - resolve each inner expression so LexicalRead refs to @let vars and pipe args are resolved
        IrExpression::ResolvedTemplateLiteral(tl) => {
            for expr in &mut tl.expressions {
                f(expr);
            }
        }

        // Arrow function - resolve the names its body reads. Reads of its own parameters
        // were already turned into plain variables during ingestion.
        IrExpression::ArrowFunction(arrow_fn) => f(arrow_fn.body.as_mut()),

        // Other expression types don't need resolution
        _ => {}
    }
}

/// Recursively resolve an AngularExpression, converting PropertyRead(ImplicitReceiver, name)
/// to ReadVariable when the name is found in scope, or to Context property read when
/// the name is not in scope (component property access).
///
/// Returns `Some(IrExpression)` if the expression was resolved (i.e., contains at least one
/// variable reference that was resolved, or needs context access). Returns `None` if no
/// resolution was needed.
fn resolve_angular_expression<'a>(
    ast_expr: &AngularExpression<'a>,
    scope: &ScopeMaps<'a>,
    root_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) -> Option<IrExpression<'a>> {
    resolve_angular_expression_with_params(ast_expr, scope, root_xref, allocator, None)
}

/// Inner resolver that also tracks arrow function parameter names.
///
/// `param_names` is the shared parameter set of the enclosing arrow function's
/// `updateParameterReferences` run: a `PropertyRead(ImplicitReceiver, name)`
/// whose name is in the set becomes a plain `ReadVarExpr` instead of resolving
/// against scope or the component context. `None` outside arrow subtrees.
fn resolve_angular_expression_with_params<'a>(
    ast_expr: &AngularExpression<'a>,
    scope: &ScopeMaps<'a>,
    root_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
    param_names: Option<&std::cell::RefCell<rustc_hash::FxHashSet<Ident<'a>>>>,
) -> Option<IrExpression<'a>> {
    match ast_expr {
        AngularExpression::PropertyRead(prop_read) => {
            // Check if this is a PropertyRead(ImplicitReceiver, name) that we can resolve
            if matches!(prop_read.receiver, AngularExpression::ImplicitReceiver(_)) {
                let name = &prop_read.name;
                let source_span = Some(prop_read.source_span.to_span());

                // A read of an enclosing arrow function's own parameter becomes a plain
                // variable read, matching Angular's updateParameterReferences which turns
                // parameter LexicalReads into o.ReadVarExpr before name resolution.
                if let Some(names) = param_names {
                    if names.borrow().contains(name) {
                        return Some(IrExpression::OutputExpr(Box::new_in(
                            crate::output::ast::OutputExpression::ReadVar(Box::new_in(
                                crate::output::ast::ReadVarExpr { name: name.clone(), source_span },
                                &allocator,
                            )),
                            &allocator,
                        )));
                    }
                }

                // Context properties ($implicit, $index, etc.) and $event are special:
                // - $event is the event handler parameter, never resolved to a variable
                // - Context properties may or may not be in scope:
                //   - In listener handlers: generate_variables adds Variables for them,
                //     so they SHOULD be resolved to ReadVariable
                //   - In update expressions: they're accessed via ctx.$index directly,
                //     so they should NOT be resolved
                //
                // The key insight is to check scope FIRST: if the name is in scope,
                // resolve it regardless of whether it's a "context property".
                let is_event_param = name.as_str() == "$event";

                // Check scope first - if in scope, resolve it
                // This handles listener handlers where generate_variables adds
                // Variables for $index, $implicit, etc.
                if !is_event_param {
                    if let Some(&xref) = scope.get(name) {
                        // Found in scope - return ReadVariable
                        return Some(IrExpression::ReadVariable(Box::new_in(
                            ReadVariableExpr { xref, name: None, source_span },
                            &allocator,
                        )));
                    }
                }

                // Not in scope - check if it's a context property that should stay as-is
                let is_context_property = matches!(
                    name.as_str(),
                    "$implicit"
                        | "$index"
                        | "$count"
                        | "$first"
                        | "$last"
                        | "$even"
                        | "$odd"
                        | "$event"
                ) || name.as_str().starts_with("ɵ$");

                if is_context_property {
                    // Context property not in scope - keep as-is for later phases
                    // (e.g., update expressions access ctx.$index directly)
                    return None;
                }

                // Not a context property and not in scope - access from component context
                // resolve_contexts will handle converting this to the appropriate
                // variable reference (ctx in root view, ctx_r in embedded views)
                Some(IrExpression::ResolvedPropertyRead(Box::new_in(
                    ResolvedPropertyReadExpr {
                        receiver: Box::new_in(
                            IrExpression::Context(Box::new_in(
                                ContextExpr { view: root_xref, source_span: None },
                                &allocator,
                            )),
                            &allocator,
                        ),
                        name: name.clone(),
                        optional: false,
                        source_span,
                    },
                    &allocator,
                )))
            } else {
                // This is a nested property read like item.name
                // Try to resolve the receiver first
                if let Some(resolved_receiver) = resolve_angular_expression_with_params(
                    &prop_read.receiver,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    // Receiver was resolved - create ResolvedPropertyRead
                    Some(IrExpression::ResolvedPropertyRead(Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver: Box::new_in(resolved_receiver, &allocator),
                            name: prop_read.name.clone(),
                            optional: false,
                            source_span: Some(prop_read.source_span.to_span()),
                        },
                        &allocator,
                    )))
                } else {
                    // Receiver couldn't be resolved - keep as-is
                    None
                }
            }
        }

        AngularExpression::ImplicitReceiver(_) => {
            // ImplicitReceiver by itself becomes Context
            Some(IrExpression::Context(Box::new_in(
                ContextExpr { view: root_xref, source_span: None },
                &allocator,
            )))
        }

        AngularExpression::Call(call) => {
            // Resolve the receiver
            let resolved_receiver = resolve_angular_expression_with_params(
                &call.receiver,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            // Resolve each argument
            let mut resolved_args = oxc_allocator::Vec::new_in(&allocator);
            let mut any_arg_resolved = false;

            for arg in call.args.iter() {
                if let Some(resolved_arg) = resolve_angular_expression_with_params(
                    arg,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    resolved_args.push(resolved_arg);
                    any_arg_resolved = true;
                } else {
                    // Keep the original argument wrapped as Ast
                    resolved_args.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(arg, &allocator),
                        &allocator,
                    )));
                }
            }

            // If receiver or any argument was resolved, create a ResolvedCall
            if resolved_receiver.is_some() || any_arg_resolved {
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    // Keep original receiver wrapped as Ast
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&call.receiver, &allocator),
                        &allocator,
                    ))
                });

                Some(IrExpression::ResolvedCall(Box::new_in(
                    ResolvedCallExpr {
                        receiver: Box::new_in(receiver, &allocator),
                        args: resolved_args,
                        optional: false,
                        source_span: Some(call.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::KeyedRead(keyed) => {
            // Resolve receiver for keyed reads like item[0]
            let resolved_receiver = resolve_angular_expression_with_params(
                &keyed.receiver,
                scope,
                root_xref,
                allocator,
                param_names,
            );
            // Also try to resolve the key expression
            let resolved_key = resolve_angular_expression_with_params(
                &keyed.key,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            if resolved_receiver.is_some() || resolved_key.is_some() {
                // At least one part was resolved, create a ResolvedKeyedRead
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &keyed.receiver,
                            &allocator,
                        ),
                        &allocator,
                    ))
                });
                let key = resolved_key.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&keyed.key, &allocator),
                        &allocator,
                    ))
                });

                Some(IrExpression::ResolvedKeyedRead(Box::new_in(
                    ResolvedKeyedReadExpr {
                        receiver: Box::new_in(receiver, &allocator),
                        key: Box::new_in(key, &allocator),
                        optional: false,
                        source_span: Some(keyed.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::SafePropertyRead(safe) => {
            // Resolve receiver for safe property reads like item?.name
            if let Some(resolved_receiver) = resolve_angular_expression_with_params(
                &safe.receiver,
                scope,
                root_xref,
                allocator,
                param_names,
            ) {
                Some(IrExpression::ResolvedSafePropertyRead(Box::new_in(
                    ResolvedSafePropertyReadExpr {
                        receiver: Box::new_in(resolved_receiver, &allocator),
                        name: safe.name.clone(),
                        source_span: Some(safe.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::Binary(binary) => {
            // Handle binary expressions, especially assignments in event handlers
            // like `todo.done = $event`
            let resolved_left = resolve_angular_expression_with_params(
                &binary.left,
                scope,
                root_xref,
                allocator,
                param_names,
            );
            let resolved_right = resolve_angular_expression_with_params(
                &binary.right,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            if resolved_left.is_some() || resolved_right.is_some() {
                // At least one side was resolved, create a ResolvedBinary
                let left = resolved_left.unwrap_or_else(|| {
                    // Wrap original left expression as Ast
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&binary.left, &allocator),
                        &allocator,
                    ))
                });
                let right = resolved_right.unwrap_or_else(|| {
                    // Wrap original right expression as Ast
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&binary.right, &allocator),
                        &allocator,
                    ))
                });

                Some(IrExpression::ResolvedBinary(Box::new_in(
                    crate::ir::expression::ResolvedBinaryExpr {
                        operator: binary.operation,
                        left: Box::new_in(left, &allocator),
                        right: Box::new_in(right, &allocator),
                        source_span: Some(binary.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::ThisReceiver(_) => {
            // `this` in a template should resolve to the component context (root view).
            // This is similar to ImplicitReceiver but explicit.
            Some(IrExpression::Context(Box::new_in(
                ContextExpr { view: root_xref, source_span: None },
                &allocator,
            )))
        }

        AngularExpression::TemplateLiteral(tl) => {
            // Handle template literal expressions like `class {{ item.name }}`
            // Need to resolve any variable references in the expressions
            let mut resolved_exprs = oxc_allocator::Vec::new_in(&allocator);
            let mut any_resolved = false;

            for expr in tl.expressions.iter() {
                if let Some(resolved) = resolve_angular_expression_with_params(
                    expr,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    resolved_exprs.push(resolved);
                    any_resolved = true;
                } else {
                    // Keep the original expression wrapped as Ast
                    resolved_exprs.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(expr, &allocator),
                        &allocator,
                    )));
                }
            }

            if any_resolved {
                // Create a ResolvedTemplateLiteral with the resolved expressions
                let mut elements = oxc_allocator::Vec::new_in(&allocator);
                for elem in tl.elements.iter() {
                    elements.push(crate::ir::expression::IrTemplateLiteralElement {
                        text: elem.text.clone(),
                        source_span: Some(elem.source_span.to_span()),
                    });
                }

                Some(IrExpression::ResolvedTemplateLiteral(Box::new_in(
                    crate::ir::expression::ResolvedTemplateLiteralExpr {
                        elements,
                        expressions: resolved_exprs,
                        tagged: false,
                        source_span: Some(tl.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::Conditional(cond) => {
            // Handle conditional (ternary) expressions
            let resolved_condition = resolve_angular_expression_with_params(
                &cond.condition,
                scope,
                root_xref,
                allocator,
                param_names,
            );
            let resolved_true = resolve_angular_expression_with_params(
                &cond.true_exp,
                scope,
                root_xref,
                allocator,
                param_names,
            );
            let resolved_false = resolve_angular_expression_with_params(
                &cond.false_exp,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            if resolved_condition.is_some() || resolved_true.is_some() || resolved_false.is_some() {
                let condition = resolved_condition.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &cond.condition,
                            &allocator,
                        ),
                        &allocator,
                    ))
                });
                let true_expr = resolved_true.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&cond.true_exp, &allocator),
                        &allocator,
                    ))
                });
                let false_expr = resolved_false.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &cond.false_exp,
                            &allocator,
                        ),
                        &allocator,
                    ))
                });

                Some(IrExpression::Ternary(Box::new_in(
                    crate::ir::expression::TernaryExpr {
                        condition: Box::new_in(condition, &allocator),
                        true_expr: Box::new_in(true_expr, &allocator),
                        false_expr: Box::new_in(false_expr, &allocator),
                        source_span: Some(cond.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::LiteralMap(map) => {
            // Handle object literals - need to resolve variable references in values
            let mut resolved_values = oxc_allocator::Vec::new_in(&allocator);
            let mut any_resolved = false;

            for value in map.values.iter() {
                if let Some(resolved) = resolve_angular_expression_with_params(
                    value,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    resolved_values.push(resolved);
                    any_resolved = true;
                } else {
                    // Keep the original value wrapped as Ast
                    resolved_values.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(value, &allocator),
                        &allocator,
                    )));
                }
            }

            if any_resolved {
                use crate::ast::expression::LiteralMapKey;
                // Create a DerivedLiteralMap with the resolved values
                let mut keys = oxc_allocator::Vec::new_in(&allocator);
                let mut quoted = oxc_allocator::Vec::new_in(&allocator);
                let mut spreads = oxc_allocator::Vec::new_in(&allocator);
                for key in map.keys.iter() {
                    match key {
                        LiteralMapKey::Property(prop) => {
                            keys.push(prop.key.clone());
                            quoted.push(prop.quoted);
                            spreads.push(false);
                        }
                        LiteralMapKey::Spread(_) => {
                            keys.push(Ident::from(""));
                            quoted.push(false);
                            spreads.push(true);
                        }
                    }
                }

                Some(IrExpression::DerivedLiteralMap(Box::new_in(
                    crate::ir::expression::DerivedLiteralMapExpr {
                        keys,
                        values: resolved_values,
                        quoted,
                        spreads,
                        source_span: Some(map.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::LiteralArray(arr) => {
            // Handle array literals - need to resolve variable references in entries
            let mut resolved_entries = oxc_allocator::Vec::new_in(&allocator);
            let mut spreads = oxc_allocator::Vec::new_in(&allocator);
            let mut any_resolved = false;

            for entry in arr.expressions.iter() {
                let is_spread = matches!(entry, AngularExpression::SpreadElement(_));
                let inner = if let AngularExpression::SpreadElement(s) = entry {
                    &s.expression
                } else {
                    entry
                };
                if let Some(resolved) = resolve_angular_expression_with_params(
                    inner,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    resolved_entries.push(resolved);
                    any_resolved = true;
                } else {
                    resolved_entries.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(inner, &allocator),
                        &allocator,
                    )));
                }
                spreads.push(is_spread);
            }

            if any_resolved {
                Some(IrExpression::DerivedLiteralArray(Box::new_in(
                    crate::ir::expression::DerivedLiteralArrayExpr {
                        entries: resolved_entries,
                        spreads,
                        source_span: Some(arr.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::PrefixNot(prefix_not) => {
            // Handle prefix not expressions (!expr) - need to resolve variable references
            // in the operand. This is critical for expressions like `!bold` in listeners.
            if let Some(resolved) = resolve_angular_expression_with_params(
                &prefix_not.expression,
                scope,
                root_xref,
                allocator,
                param_names,
            ) {
                Some(IrExpression::Not(Box::new_in(
                    crate::ir::expression::NotExpr {
                        expr: Box::new_in(resolved, &allocator),
                        source_span: Some(prefix_not.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::Unary(unary) => {
            // Handle unary expressions (+expr or -expr) - need to resolve variable references
            if let Some(resolved) = resolve_angular_expression_with_params(
                &unary.expr,
                scope,
                root_xref,
                allocator,
                param_names,
            ) {
                Some(IrExpression::Unary(Box::new_in(
                    crate::ir::expression::UnaryExpr {
                        operator: match unary.operator {
                            crate::ast::expression::UnaryOperator::Plus => {
                                crate::ir::expression::IrUnaryOperator::Plus
                            }
                            crate::ast::expression::UnaryOperator::Minus => {
                                crate::ir::expression::IrUnaryOperator::Minus
                            }
                            crate::ast::expression::UnaryOperator::Increment => {
                                crate::ir::expression::IrUnaryOperator::Increment
                            }
                            crate::ast::expression::UnaryOperator::Decrement => {
                                crate::ir::expression::IrUnaryOperator::Decrement
                            }
                        },
                        expr: Box::new_in(resolved, &allocator),
                        source_span: Some(unary.source_span.to_span()),
                        is_prefix: unary.is_prefix,
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::TypeofExpression(typeof_expr) => {
            // Handle typeof expressions - need to resolve variable references
            if let Some(resolved) = resolve_angular_expression_with_params(
                &typeof_expr.expression,
                scope,
                root_xref,
                allocator,
                param_names,
            ) {
                Some(IrExpression::Typeof(Box::new_in(
                    crate::ir::expression::TypeofExpr {
                        expr: Box::new_in(resolved, &allocator),
                        source_span: Some(typeof_expr.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::VoidExpression(void_expr) => {
            // Handle void expressions - need to resolve variable references
            if let Some(resolved) = resolve_angular_expression_with_params(
                &void_expr.expression,
                scope,
                root_xref,
                allocator,
                param_names,
            ) {
                Some(IrExpression::Void(Box::new_in(
                    crate::ir::expression::VoidExpr {
                        expr: Box::new_in(resolved, &allocator),
                        source_span: Some(void_expr.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::NonNullAssert(nna) => {
            // Handle non-null assertion expressions (expr!) - need to resolve variable references
            // NonNullAssert expressions are wrapped in Ast since IrExpression doesn't have this variant
            resolve_angular_expression_with_params(
                &nna.expression,
                scope,
                root_xref,
                allocator,
                param_names,
            )
        }

        AngularExpression::ParenthesizedExpression(paren) => {
            // Handle parenthesized expressions - need to resolve variable references
            // within the inner expression
            resolve_angular_expression_with_params(
                &paren.expression,
                scope,
                root_xref,
                allocator,
                param_names,
            )
        }

        AngularExpression::Chain(chain) => {
            // Chains are not directly representable in IR. The reify path emits only
            // the last member of a stored chain, so resolve that member here.
            match chain.expressions.last() {
                Some(last) => resolve_angular_expression_with_params(
                    last,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ),
                None => None,
            }
        }

        AngularExpression::SafeCall(safe_call) => {
            // Handle safe function calls (fn?.()) - need to resolve receiver and arguments
            let resolved_receiver = resolve_angular_expression_with_params(
                &safe_call.receiver,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            let mut resolved_args = oxc_allocator::Vec::new_in(&allocator);
            let mut any_arg_resolved = false;

            for arg in safe_call.args.iter() {
                if let Some(resolved) = resolve_angular_expression_with_params(
                    arg,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    resolved_args.push(resolved);
                    any_arg_resolved = true;
                } else {
                    // Keep the original argument wrapped as Ast
                    resolved_args.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(arg, &allocator),
                        &allocator,
                    )));
                }
            }

            if resolved_receiver.is_some() || any_arg_resolved {
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &safe_call.receiver,
                            &allocator,
                        ),
                        &allocator,
                    ))
                });

                Some(IrExpression::SafeInvokeFunction(Box::new_in(
                    crate::ir::expression::SafeInvokeFunctionExpr {
                        receiver: Box::new_in(receiver, &allocator),
                        args: resolved_args,
                        source_span: Some(safe_call.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::SafeKeyedRead(safe_keyed) => {
            // Handle safe keyed read (obj?.[key]) - need to resolve receiver and key
            let resolved_receiver = resolve_angular_expression_with_params(
                &safe_keyed.receiver,
                scope,
                root_xref,
                allocator,
                param_names,
            );
            let resolved_key = resolve_angular_expression_with_params(
                &safe_keyed.key,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            if resolved_receiver.is_some() || resolved_key.is_some() {
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &safe_keyed.receiver,
                            &allocator,
                        ),
                        &allocator,
                    ))
                });
                let key = resolved_key.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &safe_keyed.key,
                            &allocator,
                        ),
                        &allocator,
                    ))
                });

                Some(IrExpression::SafeKeyedRead(Box::new_in(
                    crate::ir::expression::SafeKeyedReadExpr {
                        receiver: Box::new_in(receiver, &allocator),
                        index: Box::new_in(key, &allocator),
                        source_span: Some(safe_keyed.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::ArrowFunction(arrow) => {
            // Mirror Angular's updateParameterReferences: reads of the arrow's own
            // parameters become plain variable reads. The parameter set is shared
            // with the enclosing arrow's run (if any) and only grows: this arrow's
            // parameters are added after its body is resolved, matching the
            // post-order traversal in ingest.ts.
            let names = match param_names {
                Some(shared) => std::cell::RefCell::new(
                    shared.borrow().iter().cloned().collect::<rustc_hash::FxHashSet<Ident<'a>>>(),
                ),
                None => std::cell::RefCell::new(rustc_hash::FxHashSet::default()),
            };
            names.borrow_mut().extend(arrow.parameters.iter().map(|p| p.name.clone()));

            let body = resolve_angular_expression_with_params(
                &arrow.body,
                scope,
                root_xref,
                allocator,
                Some(&names),
            )
            .unwrap_or_else(|| {
                IrExpression::Ast(Box::new_in(
                    crate::ir::expression::clone_angular_expression(&arrow.body, &allocator),
                    &allocator,
                ))
            });

            if let Some(shared) = param_names {
                shared.borrow_mut().extend(names.borrow().iter().cloned());
            }

            let mut params = oxc_allocator::Vec::new_in(&allocator);
            for param in arrow.parameters.iter() {
                params.push(crate::output::ast::FnParam { name: param.name.clone() });
            }

            Some(IrExpression::ArrowFunction(Box::new_in(
                crate::ir::expression::ArrowFunctionExpr {
                    params,
                    body: Box::new_in(body, &allocator),
                    ops: oxc_allocator::Vec::new_in(&allocator),
                    var_offset: None,
                    hoisted: false,
                    source_span: Some(arrow.source_span.to_span()),
                },
                &allocator,
            )))
        }

        AngularExpression::TaggedTemplateLiteral(ttl) => {
            // Resolve the tag and the template's embedded expressions.
            let resolved_tag = resolve_angular_expression_with_params(
                &ttl.tag,
                scope,
                root_xref,
                allocator,
                param_names,
            );

            let mut resolved_exprs = oxc_allocator::Vec::new_in(&allocator);
            let mut any_resolved = resolved_tag.is_some();
            for expr in ttl.template.expressions.iter() {
                if let Some(resolved) = resolve_angular_expression_with_params(
                    expr,
                    scope,
                    root_xref,
                    allocator,
                    param_names,
                ) {
                    resolved_exprs.push(resolved);
                    any_resolved = true;
                } else {
                    resolved_exprs.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(expr, &allocator),
                        &allocator,
                    )));
                }
            }

            if any_resolved {
                let tag = resolved_tag.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&ttl.tag, &allocator),
                        &allocator,
                    ))
                });
                let mut elements = oxc_allocator::Vec::new_in(&allocator);
                for elem in ttl.template.elements.iter() {
                    elements.push(crate::ir::expression::IrTemplateLiteralElement {
                        text: elem.text.clone(),
                        source_span: Some(elem.source_span.to_span()),
                    });
                }
                Some(IrExpression::TaggedTemplateLiteral(Box::new_in(
                    crate::ir::expression::IrTaggedTemplateLiteralExpr {
                        tag: Box::new_in(tag, &allocator),
                        elements,
                        expressions: resolved_exprs,
                        source_span: Some(ttl.source_span.to_span()),
                    },
                    &allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::SpreadElement(spread) => {
            // Resolve the inner expression of a spread element.
            resolve_angular_expression_with_params(
                &spread.expression,
                scope,
                root_xref,
                allocator,
                param_names,
            )
            .map(|resolved| {
                IrExpression::SpreadElement(Box::new_in(
                    crate::ir::expression::IrSpreadElementExpr {
                        expr: Box::new_in(resolved, &allocator),
                        source_span: Some(spread.source_span.to_span()),
                    },
                    &allocator,
                ))
            })
        }

        // Other expression types don't need recursive resolution
        _ => None,
    }
}

/// Resolves names for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn resolve_names_for_host(job: &mut HostBindingCompilationJob<'_>) {
    // Upstream resolveNames iterates unit.functions for all job kinds; refresh
    // the pointer list before reading it.
    super::generate_arrow_functions::collect_arrow_functions_for_host(job);
    let allocator = job.allocator;
    let root_xref = job.root.xref;
    let expression_store_ptr = &job.expressions as *const ExpressionStore<'_>;

    // Hoisted arrow functions are their own lexical scope, built from the
    // variables prepended to their ops, same as in `resolve_names`.
    for fn_ptr in job.root.functions.iter() {
        // SAFETY: The pointer was collected from this unit's operations at the
        // start of this phase, and the allocator keeps the data alive.
        let arrow_fn = unsafe { &mut **fn_ptr };
        // SAFETY: We're only reading from expression_store, not modifying it
        let expressions = unsafe { &*expression_store_ptr };
        let scope = build_scope_from_handler_ops(arrow_fn.ops.iter());
        let fresh = FreshSlots::default();
        for op in arrow_fn.ops.iter_mut() {
            transform_expressions_in_update_op(
                op,
                &|expr, _flags| {
                    resolve_in_visitor(
                        expr,
                        &fresh,
                        &scope,
                        root_xref,
                        None,
                        &allocator,
                        expressions,
                    );
                },
                VisitorContextFlag::NONE,
            );
        }
        resolve_expression(
            arrow_fn.body.as_mut(),
            &scope,
            root_xref,
            None,
            &allocator,
            expressions,
        );
    }

    // SAFETY: We're only reading from expression_store, not modifying it
    let expressions = unsafe { &*expression_store_ptr };

    // Process create ops with their own scope (no update scope merged in)
    process_lexical_scope_create(root_xref, &mut job.root.create, None, &allocator, expressions);
    process_lexical_scope_update(root_xref, &mut job.root.update, None, &allocator, expressions);

    // Verify no LexicalRead expressions remain after resolution.
    verify_no_lexical_reads_remain_for_host(job);
}

/// Verifies that no `LexicalRead` expressions remain after name resolution.
///
/// Following Angular's TypeScript implementation (lines 139-147 of resolve_names.ts),
/// this walks all expressions in all ops and reports an error for each unresolved
/// `LexicalReadExpr` found.
fn verify_no_lexical_reads_remain(job: &mut ComponentCompilationJob<'_>) {
    use std::cell::RefCell;

    // Collect errors in a RefCell to allow mutation from within the Fn closure
    let errors: RefCell<Vec<OxcDiagnostic>> = RefCell::new(Vec::new());

    // `$event` inside listener handlers (IN_CHILD_OPERATION) legitimately stays
    // a LexicalRead until reify turns it into the handler parameter. Upstream's
    // resolveDollarEvent rewrites it to a ReadVarExpr before this check, so its
    // verifier never sees it; this port leaves the LexicalRead in place instead,
    // so it is exempt here.
    let is_exempt_event_param = |lexical: &LexicalReadExpr<'_>,
                                 flags: VisitorContextFlag|
     -> bool {
        flags.contains(VisitorContextFlag::IN_CHILD_OPERATION) && lexical.name.as_str() == "$event"
    };

    for view in job.all_views() {
        // Check create ops
        for op in view.create.iter() {
            visit_expressions_in_create_op(
                op,
                &|expr, flags| {
                    if let IrExpression::LexicalRead(lexical) = expr {
                        if is_exempt_event_param(lexical, flags) {
                            return;
                        }
                        errors.borrow_mut().push(
                            OxcDiagnostic::error(format!(
                                "AssertionError: no lexical reads should remain, but found read of {}",
                                lexical.name
                            ))
                            .with_label(lexical.source_span.unwrap_or_default()),
                        );
                    }
                },
                VisitorContextFlag::NONE,
            );
        }

        // Check update ops
        for op in view.update.iter() {
            visit_expressions_in_update_op(
                op,
                &|expr, flags| {
                    if let IrExpression::LexicalRead(lexical) = expr {
                        if is_exempt_event_param(lexical, flags) {
                            return;
                        }
                        errors.borrow_mut().push(
                            OxcDiagnostic::error(format!(
                                "AssertionError: no lexical reads should remain, but found read of {}",
                                lexical.name
                            ))
                            .with_label(lexical.source_span.unwrap_or_default()),
                        );
                    }
                },
                VisitorContextFlag::NONE,
            );
        }
    }

    // Add all collected errors to job diagnostics
    job.diagnostics.extend(errors.into_inner());
}

/// Verifies that no `LexicalRead` expressions remain after name resolution for host bindings.
fn verify_no_lexical_reads_remain_for_host(job: &mut HostBindingCompilationJob<'_>) {
    use std::cell::RefCell;

    // Collect errors in a RefCell to allow mutation from within the Fn closure
    let errors: RefCell<Vec<OxcDiagnostic>> = RefCell::new(Vec::new());

    // Check create ops
    for op in job.root.create.iter() {
        visit_expressions_in_create_op(
            op,
            &|expr, _flags| {
                if let IrExpression::LexicalRead(lexical) = expr {
                    errors.borrow_mut().push(
                        OxcDiagnostic::error(format!(
                            "AssertionError: no lexical reads should remain, but found read of {}",
                            lexical.name
                        ))
                        .with_label(lexical.source_span.unwrap_or_default()),
                    );
                }
            },
            VisitorContextFlag::NONE,
        );
    }

    // Check update ops
    for op in job.root.update.iter() {
        visit_expressions_in_update_op(
            op,
            &|expr, _flags| {
                if let IrExpression::LexicalRead(lexical) = expr {
                    errors.borrow_mut().push(
                        OxcDiagnostic::error(format!(
                            "AssertionError: no lexical reads should remain, but found read of {}",
                            lexical.name
                        ))
                        .with_label(lexical.source_span.unwrap_or_default()),
                    );
                }
            },
            VisitorContextFlag::NONE,
        );
    }

    // Add all collected errors to job diagnostics
    job.diagnostics.extend(errors.into_inner());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::expression::{
        BinaryExpr, IrBinaryOperator, NotExpr, PipeBindingExpr, SafeInvokeFunctionExpr, SlotHandle,
        transform_expressions_in_expression,
    };
    use crate::parser::expression::Parser;
    use oxc_allocator::{Allocator, Vec};

    fn lexical<'a>(allocator: &'a Allocator, name: &'a str) -> IrExpression<'a> {
        IrExpression::LexicalRead(Box::new_in(
            LexicalReadExpr { name: Ident::from(name), source_span: None },
            &allocator,
        ))
    }

    fn stored<'a>(
        allocator: &'a Allocator,
        store: &mut ExpressionStore<'a>,
        source: &'a str,
    ) -> IrExpression<'a> {
        let ast = Parser::new(allocator, source).parse_action().ast;
        IrExpression::ExpressionRef(store.store(ast))
    }

    fn plus<'a>(
        allocator: &'a Allocator,
        lhs: IrExpression<'a>,
        rhs: IrExpression<'a>,
    ) -> IrExpression<'a> {
        IrExpression::Binary(Box::new_in(
            BinaryExpr {
                operator: IrBinaryOperator::Plus,
                lhs: Box::new_in(lhs, &allocator),
                rhs: Box::new_in(rhs, &allocator),
                source_span: None,
            },
            &allocator,
        ))
    }

    /// A long `+` chain whose leaves are lexical reads and stored expressions,
    /// some under parents `resolve_expression` descends into (`Binary`, `Not`,
    /// safe calls) and some under a parent it does not descend into (`PipeBinding`).
    fn build<'a>(allocator: &'a Allocator, store: &mut ExpressionStore<'a>) -> IrExpression<'a> {
        let mut expr = lexical(allocator, "x");
        for i in 0..20 {
            let leaf = match i % 5 {
                0 => stored(allocator, store, "f($event, x, y)"),
                1 => lexical(allocator, if i % 2 == 0 { "x" } else { "y" }),
                2 => {
                    let mut args = Vec::new_in(&allocator);
                    args.push(stored(allocator, store, "g($event)"));
                    IrExpression::PipeBinding(Box::new_in(
                        PipeBindingExpr {
                            target: XrefId::new(100 + i),
                            target_slot: SlotHandle::new(),
                            name: Ident::from("json"),
                            args,
                            var_offset: None,
                            source_span: None,
                        },
                        &allocator,
                    ))
                }
                3 => IrExpression::Not(Box::new_in(
                    NotExpr {
                        expr: Box::new_in(stored(allocator, store, "[$event, x]"), &allocator),
                        source_span: None,
                    },
                    &allocator,
                )),
                _ => {
                    let mut args = Vec::new_in(&allocator);
                    args.push(stored(allocator, store, "h($event)"));
                    args.push(lexical(allocator, "$event"));
                    IrExpression::SafeInvokeFunction(Box::new_in(
                        SafeInvokeFunctionExpr {
                            receiver: Box::new_in(stored(allocator, store, "k"), &allocator),
                            args,
                            source_span: None,
                        },
                        &allocator,
                    ))
                }
            };
            expr = plus(allocator, expr, leaf);
        }
        expr
    }

    #[derive(Clone, Copy)]
    enum Callback {
        /// The old callback: the recursive `resolve_expression` at every node.
        Recursive,
        /// The new callback.
        Visitor,
        /// Leaves only, without the extra pass for new subtrees.
        LeavesOnly,
    }

    /// Runs the post-order visitor over `build()` and returns the result as text.
    fn run(callback: Callback) -> String {
        let allocator = Allocator::default();
        let mut store = ExpressionStore::new(&allocator);
        let mut expr = build(&allocator, &mut store);
        let mut scope = ScopeMaps::default();
        scope.scope.insert(Ident::from("x"), XrefId::new(7));
        scope.local_definitions.insert(Ident::from("y"), XrefId::new(8));
        let root = XrefId::new(0);
        let fresh = FreshSlots::default();
        transform_expressions_in_expression(
            &mut expr,
            &|e, _flags| match callback {
                Callback::Recursive => {
                    resolve_expression(e, &scope, root, None, &allocator, &store);
                }
                Callback::Visitor => {
                    resolve_in_visitor(e, &fresh, &scope, root, None, &allocator, &store);
                }
                Callback::LeavesOnly => {
                    resolve_leaf(e, &scope, root, None, &allocator, &store);
                }
            },
            VisitorContextFlag::NONE,
        );
        format!("{expr:?}")
    }

    /// The visitor callback gives the same tree as the old callback, which ran the
    /// recursive `resolve_expression` again at every node (nodes x depth).
    #[test]
    fn visitor_callback_matches_recursive_resolution() {
        let recursive = run(Callback::Recursive);
        assert_eq!(run(Callback::Visitor), recursive);

        // Resolving only the leaves is not the same: subtrees built from stored
        // expressions under a resolving parent get one more pass.
        assert_ne!(run(Callback::LeavesOnly), recursive);
    }
}

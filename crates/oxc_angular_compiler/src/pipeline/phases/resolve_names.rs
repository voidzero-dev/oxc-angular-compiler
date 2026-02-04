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

use oxc_allocator::Box;
use oxc_diagnostics::OxcDiagnostic;
use oxc_span::Atom;
use rustc_hash::FxHashMap;

use crate::ast::expression::AngularExpression;
use crate::ir::enums::SemanticVariableKind;
use crate::ir::expression::{
    ContextExpr, IrExpression, ReadVariableExpr, ResolvedCallExpr, ResolvedKeyedReadExpr,
    ResolvedPropertyReadExpr, ResolvedSafePropertyReadExpr, RestoreViewTarget, VisitorContextFlag,
    transform_expressions_in_create_op, transform_expressions_in_update_op,
    visit_expressions_in_create_op, visit_expressions_in_update_op,
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
    let root_xref = job.root.xref;
    let allocator = job.allocator;

    // We need to borrow expression_store immutably while modifying views
    // Collect expression store data we need first
    let expression_store_ptr = &job.expressions as *const ExpressionStore<'_>;

    // Process each view's create and update operation lists
    // IMPORTANT: We need to build scope from BOTH create and update ops before processing listeners,
    // because listener handler expressions need access to variables from both phases.
    for view in job.all_views_mut() {
        // SAFETY: We're only reading from expression_store, not modifying it
        let expressions = unsafe { &*expression_store_ptr };

        // Build scope from update ops first (contains context variables like $implicit, $index, etc.)
        let update_scope = build_scope_from_update_ops(&view.update);

        // Process create ops with the update scope available for listener handler expressions
        process_lexical_scope_create(
            root_xref,
            &mut view.create,
            None,
            &update_scope,
            allocator,
            expressions,
        );
        process_lexical_scope_update(root_xref, &mut view.update, None, allocator, expressions);
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
    scope: FxHashMap<Atom<'a>, XrefId>,
    local_definitions: FxHashMap<Atom<'a>, XrefId>,
}

impl<'a> ScopeMaps<'a> {
    /// Look up a name, checking local definitions first (they take precedence).
    fn get(&self, name: &Atom<'a>) -> Option<&XrefId> {
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

/// Build scope maps from update operations (for variables like context reads).
fn build_scope_from_update_ops<'a>(ops: &crate::ir::list::UpdateOpList<'a>) -> ScopeMaps<'a> {
    let mut maps = ScopeMaps::default();
    for op in ops.iter() {
        if let UpdateOp::Variable(var_op) = op {
            match var_op.kind {
                SemanticVariableKind::Identifier => {
                    // Check if this is a local variable (@let declaration)
                    if var_op.local {
                        if !maps.local_definitions.contains_key(&var_op.name) {
                            maps.local_definitions.insert(var_op.name.clone(), var_op.xref);
                        }
                    } else if !maps.scope.contains_key(&var_op.name) {
                        maps.scope.insert(var_op.name.clone(), var_op.xref);
                    }
                    // Also add to scope for non-local (always)
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
    update_scope: &ScopeMaps<'a>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    // Maps variable names to their XrefIds
    // Start with update_scope to include context variables (like @for loop items)
    let mut scope: ScopeMaps<'a> = update_scope.clone();

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
                    SemanticVariableKind::Context => {
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
                            resolve_expression(
                                expr,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                allocator,
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
                        allocator,
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
                            resolve_expression(
                                expr,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                allocator,
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
                            resolve_expression(
                                expr,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                allocator,
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
                            resolve_expression(
                                expr,
                                &handler_scope,
                                root_xref,
                                current_saved_view.as_ref(),
                                allocator,
                                expressions,
                            );
                        },
                        VisitorContextFlag::NONE,
                    );
                }
            }
            _ => {
                transform_expressions_in_create_op(
                    op,
                    &|expr, _flags| {
                        resolve_expression(
                            expr,
                            &scope,
                            root_xref,
                            current_saved_view.as_ref(),
                            allocator,
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
                SemanticVariableKind::Context => {}
            }
        }
    }

    // Second pass: Transform expressions
    for op in ops.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                resolve_expression(
                    expr,
                    &scope,
                    root_xref,
                    current_saved_view.as_ref(),
                    allocator,
                    expressions,
                );
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Resolve a single expression.
///
/// This handles:
/// - `LexicalRead(name)` → `ReadVariable(xref)` if in scope
/// - `LexicalRead(name)` → stays as-is with context property access for component properties
/// - `RestoreView(Static(xref))` → `RestoreView(Dynamic(ReadVariable(saved_view_var)))`
/// - `Ast(PropertyRead(ImplicitReceiver, name))` → `ReadVariable` or `Context` access
/// - `ExpressionRef` → `ReadVariable` if stored expression is PropertyRead(ImplicitReceiver, name) in scope
fn resolve_expression<'a>(
    expr: &mut IrExpression<'a>,
    scope: &ScopeMaps<'a>,
    root_xref: XrefId,
    saved_view: Option<&SavedView>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    match expr {
        IrExpression::LexicalRead(lexical) => {
            // Check scope
            if let Some(&xref) = scope.get(&lexical.name) {
                // Found in scope, convert to ReadVariable
                // Leave name as None so the naming phase can assign the proper suffixed name
                *expr = IrExpression::ReadVariable(Box::new_in(
                    ReadVariableExpr { xref, name: None, source_span: lexical.source_span },
                    allocator,
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
                                    allocator,
                                )),
                                allocator,
                            ),
                            name: lexical.name.clone(),
                            source_span: lexical.source_span,
                        },
                        allocator,
                    ));
                }
            }
            // If not in scope and in root view, it stays as LexicalRead for now
            // The reify phase will convert it to ctx.propertyName
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
                                allocator,
                            )),
                            allocator,
                        ));
                    }
                }
            }
        }

        IrExpression::Ast(ast_expr) => {
            // Try to resolve the AST expression tree recursively using the same logic
            // as ExpressionRef. This handles nested property reads like `todo.done`
            // where the receiver `todo` needs to be resolved to a variable.
            if let Some(resolved) =
                resolve_angular_expression(ast_expr.as_ref(), scope, root_xref, allocator)
            {
                *expr = resolved;
                return;
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
                            allocator,
                        ));
                        return;
                    }

                    // Not in scope - check if it's a context property that should stay as-is
                    let is_context_property = matches!(
                        name.as_str(),
                        "$implicit" | "$index" | "$count" | "$first" | "$last" | "$even" | "$odd"
                    ) || name.as_str().starts_with("ɵ$");

                    if is_context_property {
                        // Context property not in scope - keep as Ast for now
                        // reify will handle it properly (e.g., ctx.$index)
                        return;
                    }

                    // Not a context property and not in scope - access from component context
                    // resolve_contexts will handle converting this to the appropriate
                    // variable reference (ctx in root view, ctx_r in embedded views)
                    *expr = IrExpression::ResolvedPropertyRead(Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver: Box::new_in(
                                IrExpression::Context(Box::new_in(
                                    ContextExpr { view: root_xref, source_span: None },
                                    allocator,
                                )),
                                allocator,
                            ),
                            name: name.clone(),
                            source_span,
                        },
                        allocator,
                    ));
                    return;
                }
            }

            // Handle ImplicitReceiver by itself (rare but possible)
            if matches!(ast_expr.as_ref(), AngularExpression::ImplicitReceiver(_)) {
                *expr = IrExpression::Context(Box::new_in(
                    ContextExpr { view: root_xref, source_span: None },
                    allocator,
                ));
            }
        }

        IrExpression::ExpressionRef(id) => {
            // Look up the expression in the store and recursively resolve it
            let stored_expr = expressions.get(*id);

            // Try to resolve the expression tree recursively
            if let Some(resolved) =
                resolve_angular_expression(stored_expr, scope, root_xref, allocator)
            {
                *expr = resolved;
            }
            // Otherwise, keep the ExpressionRef - it will be converted during reify
        }

        IrExpression::ResetView(reset) => {
            // ResetView wraps an expression - recursively resolve it
            // This is important because save_restore_view wraps handler_expression in ResetView
            // before resolve_names runs
            resolve_expression(
                reset.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Handle IR expressions that may contain nested expressions needing resolution.
        // These are created by convert_ast_to_ir during ingest and may contain LexicalRead
        // or other expressions that need resolution.
        IrExpression::ResolvedCall(call) => {
            resolve_expression(
                call.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            for arg in call.args.iter_mut() {
                resolve_expression(arg, scope, root_xref, saved_view, allocator, expressions);
            }
        }

        IrExpression::ResolvedPropertyRead(prop) => {
            resolve_expression(
                prop.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        IrExpression::ResolvedKeyedRead(keyed) => {
            resolve_expression(
                keyed.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                keyed.key.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        IrExpression::ResolvedBinary(binary) => {
            resolve_expression(
                binary.left.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                binary.right.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Binary expressions may contain LexicalRead operands that need resolution.
        // This is critical for handler expressions like `activeOption = Option.A`
        // which are converted to IrExpression::Binary during ingest.
        IrExpression::Binary(binary) => {
            resolve_expression(
                binary.lhs.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                binary.rhs.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        IrExpression::ResolvedSafePropertyRead(safe) => {
            resolve_expression(
                safe.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Ternary expressions (condition ? true : false) need all sub-expressions resolved.
        // This is critical for handler expressions like `condition ? a : b` where
        // a and b may contain variable references.
        IrExpression::Ternary(ternary) => {
            resolve_expression(
                ternary.condition.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                ternary.true_expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                ternary.false_expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Safe property read (obj?.prop) - resolve the receiver
        IrExpression::SafePropertyRead(safe) => {
            resolve_expression(
                safe.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Safe keyed read (obj?.[key]) - resolve receiver and key
        IrExpression::SafeKeyedRead(safe) => {
            resolve_expression(
                safe.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                safe.index.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Safe function invocation (fn?.()) - resolve receiver and arguments
        IrExpression::SafeInvokeFunction(safe) => {
            resolve_expression(
                safe.receiver.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            for arg in safe.args.iter_mut() {
                resolve_expression(arg, scope, root_xref, saved_view, allocator, expressions);
            }
        }

        // Safe ternary (expanded safe navigation) - resolve all parts
        IrExpression::SafeTernary(safe) => {
            resolve_expression(
                safe.guard.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
            resolve_expression(
                safe.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Literal map (object literal) - resolve all values
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter_mut() {
                resolve_expression(value, scope, root_xref, saved_view, allocator, expressions);
            }
        }

        // Derived literal map (object literal with some resolved values) - resolve all values
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter_mut() {
                resolve_expression(value, scope, root_xref, saved_view, allocator, expressions);
            }
        }

        // Literal array - resolve all entries
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter_mut() {
                resolve_expression(elem, scope, root_xref, saved_view, allocator, expressions);
            }
        }

        // Derived literal array - resolve all entries
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter_mut() {
                resolve_expression(entry, scope, root_xref, saved_view, allocator, expressions);
            }
        }

        // Not expression (!expr) - resolve the operand
        IrExpression::Not(not) => {
            resolve_expression(
                not.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Unary expression (+expr or -expr) - resolve the operand
        IrExpression::Unary(unary) => {
            resolve_expression(
                unary.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Typeof expression (typeof expr) - resolve the operand
        IrExpression::Typeof(typeof_expr) => {
            resolve_expression(
                typeof_expr.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Void expression (void expr) - resolve the operand
        IrExpression::Void(void_expr) => {
            resolve_expression(
                void_expr.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

        // Parenthesized expression - resolve the inner expression
        IrExpression::Parenthesized(paren) => {
            resolve_expression(
                paren.expr.as_mut(),
                scope,
                root_xref,
                saved_view,
                allocator,
                expressions,
            );
        }

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
    match ast_expr {
        AngularExpression::PropertyRead(prop_read) => {
            // Check if this is a PropertyRead(ImplicitReceiver, name) that we can resolve
            if matches!(prop_read.receiver, AngularExpression::ImplicitReceiver(_)) {
                let name = &prop_read.name;
                let source_span = Some(prop_read.source_span.to_span());

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
                            allocator,
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
                                allocator,
                            )),
                            allocator,
                        ),
                        name: name.clone(),
                        source_span,
                    },
                    allocator,
                )))
            } else {
                // This is a nested property read like item.name
                // Try to resolve the receiver first
                if let Some(resolved_receiver) =
                    resolve_angular_expression(&prop_read.receiver, scope, root_xref, allocator)
                {
                    // Receiver was resolved - create ResolvedPropertyRead
                    Some(IrExpression::ResolvedPropertyRead(Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver: Box::new_in(resolved_receiver, allocator),
                            name: prop_read.name.clone(),
                            source_span: Some(prop_read.source_span.to_span()),
                        },
                        allocator,
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
                allocator,
            )))
        }

        AngularExpression::Call(call) => {
            // Resolve the receiver
            let resolved_receiver =
                resolve_angular_expression(&call.receiver, scope, root_xref, allocator);

            // Resolve each argument
            let mut resolved_args = oxc_allocator::Vec::new_in(allocator);
            let mut any_arg_resolved = false;

            for arg in call.args.iter() {
                if let Some(resolved_arg) =
                    resolve_angular_expression(arg, scope, root_xref, allocator)
                {
                    resolved_args.push(resolved_arg);
                    any_arg_resolved = true;
                } else {
                    // Keep the original argument wrapped as Ast
                    resolved_args.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(arg, allocator),
                        allocator,
                    )));
                }
            }

            // If receiver or any argument was resolved, create a ResolvedCall
            if resolved_receiver.is_some() || any_arg_resolved {
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    // Keep original receiver wrapped as Ast
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&call.receiver, allocator),
                        allocator,
                    ))
                });

                Some(IrExpression::ResolvedCall(Box::new_in(
                    ResolvedCallExpr {
                        receiver: Box::new_in(receiver, allocator),
                        args: resolved_args,
                        source_span: Some(call.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::KeyedRead(keyed) => {
            // Resolve receiver for keyed reads like item[0]
            let resolved_receiver =
                resolve_angular_expression(&keyed.receiver, scope, root_xref, allocator);
            // Also try to resolve the key expression
            let resolved_key = resolve_angular_expression(&keyed.key, scope, root_xref, allocator);

            if resolved_receiver.is_some() || resolved_key.is_some() {
                // At least one part was resolved, create a ResolvedKeyedRead
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&keyed.receiver, allocator),
                        allocator,
                    ))
                });
                let key = resolved_key.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&keyed.key, allocator),
                        allocator,
                    ))
                });

                Some(IrExpression::ResolvedKeyedRead(Box::new_in(
                    ResolvedKeyedReadExpr {
                        receiver: Box::new_in(receiver, allocator),
                        key: Box::new_in(key, allocator),
                        source_span: Some(keyed.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::SafePropertyRead(safe) => {
            // Resolve receiver for safe property reads like item?.name
            if let Some(resolved_receiver) =
                resolve_angular_expression(&safe.receiver, scope, root_xref, allocator)
            {
                Some(IrExpression::ResolvedSafePropertyRead(Box::new_in(
                    ResolvedSafePropertyReadExpr {
                        receiver: Box::new_in(resolved_receiver, allocator),
                        name: safe.name.clone(),
                        source_span: Some(safe.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::Binary(binary) => {
            // Handle binary expressions, especially assignments in event handlers
            // like `todo.done = $event`
            let resolved_left =
                resolve_angular_expression(&binary.left, scope, root_xref, allocator);
            let resolved_right =
                resolve_angular_expression(&binary.right, scope, root_xref, allocator);

            if resolved_left.is_some() || resolved_right.is_some() {
                // At least one side was resolved, create a ResolvedBinary
                let left = resolved_left.unwrap_or_else(|| {
                    // Wrap original left expression as Ast
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&binary.left, allocator),
                        allocator,
                    ))
                });
                let right = resolved_right.unwrap_or_else(|| {
                    // Wrap original right expression as Ast
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&binary.right, allocator),
                        allocator,
                    ))
                });

                Some(IrExpression::ResolvedBinary(Box::new_in(
                    crate::ir::expression::ResolvedBinaryExpr {
                        operator: binary.operation,
                        left: Box::new_in(left, allocator),
                        right: Box::new_in(right, allocator),
                        source_span: Some(binary.source_span.to_span()),
                    },
                    allocator,
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
                allocator,
            )))
        }

        AngularExpression::TemplateLiteral(tl) => {
            // Handle template literal expressions like `class {{ item.name }}`
            // Need to resolve any variable references in the expressions
            let mut resolved_exprs = oxc_allocator::Vec::new_in(allocator);
            let mut any_resolved = false;

            for expr in tl.expressions.iter() {
                if let Some(resolved) =
                    resolve_angular_expression(expr, scope, root_xref, allocator)
                {
                    resolved_exprs.push(resolved);
                    any_resolved = true;
                } else {
                    // Keep the original expression wrapped as Ast
                    resolved_exprs.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(expr, allocator),
                        allocator,
                    )));
                }
            }

            if any_resolved {
                // Create a ResolvedTemplateLiteral with the resolved expressions
                let mut elements = oxc_allocator::Vec::new_in(allocator);
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
                        source_span: Some(tl.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::Conditional(cond) => {
            // Handle conditional (ternary) expressions
            let resolved_condition =
                resolve_angular_expression(&cond.condition, scope, root_xref, allocator);
            let resolved_true =
                resolve_angular_expression(&cond.true_exp, scope, root_xref, allocator);
            let resolved_false =
                resolve_angular_expression(&cond.false_exp, scope, root_xref, allocator);

            if resolved_condition.is_some() || resolved_true.is_some() || resolved_false.is_some() {
                let condition = resolved_condition.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&cond.condition, allocator),
                        allocator,
                    ))
                });
                let true_expr = resolved_true.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&cond.true_exp, allocator),
                        allocator,
                    ))
                });
                let false_expr = resolved_false.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&cond.false_exp, allocator),
                        allocator,
                    ))
                });

                Some(IrExpression::Ternary(Box::new_in(
                    crate::ir::expression::TernaryExpr {
                        condition: Box::new_in(condition, allocator),
                        true_expr: Box::new_in(true_expr, allocator),
                        false_expr: Box::new_in(false_expr, allocator),
                        source_span: Some(cond.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::LiteralMap(map) => {
            // Handle object literals - need to resolve variable references in values
            let mut resolved_values = oxc_allocator::Vec::new_in(allocator);
            let mut any_resolved = false;

            for value in map.values.iter() {
                if let Some(resolved) =
                    resolve_angular_expression(value, scope, root_xref, allocator)
                {
                    resolved_values.push(resolved);
                    any_resolved = true;
                } else {
                    // Keep the original value wrapped as Ast
                    resolved_values.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(value, allocator),
                        allocator,
                    )));
                }
            }

            if any_resolved {
                use crate::ast::expression::LiteralMapKey;
                // Create a DerivedLiteralMap with the resolved values
                let mut keys = oxc_allocator::Vec::new_in(allocator);
                let mut quoted = oxc_allocator::Vec::new_in(allocator);
                for key in map.keys.iter() {
                    // Only handle property keys; skip spread keys
                    if let LiteralMapKey::Property(prop) = key {
                        keys.push(prop.key.clone());
                        quoted.push(prop.quoted);
                    }
                }

                Some(IrExpression::DerivedLiteralMap(Box::new_in(
                    crate::ir::expression::DerivedLiteralMapExpr {
                        keys,
                        values: resolved_values,
                        quoted,
                        source_span: Some(map.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::LiteralArray(arr) => {
            // Handle array literals - need to resolve variable references in entries
            let mut resolved_entries = oxc_allocator::Vec::new_in(allocator);
            let mut any_resolved = false;

            for entry in arr.expressions.iter() {
                if let Some(resolved) =
                    resolve_angular_expression(entry, scope, root_xref, allocator)
                {
                    resolved_entries.push(resolved);
                    any_resolved = true;
                } else {
                    // Keep the original entry wrapped as Ast
                    resolved_entries.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(entry, allocator),
                        allocator,
                    )));
                }
            }

            if any_resolved {
                Some(IrExpression::DerivedLiteralArray(Box::new_in(
                    crate::ir::expression::DerivedLiteralArrayExpr {
                        entries: resolved_entries,
                        source_span: Some(arr.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::PrefixNot(prefix_not) => {
            // Handle prefix not expressions (!expr) - need to resolve variable references
            // in the operand. This is critical for expressions like `!bold` in listeners.
            if let Some(resolved) =
                resolve_angular_expression(&prefix_not.expression, scope, root_xref, allocator)
            {
                Some(IrExpression::Not(Box::new_in(
                    crate::ir::expression::NotExpr {
                        expr: Box::new_in(resolved, allocator),
                        source_span: Some(prefix_not.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::Unary(unary) => {
            // Handle unary expressions (+expr or -expr) - need to resolve variable references
            if let Some(resolved) =
                resolve_angular_expression(&unary.expr, scope, root_xref, allocator)
            {
                Some(IrExpression::Unary(Box::new_in(
                    crate::ir::expression::UnaryExpr {
                        operator: match unary.operator {
                            crate::ast::expression::UnaryOperator::Plus => {
                                crate::ir::expression::IrUnaryOperator::Plus
                            }
                            crate::ast::expression::UnaryOperator::Minus => {
                                crate::ir::expression::IrUnaryOperator::Minus
                            }
                        },
                        expr: Box::new_in(resolved, allocator),
                        source_span: Some(unary.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::TypeofExpression(typeof_expr) => {
            // Handle typeof expressions - need to resolve variable references
            if let Some(resolved) =
                resolve_angular_expression(&typeof_expr.expression, scope, root_xref, allocator)
            {
                Some(IrExpression::Typeof(Box::new_in(
                    crate::ir::expression::TypeofExpr {
                        expr: Box::new_in(resolved, allocator),
                        source_span: Some(typeof_expr.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::VoidExpression(void_expr) => {
            // Handle void expressions - need to resolve variable references
            if let Some(resolved) =
                resolve_angular_expression(&void_expr.expression, scope, root_xref, allocator)
            {
                Some(IrExpression::Void(Box::new_in(
                    crate::ir::expression::VoidExpr {
                        expr: Box::new_in(resolved, allocator),
                        source_span: Some(void_expr.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::NonNullAssert(nna) => {
            // Handle non-null assertion expressions (expr!) - need to resolve variable references
            // NonNullAssert expressions are wrapped in Ast since IrExpression doesn't have this variant
            resolve_angular_expression(&nna.expression, scope, root_xref, allocator)
        }

        AngularExpression::ParenthesizedExpression(paren) => {
            // Handle parenthesized expressions - need to resolve variable references
            // within the inner expression
            resolve_angular_expression(&paren.expression, scope, root_xref, allocator)
        }

        AngularExpression::Chain(chain) => {
            // Handle chain expressions (expr1; expr2; ...) - need to resolve all expressions
            // Chain is not directly supported in IrExpression, so we need to handle it specially
            let mut any_resolved = false;
            for expr in chain.expressions.iter() {
                if resolve_angular_expression(expr, scope, root_xref, allocator).is_some() {
                    any_resolved = true;
                    break;
                }
            }
            // Chain expressions are not representable in IR, but we still want to
            // flag if any inner expressions need resolution
            // For now, return None and let the Ast path handle it
            if any_resolved {
                // This path will not be perfectly handled, but chain expressions
                // in listeners are unusual; the primary path (Ast) will handle most cases
                None
            } else {
                None
            }
        }

        AngularExpression::SafeCall(safe_call) => {
            // Handle safe function calls (fn?.()) - need to resolve receiver and arguments
            let resolved_receiver =
                resolve_angular_expression(&safe_call.receiver, scope, root_xref, allocator);

            let mut resolved_args = oxc_allocator::Vec::new_in(allocator);
            let mut any_arg_resolved = false;

            for arg in safe_call.args.iter() {
                if let Some(resolved) = resolve_angular_expression(arg, scope, root_xref, allocator)
                {
                    resolved_args.push(resolved);
                    any_arg_resolved = true;
                } else {
                    // Keep the original argument wrapped as Ast
                    resolved_args.push(IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(arg, allocator),
                        allocator,
                    )));
                }
            }

            if resolved_receiver.is_some() || any_arg_resolved {
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &safe_call.receiver,
                            allocator,
                        ),
                        allocator,
                    ))
                });

                Some(IrExpression::SafeInvokeFunction(Box::new_in(
                    crate::ir::expression::SafeInvokeFunctionExpr {
                        receiver: Box::new_in(receiver, allocator),
                        args: resolved_args,
                        source_span: Some(safe_call.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        AngularExpression::SafeKeyedRead(safe_keyed) => {
            // Handle safe keyed read (obj?.[key]) - need to resolve receiver and key
            let resolved_receiver =
                resolve_angular_expression(&safe_keyed.receiver, scope, root_xref, allocator);
            let resolved_key =
                resolve_angular_expression(&safe_keyed.key, scope, root_xref, allocator);

            if resolved_receiver.is_some() || resolved_key.is_some() {
                let receiver = resolved_receiver.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(
                            &safe_keyed.receiver,
                            allocator,
                        ),
                        allocator,
                    ))
                });
                let key = resolved_key.unwrap_or_else(|| {
                    IrExpression::Ast(Box::new_in(
                        crate::ir::expression::clone_angular_expression(&safe_keyed.key, allocator),
                        allocator,
                    ))
                });

                Some(IrExpression::SafeKeyedRead(Box::new_in(
                    crate::ir::expression::SafeKeyedReadExpr {
                        receiver: Box::new_in(receiver, allocator),
                        index: Box::new_in(key, allocator),
                        source_span: Some(safe_keyed.source_span.to_span()),
                    },
                    allocator,
                )))
            } else {
                None
            }
        }

        // Other expression types don't need recursive resolution
        _ => None,
    }
}

/// Resolves names for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn resolve_names_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;
    let expression_store_ptr = &job.expressions as *const ExpressionStore<'_>;

    // SAFETY: We're only reading from expression_store, not modifying it
    let expressions = unsafe { &*expression_store_ptr };

    // Build scope from update ops
    let update_scope = build_scope_from_update_ops(&job.root.update);

    // Process create ops with the update scope available
    process_lexical_scope_create(
        root_xref,
        &mut job.root.create,
        None,
        &update_scope,
        allocator,
        expressions,
    );
    process_lexical_scope_update(root_xref, &mut job.root.update, None, allocator, expressions);

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

    for view in job.all_views() {
        // Check create ops
        for op in view.create.iter() {
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
        for op in view.update.iter() {
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

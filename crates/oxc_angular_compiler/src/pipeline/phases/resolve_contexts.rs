//! Resolve contexts phase.
//!
//! Resolves `ir.ContextExpr` expressions (which represent embedded view or component contexts)
//! to either the `ctx` parameter to component functions (for the current view context) or to
//! variables that store those contexts (for contexts accessed via the `nextContext()` instruction).
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_contexts.ts`.

use rustc_hash::FxHashMap;

use crate::ir::enums::SemanticVariableKind;
use crate::ir::expression::{
    IrExpression, ReadVariableExpr, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_expression, transform_expressions_in_update_op,
};
use crate::ir::list::{CreateOpList, UpdateOpList};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Represents how to access a view's context.
#[derive(Debug, Clone)]
enum ContextAccess {
    /// Access via the `ctx` parameter (current view's context).
    CtxParameter,
    /// Access via a variable read (for saved contexts).
    ReadVariable(XrefId),
}

/// Resolves context references to concrete expressions.
///
/// This phase transforms:
/// - `ContextExpr` for current view → kept as ContextExpr (reify handles ctx)
/// - `ContextExpr` for ancestor view → ReadVariableExpr referencing saved context
pub fn resolve_contexts(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;

    // Collect view xrefs to avoid borrow issues
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    // Process each view
    for view_xref in view_xrefs {
        let is_root = view_xref == root_xref;
        if let Some(view) = job.view_mut(view_xref) {
            // Process arrow functions' ops first, matching Angular's resolve_contexts.ts:
            //   for (const expr of unit.functions) { processLexicalScope(unit, expr.ops); }
            for fn_ptr in view.functions.iter() {
                // SAFETY: The pointer is valid because it was populated by generate_arrow_functions
                // and the allocator keeps the data alive.
                let arrow_fn = unsafe { &mut **fn_ptr };
                process_lexical_scope_update_vec(allocator, view_xref, is_root, &mut arrow_fn.ops);
            }

            process_lexical_scope_create(allocator, view_xref, is_root, &mut view.create);
            process_lexical_scope_update(allocator, view_xref, is_root, &mut view.update);
        }
    }
}

/// Process create operations to build scope and resolve context expressions.
fn process_lexical_scope_create<'a>(
    allocator: &'a oxc_allocator::Allocator,
    view_xref: XrefId,
    is_root: bool,
    ops: &mut CreateOpList<'a>,
) {
    // Track how to access each view's context by its XrefId.
    let mut scope: FxHashMap<XrefId, ContextAccess> = FxHashMap::default();

    // The current view's context is accessible via the `ctx` parameter.
    scope.insert(view_xref, ContextAccess::CtxParameter);

    // First pass: Build scope from Variable operations
    for op in ops.iter() {
        if let CreateOp::Variable(var_op) = op {
            if var_op.kind == SemanticVariableKind::Context {
                // Context variables store a reference to another view's context.
                if let Some(target_view) = var_op.view {
                    scope.insert(target_view, ContextAccess::ReadVariable(var_op.xref));
                }
            }
        }
    }

    // If this is the root view, prefer `ctx` over any variables
    if is_root {
        scope.insert(view_xref, ContextAccess::CtxParameter);
    }

    // Second pass: Transform ContextExpr to the appropriate expression
    // Also recursively process listener handler_ops and RepeaterCreate track_by_ops
    // (per resolve_contexts.ts lines 40-61)
    for op in ops.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                // Process listener handler_ops and handler_expression with their own scope
                process_listener_handler_ops(
                    allocator,
                    view_xref,
                    is_root,
                    &mut listener.handler_ops,
                    &mut listener.handler_expression,
                );
            }
            CreateOp::TwoWayListener(listener) => {
                // TwoWayListener has no handler_expression
                process_listener_handler_ops(
                    allocator,
                    view_xref,
                    is_root,
                    &mut listener.handler_ops,
                    &mut None,
                );
            }
            CreateOp::AnimationListener(listener) => {
                // AnimationListener has no handler_expression
                process_listener_handler_ops(
                    allocator,
                    view_xref,
                    is_root,
                    &mut listener.handler_ops,
                    &mut None,
                );
            }
            CreateOp::Animation(animation) => {
                // Animation has no handler_expression
                process_listener_handler_ops(
                    allocator,
                    view_xref,
                    is_root,
                    &mut animation.handler_ops,
                    &mut None,
                );
            }
            CreateOp::RepeaterCreate(repeater) => {
                // Process track_by_ops with their own scope first, matching Angular's
                // resolve_contexts.ts lines 55-58:
                //   case ir.OpKind.RepeaterCreate:
                //     if (op.trackByOps !== null) { processLexicalScope(view, op.trackByOps); }
                if let Some(ref mut track_by_ops) = repeater.track_by_ops {
                    process_lexical_scope_update_vec(allocator, view_xref, is_root, track_by_ops);
                }
                // Also transform expressions in the RepeaterCreate op itself (e.g., rep.track)
                // using the parent scope. Note: track_by_ops have already been processed above
                // so re-visiting them via transform_expressions_in_create_op is a no-op
                // (ContextExpr nodes in track_by_ops are already resolved).
                transform_expressions_in_create_op(
                    op,
                    &|expr, _flags| {
                        transform_context_expr(allocator, expr, &scope);
                    },
                    VisitorContextFlag::NONE,
                );
            }
            _ => {
                transform_expressions_in_create_op(
                    op,
                    &|expr, _flags| {
                        transform_context_expr(allocator, expr, &scope);
                    },
                    VisitorContextFlag::NONE,
                );
            }
        }
    }
}

/// Process listener handler_ops and handler_expression to resolve context expressions.
///
/// Listener handlers have their own scope that includes the restoreView variable.
/// The restoreView variable (with SemanticVariableKind::Context) provides access
/// to the enclosing view's context.
///
/// Per TypeScript's resolve_contexts.ts:
/// - Context variables are ALWAYS added to scope (even for the same view)
/// - After building scope from ops, only for ROOT view do we prefer `ctx` (line 59-62)
/// - For embedded views, the RestoreView variable IS used for context access
fn process_listener_handler_ops<'a>(
    allocator: &'a oxc_allocator::Allocator,
    view_xref: XrefId,
    is_root: bool,
    handler_ops: &mut oxc_allocator::Vec<'a, UpdateOp<'a>>,
    handler_expression: &mut Option<oxc_allocator::Box<'a, IrExpression<'a>>>,
) {
    // Build scope from handler_ops - look for Context variables (restoreView/nextContext)
    let mut scope: FxHashMap<XrefId, ContextAccess> = FxHashMap::default();

    // The view's context is initially accessible via the `ctx` parameter
    scope.insert(view_xref, ContextAccess::CtxParameter);

    // Look for Context variables which provide access to view contexts.
    // These come from two sources:
    // 1. save_restore_view: Creates Context variable with RestoreView for the current view
    // 2. generate_variables: Creates Context variable with NextContext for parent views
    //
    // Per TypeScript's resolve_contexts.ts, ALL Context variables are added to scope
    // without checking if target_view == view_xref. This is because in embedded views,
    // the RestoreView call returns the context, and we need to capture that in a variable
    // and use it for property access (e.g., `const ctx_r1 = restoreView(_r1); ctx_r1.item`).
    for op in handler_ops.iter() {
        if let UpdateOp::Variable(var_op) = op {
            if var_op.kind == SemanticVariableKind::Context {
                if let Some(target_view) = var_op.view {
                    scope.insert(target_view, ContextAccess::ReadVariable(var_op.xref));
                }
            }
        }
    }

    // Per TypeScript's resolve_contexts.ts lines 59-62:
    // If this is the root view, prefer `ctx` over any variables.
    // This is applied AFTER building the scope from ops, so for root view listeners,
    // `ctx` is used instead of the RestoreView variable.
    if is_root {
        scope.insert(view_xref, ContextAccess::CtxParameter);
    }

    // Transform ContextExpr in handler_ops
    for op in handler_ops.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                transform_context_expr(allocator, expr, &scope);
            },
            VisitorContextFlag::NONE,
        );
    }

    // Also transform ContextExpr in handler_expression (if present)
    // This is important because handler_expression may contain ContextExpr(view_xref)
    // for property access like `ctx_r1.clear(ctx_r1.item)` (embedded) or `ctx.handleInputChange()` (root)
    if let Some(expr) = handler_expression {
        transform_expressions_in_expression(
            expr.as_mut(),
            &|e, _flags| {
                transform_context_expr(allocator, e, &scope);
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Process update operations to resolve context expressions.
fn process_lexical_scope_update<'a>(
    allocator: &'a oxc_allocator::Allocator,
    view_xref: XrefId,
    is_root: bool,
    ops: &mut UpdateOpList<'a>,
) {
    // Track how to access each view's context by its XrefId.
    let mut scope: FxHashMap<XrefId, ContextAccess> = FxHashMap::default();

    // The current view's context is accessible via the `ctx` parameter.
    scope.insert(view_xref, ContextAccess::CtxParameter);

    // First pass: Build scope from Variable operations
    for op in ops.iter() {
        if let UpdateOp::Variable(var_op) = op {
            if var_op.kind == SemanticVariableKind::Context {
                if let Some(target_view) = var_op.view {
                    scope.insert(target_view, ContextAccess::ReadVariable(var_op.xref));
                }
            }
        }
    }

    // If this is the root view, prefer `ctx` over any variables
    if is_root {
        scope.insert(view_xref, ContextAccess::CtxParameter);
    }

    // Second pass: Transform ContextExpr to the appropriate expression
    for op in ops.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                transform_context_expr(allocator, expr, &scope);
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Process update operations in a Vec (used for arrow function ops and track_by_ops).
///
/// This is the same logic as `process_lexical_scope_update` but works with `Vec<UpdateOp>`
/// instead of `UpdateOpList`. Needed for `ArrowFunctionExpr.ops` and `RepeaterCreate.track_by_ops`.
fn process_lexical_scope_update_vec<'a>(
    allocator: &'a oxc_allocator::Allocator,
    view_xref: XrefId,
    is_root: bool,
    ops: &mut oxc_allocator::Vec<'a, UpdateOp<'a>>,
) {
    // Track how to access each view's context by its XrefId.
    let mut scope: FxHashMap<XrefId, ContextAccess> = FxHashMap::default();

    // The current view's context is accessible via the `ctx` parameter.
    scope.insert(view_xref, ContextAccess::CtxParameter);

    // First pass: Build scope from Variable operations
    for op in ops.iter() {
        if let UpdateOp::Variable(var_op) = op {
            if var_op.kind == SemanticVariableKind::Context {
                if let Some(target_view) = var_op.view {
                    scope.insert(target_view, ContextAccess::ReadVariable(var_op.xref));
                }
            }
        }
    }

    // If this is the root view, prefer `ctx` over any variables
    if is_root {
        scope.insert(view_xref, ContextAccess::CtxParameter);
    }

    // Second pass: Transform ContextExpr to the appropriate expression
    for op in ops.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                transform_context_expr(allocator, expr, &scope);
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Transforms a ContextExpr based on the scope.
///
/// - For current view context: Keep as ContextExpr (reify handles ctx)
/// - For ancestor view context: Replace with ReadVariableExpr
fn transform_context_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut IrExpression<'a>,
    scope: &FxHashMap<XrefId, ContextAccess>,
) {
    if let IrExpression::Context(ctx_expr) = expr {
        match scope.get(&ctx_expr.view) {
            Some(ContextAccess::CtxParameter) => {
                // Current view's context - keep as ContextExpr
                // The reify phase will convert this to `ctx`
            }
            Some(ContextAccess::ReadVariable(var_xref)) => {
                // Ancestor view's context - replace with ReadVariableExpr
                *expr = IrExpression::ReadVariable(oxc_allocator::Box::new_in(
                    ReadVariableExpr { xref: *var_xref, name: None, source_span: None },
                    allocator,
                ));
            }
            None => {
                // No context found - this is an error condition but we'll keep
                // the expression as-is for now. The reify phase will handle it.
            }
        }
    }
}

/// Resolves context expressions for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn resolve_contexts_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    let view_xref = job.root.xref;
    // Host bindings are always at the root level
    process_lexical_scope_create(allocator, view_xref, true, &mut job.root.create);
    process_lexical_scope_update(allocator, view_xref, true, &mut job.root.update);
}

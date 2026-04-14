//! Save and restore view phase.
//!
//! When inside of a listener, we may need access to one or more enclosing views.
//! Therefore, each view should save the current view, and each listener must have
//! the ability to restore the appropriate view.
//!
//! This phase:
//! 1. For arrow functions that need it, adds a RestoreView operation to their ops
//! 2. Prepends a SavedView variable to each view that stores `getCurrentView()`
//! 3. For listeners that need it (in embedded views or accessing local refs),
//!    adds a RestoreView operation at the start of the handler
//! 4. Wraps any return statements in the handler with `resetView()`
//!
//! Ported from Angular's `template/pipeline/src/phases/save_restore_view.ts`.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_str::Ident;

use crate::ir::enums::{ExpressionKind, SemanticVariableKind, VariableFlags};
use crate::ir::expression::{
    GetCurrentViewExpr, IrExpression, ResetViewExpr, RestoreViewExpr, RestoreViewTarget,
    VisitorContextFlag, visit_expressions_in_expression, visit_expressions_in_update_op,
};
use crate::ir::ops::{CreateOp, UpdateOp, UpdateOpBase, UpdateVariableOp, VariableOp, XrefId};
use crate::output::ast::{OutputExpression, ReadVarExpr};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Generates save/restore view context operations.
///
/// This phase ensures that listeners in embedded views can properly access
/// the enclosing view context by:
/// 1. Processing arrow functions that need RestoreView (with dynamic 'view' target)
/// 2. Saving the current view in a variable at the start of each view
/// 3. Restoring the view at the start of listener handlers that need it
///
/// Per Angular's compiler (save_restore_view.ts lines 16-17):
/// "We eagerly generate all save view variables; they will be optimized away later."
///
/// IMPORTANT: The order of operations must match Angular's TypeScript implementation.
/// Angular iterates `job.units` (which is `this.views.values()`) and for EACH unit:
/// 1. Processes arrow functions first
/// 2. Adds SavedView variable
/// 3. Processes listeners
///
/// This order is critical for XrefId allocation which affects variable naming.
pub fn save_and_restore_view(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;

    // Get view xrefs in the order they were inserted (matching Map iteration order)
    // In Angular, job.units returns this.views.values() which iterates in insertion order.
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();

    // Process root view (first unit in Angular's job.units iterator)
    // Per Angular's save_restore_view.ts, for EACH unit in order:
    // 1. Process arrow functions (lines 20-26)
    // 2. Prepend SavedView variable (lines 28-39)
    // 3. Process listeners (lines 41-52)
    process_arrow_functions_in_view(job, root_xref, true);
    let var_xref = job.allocate_xref_id();
    let saved_view_var = create_saved_view_variable(allocator, var_xref, root_xref);
    job.root.create.push_front(saved_view_var);
    process_view_listeners(job, root_xref, true);

    // Process embedded views (remaining units in Angular's job.units iterator)
    for view_xref in view_xrefs {
        // 1. Process arrow functions
        process_arrow_functions_in_view(job, view_xref, false);

        // 2. Eagerly add SavedView variable to EVERY embedded view
        // (will be optimized away later if not needed)
        let var_xref = job.allocate_xref_id();
        let saved_view_var = create_saved_view_variable(allocator, var_xref, view_xref);
        if let Some(view) = job.views.get_mut(&view_xref) {
            view.create.push_front(saved_view_var);
        }

        // 3. Process listeners in this view
        process_view_listeners(job, view_xref, false);
    }
}

/// Process arrow functions in a view, adding restore view operations where needed.
///
/// Per TypeScript (save_restore_view.ts lines 20-26):
/// ```typescript
/// for (const expr of unit.functions) {
///   if (needsRestoreView(job, unit, expr.ops)) {
///     // We don't need to capture the view in a variable for arrow
///     // functions, it will be passed in to its factory.
///     addSaveRestoreViewOperation(unit, expr.ops, o.variable(expr.currentViewName));
///   }
/// }
/// ```
///
/// Arrow functions get their view context passed in as a parameter 'view',
/// so the RestoreView target is `o.variable('view')` instead of an XrefId.
fn process_arrow_functions_in_view(
    job: &mut ComponentCompilationJob<'_>,
    view_xref: XrefId,
    is_root: bool,
) {
    let allocator = job.allocator;

    // First, check which arrow functions need RestoreView
    let function_count = {
        let view =
            if is_root { Some(&job.root) } else { job.views.get(&view_xref).map(|v| v.as_ref()) };
        view.map(|v| v.functions.len()).unwrap_or(0)
    };

    if function_count == 0 {
        return;
    }

    // Check each arrow function's ops to see if it needs RestoreView
    // Per Angular, embedded views always need restore view, and root views
    // need it if accessing local refs or context let references.
    let mut needs_restore_indices: Vec<usize> = Vec::new();

    for idx in 0..function_count {
        let needs_restore = {
            let view = if is_root {
                Some(&job.root)
            } else {
                job.views.get(&view_xref).map(|v| v.as_ref())
            };

            if let Some(view) = view {
                if idx < view.functions.len() {
                    // SAFETY: We only read through the pointer here
                    let func = unsafe { &*view.functions[idx] };

                    // Per Angular (save_restore_view.ts lines 56-75):
                    // - Embedded views always need restore view
                    // - Root views need restore if accessing local refs or context lets
                    let mut result = !is_root;

                    if !result {
                        for op in func.ops.iter() {
                            if arrow_fn_op_needs_restore_view(op) {
                                result = true;
                                break;
                            }
                        }
                    }

                    result
                } else {
                    false
                }
            } else {
                false
            }
        };

        if needs_restore {
            needs_restore_indices.push(idx);
        }
    }

    // Allocate XrefIds for all arrow functions that need RestoreView
    let var_xrefs: Vec<(usize, XrefId)> =
        needs_restore_indices.iter().map(|idx| (*idx, job.allocate_xref_id())).collect();

    // Now add RestoreView operations to the arrow functions that need them
    let view = if is_root {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    for (idx, var_xref) in var_xrefs {
        if idx < view.functions.len() {
            // SAFETY: We have mutable access to the view and can modify its functions
            let func = unsafe { &mut *view.functions[idx] };

            // Create RestoreView variable with dynamic 'view' target
            // Per Angular (save_restore_view.ts lines 77-93):
            // The target is o.variable(expr.currentViewName) where currentViewName is 'view'
            let view_var_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from("view"), source_span: None },
                allocator,
            ));

            let restore_var = UpdateOp::Variable(UpdateVariableOp {
                base: UpdateOpBase::default(),
                xref: var_xref,
                kind: SemanticVariableKind::Context,
                name: Ident::from(""), // Empty = naming phase will assign it
                initializer: Box::new_in(
                    IrExpression::RestoreView(Box::new_in(
                        RestoreViewExpr {
                            view: RestoreViewTarget::Dynamic(Box::new_in(
                                IrExpression::OutputExpr(Box::new_in(view_var_expr, allocator)),
                                allocator,
                            )),
                            source_span: None,
                        },
                        allocator,
                    )),
                    allocator,
                ),
                flags: VariableFlags::NONE,
                view: Some(view_xref),
                local: false,
            });

            // Insert at the beginning of the arrow function's ops
            func.ops.insert(0, restore_var);

            // Note: Arrow functions don't have return statements to wrap
            // because they are single-expression functions in templates.
            // The ResetView wrapping only applies to listener handlers
            // which can have explicit return statements.
        }
    }
}

/// Checks if an arrow function op contains expressions that require restore view.
/// This is a simplified version for arrow function ops.
fn arrow_fn_op_needs_restore_view(op: &UpdateOp<'_>) -> bool {
    use std::cell::Cell;
    let needs_restore = Cell::new(false);

    visit_expressions_in_update_op(
        op,
        &|expr: &IrExpression<'_>, _flags: VisitorContextFlag| {
            let kind = expr.kind();
            if kind == ExpressionKind::Reference || kind == ExpressionKind::ContextLetReference {
                needs_restore.set(true);
            }
        },
        VisitorContextFlag::NONE,
    );

    needs_restore.get()
}

/// Process listeners in a view, adding restore view operations where needed.
///
/// Per TypeScript (save_restore_view.ts lines 41-52):
/// - Handles Listener, TwoWayListener, Animation, AnimationListener ops
/// - Embedded views always need restore view
/// - Root views need restore if accessing local refs or context let references
fn process_view_listeners(job: &mut ComponentCompilationJob<'_>, view_xref: XrefId, is_root: bool) {
    let allocator = job.allocator;

    // Collect listener info first to avoid borrow issues
    let listener_info: Vec<(usize, bool)> = {
        let view = if is_root {
            &job.root
        } else if let Some(v) = job.views.get(&view_xref) {
            v.as_ref()
        } else {
            return;
        };

        let mut info = Vec::new();
        let mut idx = 0;
        for op in view.create.iter() {
            // Handle all listener-like operations: Listener, TwoWayListener, Animation, AnimationListener
            let (handler_ops, handler_expression) = match op {
                CreateOp::Listener(listener) => {
                    (Some(&listener.handler_ops), listener.handler_expression.as_ref())
                }
                CreateOp::TwoWayListener(listener) => (Some(&listener.handler_ops), None),
                CreateOp::Animation(animation) => (Some(&animation.handler_ops), None),
                CreateOp::AnimationListener(listener) => (Some(&listener.handler_ops), None),
                _ => (None, None),
            };

            if let Some(handler_ops) = handler_ops {
                // Embedded views always need restore view
                let mut needs_restore = !is_root;

                // Root listeners need restore if they reference local refs or context lets
                // Per Angular's TypeScript (save_restore_view.ts lines 47-54), we check
                // for ReferenceExpr or ContextLetReferenceExpr in handlerOps.
                // In our IR, we also need to check handler_expression since we store
                // the handler expression separately from handlerOps.
                if is_root {
                    for handler_op in handler_ops.iter() {
                        needs_restore = needs_restore || check_needs_restore_view(handler_op);
                    }
                    // Also check handler_expression for Reference or ContextLetReference
                    if let Some(expr) = handler_expression {
                        needs_restore = needs_restore || expression_needs_restore_view(expr);
                    }
                }

                info.push((idx, needs_restore));
            }
            idx += 1;
        }
        info
    };

    // Collect XrefIds for variables to allocate
    let var_xrefs: Vec<(usize, XrefId)> = listener_info
        .iter()
        .filter_map(
            |(idx, needs_restore)| {
                if *needs_restore { Some((*idx, job.allocate_xref_id())) } else { None }
            },
        )
        .collect();

    // Now add restore view operations to listeners that need them
    let view = if is_root {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    // Iterate through create ops and update listeners
    let mut current_idx = 0;
    for op in view.create.iter_mut() {
        // Check if this listener needs restore view (matches indices from first pass)
        let needs_restore =
            var_xrefs.iter().find(|(idx, _)| *idx == current_idx).map(|(_, xref)| *xref);

        // Handle all listener-like operations
        match op {
            CreateOp::Listener(listener) => {
                if let Some(var_xref) = needs_restore {
                    add_restore_view_to_listener(
                        allocator,
                        &mut listener.handler_ops,
                        &mut listener.handler_expression,
                        var_xref,
                        view_xref,
                    );
                }
            }
            CreateOp::TwoWayListener(listener) => {
                if let Some(var_xref) = needs_restore {
                    // TwoWayListener doesn't have handler_expression, only handler_ops
                    add_restore_view_ops_only(
                        allocator,
                        &mut listener.handler_ops,
                        var_xref,
                        view_xref,
                    );
                }
            }
            CreateOp::Animation(animation) => {
                if let Some(var_xref) = needs_restore {
                    // Animation doesn't have handler_expression, only handler_ops
                    add_restore_view_ops_only(
                        allocator,
                        &mut animation.handler_ops,
                        var_xref,
                        view_xref,
                    );
                }
            }
            CreateOp::AnimationListener(listener) => {
                if let Some(var_xref) = needs_restore {
                    // AnimationListener doesn't have handler_expression, only handler_ops
                    add_restore_view_ops_only(
                        allocator,
                        &mut listener.handler_ops,
                        var_xref,
                        view_xref,
                    );
                }
            }
            _ => {}
        }
        // Always increment to match first pass (which increments for ALL ops)
        current_idx += 1;
    }
}

/// Adds restore view operations to handler_ops only (for TwoWayListener, AnimationListener).
///
/// Per TypeScript's save_restore_view.ts (lines 68-79), the Context variable is created
/// with `name: null` so that the naming phase will assign it a proper name like `ctx_r0`.
///
/// Per TypeScript's save_restore_view.ts (lines 84-91), we also wrap return statements
/// in `resetView()` to reset the view context prior to returning from the listener.
fn add_restore_view_ops_only<'a>(
    allocator: &'a oxc_allocator::Allocator,
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    var_xref: XrefId,
    view_xref: XrefId,
) {
    // Create restore view variable and insert at front of handler_ops
    let restore_var = UpdateOp::Variable(UpdateVariableOp {
        base: UpdateOpBase::default(),
        xref: var_xref,
        kind: SemanticVariableKind::Context,
        name: Ident::from(""), // Empty = naming phase will assign it
        initializer: Box::new_in(
            IrExpression::RestoreView(Box::new_in(
                RestoreViewExpr { view: RestoreViewTarget::Static(view_xref), source_span: None },
                allocator,
            )),
            allocator,
        ),
        flags: VariableFlags::NONE,
        view: Some(view_xref),
        local: false,
    });

    handler_ops.insert(0, restore_var);

    // Wrap return statements in ResetViewExpr (Angular's save_restore_view.ts lines 84-91)
    // This resets the view context after the listener handler returns
    wrap_return_statements_in_reset_view(allocator, handler_ops);
}

/// Adds restore view operations to a listener.
///
/// This function:
/// 1. Prepends a RestoreView variable to the handler_ops
/// 2. Wraps the handler_expression in ResetViewExpr (per save_restore_view.ts lines 84-91)
///
/// Per TypeScript's save_restore_view.ts (lines 68-79), the Context variable is created
/// with `name: null` so that the naming phase will assign it a proper name like `ctx_r0`.
fn add_restore_view_to_listener<'a>(
    allocator: &'a oxc_allocator::Allocator,
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    handler_expression: &mut Option<Box<'a, IrExpression<'a>>>,
    var_xref: XrefId,
    view_xref: XrefId,
) {
    // Create restore view variable and insert at front of handler_ops
    let restore_var = UpdateOp::Variable(UpdateVariableOp {
        base: UpdateOpBase::default(),
        xref: var_xref,
        kind: SemanticVariableKind::Context,
        name: Ident::from(""), // Empty = naming phase will assign it
        initializer: Box::new_in(
            IrExpression::RestoreView(Box::new_in(
                RestoreViewExpr { view: RestoreViewTarget::Static(view_xref), source_span: None },
                allocator,
            )),
            allocator,
        ),
        flags: VariableFlags::NONE,
        view: Some(view_xref),
        local: false,
    });

    // Insert at the beginning of handler_ops
    handler_ops.insert(0, restore_var);

    // Wrap handler_expression in ResetViewExpr (Angular's save_restore_view.ts lines 84-91)
    // This resets the view context after the listener handler returns
    if let Some(expr) = handler_expression.take() {
        let cloned_expr = expr.clone_in(allocator);
        *handler_expression = Some(Box::new_in(
            IrExpression::ResetView(Box::new_in(
                ResetViewExpr { expr: Box::new_in(cloned_expr, allocator), source_span: None },
                allocator,
            )),
            allocator,
        ));
    }
}

/// Checks if a handler op contains expressions that require restore view.
fn check_needs_restore_view(op: &UpdateOp<'_>) -> bool {
    // Check expressions in the operation
    match op {
        UpdateOp::Variable(var) => expression_needs_restore_view(&var.initializer),
        _ => {
            // For other ops, we would check their expressions similarly
            false
        }
    }
}

/// Recursively check if an expression contains Reference or ContextLetReference.
fn expression_needs_restore_view(expr: &IrExpression<'_>) -> bool {
    use std::cell::Cell;

    let needs_restore = Cell::new(false);

    // Use the visit function to recursively traverse all nested expressions
    visit_expressions_in_expression(
        expr,
        &|inner_expr: &IrExpression<'_>, _flags: VisitorContextFlag| {
            let kind = inner_expr.kind();
            if kind == ExpressionKind::Reference || kind == ExpressionKind::ContextLetReference {
                needs_restore.set(true);
            }
        },
        VisitorContextFlag::NONE,
    );

    needs_restore.get()
}

/// Creates a saved view variable operation.
///
/// Per TypeScript's save_restore_view.ts, the variable is created with `name: null`
/// so that the naming phase will assign it a proper unique name like `_r1`.
fn create_saved_view_variable<'a>(
    allocator: &'a oxc_allocator::Allocator,
    xref: XrefId,
    view_xref: XrefId,
) -> CreateOp<'a> {
    CreateOp::Variable(VariableOp {
        base: Default::default(),
        xref,
        kind: SemanticVariableKind::SavedView,
        name: Ident::from(""), // Empty = naming phase will assign it
        initializer: Box::new_in(
            IrExpression::GetCurrentView(Box::new_in(
                GetCurrentViewExpr { source_span: None },
                allocator,
            )),
            allocator,
        ),
        flags: VariableFlags::NONE,
        view: Some(view_xref),
        local: false,
    })
}

/// Wraps return statement values in ResetViewExpr.
///
/// Per TypeScript's save_restore_view.ts (lines 84-91):
/// ```typescript
/// for (const handlerOp of op.handlerOps) {
///   if (
///     handlerOp.kind === ir.OpKind.Statement &&
///     handlerOp.statement instanceof o.ReturnStatement
///   ) {
///     handlerOp.statement.value = new ir.ResetViewExpr(handlerOp.statement.value);
///   }
/// }
/// ```
fn wrap_return_statements_in_reset_view<'a>(
    allocator: &'a oxc_allocator::Allocator,
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
) {
    use crate::output::ast::{OutputExpression, OutputStatement, ReturnStatement, WrappedIrExpr};

    for op in handler_ops.iter_mut() {
        if let UpdateOp::Statement(stmt_op) = op {
            if let OutputStatement::Return(ret_stmt) = &mut stmt_op.statement {
                // Wrap the return value in ResetViewExpr
                // The return value is an OutputExpression that may be a WrappedIrNode
                let wrapped_reset = match &ret_stmt.value {
                    OutputExpression::WrappedIrNode(wrapped) => {
                        // Wrap the inner IR expression in ResetView
                        let reset_expr = IrExpression::ResetView(Box::new_in(
                            ResetViewExpr {
                                expr: Box::new_in(wrapped.node.clone_in(allocator), allocator),
                                source_span: None,
                            },
                            allocator,
                        ));
                        OutputExpression::WrappedIrNode(Box::new_in(
                            WrappedIrExpr {
                                node: Box::new_in(reset_expr, allocator),
                                source_span: wrapped.source_span,
                            },
                            allocator,
                        ))
                    }
                    _ => {
                        // For non-wrapped expressions, we still need to wrap them
                        // Create a pass-through IR expression that just holds the output expression
                        // However, looking at Angular's TypeScript, ResetViewExpr always takes an Expression.
                        // Our IR design uses IrExpression, so we need to handle this differently.
                        // For now, leave non-wrapped expressions as-is since all return statements
                        // in listeners created during ingestion use WrappedIrNode.
                        continue;
                    }
                };

                // Replace the return statement with the wrapped version
                stmt_op.statement = OutputStatement::Return(Box::new_in(
                    ReturnStatement { value: wrapped_reset, source_span: ret_stmt.source_span },
                    allocator,
                ));
            }
        }
    }
}

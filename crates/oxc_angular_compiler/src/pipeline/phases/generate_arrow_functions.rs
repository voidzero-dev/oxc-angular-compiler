//! Generate arrow functions phase.
//!
//! Finds arrow functions written by the user and converts them into
//! pipeline-specific IR expressions. Arrow functions in event listeners
//! are preserved in place because they need to access $event.
//!
//! Ported from Angular's `template/pipeline/src/phases/generate_arrow_functions.ts`.

use crate::ir::enums::OpKind;
use crate::ir::expression::{
    ArrowFunctionExpr, IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::ir::ops::{CreateOp, Op};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr as OutputArrowFunctionExpr, FnParam,
};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use oxc_allocator::{Box as AllocBox, Vec as AllocVec};

/// Finds arrow functions written by the user and converts them into
/// pipeline-specific IR expressions.
///
/// Arrow functions in event listeners (Listener, TwoWayListener, Animation,
/// AnimationListener) are preserved in place because:
/// 1. They need to be able to access $event.
/// 2. We don't need to store them.
pub fn generate_arrow_functions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process each view
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for xref in &view_xrefs {
        if let Some(view) = job.view_mut(*xref) {
            // Process create operations
            // Skip listeners - they preserve arrow functions in place
            for op in view.create.iter_mut() {
                if !is_listener_op(op) {
                    transform_expressions_in_create_op(
                        op,
                        &|expr, flags| {
                            add_arrow_function(expr, allocator, flags);
                        },
                        VisitorContextFlag::NONE,
                    );
                }
            }

            // Process update operations
            for op in view.update.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, flags| {
                        add_arrow_function(expr, allocator, flags);
                    },
                    VisitorContextFlag::NONE,
                );
            }
        }
    }

    // Collect arrow functions into the functions set for each view
    // We need to do this in a separate pass after the transformations
    for xref in &view_xrefs {
        if let Some(view) = job.view_mut(*xref) {
            collect_arrow_functions_from_view(view);
        }
    }
}

/// Check if an operation is a listener type that should preserve arrow functions.
fn is_listener_op(op: &CreateOp<'_>) -> bool {
    matches!(
        op.kind(),
        OpKind::Animation | OpKind::AnimationListener | OpKind::Listener | OpKind::TwoWayListener
    )
}

/// Transform output AST arrow functions into IR arrow functions.
///
/// This function is called for each expression in the IR. If the expression
/// is an OutputExpr containing an ArrowFunction, it is converted to an
/// IrExpression::ArrowFunction.
fn add_arrow_function<'a>(
    expr: &mut IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    flags: VisitorContextFlag,
) {
    // Skip if we're inside a child operation (nested arrow function)
    if flags.contains(VisitorContextFlag::IN_CHILD_OPERATION) {
        return;
    }

    // Check if this is an output arrow function expression
    if let IrExpression::OutputExpr(output_expr) = expr {
        if let crate::output::ast::OutputExpression::ArrowFunction(arrow_fn) = output_expr.as_ref()
        {
            // Convert output arrow function to IR arrow function
            if let Some(ir_arrow_fn) = convert_output_arrow_to_ir(arrow_fn.as_ref(), allocator) {
                *expr = IrExpression::ArrowFunction(AllocBox::new_in(ir_arrow_fn, allocator));
            }
        }
    }
}

/// Convert an output AST arrow function to an IR arrow function.
///
/// Returns None if the arrow function has a multi-line body (Statement[]),
/// which is not supported in template expressions.
fn convert_output_arrow_to_ir<'a>(
    arrow_fn: &OutputArrowFunctionExpr<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> Option<ArrowFunctionExpr<'a>> {
    // Check for multi-line body - not supported in templates
    let body = match &arrow_fn.body {
        ArrowFunctionBody::Expression(expr) => {
            // Wrap the output expression in an IrExpression::OutputExpr
            // It will be converted later in the reify phase
            IrExpression::OutputExpr(AllocBox::new_in(expr.clone_in(allocator), allocator))
        }
        ArrowFunctionBody::Statements(_) => {
            // The expression syntax doesn't support multi-line arrow functions,
            // but the output AST does. We don't need to handle them here if
            // the user isn't able to write one.
            // Angular throws an assertion error here; we use debug_assert to
            // catch any internal compiler bugs that produce this in debug builds.
            debug_assert!(false, "unexpected multi-line arrow function in template expression");
            return None;
        }
    };

    // Clone parameters
    let mut params: AllocVec<'a, FnParam<'a>> = AllocVec::new_in(allocator);
    for param in arrow_fn.params.iter() {
        params.push(FnParam { name: param.name.clone() });
    }

    Some(ArrowFunctionExpr {
        params,
        body: AllocBox::new_in(body, allocator),
        ops: AllocVec::new_in(allocator),
        var_offset: None,
        source_span: arrow_fn.source_span,
    })
}

/// Generate arrow functions for host binding compilation.
///
/// Angular runs this phase for Kind.Both, meaning it applies to both
/// template and host compilations. Host bindings can contain arrow
/// function expressions (e.g., in @HostBinding values or event handlers).
pub fn generate_arrow_functions_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process create operations (skip listeners)
    for op in job.root.create.iter_mut() {
        if !is_listener_op(op) {
            transform_expressions_in_create_op(
                op,
                &|expr, flags| {
                    add_arrow_function(expr, allocator, flags);
                },
                VisitorContextFlag::NONE,
            );
        }
    }

    // Process update operations
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, flags| {
                add_arrow_function(expr, allocator, flags);
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Collect arrow functions from a view's operations into its functions set.
fn collect_arrow_functions_from_view(
    view: &mut crate::pipeline::compilation::ViewCompilationUnit<'_>,
) {
    // Clear existing functions
    view.functions.clear();

    // We use RefCell to allow mutable access from within the visitor closure
    use std::cell::RefCell;
    let collected: RefCell<std::vec::Vec<*mut ArrowFunctionExpr<'_>>> =
        RefCell::new(std::vec::Vec::new());

    // Collect from create ops
    for op in view.create.iter() {
        crate::ir::expression::visit_expressions_in_create_op(
            op,
            &|expr, _flags| {
                if let IrExpression::ArrowFunction(arrow_fn) = expr {
                    let ptr = arrow_fn.as_ref() as *const ArrowFunctionExpr<'_>
                        as *mut ArrowFunctionExpr<'_>;
                    collected.borrow_mut().push(ptr);
                }
            },
            VisitorContextFlag::NONE,
        );
    }

    // Collect from update ops
    for op in view.update.iter() {
        crate::ir::expression::visit_expressions_in_update_op(
            op,
            &|expr, _flags| {
                if let IrExpression::ArrowFunction(arrow_fn) = expr {
                    let ptr = arrow_fn.as_ref() as *const ArrowFunctionExpr<'_>
                        as *mut ArrowFunctionExpr<'_>;
                    collected.borrow_mut().push(ptr);
                }
            },
            VisitorContextFlag::NONE,
        );
    }

    // Move collected pointers into the allocator Vec
    let ptrs = collected.into_inner();
    for ptr in ptrs {
        view.functions.push(ptr);
    }
}

//! Generate arrow functions phase.
//!
//! Finds arrow functions written by the user and marks them to be hoisted into
//! shared factories instantiated through `ɵɵarrowFunction`. Arrow functions in
//! event listeners are preserved in place because they need to access $event.
//!
//! Ported from Angular's `template/pipeline/src/phases/generate_arrow_functions.ts`.

use std::cell::Cell;

use crate::ir::enums::OpKind;
use crate::ir::expression::{
    ArrowFunctionExpr, IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::ir::ops::{CreateOp, Op};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Finds arrow functions written by the user and marks them as hoisted.
///
/// Arrow functions in event listeners (Listener, TwoWayListener, Animation,
/// AnimationListener) are preserved in place because:
/// 1. They need to be able to access $event.
/// 2. We don't need to store them.
pub fn generate_arrow_functions(job: &mut ComponentCompilationJob<'_>) {
    let found = Cell::new(false);

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
                        &|expr, flags| add_arrow_function(expr, flags, &found),
                        VisitorContextFlag::NONE,
                    );
                }
            }

            // Process update operations
            for op in view.update.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, flags| add_arrow_function(expr, flags, &found),
                    VisitorContextFlag::NONE,
                );
            }
        }
    }

    job.has_hoisted_arrow_functions = found.get();
    collect_arrow_functions(job);
}

/// Rebuilds every view's `functions` list from the hoisted arrow functions currently in
/// its operations.
///
/// The list holds pointers into the expression tree, and phases that rewrite expressions
/// move arrow functions around, so phases that walk `functions` refresh it first.
///
/// When no arrow function was hoisted, every list is already empty and stays so.
pub fn collect_arrow_functions(job: &mut ComponentCompilationJob<'_>) {
    if !job.has_hoisted_arrow_functions {
        return;
    }
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();
    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            collect_arrow_functions_from_view(view);
        }
    }
}

/// Check if an operation keeps the arrow functions written in it in place.
///
/// Listeners do, as in Angular. So does a `@for` block, whose only create-time expression
/// is `track`: it compiles to a standalone function with no `ctx` in scope, so the
/// `ɵɵarrowFunction(0, arrowFn, ctx)` call Angular emits there throws a ReferenceError.
fn is_listener_op(op: &CreateOp<'_>) -> bool {
    matches!(
        op.kind(),
        OpKind::Animation
            | OpKind::AnimationListener
            | OpKind::Listener
            | OpKind::TwoWayListener
            | OpKind::RepeaterCreate
    )
}

/// Marks a user-written arrow function as hoisted, recording in `found` that one was.
///
/// Arrow functions nested inside another arrow function are part of that function's
/// body and stay in place.
fn add_arrow_function(expr: &mut IrExpression<'_>, flags: VisitorContextFlag, found: &Cell<bool>) {
    if flags.contains(VisitorContextFlag::IN_CHILD_OPERATION) {
        return;
    }
    if let IrExpression::ArrowFunction(arrow_fn) = expr {
        arrow_fn.hoisted = true;
        found.set(true);
    }
}

/// Generate arrow functions for host binding compilation.
///
/// Angular runs this phase for Kind.Both, meaning it applies to both
/// template and host compilations. Host bindings can contain arrow
/// function expressions (e.g., in @HostBinding values or event handlers).
pub fn generate_arrow_functions_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let found = Cell::new(false);

    // Process create operations (skip listeners)
    for op in job.root.create.iter_mut() {
        if !is_listener_op(op) {
            transform_expressions_in_create_op(
                op,
                &|expr, flags| add_arrow_function(expr, flags, &found),
                VisitorContextFlag::NONE,
            );
        }
    }

    // Process update operations
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, flags| add_arrow_function(expr, flags, &found),
            VisitorContextFlag::NONE,
        );
    }

    // Collect arrow functions into the root unit's functions set.
    // Only hoisted arrows are collected, matching the template path.
    job.has_hoisted_arrow_functions = found.get();
    collect_arrow_functions_for_host(job);
}

/// Rebuilds the root unit's `functions` list from the hoisted arrow functions
/// currently in its operations, mirroring [`collect_arrow_functions`] for
/// template jobs.
pub fn collect_arrow_functions_for_host(job: &mut HostBindingCompilationJob<'_>) {
    if !job.has_hoisted_arrow_functions {
        return;
    }
    job.root.functions.clear();
    for ptr in collect_hoisted_arrow_functions(&mut job.root.create, &mut job.root.update) {
        job.root.functions.push(ptr);
    }
}

/// Collect hoisted arrow functions from a view's operations into its functions set.
fn collect_arrow_functions_from_view<'a>(
    view: &mut crate::pipeline::compilation::ViewCompilationUnit<'a>,
) {
    view.functions.clear();

    for ptr in collect_hoisted_arrow_functions(&mut view.create, &mut view.update) {
        view.functions.push(ptr);
    }
}

/// Return pointers to the hoisted (top-level) arrow functions in the given op lists.
///
/// The transforming visitors are used, rather than the read-only ones, because they
/// also walk into the statements that variable optimization turns some ops into.
fn collect_hoisted_arrow_functions<'a>(
    create: &mut crate::ir::list::CreateOpList<'a>,
    update: &mut crate::ir::list::UpdateOpList<'a>,
) -> std::vec::Vec<*mut ArrowFunctionExpr<'a>> {
    // We use RefCell to allow mutable access from within the visitor closure
    use std::cell::RefCell;
    let collected: RefCell<std::vec::Vec<*mut ArrowFunctionExpr<'a>>> =
        RefCell::new(std::vec::Vec::new());
    let collect = |expr: &mut IrExpression<'a>, _flags: VisitorContextFlag| {
        if let IrExpression::ArrowFunction(arrow_fn) = expr
            && arrow_fn.hoisted
        {
            collected.borrow_mut().push(std::ptr::from_mut(arrow_fn.as_mut()));
        }
    };

    for op in create.iter_mut() {
        if !is_listener_op(op) {
            transform_expressions_in_create_op(op, &collect, VisitorContextFlag::NONE);
        }
    }
    for op in update.iter_mut() {
        transform_expressions_in_update_op(op, &collect, VisitorContextFlag::NONE);
    }

    collected.into_inner()
}

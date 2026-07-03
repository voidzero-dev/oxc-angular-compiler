//! Collapse singleton interpolations phase.
//!
//! Attribute or style interpolations of the form `[attr.foo]="{{foo}}"` should be "collapsed"
//! into a plain instruction, instead of an interpolated one.
//!
//! This optimization applies to:
//! - `AttributeOp`
//! - `StylePropOp`
//! - `StyleMapOp`
//! - `ClassMapOp`
//!
//! (We cannot do this for singleton property interpolations,
//! because they need to stringify their expressions)
//!
//! The reification step is also capable of performing this transformation, but doing it early
//! in the pipeline allows other phases to accurately know what instruction will be emitted.
//!
//! Ported from Angular's `template/pipeline/src/phases/collapse_singleton_interpolations.ts`.

use oxc_allocator::Box;

use crate::ast::expression::AngularExpression;
use crate::ir::expression::{IrExpression, clone_angular_expression};
use crate::ir::ops::UpdateOp;
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Collapses singleton interpolations into plain expressions.
///
/// This simplifies `{{ expr }}` to just `expr` for Attribute, StyleProp, StyleMap,
/// and ClassMap operations where the interpolation only contains a single expression
/// with no surrounding text.
pub fn collapse_singleton_interpolations(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process root view
    collapse_in_view(&mut job.root.update, &allocator);

    // Process embedded views
    for view in job.views.values_mut() {
        collapse_in_view(&mut view.update, &allocator);
    }
}

/// Collapses a singleton interpolation in place if applicable.
///
/// This modifies the expression in place, replacing the Interpolation wrapper
/// with its single inner expression.
///
/// Handles both:
/// - `IrExpression::Ast(AngularExpression::Interpolation)` - from AST expressions
/// - `IrExpression::Interpolation` - from ingest phase conversion
fn try_collapse_interpolation<'a>(
    expr: &mut Box<'a, IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) {
    match expr.as_ref() {
        // Case 1: AST-wrapped interpolation
        IrExpression::Ast(ast_expr) => {
            if let AngularExpression::Interpolation(interp) = ast_expr.as_ref() {
                if interp.strings.len() == 2
                    && interp.expressions.len() == 1
                    && interp.strings.iter().all(|s| s.as_str().is_empty())
                {
                    // Clone the inner expression and replace
                    let cloned_inner = clone_angular_expression(&interp.expressions[0], &allocator);
                    *expr = Box::new_in(
                        IrExpression::Ast(Box::new_in(cloned_inner, &allocator)),
                        &allocator,
                    );
                }
            }
        }
        // Case 2: IR Interpolation (from ingest phase)
        IrExpression::Interpolation(interp) => {
            if interp.strings.len() == 2
                && interp.expressions.len() == 1
                && interp.strings.iter().all(|s| s.as_str().is_empty())
            {
                // Clone the inner IR expression and replace
                let cloned_inner = interp.expressions[0].clone_in(allocator);
                *expr = Box::new_in(cloned_inner, &allocator);
            }
        }
        _ => {}
    }
}

/// Collapses singleton interpolations within a single view's update list.
fn collapse_in_view<'a>(
    update_ops: &mut crate::ir::list::UpdateOpList<'a>,
    allocator: &'a oxc_allocator::Allocator,
) {
    let mut cursor = update_ops.cursor();

    while cursor.move_next() {
        match cursor.current_mut() {
            Some(UpdateOp::Attribute(attr)) => {
                try_collapse_interpolation(&mut attr.expression, &allocator);
            }
            Some(UpdateOp::StyleProp(style)) => {
                try_collapse_interpolation(&mut style.expression, &allocator);
            }
            Some(UpdateOp::StyleMap(style)) => {
                try_collapse_interpolation(&mut style.expression, &allocator);
            }
            Some(UpdateOp::ClassMap(class)) => {
                try_collapse_interpolation(&mut class.expression, &allocator);
            }
            _ => {}
        }
    }
}

/// Collapses singleton interpolations for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn collapse_singleton_interpolations_for_host(job: &mut HostBindingCompilationJob<'_>) {
    collapse_in_view(&mut job.root.update, &job.allocator);
}

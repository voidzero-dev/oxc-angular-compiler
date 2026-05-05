//! Variadic pipe phase.
//!
//! Pipes that accept more than 4 arguments are variadic, and are handled with a different
//! runtime instruction.
//!
//! Ported from Angular's `template/pipeline/src/phases/pipe_variadic.ts`.

use oxc_allocator::{Box, Vec as ArenaVec};

use crate::ir::expression::{
    DerivedLiteralArrayExpr, IrExpression, PipeBindingVariadicExpr, VisitorContextFlag,
    transform_expressions_in_update_op,
};
use crate::ir::ops::XrefId;
use crate::pipeline::compilation::ComponentCompilationJob;

/// Maximum number of pipe arguments before switching to variadic form.
const MAX_PIPE_ARGS: usize = 4;

/// Creates variadic pipe bindings for pipes with many arguments.
///
/// This phase transforms PipeBinding expressions with more than 4 arguments
/// into PipeBindingVariadic expressions, which use a different runtime instruction.
pub fn create_variadic_pipes(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect all view xrefs
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            for op in view.update.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, _flags| transform_pipe_binding(expr, allocator),
                    VisitorContextFlag::NONE,
                );
            }
        }
    }
}

/// Transforms a PipeBinding with >4 args into PipeBindingVariadic.
fn transform_pipe_binding<'a>(
    expr: &mut IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
) {
    if let IrExpression::PipeBinding(pipe) = expr {
        // Pipes are variadic if they have more than 4 arguments
        if pipe.args.len() <= MAX_PIPE_ARGS {
            return;
        }

        let num_args = pipe.args.len() as u32;

        // Clone all arguments into a DerivedLiteralArray, preserving all IrExpression types
        let mut entries = ArenaVec::with_capacity_in(pipe.args.len(), allocator);
        let mut spreads = oxc_allocator::Vec::with_capacity_in(pipe.args.len(), allocator);
        for arg in pipe.args.iter() {
            entries.push(arg.clone_in(allocator));
            spreads.push(false);
        }

        // Create a DerivedLiteralArray to hold the arguments
        // This properly handles all IrExpression types, not just Ast
        let args_array = Box::new_in(
            IrExpression::DerivedLiteralArray(Box::new_in(
                DerivedLiteralArrayExpr { entries, spreads, source_span: pipe.source_span },
                allocator,
            )),
            allocator,
        );

        // Replace with variadic version
        *expr = IrExpression::PipeBindingVariadic(Box::new_in(
            PipeBindingVariadicExpr {
                target: pipe.target,
                target_slot: pipe.target_slot,
                name: pipe.name.clone(),
                args: args_array,
                num_args,
                var_offset: pipe.var_offset,
                source_span: pipe.source_span,
            },
            allocator,
        ));
    }
}

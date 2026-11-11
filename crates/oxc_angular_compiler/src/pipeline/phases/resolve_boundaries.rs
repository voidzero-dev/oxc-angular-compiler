//! Resolve boundaries phase.
//!
//! Replaces each `BoundaryStateExpr` with a read of the variable that holds the state of
//! that boundary (`const bnd_rN = ɵɵgetBoundary(slot)`, created by `generate_variables`).
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_boundaries.ts`.

use oxc_allocator::Box;
use rustc_hash::FxHashMap;

use crate::ir::enums::SemanticVariableKind;
use crate::ir::expression::{
    IrExpression, ReadVariableExpr, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Resolves `BoundaryStateExpr` to `ReadVariableExpr` in every view.
pub fn resolve_boundaries<'a>(job: &mut ComponentCompilationJob<'a>) {
    let allocator: &'a oxc_allocator::Allocator = job.allocator;

    for view in job.all_views_mut() {
        // Angular keeps one map per unit, filled in `unit.ops()` order: arrow function ops
        // first, then create ops (each followed by its listener or track ops), then update
        // ops. A later variable for the same boundary wins, so the update list's one does.
        let mut variables: FxHashMap<XrefId, XrefId> = FxHashMap::default();
        for func in &view.functions {
            // SAFETY: the pointers in `functions` point to arrow functions allocated in
            // the arena and owned by this view's ops.
            let func = unsafe { &**func };
            record_variables(func.ops.iter(), &mut variables);
        }
        for op in view.create.iter() {
            if let CreateOp::Variable(var) = op {
                record_variable(var.kind, &var.initializer, var.xref, &mut variables);
            } else if let Some(ops) = nested_ops(op) {
                record_variables(ops.iter(), &mut variables);
            }
        }
        record_variables(view.update.iter(), &mut variables);
        if variables.is_empty() {
            continue;
        }

        let transform = |expr: &mut IrExpression<'a>, _flags: VisitorContextFlag| {
            if let IrExpression::BoundaryState(state) = expr
                && let Some(&xref) = variables.get(&state.target)
            {
                *expr = IrExpression::ReadVariable(Box::new_in(
                    ReadVariableExpr { xref, name: None, source_span: None },
                    &allocator,
                ));
            }
        };

        // Arrow function ops are also reached through the ops that hold the functions.
        for func in &view.functions {
            // SAFETY: see above.
            let func = unsafe { &mut **func };
            for op in &mut func.ops {
                transform_expressions_in_update_op(op, &transform, VisitorContextFlag::NONE);
            }
        }
        for op in view.create.iter_mut() {
            transform_expressions_in_create_op(op, &transform, VisitorContextFlag::NONE);
        }
        for op in view.update.iter_mut() {
            transform_expressions_in_update_op(op, &transform, VisitorContextFlag::NONE);
        }
    }
}

fn record_variables<'a, 'b>(
    ops: impl Iterator<Item = &'b UpdateOp<'a>>,
    variables: &mut FxHashMap<XrefId, XrefId>,
) where
    'a: 'b,
{
    for op in ops {
        if let UpdateOp::Variable(var) = op {
            record_variable(var.kind, &var.initializer, var.xref, variables);
        }
    }
}

fn record_variable(
    kind: SemanticVariableKind,
    initializer: &IrExpression<'_>,
    xref: XrefId,
    variables: &mut FxHashMap<XrefId, XrefId>,
) {
    if kind == SemanticVariableKind::BoundaryState
        && let IrExpression::GetBoundary(get) = initializer
    {
        variables.insert(get.target, xref);
    }
}

/// The ops nested in a create op: listener handler ops and track-by ops.
fn nested_ops<'b, 'a>(op: &'b CreateOp<'a>) -> Option<&'b oxc_allocator::Vec<'a, UpdateOp<'a>>> {
    match op {
        CreateOp::Listener(listener) => Some(&listener.handler_ops),
        CreateOp::TwoWayListener(listener) => Some(&listener.handler_ops),
        CreateOp::AnimationListener(listener) => Some(&listener.handler_ops),
        CreateOp::Animation(animation) => Some(&animation.handler_ops),
        CreateOp::RepeaterCreate(repeater) => repeater.track_by_ops.as_ref(),
        _ => None,
    }
}

//! Boundary conditions phase.
//!
//! Builds the expression that picks which branch of a `@boundary` block to render:
//!
//! ```text
//! bnd.error === null ? primarySlot : (when1 ? errorSlot1 : ... : fallbackSlot)
//! ```
//!
//! `-1` is used when no `@error` block is unconditional.
//!
//! Ported from Angular's `template/pipeline/src/phases/boundary_conditions.ts`.

use oxc_allocator::Box;
use oxc_str::Ident;
use rustc_hash::FxHashMap;

use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, LiteralPrimitive, LiteralValue, ParseSpan,
};
use crate::ir::expression::{
    BinaryExpr, BoundaryStateExpr, ConditionalCaseExpr, IrBinaryOperator, IrExpression,
    ResolvedPropertyReadExpr, SlotHandle, SlotLiteralExpr, TernaryExpr, VisitorContextFlag,
    transform_expressions_in_expression,
};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Sets `processed` on every `BoundaryUpdateOp`.
///
/// Inside a `when` condition, reads of the block's own `$error` aliases are replaced by
/// `bnd.error`. The conditions are kept on the op, as in Angular.
pub fn generate_boundary_conditions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    for view in job.all_views_mut() {
        // `$error` alias names of each @error view declared in this view.
        let mut error_names: FxHashMap<XrefId, std::vec::Vec<Ident<'_>>> = FxHashMap::default();
        for op in view.create.iter() {
            if let CreateOp::BoundaryErrorCreate(error) = op {
                error_names
                    .insert(error.xref, error.error_variable_names.iter().copied().collect());
            }
        }

        for op in view.update.iter_mut() {
            let UpdateOp::Boundary(boundary) = op else {
                continue;
            };
            let boundary_xref = boundary.target;

            let mut result = match boundary.conditions.iter().find(|c| c.expr.is_none()) {
                Some(fallback) => slot_literal(allocator, fallback),
                None => number_literal(allocator, -1.0),
            };

            for condition in boundary.conditions.iter_mut().rev() {
                let Some(expr) = condition.expr.as_mut() else {
                    continue;
                };
                if let Some(names) = error_names.get(&condition.target)
                    && !names.is_empty()
                {
                    transform_expressions_in_expression(
                        expr,
                        &|e, _| {
                            if let IrExpression::LexicalRead(read) = e
                                && names.contains(&read.name)
                            {
                                *e = boundary_error(allocator, boundary_xref);
                            }
                        },
                        VisitorContextFlag::NONE,
                    );
                }
                let condition_expr = Box::new_in(expr.clone_in(allocator), &allocator);
                result =
                    ternary(allocator, condition_expr, slot_literal(allocator, condition), result);
            }

            let no_error = Box::new_in(
                IrExpression::Binary(Box::new_in(
                    BinaryExpr {
                        operator: IrBinaryOperator::Identical,
                        lhs: Box::new_in(boundary_error(allocator, boundary_xref), &allocator),
                        rhs: null_literal(allocator),
                        source_span: None,
                    },
                    &allocator,
                )),
                &allocator,
            );
            let primary = slot_literal(allocator, &boundary.guarded);
            boundary.processed = Some(ternary(allocator, no_error, primary, result));
        }
    }
}

/// `bnd.error` for the given boundary.
fn boundary_error(allocator: &oxc_allocator::Allocator, target: XrefId) -> IrExpression<'_> {
    let state = IrExpression::BoundaryState(Box::new_in(
        BoundaryStateExpr { target, target_slot: SlotHandle::new() },
        &allocator,
    ));
    IrExpression::ResolvedPropertyRead(Box::new_in(
        ResolvedPropertyReadExpr {
            receiver: Box::new_in(state, &allocator),
            name: Ident::from("error"),
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}

fn slot_literal<'a>(
    allocator: &'a oxc_allocator::Allocator,
    case: &ConditionalCaseExpr<'a>,
) -> Box<'a, IrExpression<'a>> {
    Box::new_in(
        IrExpression::SlotLiteral(Box::new_in(
            SlotLiteralExpr {
                slot: case.target_slot,
                target_xref: Some(case.target),
                source_span: None,
            },
            &allocator,
        )),
        &allocator,
    )
}

fn ternary<'a>(
    allocator: &'a oxc_allocator::Allocator,
    condition: Box<'a, IrExpression<'a>>,
    true_expr: Box<'a, IrExpression<'a>>,
    false_expr: Box<'a, IrExpression<'a>>,
) -> Box<'a, IrExpression<'a>> {
    Box::new_in(
        IrExpression::Ternary(Box::new_in(
            TernaryExpr { condition, true_expr, false_expr, source_span: None },
            &allocator,
        )),
        &allocator,
    )
}

fn literal<'a>(
    allocator: &'a oxc_allocator::Allocator,
    value: LiteralValue<'a>,
) -> Box<'a, IrExpression<'a>> {
    Box::new_in(
        IrExpression::Ast(Box::new_in(
            AngularExpression::LiteralPrimitive(Box::new_in(
                LiteralPrimitive {
                    span: ParseSpan::new(0, 0),
                    source_span: AbsoluteSourceSpan::new(0, 0),
                    value,
                },
                &allocator,
            )),
            &allocator,
        )),
        &allocator,
    )
}

fn number_literal(allocator: &oxc_allocator::Allocator, value: f64) -> Box<'_, IrExpression<'_>> {
    literal(allocator, LiteralValue::Number(value))
}

fn null_literal(allocator: &oxc_allocator::Allocator) -> Box<'_, IrExpression<'_>> {
    literal(allocator, LiteralValue::Null)
}

//! Conditional expressions phase.
//!
//! Generates conditional expressions for @if/@switch blocks.
//!
//! This phase collapses the various conditions of conditional ops (if, switch)
//! into a single test expression that can be used at runtime.
//!
//! For @if (condition) { ... } @else { ... }:
//!   Builds: condition ? slot0 : slot1
//!
//! For @switch (test) { @case (val1) {...} @case (val2) {...} @default {...} }:
//!   Builds: test === val1 ? slot0 : test === val2 ? slot1 : default_slot
//!
//! Ported from Angular's `template/pipeline/src/phases/conditionals.ts`.

use oxc_allocator::Box;

use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, LiteralPrimitive, LiteralValue as AstLiteralValue,
    ParseSpan,
};
use crate::ir::expression::{
    AssignTemporaryExpr, BinaryExpr, ConditionalCaseExpr, IrBinaryOperator, IrExpression,
    ReadTemporaryExpr, SlotHandle, SlotLiteralExpr,
};
use crate::ir::ops::{UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Generates conditional ternary expressions for control flow.
///
/// This phase processes all Conditional UPDATE ops and:
/// 1. Finds any default case (condition with None expression)
/// 2. Builds a nested ternary expression from all conditions
/// 3. Stores the result in `processed`
///
/// Ported from Angular's `generateConditionalExpressions` in `conditionals.ts`.
pub fn generate_conditional_expressions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect view xrefs to process
    let view_xrefs: std::vec::Vec<_> = job.all_views().map(|v| v.xref).collect();

    for xref in view_xrefs {
        // Pre-allocate xref IDs needed for @switch temporaries and alias temporaries
        // We need to do this outside the view borrow
        let (switch_temp_xrefs, alias_temp_xrefs): (std::vec::Vec<_>, std::vec::Vec<_>) = {
            if let Some(view) = job.view(xref) {
                let mut switch_xrefs = std::vec::Vec::new();
                let mut alias_xrefs = std::vec::Vec::new();

                for op in view.update.iter() {
                    if let UpdateOp::Conditional(cond) = op {
                        // Need temp xref for @switch (has test expression)
                        if cond.test.is_some() {
                            switch_xrefs.push(true);
                        }
                        // Need temp xref for alias capture
                        if cond.conditions.iter().any(|c| c.alias.is_some()) {
                            alias_xrefs.push(true);
                        }
                    }
                }
                (switch_xrefs, alias_xrefs)
            } else {
                (std::vec::Vec::new(), std::vec::Vec::new())
            }
        };

        // Allocate xref IDs for switch temporaries and alias temporaries
        let allocated_switch_xrefs: std::vec::Vec<_> =
            switch_temp_xrefs.iter().map(|_| job.allocate_xref_id()).collect();
        let allocated_alias_xrefs: std::vec::Vec<_> =
            alias_temp_xrefs.iter().map(|_| job.allocate_xref_id()).collect();

        if let Some(view) = job.view_mut(xref) {
            let mut switch_xref_idx = 0;
            let mut alias_xref_idx = 0;

            // Process conditional ops in this view's update list
            for op in view.update.iter_mut() {
                if let UpdateOp::Conditional(cond) = op {
                    // Get pre-allocated xref for temporary if needed for @switch
                    let temp_xref = if cond.test.is_some() {
                        let xid = allocated_switch_xrefs.get(switch_xref_idx).copied();
                        switch_xref_idx += 1;
                        xid
                    } else {
                        None
                    };

                    // Get pre-allocated xref for alias capture if needed
                    let alias_xref = if cond.conditions.iter().any(|c| c.alias.is_some()) {
                        let xid = allocated_alias_xrefs.get(alias_xref_idx).copied();
                        alias_xref_idx += 1;
                        xid
                    } else {
                        None
                    };

                    // Build the conditional expression
                    let (processed, context_value) = build_conditional_expression(
                        &allocator,
                        &mut cond.conditions,
                        cond.test.as_ref(),
                        temp_xref,
                        alias_xref,
                    );
                    cond.processed = Some(processed);
                    if context_value.is_some() {
                        cond.context_value = context_value;
                    }

                    // Clear the original conditions array, since we no longer need it, and don't
                    // want it to affect subsequent phases (e.g. pipe creation).
                    cond.conditions.clear();
                }
            }
        }
    }
}

/// Builds a nested conditional expression from conditions.
///
/// For @if: condition ? slot0 : slot1
/// For @switch: test === case1 ? slot0 : test === case2 ? slot1 : default
///
/// Returns the processed expression and optional context value for alias capture.
fn build_conditional_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    conditions: &mut [ConditionalCaseExpr<'a>],
    test: Option<&Box<'a, IrExpression<'a>>>,
    temp_xref: Option<XrefId>,
    alias_xref: Option<XrefId>,
) -> (Box<'a, IrExpression<'a>>, Option<Box<'a, IrExpression<'a>>>) {
    if conditions.is_empty() {
        // Empty conditional - return -1 (no template)
        return (create_literal_number(allocator, -1.0), None);
    }

    // Find the default case (condition with None expression) and splice it out
    let mut default_idx: Option<usize> = None;
    for (idx, cond) in conditions.iter().enumerate() {
        if cond.expr.is_none() {
            default_idx = Some(idx);
            break;
        }
    }

    // Start with the default case (or -1 if none)
    let mut result: Box<'a, IrExpression<'a>> = if let Some(idx) = default_idx {
        let cond = &conditions[idx];
        create_slot_literal_from_handle(allocator, &cond.target_slot, cond.target)
    } else {
        create_literal_number(allocator, -1.0)
    };

    // Track context value for alias capture
    let mut context_value: Option<Box<'a, IrExpression<'a>>> = None;

    // Build nested conditionals from right to left, skipping the default case
    let non_default_conditions: std::vec::Vec<_> =
        conditions.iter_mut().enumerate().filter(|(idx, _)| Some(*idx) != default_idx).collect();

    let total = non_default_conditions.len();
    for (i, (_, cond)) in non_default_conditions.into_iter().rev().enumerate() {
        let branch_slot =
            create_slot_literal_from_handle(allocator, &cond.target_slot, cond.target);

        // Build the condition expression
        let condition = if let Some(ref mut cond_expr) = cond.expr {
            if let Some(switch_test) = test {
                // @switch case: test === case_value
                // First case (in reverse order, so last in iteration) uses AssignTemporary
                let is_first = i == total - 1;
                let use_tmp = if is_first {
                    if let Some(xref) = temp_xref {
                        // Assign test to temporary
                        Box::new_in(
                            IrExpression::AssignTemporary(Box::new_in(
                                AssignTemporaryExpr {
                                    expr: Box::new_in(switch_test.clone_in(allocator), &allocator),
                                    xref,
                                    name: None,
                                    source_span: None,
                                },
                                &allocator,
                            )),
                            &allocator,
                        )
                    } else {
                        Box::new_in(switch_test.clone_in(allocator), &allocator)
                    }
                } else if let Some(xref) = temp_xref {
                    // Read from temporary
                    Box::new_in(
                        IrExpression::ReadTemporary(Box::new_in(
                            ReadTemporaryExpr { xref, name: None, source_span: None },
                            &allocator,
                        )),
                        &allocator,
                    )
                } else {
                    Box::new_in(switch_test.clone_in(allocator), &allocator)
                };

                // Build: test === case_value
                build_equality_check(
                    &allocator,
                    use_tmp,
                    Box::new_in(cond_expr.clone_in(allocator), &allocator),
                )
            } else {
                // @if case: handle alias capture
                if cond.alias.is_some() {
                    if let Some(xref) = alias_xref {
                        // Wrap the expression in AssignTemporary for alias capture
                        let assign_expr = Box::new_in(
                            IrExpression::AssignTemporary(Box::new_in(
                                AssignTemporaryExpr {
                                    expr: Box::new_in(cond_expr.clone_in(allocator), &allocator),
                                    xref,
                                    name: None,
                                    source_span: None,
                                },
                                &allocator,
                            )),
                            &allocator,
                        );

                        // Set context value to read from the temporary
                        if context_value.is_none() {
                            context_value = Some(Box::new_in(
                                IrExpression::ReadTemporary(Box::new_in(
                                    ReadTemporaryExpr { xref, name: None, source_span: None },
                                    &allocator,
                                )),
                                &allocator,
                            ));
                        }

                        // Update the condition's expression to the assignment
                        *cond_expr = assign_expr;
                        Box::new_in(cond_expr.clone_in(allocator), &allocator)
                    } else {
                        Box::new_in(cond_expr.clone_in(allocator), &allocator)
                    }
                } else {
                    Box::new_in(cond_expr.clone_in(allocator), &allocator)
                }
            }
        } else {
            // Default case - should not reach here as we filter above
            continue;
        };

        // Build: condition ? branch_slot : result
        result = build_ternary(allocator, condition, branch_slot, result);
    }

    (result, context_value)
}

/// Creates a slot literal expression from a SlotHandle with target xref.
fn create_slot_literal_from_handle<'a>(
    allocator: &'a oxc_allocator::Allocator,
    handle: &SlotHandle,
    target_xref: XrefId,
) -> Box<'a, IrExpression<'a>> {
    Box::new_in(
        IrExpression::SlotLiteral(Box::new_in(
            SlotLiteralExpr {
                slot: handle.clone(),
                target_xref: Some(target_xref),
                source_span: None,
            },
            &allocator,
        )),
        &allocator,
    )
}

/// Creates a literal number expression.
fn create_literal_number<'a>(
    allocator: &'a oxc_allocator::Allocator,
    value: f64,
) -> Box<'a, IrExpression<'a>> {
    let span = ParseSpan::new(0, 0);
    let source_span = AbsoluteSourceSpan::new(0, 0);
    Box::new_in(
        IrExpression::Ast(Box::new_in(
            AngularExpression::LiteralPrimitive(Box::new_in(
                LiteralPrimitive { span, source_span, value: AstLiteralValue::Number(value) },
                &allocator,
            )),
            &allocator,
        )),
        &allocator,
    )
}

/// Builds an equality check for @switch: test === case_value
///
/// Uses IrExpression::Binary to keep the expression in IR form so that
/// temporaries can be properly resolved during the reify phase.
fn build_equality_check<'a>(
    allocator: &'a oxc_allocator::Allocator,
    test: Box<'a, IrExpression<'a>>,
    case_value: Box<'a, IrExpression<'a>>,
) -> Box<'a, IrExpression<'a>> {
    // Build: test === case_value using IR binary expression
    Box::new_in(
        IrExpression::Binary(Box::new_in(
            BinaryExpr {
                operator: IrBinaryOperator::Identical,
                lhs: test,
                rhs: case_value,
                source_span: None,
            },
            &allocator,
        )),
        &allocator,
    )
}

/// Build a ternary expression: condition ? true_case : false_case
///
/// Uses IrExpression::Ternary to keep the expression in IR form so that
/// slot literals can be updated by allocate_slots phase.
fn build_ternary<'a>(
    allocator: &'a oxc_allocator::Allocator,
    condition: Box<'a, IrExpression<'a>>,
    true_case: Box<'a, IrExpression<'a>>,
    false_case: Box<'a, IrExpression<'a>>,
) -> Box<'a, IrExpression<'a>> {
    Box::new_in(
        IrExpression::Ternary(Box::new_in(
            crate::ir::expression::TernaryExpr {
                condition,
                true_expr: true_case,
                false_expr: false_case,
                source_span: None,
            },
            &allocator,
        )),
        &allocator,
    )
}

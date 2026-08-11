//! Const expression collection phase.
//!
//! `ir.ConstCollectedExpr` may be present in any IR expression. This means that
//! expression needs to be lifted into the component const array, and replaced
//! with a reference to the const array at its usage site.
//!
//! This phase walks the IR and performs this transformation.
//!
//! Ported from Angular's `template/pipeline/src/phases/has_const_expression_collection.ts`.

use std::cell::RefCell;

use oxc_allocator::Box;

use crate::ir::expression::{
    ConstReferenceExpr, IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::ir::ops::XrefId;
use crate::pipeline::compilation::{ComponentCompilationJob, ConstValue, ViewCompilationUnit};
use crate::pipeline::phases::reify::ir_expression::convert_ir_expression;

/// Collects constant expressions to the consts array.
///
/// This phase:
/// 1. Finds all ConstCollectedExpr in the IR
/// 2. Adds the wrapped expression to the const array
/// 3. Replaces ConstCollectedExpr with ConstReference(index)
///
/// Implementation approach:
/// We use a two-pass approach to allow adding consts during transformation.
/// Pass 1: Transform ConstCollectedExpr to ConstReference, collecting expressions
/// Pass 2: Add collected expressions to const pool (indices are pre-allocated)
pub fn collect_const_expressions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;

    // Collect expressions during transformation using RefCell for interior mutability
    // We store (inner_expression, assigned_index) pairs
    let collected: RefCell<Vec<(IrExpression<'_>, u32)>> = RefCell::new(Vec::new());
    let next_index: RefCell<u32> = RefCell::new(job.consts.len() as u32);

    // ========================================================================
    // Pass 1: Transform ConstCollectedExpr to ConstReference, collecting inner expressions
    // ========================================================================
    // Angular's ops() generator iterates in this order:
    // 1. First: this.functions - ArrowFunctionExpr ops
    // 2. Then: this.create - Create ops
    // 3. Finally: this.update - Update ops
    //
    // We must match this order exactly for consistent const indices.

    // Collect view xrefs first to avoid borrow issues
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    // Process root view first
    process_view_const_expressions(&mut job.root, &allocator, &collected, &next_index);

    // Process embedded views
    for xref in view_xrefs {
        if xref != root_xref {
            if let Some(view) = job.views.get_mut(&xref) {
                process_view_const_expressions(view.as_mut(), &allocator, &collected, &next_index);
            }
        }
    }

    // ========================================================================
    // Pass 2: Add collected expressions to const pool
    // ========================================================================
    let mut collected = collected.into_inner();
    if collected.is_empty() {
        return;
    }

    // Sort by index to ensure they're added in the right order
    collected.sort_by_key(|(_, idx)| *idx);

    for (inner_expr, _index) in collected {
        // Convert IrExpression to OutputExpression
        let output_expr =
            convert_ir_expression(allocator, &inner_expr, &job.expressions, root_xref);

        // Add to const pool - indices should match what we pre-allocated
        job.add_const(ConstValue::Expression(output_expr));
    }
}

/// Process a single view's operations for const expression collection.
///
/// Angular's ops() generator iterates in this order:
/// 1. First: this.functions - ArrowFunctionExpr ops
/// 2. Then: this.create - Create ops
/// 3. Finally: this.update - Update ops
fn process_view_const_expressions<'a>(
    view: &mut ViewCompilationUnit<'a>,
    allocator: &'a oxc_allocator::Allocator,
    collected: &RefCell<Vec<(IrExpression<'a>, u32)>>,
    next_index: &RefCell<u32>,
) {
    // Transform function operations FIRST (matching Angular's ops() order)
    for func_ptr in view.functions.iter() {
        // SAFETY: function pointers are valid and point to ArrowFunctionExpr in the allocator
        let func = unsafe { &mut **func_ptr };
        for op in func.ops.iter_mut() {
            transform_expressions_in_update_op(
                op,
                &|expr, _flags| {
                    collect_const_expr(expr, &allocator, collected, next_index);
                },
                VisitorContextFlag::NONE,
            );
        }
    }

    // Transform create operations
    for op in view.create.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| {
                collect_const_expr(expr, &allocator, collected, next_index);
            },
            VisitorContextFlag::NONE,
        );
    }

    // Transform update operations
    for op in view.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                collect_const_expr(expr, &allocator, collected, next_index);
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Collect a single ConstCollectedExpr, replacing it with ConstReference.
fn collect_const_expr<'a>(
    expr: &mut IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    collected: &RefCell<Vec<(IrExpression<'a>, u32)>>,
    next_index: &RefCell<u32>,
) {
    if let IrExpression::ConstCollected(cc) = expr {
        // Allocate the next index
        let index = {
            let mut idx = next_index.borrow_mut();
            let current = *idx;
            *idx += 1;
            current
        };

        // Clone the inner expression for later processing
        let inner = cc.expr.clone_in(allocator);
        collected.borrow_mut().push((inner, index));

        // Replace with ConstReference
        let source_span = cc.source_span;
        *expr = IrExpression::ConstReference(Box::new_in(
            ConstReferenceExpr { index, source_span },
            &allocator,
        ));
    }
}

//! Store let optimization phase.
//!
//! Optimizes @let storage operations to minimize runtime overhead.
//!
//! This phase analyzes @let declarations and their usage patterns to:
//! 1. Remove the `ɵɵstoreLet` call for @let values not used in other views
//! 2. Remove the `ɵɵdeclareLet` call only if the value has no pipes (pipes need DI/TNode)
//!
//! Per Angular's comment in store_let_optimization.ts:
//! "We need to keep the declareLet if there are pipes, because they can use DI which
//!  requires the TNode created by declareLet."
//!
//! Ported from Angular's `template/pipeline/src/phases/store_let_optimization.ts`.

use std::collections::HashSet;
use std::ptr::NonNull;

use rustc_hash::FxHashMap;

use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Optimizes @let declaration storage.
///
/// This phase:
/// 1. Finds all `ContextLetReferenceExpr` usages (indicates cross-view usage)
/// 2. Finds all `StoreLetExpr` in update ops
/// 3. For @let not used externally:
///    - Replace `StoreLetExpr` with just its value (removes runtime storage call)
///    - If no pipes in value: also remove the `DeclareLetOp`
pub fn optimize_store_let(job: &mut ComponentCompilationJob<'_>) {
    // Phase 1: Collect which @let declarations are used externally (via ContextLetReferenceExpr)
    // and collect DeclareLetOp references
    let let_used_externally: HashSet<XrefId> = collect_context_let_references(job);
    let declare_let_ops: FxHashMap<XrefId, DeclareLetOpInfo> = collect_declare_let_ops(job);

    // Phase 2: Transform StoreLetExpr expressions in update ops
    // For @let not used externally:
    // - Replace StoreLetExpr with just its value
    // - If no pipes, remove the DeclareLetOp

    // Collect which DeclareLetOps need to be removed
    let mut declare_lets_to_remove: HashSet<XrefId> = HashSet::new();

    // Process all views
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();
    for view_xref in &view_xrefs {
        transform_store_let_exprs_in_view(
            job,
            *view_xref,
            &let_used_externally,
            &mut declare_lets_to_remove,
        );
    }

    // Phase 3: Remove DeclareLetOps that were marked for removal
    remove_declare_let_ops(job, &declare_let_ops, &declare_lets_to_remove);
}

/// Information about a DeclareLetOp.
struct DeclareLetOpInfo {
    view_xref: XrefId,
}

/// Collect all ContextLetReferenceExpr targets across all ops.
/// These represent @let values that are used externally (cross-view).
fn collect_context_let_references(job: &ComponentCompilationJob<'_>) -> HashSet<XrefId> {
    let mut used_externally: HashSet<XrefId> = HashSet::new();

    for view in job.all_views() {
        // Check create ops (including listener handler ops)
        for op in view.create.iter() {
            visit_expressions_in_create_op(op, &mut |expr| {
                collect_context_let_refs_in_expr(expr, &mut used_externally);
            });
        }

        // Check update ops
        for op in view.update.iter() {
            visit_expressions_in_update_op(op, &mut |expr| {
                collect_context_let_refs_in_expr(expr, &mut used_externally);
            });
        }
    }

    used_externally
}

/// Collect all DeclareLetOp references for later removal.
fn collect_declare_let_ops(
    job: &ComponentCompilationJob<'_>,
) -> FxHashMap<XrefId, DeclareLetOpInfo> {
    let mut ops: FxHashMap<XrefId, DeclareLetOpInfo> = FxHashMap::default();

    for view in job.all_views() {
        for op in view.create.iter() {
            if let CreateOp::DeclareLet(decl) = op {
                ops.insert(decl.xref, DeclareLetOpInfo { view_xref: view.xref });
            }
        }
    }

    ops
}

/// Transform StoreLetExpr expressions in a view's update ops.
fn transform_store_let_exprs_in_view(
    job: &mut ComponentCompilationJob<'_>,
    view_xref: XrefId,
    let_used_externally: &HashSet<XrefId>,
    declare_lets_to_remove: &mut HashSet<XrefId>,
) {
    let allocator = job.allocator;

    let view = if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(view_xref) };
    let Some(view) = view else {
        return;
    };

    // Transform expressions in each update op
    for op in view.update.iter_mut() {
        transform_store_let_in_update_op(
            allocator,
            op,
            let_used_externally,
            declare_lets_to_remove,
        );
    }
}

/// Transform StoreLetExpr in a single update op.
fn transform_store_let_in_update_op<'a>(
    allocator: &'a oxc_allocator::Allocator,
    op: &mut UpdateOp<'a>,
    let_used_externally: &HashSet<XrefId>,
    declare_lets_to_remove: &mut HashSet<XrefId>,
) {
    match op {
        UpdateOp::Variable(var) => {
            transform_store_let_in_expr(
                allocator,
                &mut var.initializer,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        UpdateOp::Property(prop) => {
            transform_store_let_in_expr(
                allocator,
                &mut prop.expression,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        UpdateOp::StyleProp(style) => {
            transform_store_let_in_expr(
                allocator,
                &mut style.expression,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        UpdateOp::ClassProp(class) => {
            transform_store_let_in_expr(
                allocator,
                &mut class.expression,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        UpdateOp::Attribute(attr) => {
            transform_store_let_in_expr(
                allocator,
                &mut attr.expression,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        UpdateOp::InterpolateText(text) => {
            transform_store_let_in_expr(
                allocator,
                &mut text.interpolation,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        UpdateOp::Conditional(cond) => {
            if let Some(ref mut test) = cond.test {
                transform_store_let_in_expr(
                    allocator,
                    test,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
            for condition in cond.conditions.iter_mut() {
                if let Some(ref mut expr) = condition.expr {
                    transform_store_let_in_expr(
                        allocator,
                        expr,
                        let_used_externally,
                        declare_lets_to_remove,
                    );
                }
            }
            if let Some(ref mut processed) = cond.processed {
                transform_store_let_in_expr(
                    allocator,
                    processed,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
            if let Some(ref mut ctx_val) = cond.context_value {
                transform_store_let_in_expr(
                    allocator,
                    ctx_val,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        // Other update ops don't contain expressions that need transformation
        _ => {}
    }
}

/// Transform StoreLetExpr in a boxed expression.
fn transform_store_let_in_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut oxc_allocator::Box<'a, IrExpression<'a>>,
    let_used_externally: &HashSet<XrefId>,
    declare_lets_to_remove: &mut HashSet<XrefId>,
) {
    // Check if this is a StoreLetExpr that should be optimized
    if let IrExpression::StoreLet(store_let) = expr.as_ref() {
        if !let_used_externally.contains(&store_let.target) {
            // This @let is not used externally - can optimize

            // Check if value has pipes - if so, keep DeclareLetOp (needed for DI/TNode)
            if !has_pipe(&store_let.value) {
                declare_lets_to_remove.insert(store_let.target);
            }

            // Replace StoreLetExpr with just its value
            let value = (*store_let.value).clone_in(allocator);
            **expr = value;

            // Continue transforming the replaced value
            transform_store_let_in_expr(
                allocator,
                expr,
                let_used_externally,
                declare_lets_to_remove,
            );
            return;
        }
    }

    // Recursively transform nested expressions
    transform_nested_expressions(allocator, expr, let_used_externally, declare_lets_to_remove);
}

/// Transform StoreLetExpr in a non-boxed expression (mutates in place).
fn transform_store_let_in_expr_value<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut IrExpression<'a>,
    let_used_externally: &HashSet<XrefId>,
    declare_lets_to_remove: &mut HashSet<XrefId>,
) {
    // Check if this is a StoreLetExpr that should be optimized
    if let IrExpression::StoreLet(store_let) = expr {
        if !let_used_externally.contains(&store_let.target) {
            // This @let is not used externally - can optimize

            // Check if value has pipes - if so, keep DeclareLetOp
            if !has_pipe(&store_let.value) {
                declare_lets_to_remove.insert(store_let.target);
            }

            // Replace StoreLetExpr with just its value
            let value = (*store_let.value).clone_in(allocator);
            *expr = value;

            // Continue transforming the replaced value
            transform_store_let_in_expr_value(
                allocator,
                expr,
                let_used_externally,
                declare_lets_to_remove,
            );
            return;
        }
    }

    // Recursively transform nested expressions
    transform_nested_in_expr_value(allocator, expr, let_used_externally, declare_lets_to_remove);
}

/// Transform nested expressions within a boxed expression.
fn transform_nested_expressions<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut oxc_allocator::Box<'a, IrExpression<'a>>,
    let_used_externally: &HashSet<XrefId>,
    declare_lets_to_remove: &mut HashSet<XrefId>,
) {
    match expr.as_mut() {
        IrExpression::SafeNavigationMigration(m) => {
            transform_store_let_in_expr_value(
                allocator,
                &mut m.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::PureFunction(pf) => {
            if let Some(ref mut body) = pf.body {
                transform_store_let_in_expr(
                    allocator,
                    body,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
            if let Some(ref mut fn_ref) = pf.fn_ref {
                transform_store_let_in_expr(
                    allocator,
                    fn_ref,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
            for arg in pf.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::Interpolation(interp) => {
            for expr in interp.expressions.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    expr,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            transform_store_let_in_expr(
                allocator,
                &mut pbv.args,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::SafePropertyRead(spr) => {
            transform_store_let_in_expr(
                allocator,
                &mut spr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::SafeKeyedRead(skr) => {
            transform_store_let_in_expr(
                allocator,
                &mut skr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut skr.index,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::SafeInvokeFunction(sif) => {
            transform_store_let_in_expr(
                allocator,
                &mut sif.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            for arg in sif.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::SafeTernary(st) => {
            transform_store_let_in_expr(
                allocator,
                &mut st.guard,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut st.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Ternary(t) => {
            transform_store_let_in_expr(
                allocator,
                &mut t.condition,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut t.true_expr,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut t.false_expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::AssignTemporary(at) => {
            transform_store_let_in_expr(
                allocator,
                &mut at.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref mut e) = cc.expr {
                transform_store_let_in_expr(
                    allocator,
                    e,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::ConstCollected(cc) => {
            transform_store_let_in_expr(
                allocator,
                &mut cc.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::TwoWayBindingSet(twb) => {
            transform_store_let_in_expr(
                allocator,
                &mut twb.target,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut twb.value,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::StoreLet(sl) => {
            // If we get here, this StoreLet is used externally, but still transform its value
            transform_store_let_in_expr(
                allocator,
                &mut sl.value,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Binary(binary) => {
            transform_store_let_in_expr(
                allocator,
                &mut binary.lhs,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut binary.rhs,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            transform_store_let_in_expr(
                allocator,
                &mut rpr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedBinary(rb) => {
            transform_store_let_in_expr(
                allocator,
                &mut rb.left,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut rb.right,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedCall(rc) => {
            transform_store_let_in_expr(
                allocator,
                &mut rc.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            for arg in rc.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            transform_store_let_in_expr(
                allocator,
                &mut rkr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut rkr.key,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            transform_store_let_in_expr(
                allocator,
                &mut rspr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref mut inner) = rv.view {
                transform_store_let_in_expr(
                    allocator,
                    inner,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::ResetView(rv) => {
            transform_store_let_in_expr(
                allocator,
                &mut rv.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    entry,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    value,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        // Leaf expressions - no nested expressions to transform
        IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::ReadVariable(_) => {}

        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    elem,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }

        IrExpression::LiteralMap(map) => {
            for value in map.values.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    value,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::Not(n) => {
            transform_store_let_in_expr(
                allocator,
                &mut n.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Unary(u) => {
            transform_store_let_in_expr(
                allocator,
                &mut u.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Typeof(t) => {
            transform_store_let_in_expr(
                allocator,
                &mut t.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Void(v) => {
            transform_store_let_in_expr(
                allocator,
                &mut v.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    e,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            transform_store_let_in_expr_value(
                allocator,
                &mut arrow_fn.body,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Parenthesized(paren) => {
            transform_store_let_in_expr_value(
                allocator,
                &mut paren.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
    }
}

/// Transform nested expressions within a non-boxed expression.
fn transform_nested_in_expr_value<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut IrExpression<'a>,
    let_used_externally: &HashSet<XrefId>,
    declare_lets_to_remove: &mut HashSet<XrefId>,
) {
    match expr {
        IrExpression::SafeNavigationMigration(m) => {
            transform_store_let_in_expr_value(
                allocator,
                &mut m.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::PureFunction(pf) => {
            if let Some(ref mut body) = pf.body {
                transform_store_let_in_expr(
                    allocator,
                    body,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
            if let Some(ref mut fn_ref) = pf.fn_ref {
                transform_store_let_in_expr(
                    allocator,
                    fn_ref,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
            for arg in pf.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::Interpolation(interp) => {
            for expr in interp.expressions.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    expr,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            transform_store_let_in_expr(
                allocator,
                &mut pbv.args,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::SafePropertyRead(spr) => {
            transform_store_let_in_expr(
                allocator,
                &mut spr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::SafeKeyedRead(skr) => {
            transform_store_let_in_expr(
                allocator,
                &mut skr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut skr.index,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::SafeInvokeFunction(sif) => {
            transform_store_let_in_expr(
                allocator,
                &mut sif.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            for arg in sif.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::SafeTernary(st) => {
            transform_store_let_in_expr(
                allocator,
                &mut st.guard,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut st.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Ternary(t) => {
            transform_store_let_in_expr(
                allocator,
                &mut t.condition,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut t.true_expr,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut t.false_expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::AssignTemporary(at) => {
            transform_store_let_in_expr(
                allocator,
                &mut at.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref mut e) = cc.expr {
                transform_store_let_in_expr(
                    allocator,
                    e,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::ConstCollected(cc) => {
            transform_store_let_in_expr(
                allocator,
                &mut cc.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::TwoWayBindingSet(twb) => {
            transform_store_let_in_expr(
                allocator,
                &mut twb.target,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut twb.value,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::StoreLet(sl) => {
            // If we get here, this StoreLet is used externally, but still transform its value
            transform_store_let_in_expr(
                allocator,
                &mut sl.value,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Binary(binary) => {
            transform_store_let_in_expr(
                allocator,
                &mut binary.lhs,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut binary.rhs,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            transform_store_let_in_expr(
                allocator,
                &mut rpr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedBinary(rb) => {
            transform_store_let_in_expr(
                allocator,
                &mut rb.left,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut rb.right,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedCall(rc) => {
            transform_store_let_in_expr(
                allocator,
                &mut rc.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            for arg in rc.args.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    arg,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            transform_store_let_in_expr(
                allocator,
                &mut rkr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
            transform_store_let_in_expr(
                allocator,
                &mut rkr.key,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            transform_store_let_in_expr(
                allocator,
                &mut rspr.receiver,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref mut inner) = rv.view {
                transform_store_let_in_expr(
                    allocator,
                    inner,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::ResetView(rv) => {
            transform_store_let_in_expr(
                allocator,
                &mut rv.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    entry,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    value,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    elem,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    value,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }
        // Leaf expressions - no nested expressions to transform
        IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::ReadVariable(_) => {}
        IrExpression::Not(n) => {
            transform_store_let_in_expr_value(
                allocator,
                n.expr.as_mut(),
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Unary(u) => {
            transform_store_let_in_expr_value(
                allocator,
                u.expr.as_mut(),
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Typeof(t) => {
            transform_store_let_in_expr_value(
                allocator,
                t.expr.as_mut(),
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Void(v) => {
            transform_store_let_in_expr_value(
                allocator,
                v.expr.as_mut(),
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter_mut() {
                transform_store_let_in_expr_value(
                    allocator,
                    e,
                    let_used_externally,
                    declare_lets_to_remove,
                );
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            transform_store_let_in_expr_value(
                allocator,
                &mut arrow_fn.body,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
        IrExpression::Parenthesized(paren) => {
            transform_store_let_in_expr_value(
                allocator,
                &mut paren.expr,
                let_used_externally,
                declare_lets_to_remove,
            );
        }
    }
}

/// Check if an expression contains any pipe bindings.
fn has_pipe(expr: &IrExpression<'_>) -> bool {
    match expr {
        IrExpression::PipeBinding(_) | IrExpression::PipeBindingVariadic(_) => true,
        IrExpression::SafeNavigationMigration(m) => has_pipe(&m.expr),
        IrExpression::PureFunction(pf) => {
            pf.body.as_ref().is_some_and(|b| has_pipe(b))
                || pf.fn_ref.as_ref().is_some_and(|f| has_pipe(f))
                || pf.args.iter().any(has_pipe)
        }
        IrExpression::Interpolation(interp) => interp.expressions.iter().any(has_pipe),
        IrExpression::SafePropertyRead(spr) => has_pipe(&spr.receiver),
        IrExpression::SafeKeyedRead(skr) => has_pipe(&skr.receiver) || has_pipe(&skr.index),
        IrExpression::SafeInvokeFunction(sif) => {
            has_pipe(&sif.receiver) || sif.args.iter().any(has_pipe)
        }
        IrExpression::SafeTernary(st) => has_pipe(&st.guard) || has_pipe(&st.expr),
        IrExpression::Ternary(t) => {
            has_pipe(&t.condition) || has_pipe(&t.true_expr) || has_pipe(&t.false_expr)
        }
        IrExpression::AssignTemporary(at) => has_pipe(&at.expr),
        IrExpression::ConditionalCase(cc) => cc.expr.as_ref().is_some_and(|e| has_pipe(e)),
        IrExpression::ConstCollected(cc) => has_pipe(&cc.expr),
        IrExpression::TwoWayBindingSet(twb) => has_pipe(&twb.target) || has_pipe(&twb.value),
        IrExpression::StoreLet(sl) => has_pipe(&sl.value),
        IrExpression::Binary(binary) => has_pipe(&binary.lhs) || has_pipe(&binary.rhs),
        IrExpression::ResolvedPropertyRead(rpr) => has_pipe(&rpr.receiver),
        IrExpression::ResolvedBinary(rb) => has_pipe(&rb.left) || has_pipe(&rb.right),
        IrExpression::ResolvedCall(rc) => has_pipe(&rc.receiver) || rc.args.iter().any(has_pipe),
        IrExpression::ResolvedKeyedRead(rkr) => has_pipe(&rkr.receiver) || has_pipe(&rkr.key),
        IrExpression::ResolvedSafePropertyRead(rspr) => has_pipe(&rspr.receiver),
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref inner) = rv.view {
                has_pipe(inner)
            } else {
                false
            }
        }
        IrExpression::ResetView(rv) => has_pipe(&rv.expr),
        IrExpression::DerivedLiteralArray(arr) => arr.entries.iter().any(has_pipe),
        IrExpression::DerivedLiteralMap(map) => map.values.iter().any(has_pipe),
        IrExpression::LiteralArray(arr) => arr.elements.iter().any(has_pipe),
        IrExpression::LiteralMap(map) => map.values.iter().any(has_pipe),
        IrExpression::Not(n) => has_pipe(&n.expr),
        IrExpression::Unary(u) => has_pipe(&u.expr),
        IrExpression::Typeof(t) => has_pipe(&t.expr),
        IrExpression::Void(v) => has_pipe(&v.expr),
        IrExpression::ResolvedTemplateLiteral(rtl) => rtl.expressions.iter().any(has_pipe),
        // Leaf expressions don't contain pipes
        IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::ReadVariable(_) => false,

        // Arrow function: check the body
        IrExpression::ArrowFunction(arrow_fn) => has_pipe(&arrow_fn.body),
        IrExpression::Parenthesized(paren) => has_pipe(&paren.expr),
    }
}

/// Visit expressions in a create op.
fn visit_expressions_in_create_op<F>(op: &CreateOp<'_>, visitor: &mut F)
where
    F: FnMut(&IrExpression<'_>),
{
    match op {
        CreateOp::Variable(var) => {
            visitor(&var.initializer);
        }
        CreateOp::Listener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor);
            }
        }
        CreateOp::TwoWayListener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor);
            }
        }
        CreateOp::AnimationListener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor);
            }
        }
        _ => {}
    }
}

/// Visit expressions in an update op.
fn visit_expressions_in_update_op<F>(op: &UpdateOp<'_>, visitor: &mut F)
where
    F: FnMut(&IrExpression<'_>),
{
    match op {
        UpdateOp::Variable(var) => visitor(&var.initializer),
        UpdateOp::Property(prop) => visitor(&prop.expression),
        UpdateOp::StyleProp(style) => visitor(&style.expression),
        UpdateOp::ClassProp(class) => visitor(&class.expression),
        UpdateOp::Attribute(attr) => visitor(&attr.expression),
        UpdateOp::InterpolateText(text) => {
            visitor(&text.interpolation);
        }
        UpdateOp::Conditional(cond) => {
            if let Some(ref test) = cond.test {
                visitor(test);
            }
            for condition in cond.conditions.iter() {
                if let Some(ref expr) = condition.expr {
                    visitor(expr);
                }
            }
            if let Some(ref processed) = cond.processed {
                visitor(processed);
            }
            if let Some(ref ctx_val) = cond.context_value {
                visitor(ctx_val);
            }
        }
        _ => {}
    }
}

/// Recursively collect ContextLetReference targets from an expression.
fn collect_context_let_refs_in_expr(expr: &IrExpression<'_>, refs: &mut HashSet<XrefId>) {
    match expr {
        IrExpression::ContextLetReference(let_ref) => {
            refs.insert(let_ref.target);
        }
        IrExpression::SafeNavigationMigration(m) => {
            collect_context_let_refs_in_expr(&m.expr, refs);
        }
        IrExpression::PureFunction(pf) => {
            if let Some(ref body) = pf.body {
                collect_context_let_refs_in_expr(body, refs);
            }
            if let Some(ref fn_ref) = pf.fn_ref {
                collect_context_let_refs_in_expr(fn_ref, refs);
            }
            for arg in pf.args.iter() {
                collect_context_let_refs_in_expr(arg, refs);
            }
        }
        IrExpression::Interpolation(interp) => {
            for e in interp.expressions.iter() {
                collect_context_let_refs_in_expr(e, refs);
            }
        }
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref inner) = rv.view {
                collect_context_let_refs_in_expr(inner, refs);
            }
        }
        IrExpression::ResetView(rv) => {
            collect_context_let_refs_in_expr(&rv.expr, refs);
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter() {
                collect_context_let_refs_in_expr(arg, refs);
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            collect_context_let_refs_in_expr(&pbv.args, refs);
        }
        IrExpression::SafePropertyRead(spr) => {
            collect_context_let_refs_in_expr(&spr.receiver, refs);
        }
        IrExpression::SafeKeyedRead(skr) => {
            collect_context_let_refs_in_expr(&skr.receiver, refs);
            collect_context_let_refs_in_expr(&skr.index, refs);
        }
        IrExpression::SafeInvokeFunction(sif) => {
            collect_context_let_refs_in_expr(&sif.receiver, refs);
            for arg in sif.args.iter() {
                collect_context_let_refs_in_expr(arg, refs);
            }
        }
        IrExpression::SafeTernary(st) => {
            collect_context_let_refs_in_expr(&st.guard, refs);
            collect_context_let_refs_in_expr(&st.expr, refs);
        }
        IrExpression::Ternary(t) => {
            collect_context_let_refs_in_expr(&t.condition, refs);
            collect_context_let_refs_in_expr(&t.true_expr, refs);
            collect_context_let_refs_in_expr(&t.false_expr, refs);
        }
        IrExpression::AssignTemporary(at) => {
            collect_context_let_refs_in_expr(&at.expr, refs);
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref e) = cc.expr {
                collect_context_let_refs_in_expr(e, refs);
            }
        }
        IrExpression::ConstCollected(cc) => {
            collect_context_let_refs_in_expr(&cc.expr, refs);
        }
        IrExpression::TwoWayBindingSet(twb) => {
            collect_context_let_refs_in_expr(&twb.target, refs);
            collect_context_let_refs_in_expr(&twb.value, refs);
        }
        IrExpression::StoreLet(sl) => {
            collect_context_let_refs_in_expr(&sl.value, refs);
        }
        IrExpression::Binary(binary) => {
            collect_context_let_refs_in_expr(&binary.lhs, refs);
            collect_context_let_refs_in_expr(&binary.rhs, refs);
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            collect_context_let_refs_in_expr(&rpr.receiver, refs);
        }
        IrExpression::ResolvedBinary(rb) => {
            collect_context_let_refs_in_expr(&rb.left, refs);
            collect_context_let_refs_in_expr(&rb.right, refs);
        }
        IrExpression::ResolvedCall(rc) => {
            collect_context_let_refs_in_expr(&rc.receiver, refs);
            for arg in rc.args.iter() {
                collect_context_let_refs_in_expr(arg, refs);
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            collect_context_let_refs_in_expr(&rkr.receiver, refs);
            collect_context_let_refs_in_expr(&rkr.key, refs);
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            collect_context_let_refs_in_expr(&rspr.receiver, refs);
        }
        IrExpression::ReadVariable(_rv) => {
            // ReadVariable doesn't reference @let, it has no sub-expressions (leaf node)
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter() {
                collect_context_let_refs_in_expr(entry, refs);
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter() {
                collect_context_let_refs_in_expr(value, refs);
            }
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter() {
                collect_context_let_refs_in_expr(elem, refs);
            }
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter() {
                collect_context_let_refs_in_expr(value, refs);
            }
        }
        // Leaf expressions - no sub-expressions to recurse into
        IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_) => {}
        IrExpression::Not(n) => {
            collect_context_let_refs_in_expr(&n.expr, refs);
        }
        IrExpression::Unary(u) => {
            collect_context_let_refs_in_expr(&u.expr, refs);
        }
        IrExpression::Typeof(t) => {
            collect_context_let_refs_in_expr(&t.expr, refs);
        }
        IrExpression::Void(v) => {
            collect_context_let_refs_in_expr(&v.expr, refs);
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter() {
                collect_context_let_refs_in_expr(e, refs);
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            collect_context_let_refs_in_expr(&arrow_fn.body, refs);
        }
        IrExpression::Parenthesized(paren) => {
            collect_context_let_refs_in_expr(&paren.expr, refs);
        }
    }
}

/// Remove DeclareLetOps that were marked for removal.
fn remove_declare_let_ops(
    job: &mut ComponentCompilationJob<'_>,
    declare_let_ops: &FxHashMap<XrefId, DeclareLetOpInfo>,
    declare_lets_to_remove: &HashSet<XrefId>,
) {
    // Group by view
    let mut by_view: FxHashMap<XrefId, Vec<XrefId>> = FxHashMap::default();
    for xref in declare_lets_to_remove {
        if let Some(info) = declare_let_ops.get(xref) {
            by_view.entry(info.view_xref).or_default().push(*xref);
        }
    }

    // Remove from each view
    for (view_xref, xrefs_to_remove) in by_view {
        let xref_set: HashSet<XrefId> = xrefs_to_remove.into_iter().collect();

        let mut ops_to_remove: Vec<NonNull<CreateOp<'_>>> = Vec::new();

        // First pass: collect pointers
        {
            let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };
            if let Some(view) = view {
                for op in view.create.iter() {
                    if let CreateOp::DeclareLet(let_op) = op {
                        if xref_set.contains(&let_op.xref) {
                            ops_to_remove.push(NonNull::from(op));
                        }
                    }
                }
            }
        }

        // Second pass: remove
        {
            let view = if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(view_xref) };
            if let Some(view) = view {
                for op_ptr in ops_to_remove {
                    // SAFETY: The pointer came from the list and we're removing it
                    unsafe {
                        view.create.remove(op_ptr);
                    }
                }
            }
        }
    }
}

/// Optimizes store let for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn optimize_store_let_for_host(_job: &mut HostBindingCompilationJob<'_>) {
    // Host bindings don't have @let declarations, so this phase is a no-op
}

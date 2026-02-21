//! Next context merging phase.
//!
//! Merges logically sequential `NextContextExpr` operations.
//!
//! `NextContextExpr` can be referenced repeatedly, "popping" the runtime's context stack each time.
//! When two such expressions appear back-to-back, it's possible to merge them together into a single
//! `NextContextExpr` that steps multiple contexts.
//!
//! This merging is possible if all conditions are met:
//! - The result of the `NextContextExpr` that's folded into the subsequent one is not stored
//!   (that is, the call is purely side-effectful)
//! - No operations in between them uses the implicit context
//!
//! Ported from Angular's `template/pipeline/src/phases/next_context_merging.ts`.

use crate::ir::enums::ExpressionKind;
use crate::ir::expression::IrExpression;
use crate::ir::list::UpdateOpList;
use crate::ir::ops::{CreateOp, UpdateOp};
use crate::output::ast::{OutputExpression, OutputStatement};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Merges consecutive nextContext() expressions.
///
/// This optimization reduces the number of runtime calls by combining
/// multiple nextContext(1) calls into a single nextContext(n) call.
pub fn merge_next_context_expressions(job: &mut ComponentCompilationJob<'_>) {
    // Collect view xrefs
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            // Merge in arrow function op lists (matches Angular's unit.functions traversal)
            for fn_ptr in view.functions.iter() {
                // SAFETY: These pointers are valid for the duration of the compilation
                let arrow_fn = unsafe { &mut **fn_ptr };
                merge_next_contexts_in_handler_ops(&mut arrow_fn.ops);
            }

            // Merge in create ops for listeners
            for op in view.create.iter_mut() {
                match op {
                    CreateOp::Listener(listener) => {
                        merge_next_contexts_in_handler_ops(&mut listener.handler_ops);
                    }
                    CreateOp::TwoWayListener(listener) => {
                        merge_next_contexts_in_handler_ops(&mut listener.handler_ops);
                    }
                    CreateOp::AnimationListener(listener) => {
                        merge_next_contexts_in_handler_ops(&mut listener.handler_ops);
                    }
                    CreateOp::Animation(animation) => {
                        merge_next_contexts_in_handler_ops(&mut animation.handler_ops);
                    }
                    _ => {}
                }
            }

            // Merge in update ops (main optimization)
            merge_next_contexts_in_update_ops(&mut view.update);
        }
    }
}

/// Merge NextContext expressions within an UpdateOpList (doubly-linked list).
///
/// Algorithm (following TypeScript's next_context_merging.ts):
/// For each op that is a Statement with ExpressionStatement containing NextContextExpr:
///   1. Walk forward through subsequent ops
///   2. For each candidate op, check all expressions:
///      - If NextContext found: merge steps into it, remove source op, STOP
///      - If blocker found (GetCurrentView, Reference, ContextLetReference): STOP without merging
///   3. Stop at first target or first blocker - don't continue looking
///
/// Key differences from the old implementation:
/// - Only Statement-based NextContext ops are merge sources (NOT Variable-based)
/// - We collect all Statement sources upfront, then process them in order
/// - For each source, we stop at the first target or blocker (don't keep looking)
fn merge_next_contexts_in_update_ops<'a>(ops: &mut UpdateOpList<'a>) {
    // Collect all Statement sources with their pointers and steps
    // We do this upfront because we'll be modifying the list
    let sources: Vec<_> = ops
        .iter()
        .filter_map(|op| {
            get_statement_next_context_steps(op).map(|steps| (std::ptr::NonNull::from(op), steps))
        })
        .collect();

    // Process each source in order
    for (source_ptr, _cached_steps) in sources.iter() {
        // Check if this source still exists (might have been removed as a target)
        // Also re-read the steps from the source in case it was updated by a previous merge
        let mut source_still_exists = false;
        let mut merge_steps = 0u32;
        for op in ops.iter() {
            if std::ptr::NonNull::from(op) == *source_ptr {
                source_still_exists = true;
                // Re-read the steps from the op in case it was updated
                if let Some(steps) = get_statement_next_context_steps(op) {
                    merge_steps = steps;
                }
                break;
            }
        }
        if !source_still_exists {
            continue;
        }

        // Walk forward from this source looking for a merge target or blocker
        let mut found_source = false;
        let mut should_remove_source = false;

        for op in ops.iter_mut() {
            let current_ptr = std::ptr::NonNull::from(&*op);

            if current_ptr == *source_ptr {
                found_source = true;
                continue;
            }

            if !found_source {
                continue;
            }

            // Check this candidate for NextContext (target) or blockers
            let result = visit_expressions_for_merge(op, merge_steps);

            match result {
                MergeResult::FoundTarget => {
                    should_remove_source = true;
                    break;
                }
                MergeResult::FoundBlocker => {
                    // Can't merge past a blocker, stop looking
                    break;
                }
                MergeResult::Continue => {
                    // Keep looking at next candidates
                }
            }
        }

        if should_remove_source {
            // SAFETY: source_ptr came from this list and we're removing it
            unsafe {
                ops.remove(*source_ptr);
            }
        }
        // If we didn't merge, just continue to the next source
    }
}

/// Result of checking an op for merge targets or blockers.
#[derive(PartialEq, Eq)]
enum MergeResult {
    /// Found a NextContext expression and merged into it
    FoundTarget,
    /// Found a blocking expression (GetCurrentView, Reference, ContextLetReference)
    FoundBlocker,
    /// No target or blocker found in this op, continue checking next op
    Continue,
}

/// Visit all expressions in an UpdateOp, looking for merge targets or blockers.
fn visit_expressions_for_merge(op: &mut UpdateOp<'_>, merge_steps: u32) -> MergeResult {
    match op {
        UpdateOp::Variable(var) => visit_ir_expr_for_merge(&mut var.initializer, merge_steps),
        UpdateOp::Property(prop) => visit_ir_expr_for_merge(&mut prop.expression, merge_steps),
        UpdateOp::StyleProp(style) => visit_ir_expr_for_merge(&mut style.expression, merge_steps),
        UpdateOp::ClassProp(class) => visit_ir_expr_for_merge(&mut class.expression, merge_steps),
        UpdateOp::StyleMap(style) => visit_ir_expr_for_merge(&mut style.expression, merge_steps),
        UpdateOp::ClassMap(class) => visit_ir_expr_for_merge(&mut class.expression, merge_steps),
        UpdateOp::Attribute(attr) => visit_ir_expr_for_merge(&mut attr.expression, merge_steps),
        UpdateOp::DomProperty(dom) => visit_ir_expr_for_merge(&mut dom.expression, merge_steps),
        UpdateOp::Binding(bind) => visit_ir_expr_for_merge(&mut bind.expression, merge_steps),
        UpdateOp::InterpolateText(interp) => {
            visit_ir_expr_for_merge(&mut interp.interpolation, merge_steps)
        }
        UpdateOp::TwoWayProperty(two_way) => {
            visit_ir_expr_for_merge(&mut two_way.expression, merge_steps)
        }
        UpdateOp::StoreLet(store) => visit_ir_expr_for_merge(&mut store.value, merge_steps),
        UpdateOp::Control(ctrl) => visit_ir_expr_for_merge(&mut ctrl.expression, merge_steps),
        UpdateOp::I18nExpression(i18n) => {
            visit_ir_expr_for_merge(&mut i18n.expression, merge_steps)
        }
        UpdateOp::AnimationBinding(anim) => {
            visit_ir_expr_for_merge(&mut anim.expression, merge_steps)
        }
        UpdateOp::Conditional(cond) => {
            if let Some(test) = cond.test.as_mut() {
                let result = visit_ir_expr_for_merge(test, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            for c in cond.conditions.iter_mut() {
                if let Some(expr) = c.expr.as_mut() {
                    let result = visit_ir_expr_for_merge(expr, merge_steps);
                    if !matches!(result, MergeResult::Continue) {
                        return result;
                    }
                }
            }
            if let Some(processed) = cond.processed.as_mut() {
                let result = visit_ir_expr_for_merge(processed, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            if let Some(context_value) = cond.context_value.as_mut() {
                let result = visit_ir_expr_for_merge(context_value, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }
        UpdateOp::Repeater(rep) => visit_ir_expr_for_merge(&mut rep.collection, merge_steps),
        UpdateOp::DeferWhen(defer) => visit_ir_expr_for_merge(&mut defer.condition, merge_steps),
        UpdateOp::Statement(stmt) => visit_statement_for_merge(&mut stmt.statement, merge_steps),
        UpdateOp::ListEnd(_) | UpdateOp::Advance(_) | UpdateOp::I18nApply(_) => {
            MergeResult::Continue
        }
    }
}

/// Visit a statement for merge targets or blockers.
fn visit_statement_for_merge(stmt: &mut OutputStatement<'_>, merge_steps: u32) -> MergeResult {
    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            visit_output_expr_for_merge(&mut expr_stmt.expr, merge_steps)
        }
        OutputStatement::Return(ret_stmt) => {
            visit_output_expr_for_merge(&mut ret_stmt.value, merge_steps)
        }
        _ => MergeResult::Continue,
    }
}

/// Visit an output expression for merge targets or blockers.
fn visit_output_expr_for_merge(expr: &mut OutputExpression<'_>, merge_steps: u32) -> MergeResult {
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            visit_ir_expr_for_merge(&mut wrapped.node, merge_steps)
        }
        _ => MergeResult::Continue,
    }
}

/// Visit an IR expression for merge targets or blockers.
fn visit_ir_expr_for_merge(expr: &mut IrExpression<'_>, merge_steps: u32) -> MergeResult {
    match expr.kind() {
        // Blockers - expressions that depend on context
        ExpressionKind::GetCurrentView
        | ExpressionKind::Reference
        | ExpressionKind::ContextLetReference => MergeResult::FoundBlocker,

        // Target - NextContext expression to merge into
        ExpressionKind::NextContext => {
            if let IrExpression::NextContext(nc) = expr {
                nc.steps += merge_steps;
                return MergeResult::FoundTarget;
            }
            MergeResult::Continue
        }

        // For other expressions, check nested expressions
        _ => visit_nested_exprs_for_merge(expr, merge_steps),
    }
}

/// Visit nested expressions in an IR expression.
fn visit_nested_exprs_for_merge(expr: &mut IrExpression<'_>, merge_steps: u32) -> MergeResult {
    match expr {
        // Safe navigation expressions
        IrExpression::SafeTernary(st) => {
            let result = visit_ir_expr_for_merge(&mut st.guard, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut st.expr, merge_steps)
        }
        IrExpression::SafePropertyRead(sp) => {
            visit_ir_expr_for_merge(&mut sp.receiver, merge_steps)
        }
        IrExpression::SafeKeyedRead(sk) => {
            let result = visit_ir_expr_for_merge(&mut sk.receiver, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut sk.index, merge_steps)
        }
        IrExpression::SafeInvokeFunction(sf) => {
            let result = visit_ir_expr_for_merge(&mut sf.receiver, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            for arg in sf.args.iter_mut() {
                let result = visit_ir_expr_for_merge(arg, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }

        // Resolved expressions (created during name resolution)
        IrExpression::ResolvedPropertyRead(rpr) => {
            visit_ir_expr_for_merge(&mut rpr.receiver, merge_steps)
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            let result = visit_ir_expr_for_merge(&mut rkr.receiver, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut rkr.key, merge_steps)
        }
        IrExpression::ResolvedCall(rc) => {
            let result = visit_ir_expr_for_merge(&mut rc.receiver, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            for arg in rc.args.iter_mut() {
                let result = visit_ir_expr_for_merge(arg, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }
        IrExpression::ResolvedBinary(rb) => {
            let result = visit_ir_expr_for_merge(&mut rb.left, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut rb.right, merge_steps)
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            visit_ir_expr_for_merge(&mut rspr.receiver, merge_steps)
        }

        // Pipe expressions
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter_mut() {
                let result = visit_ir_expr_for_merge(arg, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            visit_ir_expr_for_merge(&mut pbv.args, merge_steps)
        }

        // Pure function expressions
        IrExpression::PureFunction(pf) => {
            if let Some(body) = pf.body.as_mut() {
                let result = visit_ir_expr_for_merge(body, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            for arg in pf.args.iter_mut() {
                let result = visit_ir_expr_for_merge(arg, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            if let Some(fn_ref) = pf.fn_ref.as_mut() {
                let result = visit_ir_expr_for_merge(fn_ref, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }

        // Binary and ternary expressions
        IrExpression::Binary(b) => {
            let result = visit_ir_expr_for_merge(&mut b.lhs, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut b.rhs, merge_steps)
        }
        IrExpression::Ternary(t) => {
            let result = visit_ir_expr_for_merge(&mut t.condition, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            let result = visit_ir_expr_for_merge(&mut t.true_expr, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut t.false_expr, merge_steps)
        }

        // Interpolation
        IrExpression::Interpolation(interp) => {
            for e in interp.expressions.iter_mut() {
                let result = visit_ir_expr_for_merge(e, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }

        // Two-way binding
        IrExpression::TwoWayBindingSet(tbs) => {
            let result = visit_ir_expr_for_merge(&mut tbs.target, merge_steps);
            if !matches!(result, MergeResult::Continue) {
                return result;
            }
            visit_ir_expr_for_merge(&mut tbs.value, merge_steps)
        }

        // Store @let
        IrExpression::StoreLet(sl) => visit_ir_expr_for_merge(&mut sl.value, merge_steps),

        // Const collected
        IrExpression::ConstCollected(cc) => visit_ir_expr_for_merge(&mut cc.expr, merge_steps),

        // View expressions
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref mut inner) = rv.view {
                visit_ir_expr_for_merge(inner, merge_steps)
            } else {
                MergeResult::Continue
            }
        }
        IrExpression::ResetView(rv) => visit_ir_expr_for_merge(&mut rv.expr, merge_steps),

        // Literal array/map expressions
        IrExpression::LiteralArray(la) => {
            for elem in la.elements.iter_mut() {
                let result = visit_ir_expr_for_merge(elem, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }
        IrExpression::LiteralMap(lm) => {
            for value in lm.values.iter_mut() {
                let result = visit_ir_expr_for_merge(value, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }
        IrExpression::DerivedLiteralArray(dla) => {
            for entry in dla.entries.iter_mut() {
                let result = visit_ir_expr_for_merge(entry, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }
        IrExpression::DerivedLiteralMap(dlm) => {
            for value in dlm.values.iter_mut() {
                let result = visit_ir_expr_for_merge(value, merge_steps);
                if !matches!(result, MergeResult::Continue) {
                    return result;
                }
            }
            MergeResult::Continue
        }

        // Unary expressions
        IrExpression::Not(n) => visit_ir_expr_for_merge(&mut n.expr, merge_steps),
        IrExpression::Unary(u) => visit_ir_expr_for_merge(&mut u.expr, merge_steps),
        IrExpression::Typeof(t) => visit_ir_expr_for_merge(&mut t.expr, merge_steps),
        IrExpression::Void(v) => visit_ir_expr_for_merge(&mut v.expr, merge_steps),
        IrExpression::Parenthesized(p) => visit_ir_expr_for_merge(&mut p.expr, merge_steps),
        // Temporary expressions
        IrExpression::AssignTemporary(at) => visit_ir_expr_for_merge(&mut at.expr, merge_steps),

        // Conditional case (may have optional expression)
        IrExpression::ConditionalCase(cc) => {
            if let Some(e) = cc.expr.as_mut() {
                visit_ir_expr_for_merge(e, merge_steps)
            } else {
                MergeResult::Continue
            }
        }

        // ResolvedTemplateLiteral: recurse into embedded expressions
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter_mut() {
                let result = visit_ir_expr_for_merge(e, merge_steps);
                if result != MergeResult::Continue {
                    return result;
                }
            }
            MergeResult::Continue
        }

        // Leaf expressions with no sub-expressions - no need to recurse
        IrExpression::LexicalRead(_)
        | IrExpression::Context(_)
        | IrExpression::ReadVariable(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_) => MergeResult::Continue,

        // NOTE: NextContext, GetCurrentView, Reference, and ContextLetReference
        // are handled in the caller (visit_ir_expr_for_merge) before we get here,
        // so they will never reach this match arm. But we list them explicitly
        // for exhaustiveness.
        IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::Reference(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::ArrowFunction(_) => MergeResult::Continue,
    }
}

/// Merge NextContext expressions within handler ops (Vec-based).
///
/// Handler ops use Vec instead of linked list, so the algorithm is slightly different.
/// But the core logic is the same:
/// - Only Statement-based NextContext are merge sources
/// - Stop at first target or blocker
fn merge_next_contexts_in_handler_ops<'a>(ops: &mut oxc_allocator::Vec<'a, UpdateOp<'a>>) {
    if ops.len() < 2 {
        return;
    }

    // Collect indices and steps of Statement-based NextContext ops only
    // These are our merge sources
    let sources: Vec<(usize, u32)> = ops
        .iter()
        .enumerate()
        .filter_map(|(idx, op)| get_statement_next_context_steps(op).map(|steps| (idx, steps)))
        .collect();

    // Track which indices to remove (after merging)
    let mut indices_to_remove: Vec<usize> = Vec::new();

    // For each Statement NextContext source, look ahead for a target or blocker
    for (source_idx, _cached_steps) in &sources {
        // Skip if this source was already marked for removal (it was a target of an earlier merge)
        if indices_to_remove.contains(source_idx) {
            continue;
        }

        // Re-read the steps from the source in case it was updated by a previous merge
        let merge_steps = match get_statement_next_context_steps(&ops[*source_idx]) {
            Some(steps) => steps,
            None => continue, // Source was modified and is no longer a NextContext statement
        };

        // Walk forward from this source looking for a target or blocker
        for candidate_idx in (*source_idx + 1)..ops.len() {
            // Skip candidates that are marked for removal
            if indices_to_remove.contains(&candidate_idx) {
                continue;
            }

            // Check this candidate for NextContext (target) or blockers
            let result = visit_expressions_for_merge(&mut ops[candidate_idx], merge_steps);

            match result {
                MergeResult::FoundTarget => {
                    // Merged successfully, mark source for removal
                    indices_to_remove.push(*source_idx);
                    break;
                }
                MergeResult::FoundBlocker => {
                    // Can't merge past a blocker, stop looking for this source
                    break;
                }
                MergeResult::Continue => {
                    // Keep looking at next candidates
                }
            }
        }
    }

    // Remove merged ops in reverse order to preserve indices
    indices_to_remove.sort();
    indices_to_remove.reverse();
    for idx in indices_to_remove {
        ops.remove(idx);
    }
}

/// Get the steps from a Statement op containing a NextContext expression.
/// Returns None if the op is not a Statement with NextContext.
fn get_statement_next_context_steps(op: &UpdateOp<'_>) -> Option<u32> {
    if let UpdateOp::Statement(stmt) = op {
        if let OutputStatement::Expression(expr_stmt) = &stmt.statement {
            if let OutputExpression::WrappedIrNode(wrapped) = &expr_stmt.expr {
                if let IrExpression::NextContext(nc) = wrapped.node.as_ref() {
                    return Some(nc.steps);
                }
            }
        }
    }
    None
}

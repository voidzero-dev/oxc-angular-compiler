//! Pipe creation phase.
//!
//! This phase generates pipe creation instructions. We do this based on the
//! pipe bindings found in the update block, in the order we see them.
//!
//! When not in compatibility mode, we can simply group all these creation
//! instructions together at the end of the create block to maximize chaining
//! opportunities.
//!
//! In compatibility mode (TemplateDefinitionBuilder), pipes must be inserted
//! after their target element in the create block to match the original output.
//!
//! Ported from Angular's `template/pipeline/src/phases/pipe_creation.ts`.

use std::collections::HashSet;
use std::ptr::NonNull;

use crate::ir::enums::{CompatibilityMode, OpKind};
use crate::ir::expression::{IrExpression, SlotHandle};
use crate::ir::ops::{CreateOp, CreateOpBase, Op, PipeOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Creates pipe instances for use in expressions.
///
/// This phase scans all update operations for PipeBinding expressions and
/// creates corresponding PipeOp operations in the create block.
pub fn create_pipes(job: &mut ComponentCompilationJob<'_>) {
    // Process root view
    process_pipe_bindings_in_view(job, job.root.xref);

    // Process embedded views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs {
        process_pipe_bindings_in_view(job, view_xref);
    }
}

/// Process pipe bindings in a single view.
fn process_pipe_bindings_in_view(job: &mut ComponentCompilationJob<'_>, view_xref: XrefId) {
    let compatibility_mode = job.compatibility_mode;

    // Collect pipe bindings first to avoid borrow issues
    let pipe_bindings: Vec<PipeInfo> = {
        let view = if view_xref.0 == 0 {
            &job.root
        } else if let Some(v) = job.views.get(&view_xref) {
            v.as_ref()
        } else {
            return;
        };

        let mut bindings = Vec::new();

        for op in view.update.iter() {
            // Visit all expressions in this operation to find PipeBinding
            collect_pipe_bindings(op, &mut bindings);
        }

        // Deduplicate by xref - the same pipe can appear multiple times
        // (e.g., in @switch test and processed expressions), but we only
        // want to create one PipeOp per unique xref
        let mut seen_xrefs = HashSet::new();
        bindings.into_iter().filter(|p| seen_xrefs.insert(p.xref)).collect()
    };

    // Create PipeOp for each binding
    for pipe_info in pipe_bindings {
        let pipe_op = CreateOp::Pipe(PipeOp {
            base: CreateOpBase::default(),
            xref: pipe_info.xref,
            slot: pipe_info.target_slot.slot,
            name: pipe_info.name,
            num_args: pipe_info.num_args,
        });

        let view = if view_xref.0 == 0 {
            &mut job.root
        } else if let Some(v) = job.views.get_mut(&view_xref) {
            v.as_mut()
        } else {
            continue;
        };

        if compatibility_mode == CompatibilityMode::TemplateDefinitionBuilder {
            // In compatibility mode, insert the pipe after its target element
            // and after any existing pipes that follow the target element.
            if let Some(target_xref) = pipe_info.target_element {
                add_pipe_to_creation_block(view, target_xref, pipe_op);
            } else {
                // No target element, append to end
                view.create.push(pipe_op);
            }
        } else {
            // In normal mode, append to end of create block for better chaining
            view.create.push(pipe_op);
        }
    }
}

/// Adds a pipe to the creation block after the target element.
///
/// This function finds the target element in the create block by its xref,
/// then skips past any existing pipe operations, and inserts the new pipe there.
///
/// This matches Angular's `addPipeToCreationBlock` from `pipe_creation.ts`.
fn add_pipe_to_creation_block<'a>(
    view: &mut crate::pipeline::compilation::ViewCompilationUnit<'a>,
    target_xref: XrefId,
    pipe_op: CreateOp<'a>,
) {
    // Find the target element and insertion point
    let insertion_point = find_pipe_insertion_point(&view.create, target_xref);

    if let Some(insert_before) = insertion_point {
        // Insert the pipe before the next non-pipe operation
        // SAFETY: insert_before is a valid pointer in this list
        unsafe {
            view.create.insert_before(insert_before, pipe_op);
        }
    } else {
        // Fallback: no insertion point found, append to end
        // This can happen if the target element is not found or is at the end
        view.create.push(pipe_op);
    }
}

/// Finds the insertion point for a pipe in the create block.
///
/// Returns the pointer to the operation BEFORE which the new pipe should be inserted.
/// Returns None if the pipe should be appended to the end.
///
/// This matches Angular's `addPipeToCreationBlock` from `pipe_creation.ts`:
/// 1. Find the target element by xref
/// 2. Skip past any existing Pipe operations that follow it
/// 3. Insert before the next non-Pipe operation (which could be a Listener, Template, etc.)
fn find_pipe_insertion_point<'a>(
    create: &crate::ir::list::CreateOpList<'a>,
    target_xref: XrefId,
) -> Option<NonNull<CreateOp<'a>>> {
    let mut current = create.head_ptr();

    // Phase 1: Find the target element by xref
    while let Some(ptr) = current {
        // SAFETY: ptr is valid as it was obtained from the list
        let op = unsafe { ptr.as_ref() };

        if let Some(op_xref) = get_create_op_xref(op) {
            if op_xref == target_xref {
                // Found the target element, now move to phase 2
                current = op.next();
                break;
            }
        }

        current = op.next();
    }

    // Phase 2: Skip past any existing Pipe operations
    while let Some(ptr) = current {
        // SAFETY: ptr is valid as it was obtained from the list
        let op = unsafe { ptr.as_ref() };

        if op.kind() == OpKind::Pipe {
            // Skip this pipe and continue looking
            current = op.next();
        } else {
            // Found a non-Pipe operation - insert before this
            return Some(ptr);
        }
    }

    // Reached the end of the list - append to end
    None
}

/// Gets the xref from a create operation if it has one.
///
/// This matches operations that "consume slots" in Angular's IR.
fn get_create_op_xref(op: &CreateOp<'_>) -> Option<XrefId> {
    match op {
        CreateOp::ElementStart(e) => Some(e.xref),
        CreateOp::Element(e) => Some(e.xref),
        CreateOp::Template(t) => Some(t.xref),
        CreateOp::ContainerStart(c) => Some(c.xref),
        CreateOp::Container(c) => Some(c.xref),
        CreateOp::Text(t) => Some(t.xref),
        CreateOp::Pipe(p) => Some(p.xref),
        CreateOp::Projection(p) => Some(p.xref),
        CreateOp::RepeaterCreate(r) => Some(r.xref),
        CreateOp::Defer(d) => Some(d.xref),
        CreateOp::Conditional(c) => Some(c.xref),
        CreateOp::ConditionalBranch(c) => Some(c.xref),
        CreateOp::I18nStart(i) => Some(i.xref),
        CreateOp::I18n(i) => Some(i.xref),
        CreateOp::DeclareLet(l) => Some(l.xref),
        CreateOp::IcuStart(i) => Some(i.xref),
        CreateOp::IcuPlaceholder(i) => Some(i.xref),
        CreateOp::I18nContext(i) => Some(i.xref),
        CreateOp::I18nAttributes(i) => Some(i.xref),
        CreateOp::I18nMessage(i) => Some(i.xref),
        CreateOp::Variable(v) => Some(v.xref),
        _ => None,
    }
}

/// Information about a pipe binding.
#[derive(Debug)]
struct PipeInfo<'a> {
    /// Cross-reference ID for this pipe.
    xref: XrefId,
    /// Target slot handle from the PipeBindingExpr.
    /// This is preserved from the expression to match Angular's behavior.
    target_slot: SlotHandle,
    /// Pipe name.
    name: oxc_span::Ident<'a>,
    /// Number of arguments (including the input expression).
    num_args: u32,
    /// Target element xref (for compatibility mode insertion ordering).
    /// This is the element that the update op containing this pipe targets.
    target_element: Option<XrefId>,
}

/// Collects pipe bindings from an update operation.
///
/// IMPORTANT: Angular's visitor pattern processes children BEFORE the parent.
/// This is because `transformExpressionsInExpression` calls `transformInternalExpressions`
/// (which processes args) BEFORE calling `transform(expr, flags)` (the visitor callback).
/// We must match this order to ensure pipes are created in textual order and var offsets
/// are calculated correctly.
fn collect_pipe_bindings<'a>(op: &crate::ir::ops::UpdateOp<'a>, bindings: &mut Vec<PipeInfo<'a>>) {
    // Helper function to check an expression for pipe bindings
    fn check_expression<'a>(
        expr: &IrExpression<'a>,
        target_element: Option<XrefId>,
        bindings: &mut Vec<PipeInfo<'a>>,
    ) {
        // First, recursively check nested expressions (children before parent)
        match expr {
            IrExpression::PipeBinding(pipe) => {
                // Check arguments FIRST (nested pipes) before adding this pipe
                for arg in pipe.args.iter() {
                    check_expression(arg, target_element, bindings);
                }
                // Then add THIS pipe (matching Angular's visitor order)
                bindings.push(PipeInfo {
                    xref: pipe.target,
                    target_slot: pipe.target_slot.clone(),
                    name: pipe.name.clone(),
                    num_args: pipe.args.len() as u32,
                    target_element,
                });
            }
            IrExpression::PureFunction(pf) => {
                for arg in pf.args.iter() {
                    check_expression(arg, target_element, bindings);
                }
            }
            IrExpression::SafePropertyRead(safe) => {
                check_expression(&safe.receiver, target_element, bindings);
            }
            IrExpression::SafeKeyedRead(safe) => {
                check_expression(&safe.receiver, target_element, bindings);
                check_expression(&safe.index, target_element, bindings);
            }
            IrExpression::SafeInvokeFunction(safe) => {
                check_expression(&safe.receiver, target_element, bindings);
                for arg in safe.args.iter() {
                    check_expression(arg, target_element, bindings);
                }
            }
            IrExpression::Interpolation(interp) => {
                // Recursively check all expressions in the interpolation
                for inner in interp.expressions.iter() {
                    check_expression(inner, target_element, bindings);
                }
            }
            IrExpression::Ternary(t) => {
                check_expression(&t.condition, target_element, bindings);
                check_expression(&t.true_expr, target_element, bindings);
                check_expression(&t.false_expr, target_element, bindings);
            }
            IrExpression::Binary(b) => {
                check_expression(&b.lhs, target_element, bindings);
                check_expression(&b.rhs, target_element, bindings);
            }
            IrExpression::AssignTemporary(a) => {
                check_expression(&a.expr, target_element, bindings);
            }
            IrExpression::ConditionalCase(c) => {
                if let Some(ref expr) = c.expr {
                    check_expression(expr, target_element, bindings);
                }
            }
            IrExpression::ResolvedCall(call) => {
                check_expression(&call.receiver, target_element, bindings);
                for arg in call.args.iter() {
                    check_expression(arg, target_element, bindings);
                }
            }
            IrExpression::ResolvedPropertyRead(pr) => {
                check_expression(&pr.receiver, target_element, bindings);
            }
            IrExpression::ResolvedKeyedRead(kr) => {
                check_expression(&kr.receiver, target_element, bindings);
                check_expression(&kr.key, target_element, bindings);
            }
            IrExpression::ResolvedSafePropertyRead(pr) => {
                check_expression(&pr.receiver, target_element, bindings);
            }
            IrExpression::ResolvedBinary(b) => {
                check_expression(&b.left, target_element, bindings);
                check_expression(&b.right, target_element, bindings);
            }
            IrExpression::SafeTernary(t) => {
                check_expression(&t.guard, target_element, bindings);
                check_expression(&t.expr, target_element, bindings);
            }
            IrExpression::LiteralArray(arr) => {
                for elem in arr.elements.iter() {
                    check_expression(elem, target_element, bindings);
                }
            }
            IrExpression::LiteralMap(map) => {
                for value in map.values.iter() {
                    check_expression(value, target_element, bindings);
                }
            }
            IrExpression::DerivedLiteralArray(arr) => {
                for entry in arr.entries.iter() {
                    check_expression(entry, target_element, bindings);
                }
            }
            IrExpression::DerivedLiteralMap(map) => {
                for value in map.values.iter() {
                    check_expression(value, target_element, bindings);
                }
            }
            IrExpression::Not(e) => {
                check_expression(&e.expr, target_element, bindings);
            }
            IrExpression::Unary(e) => {
                check_expression(&e.expr, target_element, bindings);
            }
            IrExpression::Typeof(e) => {
                check_expression(&e.expr, target_element, bindings);
            }
            IrExpression::Void(e) => {
                check_expression(&e.expr, target_element, bindings);
            }
            IrExpression::Parenthesized(paren) => {
                check_expression(&paren.expr, target_element, bindings);
            }
            _ => {}
        }
    }

    // Get the target element xref from the update operation
    let target_element = get_update_op_target(op);

    // Check the operation's main expression
    match op {
        crate::ir::ops::UpdateOp::Property(prop) => {
            check_expression(&prop.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::Attribute(attr) => {
            check_expression(&attr.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::StyleProp(style) => {
            check_expression(&style.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::ClassProp(class) => {
            check_expression(&class.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::StyleMap(style) => {
            check_expression(&style.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::ClassMap(class) => {
            check_expression(&class.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::InterpolateText(interp) => {
            // InterpolateText has embedded expressions
            check_expression(&interp.interpolation, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::Variable(var) => {
            check_expression(&var.initializer, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::Conditional(cond) => {
            // Check test expression (for @switch)
            if let Some(ref test) = cond.test {
                check_expression(test, target_element, bindings);
            }
            // Check processed expression (contains the ternary with pipe expressions after conditionals phase)
            if let Some(ref processed) = cond.processed {
                check_expression(processed, target_element, bindings);
            }
            // Check context value (for @if (x as alias))
            if let Some(ref ctx_val) = cond.context_value {
                check_expression(ctx_val, target_element, bindings);
            }
            // Also check conditions in case pipe_creation runs before conditionals phase
            for condition in cond.conditions.iter() {
                if let Some(ref expr) = condition.expr {
                    check_expression(expr, target_element, bindings);
                }
            }
        }
        crate::ir::ops::UpdateOp::StoreLet(store_let) => {
            // StoreLet contains a value expression that may contain pipes
            check_expression(&store_let.value, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::DomProperty(dom) => {
            check_expression(&dom.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::TwoWayProperty(tw) => {
            check_expression(&tw.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::Binding(binding) => {
            check_expression(&binding.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::Repeater(repeater) => {
            // Repeater has a collection expression
            check_expression(&repeater.collection, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::DeferWhen(defer_when) => {
            // DeferWhen has a condition expression
            check_expression(&defer_when.condition, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::I18nExpression(i18n) => {
            check_expression(&i18n.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::AnimationBinding(anim) => {
            check_expression(&anim.expression, target_element, bindings);
        }
        crate::ir::ops::UpdateOp::Control(ctrl) => {
            check_expression(&ctrl.expression, target_element, bindings);
        }
        _ => {}
    }
}

/// Gets the target element xref from an update operation.
///
/// This is used in compatibility mode to determine where to insert the pipe
/// in the create block (after the target element).
fn get_update_op_target(op: &crate::ir::ops::UpdateOp<'_>) -> Option<XrefId> {
    match op {
        crate::ir::ops::UpdateOp::Property(prop) => Some(prop.target),
        crate::ir::ops::UpdateOp::Attribute(attr) => Some(attr.target),
        crate::ir::ops::UpdateOp::StyleProp(style) => Some(style.target),
        crate::ir::ops::UpdateOp::ClassProp(class) => Some(class.target),
        crate::ir::ops::UpdateOp::StyleMap(style) => Some(style.target),
        crate::ir::ops::UpdateOp::ClassMap(class) => Some(class.target),
        crate::ir::ops::UpdateOp::InterpolateText(interp) => Some(interp.target),
        crate::ir::ops::UpdateOp::DomProperty(dom) => Some(dom.target),
        crate::ir::ops::UpdateOp::TwoWayProperty(tw) => Some(tw.target),
        crate::ir::ops::UpdateOp::Binding(binding) => Some(binding.target),
        crate::ir::ops::UpdateOp::AnimationBinding(anim) => Some(anim.target),
        crate::ir::ops::UpdateOp::Control(ctrl) => Some(ctrl.target),
        crate::ir::ops::UpdateOp::Conditional(cond) => Some(cond.target),
        crate::ir::ops::UpdateOp::StoreLet(store_let) => Some(store_let.target),
        crate::ir::ops::UpdateOp::Repeater(repeater) => Some(repeater.target),
        crate::ir::ops::UpdateOp::I18nExpression(i18n) => Some(i18n.target),
        crate::ir::ops::UpdateOp::DeferWhen(defer_when) => Some(defer_when.defer),
        _ => None,
    }
}

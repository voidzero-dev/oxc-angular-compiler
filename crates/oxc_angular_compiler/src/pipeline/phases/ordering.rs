//! Operation ordering phase.
//!
//! Orders operations for correct execution sequence. Many types of operations
//! have ordering constraints that must be respected. For example, a `ClassMap`
//! instruction must be ordered after a `StyleMap` instruction for predictable
//! semantics that match TemplateDefinitionBuilder.
//!
//! Ported from Angular's `template/pipeline/src/phases/ordering.ts`.

use std::ptr::NonNull;

use crate::ir::enums::OpKind;
use crate::ir::expression::IrExpression;
use crate::ir::list::{CreateOpList, UpdateOpList};
use crate::ir::ops::{CreateOp, Op, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// The set of update op kinds we handle in the reordering phase.
fn is_handled_update_op_kind(kind: OpKind) -> bool {
    matches!(
        kind,
        OpKind::StyleMap
            | OpKind::ClassMap
            | OpKind::StyleProp
            | OpKind::ClassProp
            | OpKind::Property
            | OpKind::TwoWayProperty
            | OpKind::DomProperty
            | OpKind::Attribute
            | OpKind::Control
    )
}

/// The set of create op kinds we handle in the reordering phase.
/// Matches Angular's handledOpKinds for create ops: Listener, TwoWayListener, AnimationListener, Animation.
fn is_handled_create_op_kind(kind: OpKind) -> bool {
    matches!(
        kind,
        OpKind::Listener | OpKind::TwoWayListener | OpKind::AnimationListener | OpKind::Animation
    )
}

/// Ordering priority for update operations in template bindings.
/// Lower values are processed first.
///
/// Matches Angular's UPDATE_ORDERING:
/// StyleMap < ClassMap < StyleProp < ClassProp < Attr(interp) < Prop(interp) < Prop(no-interp) < Attr(no-interp)
fn update_op_priority(op: &UpdateOp<'_>) -> u32 {
    let kind = op.kind();
    let is_interpolation = match op {
        UpdateOp::Property(p) => matches!(*p.expression, IrExpression::Interpolation(_)),
        UpdateOp::TwoWayProperty(p) => matches!(*p.expression, IrExpression::Interpolation(_)),
        UpdateOp::Attribute(a) => matches!(*a.expression, IrExpression::Interpolation(_)),
        UpdateOp::DomProperty(d) => matches!(*d.expression, IrExpression::Interpolation(_)),
        _ => false,
    };

    match (kind, is_interpolation) {
        (OpKind::StyleMap, _) => 0,
        (OpKind::ClassMap, _) => 1,
        (OpKind::StyleProp, _) => 2,
        (OpKind::ClassProp, _) => 3,
        (OpKind::Attribute, true) => 4, // Attribute with interpolation
        (OpKind::Property, true) => 5,  // Property with interpolation
        (OpKind::TwoWayProperty, _) => 6, // Non-interpolation TwoWayProperty
        (OpKind::Property, false) => 6, // Non-interpolation Property
        (OpKind::Attribute, false) => 7, // Attribute without interpolation
        // Control comes last per Angular's UPDATE_ORDERING (line 69 of ordering.ts)
        (OpKind::Control, _) => 8,
        // DomProperty is not used in template bindings, but include for completeness
        (OpKind::DomProperty, _) => 9,
        _ => 100, // Other ops go last
    }
}

/// Ordering priority for update operations in host bindings.
/// Lower values are processed first.
///
/// Matches Angular's UPDATE_HOST_ORDERING:
/// DomProperty(interp) < DomProperty(no-interp) < Attribute < StyleMap < ClassMap < StyleProp < ClassProp
fn update_op_priority_host(op: &UpdateOp<'_>) -> u32 {
    let kind = op.kind();
    let is_interpolation = match op {
        UpdateOp::DomProperty(d) => matches!(*d.expression, IrExpression::Interpolation(_)),
        _ => false,
    };

    match (kind, is_interpolation) {
        (OpKind::DomProperty, true) => 0,  // DomProperty with interpolation
        (OpKind::DomProperty, false) => 1, // DomProperty without interpolation
        (OpKind::Attribute, _) => 2,
        (OpKind::StyleMap, _) => 3,
        (OpKind::ClassMap, _) => 4,
        (OpKind::StyleProp, _) => 5,
        (OpKind::ClassProp, _) => 6,
        _ => 100, // Other ops go last
    }
}

/// Ordering priority for create operations.
/// Lower values are processed first.
///
/// Matches Angular's CREATE_ORDERING (ordering.ts).
///
/// LegacyAnimation host listeners are placed before regular listeners to match
/// Angular's reference compiler output. The two instructions differ only in which
/// renderer they use: `ɵɵsyntheticHostListener` calls `loadComponentRenderer` to
/// use the component's own renderer (so @trigger events are handled by the
/// animation engine), while `ɵɵlistener` uses the parent lView's renderer.
/// The ordering itself is a spec requirement — it ensures our output is consistent
/// with TemplateDefinitionBuilder and Angular's compliance tests.
fn create_op_priority(op: &CreateOp<'_>) -> u32 {
    match op {
        // LegacyAnimation host listeners before regular listeners (spec compliance)
        CreateOp::Listener(l) if l.host_listener && l.is_animation_listener => 0,
        // Basic listeners (Listener, TwoWayListener, Animation, AnimationListener)
        CreateOp::Listener(_) => 1,
        CreateOp::TwoWayListener(_) => 1,
        CreateOp::Animation(_) => 1,
        CreateOp::AnimationListener(_) => 1,
        // Any new CreateOp variants default to 100 (maintain original order).
        // If a new variant has an ordering constraint, add an explicit arm above.
        _ => 100,
    }
}

/// Gets the target xref for an update operation (for grouping).
fn get_update_op_target(op: &UpdateOp<'_>) -> Option<XrefId> {
    match op {
        UpdateOp::Property(p) => Some(p.target),
        UpdateOp::TwoWayProperty(p) => Some(p.target),
        UpdateOp::StyleProp(p) => Some(p.target),
        UpdateOp::ClassProp(p) => Some(p.target),
        UpdateOp::StyleMap(p) => Some(p.target),
        UpdateOp::ClassMap(p) => Some(p.target),
        UpdateOp::Attribute(p) => Some(p.target),
        UpdateOp::DomProperty(p) => Some(p.target),
        UpdateOp::Control(c) => Some(c.target),
        _ => None,
    }
}

/// Gets the target xref for a create operation (for grouping).
fn get_create_op_target(op: &CreateOp<'_>) -> Option<XrefId> {
    match op {
        CreateOp::Listener(l) => Some(l.target),
        CreateOp::TwoWayListener(l) => Some(l.target),
        CreateOp::Animation(a) => Some(a.target),
        CreateOp::AnimationListener(l) => Some(l.target),
        _ => None,
    }
}

/// Orders operations for proper execution sequence.
///
/// This phase reorders operations within each view to ensure correct semantics:
/// - For create mode: orders listeners (legacy animation listeners before regular)
/// - For update mode: orders bindings in the correct sequence (styleMap, classMap, etc.)
pub fn order_ops(job: &mut ComponentCompilationJob<'_>) {
    // Process root view
    order_create_ops(&mut job.root.create);
    order_update_ops(&mut job.root.update);

    // Process embedded views
    for view in job.views.values_mut() {
        order_create_ops(&mut view.create);
        order_update_ops(&mut view.update);
    }
}

/// Orders create operations within a list.
///
/// Reorders Listener operations to ensure legacy animation listeners on host
/// come before regular listeners. Uses the same algorithm as Angular's ordering.ts.
fn order_create_ops(list: &mut CreateOpList<'_>) {
    if list.is_empty() {
        return;
    }

    // Collect ops that need reordering, grouped by target
    let mut ops_to_order: Vec<NonNull<CreateOp<'_>>> = Vec::new();
    let mut first_target_in_group: Option<XrefId> = None;
    let mut insertion_points: Vec<(NonNull<CreateOp<'_>>, Vec<NonNull<CreateOp<'_>>>)> = Vec::new();

    // First pass: collect ops and identify insertion points
    let mut cursor = list.cursor_front();
    loop {
        if let Some(ptr) = cursor.current_ptr() {
            // SAFETY: pointer is valid from cursor
            let op = unsafe { ptr.as_ref() };
            let kind = op.kind();
            let current_target = get_create_op_target(op);

            // Check if we should flush the current group
            let should_flush = !is_handled_create_op_kind(kind)
                || (current_target.is_some()
                    && first_target_in_group.is_some()
                    && current_target != first_target_in_group);

            if should_flush && !ops_to_order.is_empty() {
                // Reorder and record for later insertion
                let reordered = reorder_create_ops(&ops_to_order);
                insertion_points.push((ptr, reordered));
                ops_to_order.clear();
                first_target_in_group = None;
            }

            if is_handled_create_op_kind(kind) {
                ops_to_order.push(ptr);
                if first_target_in_group.is_none() {
                    first_target_in_group = current_target;
                }
            }
        }

        if !cursor.move_next() {
            break;
        }
    }

    // Handle remaining ops at end of list
    if !ops_to_order.is_empty() {
        let reordered = reorder_create_ops(&ops_to_order);
        // These go at the end, we'll handle them separately
        for ptr in &reordered {
            // SAFETY: pointer is valid
            unsafe { list.remove(*ptr) };
        }
        for ptr in reordered {
            // SAFETY: pointer is valid
            let op = unsafe { std::ptr::read(ptr.as_ptr()) };
            list.push(op);
        }
    }

    // Apply collected insertions (in reverse to maintain correct positions)
    for (before_ptr, reordered) in insertion_points.into_iter().rev() {
        // First remove all ops from their current positions
        for ptr in &reordered {
            // SAFETY: pointer is valid
            unsafe { list.remove(*ptr) };
        }
        // Then insert them in order before the insertion point
        // Note: iterate forward because insert_before with fixed before_ptr
        // places each new item right before the same position
        for ptr in reordered.into_iter() {
            // SAFETY: pointer is valid
            let op = unsafe { std::ptr::read(ptr.as_ptr()) };
            unsafe { list.insert_before(before_ptr, op) };
        }
    }
}

/// Reorders create ops by priority (stable sort).
fn reorder_create_ops<'a>(ops: &[NonNull<CreateOp<'a>>]) -> Vec<NonNull<CreateOp<'a>>> {
    let mut sorted: Vec<_> = ops
        .iter()
        .map(|&ptr| {
            // SAFETY: pointer is valid
            let op = unsafe { ptr.as_ref() };
            (ptr, create_op_priority(op))
        })
        .collect();

    // Stable sort by priority
    sorted.sort_by_key(|(_, priority)| *priority);

    sorted.into_iter().map(|(ptr, _)| ptr).collect()
}

/// Orders update operations within a list.
///
/// Reorders binding operations to ensure correct semantics:
/// StyleMap before ClassMap, StyleProp before ClassProp, etc.
/// Also applies "keepLast" transform for StyleMap and ClassMap.
///
/// This matches Angular's ordering algorithm exactly:
/// 1. Iterate through all ops one by one
/// 2. For handled ops: add to collection and remove from list
/// 3. When encountering an unhandled op OR a target change: insert collected ops BEFORE current op
/// 4. At the end: push any remaining collected ops to the end of the list
fn order_update_ops(list: &mut UpdateOpList<'_>) {
    if list.is_empty() {
        return;
    }

    // Collect ops that need reordering, grouped by target
    let mut ops_to_order: Vec<NonNull<UpdateOp<'_>>> = Vec::new();
    let mut first_target_in_group: Option<XrefId> = None;

    // First pass: collect all ops with their info so we can iterate safely
    let mut all_ops: Vec<(NonNull<UpdateOp<'_>>, Option<XrefId>, bool)> = Vec::new();

    let mut cursor = list.cursor_front();
    loop {
        if let Some(ptr) = cursor.current_ptr() {
            // SAFETY: pointer is valid from cursor
            let op = unsafe { ptr.as_ref() };
            let kind = op.kind();
            let is_handled = is_handled_update_op_kind(kind);
            let current_target = if is_handled { get_update_op_target(op) } else { None };
            all_ops.push((ptr, current_target, is_handled));
        }
        if !cursor.move_next() {
            break;
        }
    }

    // Second pass: process ops following Angular's algorithm exactly
    for (ptr, current_target, is_handled) in &all_ops {
        // Check if we should flush the current group:
        // 1. Current op is NOT handled, OR
        // 2. Current op has a different target than the first op in the group
        let should_flush = !is_handled
            || (current_target.is_some()
                && first_target_in_group.is_some()
                && *current_target != first_target_in_group);

        if should_flush && !ops_to_order.is_empty() {
            // Reorder the collected ops
            let reordered = reorder_update_ops(&ops_to_order);

            // Remove all ops from their current positions
            for reordered_ptr in &reordered {
                // SAFETY: pointer is valid
                unsafe { list.remove(*reordered_ptr) };
            }

            // Insert them before the current op
            for reordered_ptr in reordered {
                // SAFETY: pointer is valid
                let op = unsafe { std::ptr::read(reordered_ptr.as_ptr()) };
                unsafe { list.insert_before(*ptr, op) };
            }

            ops_to_order.clear();
            first_target_in_group = None;
        }

        if *is_handled {
            ops_to_order.push(*ptr);
            if first_target_in_group.is_none() {
                first_target_in_group = *current_target;
            }
        }
    }

    // Handle remaining ops at end of list
    if !ops_to_order.is_empty() {
        let reordered = reorder_update_ops(&ops_to_order);

        // Remove all ops from their current positions
        for ptr in &reordered {
            // SAFETY: pointer is valid
            unsafe { list.remove(*ptr) };
        }

        // Push to end of list
        for ptr in reordered {
            // SAFETY: pointer is valid
            let op = unsafe { std::ptr::read(ptr.as_ptr()) };
            list.push(op);
        }
    }
}

/// Reorders update ops by priority and applies keepLast transform.
fn reorder_update_ops<'a>(ops: &[NonNull<UpdateOp<'a>>]) -> Vec<NonNull<UpdateOp<'a>>> {
    let mut sorted: Vec<_> = ops
        .iter()
        .map(|&ptr| {
            // SAFETY: pointer is valid
            let op = unsafe { ptr.as_ref() };
            (ptr, update_op_priority(op), op.kind())
        })
        .collect();

    // Stable sort by priority
    sorted.sort_by_key(|(_, priority, _)| *priority);

    // Apply keepLast transform: only keep last StyleMap and last ClassMap
    let mut last_style_map_idx: Option<usize> = None;
    let mut last_class_map_idx: Option<usize> = None;

    for (i, (_, _, kind)) in sorted.iter().enumerate() {
        match kind {
            OpKind::StyleMap => last_style_map_idx = Some(i),
            OpKind::ClassMap => last_class_map_idx = Some(i),
            _ => {}
        }
    }

    sorted
        .into_iter()
        .enumerate()
        .filter(|(i, (_, _, kind))| match kind {
            OpKind::StyleMap => last_style_map_idx == Some(*i),
            OpKind::ClassMap => last_class_map_idx == Some(*i),
            _ => true,
        })
        .map(|(_, (ptr, _, _))| ptr)
        .collect()
}

/// Reorders update ops by priority for host bindings and applies keepLast transform.
fn reorder_update_ops_host<'a>(ops: &[NonNull<UpdateOp<'a>>]) -> Vec<NonNull<UpdateOp<'a>>> {
    let mut sorted: Vec<_> = ops
        .iter()
        .map(|&ptr| {
            // SAFETY: pointer is valid
            let op = unsafe { ptr.as_ref() };
            (ptr, update_op_priority_host(op), op.kind())
        })
        .collect();

    // Stable sort by priority
    sorted.sort_by_key(|(_, priority, _)| *priority);

    // Apply keepLast transform: only keep last StyleMap and last ClassMap
    let mut last_style_map_idx: Option<usize> = None;
    let mut last_class_map_idx: Option<usize> = None;

    for (i, (_, _, kind)) in sorted.iter().enumerate() {
        match kind {
            OpKind::StyleMap => last_style_map_idx = Some(i),
            OpKind::ClassMap => last_class_map_idx = Some(i),
            _ => {}
        }
    }

    sorted
        .into_iter()
        .enumerate()
        .filter(|(i, (_, _, kind))| match kind {
            OpKind::StyleMap => last_style_map_idx == Some(*i),
            OpKind::ClassMap => last_class_map_idx == Some(*i),
            _ => true,
        })
        .map(|(_, (ptr, _, _))| ptr)
        .collect()
}

/// Orders update operations within a list for host bindings.
///
/// Uses host-specific ordering: DomProperty < Attribute < StyleMap < ClassMap < StyleProp < ClassProp
///
/// This matches Angular's ordering algorithm exactly (same as order_update_ops but with host-specific priority).
fn order_update_ops_host(list: &mut UpdateOpList<'_>) {
    if list.is_empty() {
        return;
    }

    // Collect ops that need reordering, grouped by target
    let mut ops_to_order: Vec<NonNull<UpdateOp<'_>>> = Vec::new();
    let mut first_target_in_group: Option<XrefId> = None;

    // First pass: collect all ops with their info so we can iterate safely
    let mut all_ops: Vec<(NonNull<UpdateOp<'_>>, Option<XrefId>, bool)> = Vec::new();

    let mut cursor = list.cursor_front();
    loop {
        if let Some(ptr) = cursor.current_ptr() {
            // SAFETY: pointer is valid from cursor
            let op = unsafe { ptr.as_ref() };
            let kind = op.kind();
            let is_handled = is_handled_update_op_kind(kind);
            let current_target = if is_handled { get_update_op_target(op) } else { None };
            all_ops.push((ptr, current_target, is_handled));
        }
        if !cursor.move_next() {
            break;
        }
    }

    // Second pass: process ops following Angular's algorithm exactly
    for (ptr, current_target, is_handled) in &all_ops {
        // Check if we should flush the current group:
        // 1. Current op is NOT handled, OR
        // 2. Current op has a different target than the first op in the group
        let should_flush = !is_handled
            || (current_target.is_some()
                && first_target_in_group.is_some()
                && *current_target != first_target_in_group);

        if should_flush && !ops_to_order.is_empty() {
            // Reorder the collected ops using host-specific priority
            let reordered = reorder_update_ops_host(&ops_to_order);

            // Remove all ops from their current positions
            for reordered_ptr in &reordered {
                // SAFETY: pointer is valid
                unsafe { list.remove(*reordered_ptr) };
            }

            // Insert them before the current op
            for reordered_ptr in reordered {
                // SAFETY: pointer is valid
                let op = unsafe { std::ptr::read(reordered_ptr.as_ptr()) };
                unsafe { list.insert_before(*ptr, op) };
            }

            ops_to_order.clear();
            first_target_in_group = None;
        }

        if *is_handled {
            ops_to_order.push(*ptr);
            if first_target_in_group.is_none() {
                first_target_in_group = *current_target;
            }
        }
    }

    // Handle remaining ops at end of list
    if !ops_to_order.is_empty() {
        let reordered = reorder_update_ops_host(&ops_to_order);

        // Remove all ops from their current positions
        for ptr in &reordered {
            // SAFETY: pointer is valid
            unsafe { list.remove(*ptr) };
        }

        // Push to end of list
        for ptr in reordered {
            // SAFETY: pointer is valid
            let op = unsafe { std::ptr::read(ptr.as_ptr()) };
            list.push(op);
        }
    }
}

/// Orders operations for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// Uses host-specific ordering for update ops.
pub fn order_ops_for_host(job: &mut HostBindingCompilationJob<'_>) {
    order_create_ops(&mut job.root.create);
    order_update_ops_host(&mut job.root.update);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ops::{CreateOpBase, ListenerOp, SlotId, XrefId};
    use oxc_allocator::{Allocator, Vec as AllocVec};
    use oxc_str::Ident;

    fn make_listener_op<'a>(
        allocator: &'a Allocator,
        host_listener: bool,
        is_animation_listener: bool,
        legacy_animation_phase: Option<Ident<'a>>,
    ) -> CreateOp<'a> {
        CreateOp::Listener(ListenerOp {
            base: CreateOpBase::default(),
            target: XrefId(0),
            target_slot: SlotId(0),
            tag: None,
            host_listener,
            name: Ident::from(""),
            handler_expression: None,
            handler_ops: AllocVec::new_in(allocator),
            handler_fn_name: None,
            consume_fn_name: None,
            is_animation_listener,
            legacy_animation_phase,
            event_target: None,
            consumes_dollar_event: false,
        })
    }

    #[test]
    fn legacy_animation_host_listener_has_priority_zero() {
        let allocator = Allocator::default();
        // LegacyAnimation host listener (host_listener=true, is_animation_listener=true)
        // must get priority 0 so it is ordered before regular listeners.
        let op = make_listener_op(&allocator, true, true, Some(Ident::from("done")));
        assert_eq!(create_op_priority(&op), 0);
    }

    #[test]
    fn regular_host_listener_has_priority_one() {
        let allocator = Allocator::default();
        let op = make_listener_op(&allocator, true, false, None);
        assert_eq!(create_op_priority(&op), 1);
    }

    #[test]
    fn template_animation_listener_has_priority_one() {
        let allocator = Allocator::default();
        // Template-level animation listeners (host_listener=false) are NOT synthetic
        // host listeners and should NOT get priority 0.
        let op = make_listener_op(&allocator, false, true, Some(Ident::from("start")));
        assert_eq!(create_op_priority(&op), 1);
    }
}

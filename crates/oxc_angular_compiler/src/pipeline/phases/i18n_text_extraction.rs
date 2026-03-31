//! I18n text extraction phase.
//!
//! Removes text nodes within i18n blocks since they are already hardcoded into the i18n message.
//! Also, replaces interpolations on these text nodes with i18n expressions of the non-text portions,
//! which will be applied later. For text nodes with ICU placeholders, creates IcuPlaceholderOp
//! to track the static text and expression placeholders.
//!
//! Ported from Angular's `template/pipeline/src/phases/i18n_text_extraction.ts`.

use std::ptr::NonNull;

use oxc_span::Ident;
use rustc_hash::FxHashMap;

use crate::ir::enums::{I18nExpressionFor, I18nParamResolutionTime};
use crate::ir::ops::{
    CreateOp, CreateOpBase, I18nExpressionOp, I18nSlotHandle, IcuPlaceholderOp, SlotId, UpdateOp,
    UpdateOpBase, XrefId,
};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Converts i18n text to runtime i18n calls.
///
/// This phase:
/// 1. Removes text nodes within i18n blocks (their content is hardcoded in the i18n message)
/// 2. Replaces interpolations on text nodes with i18n expressions
/// 3. Handles ICU placeholders for text inside ICU expressions
pub fn convert_i18n_text(job: &mut ComponentCompilationJob<'_>) {
    // Process root view
    convert_i18n_text_in_view(job, job.root.xref);

    // Process all other views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for xref in view_xrefs {
        convert_i18n_text_in_view(job, xref);
    }
}

/// Converts i18n text in a single view.
fn convert_i18n_text_in_view(job: &mut ComponentCompilationJob<'_>, view_xref: XrefId) {
    let allocator = job.allocator;

    // Track text nodes within i18n blocks
    // Maps text xref -> (i18n_op_xref, i18n_context)
    let mut text_node_i18n_blocks: FxHashMap<XrefId, (XrefId, Option<XrefId>)> =
        FxHashMap::default();
    // Maps text xref -> (icu_xref, icu_context)
    let mut text_node_icus: FxHashMap<XrefId, Option<(XrefId, Option<XrefId>)>> =
        FxHashMap::default();

    // Track text ops to be replaced with IcuPlaceholder ops
    // (text_op_ptr, text_xref, icu_placeholder_name, initial_value)
    let mut text_ops_to_replace_with_icu: Vec<(
        NonNull<CreateOp<'_>>,
        XrefId,
        Ident<'_>,
        Ident<'_>,
    )> = Vec::new();

    // Track text ops to be removed (those without ICU placeholder)
    let mut text_nodes_to_remove: Vec<NonNull<CreateOp<'_>>> = Vec::new();

    // Maps text xref -> IcuPlaceholder xref (for InterpolateText conversion)
    let mut icu_placeholder_by_text: FxHashMap<XrefId, XrefId> = FxHashMap::default();

    // First pass: identify text nodes within i18n blocks
    {
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

        if let Some(view) = view {
            let mut current_i18n: Option<(XrefId, Option<XrefId>)> = None; // (xref, context)
            let mut current_icu: Option<(XrefId, Option<XrefId>)> = None; // (xref, context)

            for op in view.create.iter() {
                match op {
                    CreateOp::I18nStart(i18n_op) => {
                        current_i18n = Some((i18n_op.xref, i18n_op.context));
                    }
                    CreateOp::I18nEnd(_) => {
                        current_i18n = None;
                    }
                    CreateOp::IcuStart(icu_op) => {
                        current_icu = Some((icu_op.xref, icu_op.context));
                    }
                    CreateOp::IcuEnd(_) => {
                        current_icu = None;
                    }
                    CreateOp::Text(text_op) => {
                        if let Some((i18n_xref, i18n_context)) = current_i18n {
                            text_node_i18n_blocks.insert(text_op.xref, (i18n_xref, i18n_context));
                            text_node_icus.insert(text_op.xref, current_icu);

                            if let Some(ref icu_placeholder) = text_op.icu_placeholder {
                                // Text with ICU placeholder: will be replaced with IcuPlaceholderOp
                                text_ops_to_replace_with_icu.push((
                                    NonNull::from(op),
                                    text_op.xref,
                                    icu_placeholder.clone(),
                                    text_op.initial_value.clone(),
                                ));
                            } else {
                                // Text without ICU placeholder: will be removed
                                text_nodes_to_remove.push(NonNull::from(op));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Replace text ops with IcuPlaceholder ops
    for (text_ptr, text_xref, icu_placeholder_name, initial_value) in text_ops_to_replace_with_icu {
        // Allocate a new xref for the IcuPlaceholder op
        let icu_xref = job.allocate_xref_id();

        // Track mapping from text xref to IcuPlaceholder xref
        icu_placeholder_by_text.insert(text_xref, icu_xref);

        // Create the IcuPlaceholder op with initial_value as the first string
        let mut strings = oxc_allocator::Vec::new_in(allocator);
        strings.push(initial_value);

        let icu_placeholder_op = CreateOp::IcuPlaceholder(IcuPlaceholderOp {
            base: CreateOpBase::default(),
            xref: icu_xref,
            name: icu_placeholder_name,
            strings,
            expression_placeholders: oxc_allocator::Vec::new_in(allocator),
        });

        // Replace the Text op with the IcuPlaceholder op
        if view_xref.0 == 0 {
            // SAFETY: text_ptr is a valid pointer we obtained from iteration
            unsafe {
                job.root.create.replace(text_ptr, icu_placeholder_op);
            }
        } else if let Some(view) = job.view_mut(view_xref) {
            unsafe {
                view.create.replace(text_ptr, icu_placeholder_op);
            }
        }
    }

    // Remove text nodes that are inside i18n blocks (those without ICU placeholder)
    // (These are already accounted for in the translated message)
    for text_ptr in text_nodes_to_remove {
        if view_xref.0 == 0 {
            // SAFETY: text_ptr is a valid pointer we obtained from iteration
            unsafe {
                job.root.create.remove(text_ptr);
            }
        } else if let Some(view) = job.view_mut(view_xref) {
            unsafe {
                view.create.remove(text_ptr);
            }
        }
    }

    // Second pass: convert InterpolateText ops targeting removed text nodes
    // Collect info about InterpolateText ops that need conversion
    // We need to store enough info to recreate the I18nExpressionOps
    struct InterpolateOpInfo<'a> {
        op_ptr: NonNull<UpdateOp<'a>>,
        i18n_xref: XrefId,
        i18n_context: Option<XrefId>,
        icu_info: Option<(XrefId, Option<XrefId>)>,
        icu_placeholder_xref: Option<XrefId>,
        i18n_handle: I18nSlotHandle,
    }

    let mut interpolate_ops_to_convert: Vec<InterpolateOpInfo<'_>> = Vec::new();

    {
        // First, collect I18nStart handles
        let mut i18n_start_handles: FxHashMap<XrefId, I18nSlotHandle> = FxHashMap::default();
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

        if let Some(view) = view {
            for op in view.create.iter() {
                if let CreateOp::I18nStart(i18n_op) = op {
                    if let Some(slot) = i18n_op.slot {
                        i18n_start_handles.insert(i18n_op.xref, I18nSlotHandle::Single(slot));
                    }
                }
            }
        }

        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

        if let Some(view) = view {
            for op in view.update.iter() {
                if let UpdateOp::InterpolateText(interp_op) = op {
                    if let Some(&(i18n_xref, i18n_context)) =
                        text_node_i18n_blocks.get(&interp_op.target)
                    {
                        let icu_info = text_node_icus.get(&interp_op.target).copied().flatten();
                        // Get the IcuPlaceholder xref if this text was replaced by one
                        let icu_placeholder_xref =
                            icu_placeholder_by_text.get(&interp_op.target).copied();
                        // Get the handle from the I18nStart op
                        let i18n_handle = i18n_start_handles
                            .get(&i18n_xref)
                            .copied()
                            .unwrap_or(I18nSlotHandle::Single(SlotId(0)));

                        interpolate_ops_to_convert.push(InterpolateOpInfo {
                            op_ptr: NonNull::from(op),
                            i18n_xref,
                            i18n_context,
                            icu_info,
                            icu_placeholder_xref,
                            i18n_handle,
                        });
                    }
                }
            }
        }
    }

    // Convert InterpolateText ops to I18nExpression ops
    // We need to iterate through each expression in the interpolation
    for info in interpolate_ops_to_convert {
        // Determine the context and resolution time based on whether we're in an ICU
        let (context_id, resolution_time) = if let Some((_, icu_context)) = info.icu_info {
            (
                icu_context.unwrap_or(info.i18n_context.unwrap_or(info.i18n_xref)),
                I18nParamResolutionTime::Postprocessing,
            )
        } else {
            (info.i18n_context.unwrap_or(info.i18n_xref), I18nParamResolutionTime::Creation)
        };

        // Extract expressions and i18n placeholders from the interpolation
        // SAFETY: op_ptr is valid as it came from iteration
        let interp_op = unsafe { &*info.op_ptr.as_ptr() };
        let UpdateOp::InterpolateText(interp) = interp_op else {
            continue;
        };

        // Get the interpolation expressions
        let interpolation = &interp.interpolation;

        // Create an I18nExpression op for each expression in the interpolation
        // Ported from Angular's i18n_text_extraction.ts lines 88-107
        let mut i18n_expressions: Vec<UpdateOp<'_>> = Vec::new();

        // Use the interpolate op's source span for all expressions
        let source_span = interp.base.source_span;

        match interpolation.as_ref() {
            crate::ir::expression::IrExpression::Interpolation(ir_interp) => {
                for (i, expr) in ir_interp.expressions.iter().enumerate() {
                    // Get the i18n placeholder for this expression if available
                    let i18n_placeholder = ir_interp.i18n_placeholders.get(i).cloned();

                    i18n_expressions.push(UpdateOp::I18nExpression(I18nExpressionOp {
                        base: UpdateOpBase { source_span, ..Default::default() },
                        i18n_owner: info.i18n_xref,
                        target: info.i18n_xref,
                        context: context_id,
                        handle: info.i18n_handle,
                        expression: oxc_allocator::Box::new_in(expr.clone_in(allocator), allocator),
                        resolution_time,
                        usage: I18nExpressionFor::I18nText,
                        name: oxc_span::Ident::from(""),
                        i18n_placeholder,
                        icu_placeholder: info.icu_placeholder_xref,
                    }));
                }

                // If this interpolation is part of an ICU placeholder, update its strings
                if let Some(icu_xref) = info.icu_placeholder_xref {
                    // Find and update the IcuPlaceholder op with the interpolation strings
                    let view = if view_xref.0 == 0 {
                        Some(&mut job.root)
                    } else {
                        job.view_mut(view_xref)
                    };

                    if let Some(view) = view {
                        for op in view.create.iter_mut() {
                            if let CreateOp::IcuPlaceholder(icu_op) = op {
                                if icu_op.xref == icu_xref {
                                    // Update strings from the interpolation
                                    icu_op.strings.clear();
                                    for s in ir_interp.strings.iter() {
                                        icu_op.strings.push(s.clone());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                // Not an interpolation expression - create a single I18nExpression
                i18n_expressions.push(UpdateOp::I18nExpression(I18nExpressionOp {
                    base: UpdateOpBase { source_span, ..Default::default() },
                    i18n_owner: info.i18n_xref,
                    target: info.i18n_xref,
                    context: context_id,
                    handle: info.i18n_handle,
                    expression: oxc_allocator::Box::new_in(
                        interpolation.clone_in(allocator),
                        allocator,
                    ),
                    resolution_time,
                    usage: I18nExpressionFor::I18nText,
                    name: oxc_span::Ident::from(""),
                    i18n_placeholder: None,
                    icu_placeholder: info.icu_placeholder_xref,
                }));
            }
        }

        // Replace the original InterpolateText op with the I18nExpression ops
        // maintaining the correct position in the update list
        if !i18n_expressions.is_empty() {
            if view_xref.0 == 0 {
                // SAFETY: op_ptr is a valid pointer we obtained from iteration
                unsafe {
                    // Replace the first expression at the same position
                    let mut current_ptr = job
                        .root
                        .update
                        .replace_returning_new(info.op_ptr, i18n_expressions.remove(0));
                    // Insert additional expressions after it
                    for expr_op in i18n_expressions {
                        current_ptr =
                            job.root.update.insert_after_returning_new(current_ptr, expr_op);
                    }
                }
            } else if let Some(view) = job.view_mut(view_xref) {
                unsafe {
                    let mut current_ptr =
                        view.update.replace_returning_new(info.op_ptr, i18n_expressions.remove(0));
                    for expr_op in i18n_expressions {
                        current_ptr = view.update.insert_after_returning_new(current_ptr, expr_op);
                    }
                }
            }
        } else if view_xref.0 == 0 {
            // No expressions to add, just remove the InterpolateText op
            unsafe {
                job.root.update.remove(info.op_ptr);
            }
        } else if let Some(view) = job.view_mut(view_xref) {
            unsafe {
                view.update.remove(info.op_ptr);
            }
        }
    }
}

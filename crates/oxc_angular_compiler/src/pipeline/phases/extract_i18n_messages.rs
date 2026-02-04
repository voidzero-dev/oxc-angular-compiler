//! Extract i18n messages phase.
//!
//! Formats the param maps on extracted message ops into maps of Expression objects that can be
//! used in the final output.
//!
//! Ported from Angular's `template/pipeline/src/phases/extract_i18n_messages.ts`.

use std::ptr::NonNull;

use oxc_span::Atom;
use rustc_hash::FxHashMap;

use crate::ir::enums::{I18nContextKind, I18nParamValueFlags};
use crate::ir::i18n_params::{I18nParamValue, I18nParamValueContent};
use crate::ir::ops::{CreateOp, CreateOpBase, I18nMessageOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

// ============================================================================
// The following constants and functions are for formatting i18n params.
// They will be used by the i18n_const_collection phase for code generation.
// ============================================================================

/// The escape sequence used to indicate message param values.
const ESCAPE: char = '\u{FFFD}';

/// Marker used to indicate an element tag.
const ELEMENT_MARKER: char = '#';

/// Marker used to indicate a template tag.
const TEMPLATE_MARKER: char = '*';

/// Marker used to indicate closing of an element or template tag.
const TAG_CLOSE_MARKER: char = '/';

/// Marker used to indicate the sub-template context.
const CONTEXT_MARKER: char = ':';

/// Marker used to indicate the start of a list of values.
const LIST_START_MARKER: char = '[';

/// Marker used to indicate the end of a list of values.
const LIST_END_MARKER: char = ']';

/// Delimiter used to separate multiple values in a list.
const LIST_DELIMITER: char = '|';

/// Extracts i18n messages to the consts array.
///
/// This phase:
/// 1. Creates an I18nMessage op for each I18nContext op
/// 2. Associates sub-messages for ICUs with their root message
/// 3. Removes IcuStart/IcuEnd ops as they are no longer needed
pub fn extract_i18n_messages(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect i18n blocks and contexts
    let mut i18n_blocks: FxHashMap<XrefId, (Option<XrefId>, Option<XrefId>)> = FxHashMap::default(); // xref -> (root, context)
    let mut i18n_contexts: FxHashMap<XrefId, (I18nContextKind, Option<XrefId>)> =
        FxHashMap::default(); // xref -> (kind, i18n_block)
    let mut context_params: FxHashMap<XrefId, Vec<(Atom<'_>, Vec<I18nParamValue>)>> =
        FxHashMap::default();

    // Create an i18n message for each context.
    let mut i18n_messages_by_context: FxHashMap<XrefId, XrefId> = FxHashMap::default();

    // First pass: collect info from views
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    // Collect context info
    for view in job.all_views() {
        for op in view.create.iter() {
            match op {
                CreateOp::I18nStart(i18n_op) => {
                    i18n_blocks.insert(i18n_op.xref, (i18n_op.root, i18n_op.context));
                }
                CreateOp::I18nContext(ctx_op) => {
                    i18n_contexts.insert(ctx_op.xref, (ctx_op.context_kind, ctx_op.i18n_block));

                    // Collect params as Vec for checking if postprocessing is needed
                    let params: Vec<_> = ctx_op
                        .params
                        .iter()
                        .map(|(k, v)| (k.clone(), v.iter().copied().collect::<Vec<_>>()))
                        .collect();
                    context_params.insert(ctx_op.xref, params);
                }
                _ => {}
            }
        }
    }

    // Allocate message xrefs
    let context_xrefs: Vec<XrefId> = i18n_contexts.keys().copied().collect();
    for ctx_xref in context_xrefs {
        let message_xref = job.allocate_xref_id();
        i18n_messages_by_context.insert(ctx_xref, message_xref);
    }

    // Second pass: create message ops with formatted params
    for view_xref in &view_xrefs {
        let mut messages_to_add: Vec<I18nMessageOp<'_>> = Vec::new();

        {
            let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(*view_xref) };

            if let Some(view) = view {
                for op in view.create.iter() {
                    if let CreateOp::I18nContext(ctx_op) = op {
                        if let Some(&message_xref) = i18n_messages_by_context.get(&ctx_op.xref) {
                            // Check if postprocessing is needed (any param has multiple values)
                            let params_data =
                                context_params.get(&ctx_op.xref).cloned().unwrap_or_default();
                            let needs_postprocessing =
                                params_data.iter().any(|(_, values)| values.len() > 1);

                            // Get the i18n_block from the context
                            let i18n_block =
                                i18n_contexts.get(&ctx_op.xref).and_then(|(_, block)| *block);

                            let metadata = ctx_op.message.and_then(|instance_id| {
                                job.i18n_message_metadata.get(&instance_id)
                            });

                            messages_to_add.push(I18nMessageOp {
                                base: CreateOpBase::default(),
                                xref: message_xref,
                                i18n_context: Some(ctx_op.xref),
                                i18n_block,
                                message_placeholder: None, // Set for ICU sub-messages
                                message_id: metadata.and_then(|m| m.message_id.clone()),
                                custom_id: metadata.and_then(|m| m.custom_id.clone()),
                                meaning: metadata.and_then(|m| m.meaning.clone()),
                                description: metadata.and_then(|m| m.description.clone()),
                                message_string: metadata.and_then(|m| m.message_string.clone()),
                                needs_postprocessing,
                                sub_messages: oxc_allocator::Vec::new_in(allocator),
                            });
                        }
                    }
                }
            }
        }

        // Add the message ops
        for msg in messages_to_add {
            if view_xref.0 == 0 {
                job.root.create.push(CreateOp::I18nMessage(msg));
            } else if let Some(view) = job.view_mut(*view_xref) {
                view.create.push(CreateOp::I18nMessage(msg));
            }
        }
    }

    // Third pass: handle ICU sub-messages
    // Collect ICU info first to avoid borrow issues
    let mut icu_sub_message_associations: Vec<(XrefId, XrefId, Option<Atom<'_>>)> = Vec::new();

    for view_xref in &view_xrefs {
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(*view_xref) };

        if let Some(view) = view {
            for op in view.create.iter() {
                if let CreateOp::IcuStart(icu_op) = op {
                    if let Some(ctx_xref) = icu_op.context {
                        // Skip non-ICU contexts
                        if let Some(&(kind, i18n_block)) = i18n_contexts.get(&ctx_xref) {
                            if kind != I18nContextKind::Icu {
                                continue;
                            }

                            // Skip ICUs that share context with their i18n message (root-level ICUs)
                            if let Some(block_xref) = i18n_block {
                                if let Some(&(root_block_xref_opt, block_ctx)) =
                                    i18n_blocks.get(&block_xref)
                                {
                                    if block_ctx == Some(ctx_xref) {
                                        continue;
                                    }

                                    // Find root message via root i18n block
                                    // If root_block_xref is None, the block itself IS the root
                                    // (matches TypeScript's `root: root ?? xref`)
                                    let actual_root_xref =
                                        root_block_xref_opt.unwrap_or(block_xref);

                                    if let Some(&(_, root_ctx)) = i18n_blocks.get(&actual_root_xref)
                                    {
                                        if let Some(root_ctx_xref) = root_ctx {
                                            // Record the association: (root_ctx, sub_ctx, placeholder)
                                            icu_sub_message_associations.push((
                                                root_ctx_xref,
                                                ctx_xref,
                                                icu_op.icu_placeholder.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Apply the ICU sub-message associations to the message ops
    for (root_ctx_xref, sub_ctx_xref, placeholder) in icu_sub_message_associations {
        if let (Some(&root_msg_xref), Some(&sub_msg_xref)) = (
            i18n_messages_by_context.get(&root_ctx_xref),
            i18n_messages_by_context.get(&sub_ctx_xref),
        ) {
            // Find the sub-message and set its placeholder
            for view_xref in &view_xrefs {
                let view =
                    if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(*view_xref) };

                if let Some(view) = view {
                    for op in view.create.iter_mut() {
                        if let CreateOp::I18nMessage(msg) = op {
                            if msg.xref == sub_msg_xref {
                                msg.message_placeholder = placeholder.clone();
                            } else if msg.xref == root_msg_xref {
                                msg.sub_messages.push(sub_msg_xref);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fourth pass: process IcuPlaceholder ops
    // Format ICU placeholders and add to context's icu_placeholder_literals
    {
        let view_xrefs_for_icu: Vec<XrefId> =
            std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

        // Collect IcuPlaceholder info first (to avoid borrow issues)
        let mut icu_placeholders_to_process: Vec<(
            XrefId,                // view_xref
            NonNull<CreateOp<'_>>, // op_ptr
            XrefId,                // icu_context_xref
            Atom<'_>,              // placeholder name
            String,                // formatted value
        )> = Vec::new();

        for view_xref in &view_xrefs_for_icu {
            let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(*view_xref) };

            if let Some(view) = view {
                let mut current_icu: Option<XrefId> = None; // current ICU's context

                for op in view.create.iter() {
                    match op {
                        CreateOp::IcuStart(icu_op) => {
                            current_icu = icu_op.context;
                        }
                        CreateOp::IcuEnd(_) => {
                            current_icu = None;
                        }
                        CreateOp::IcuPlaceholder(icu_placeholder) => {
                            if let Some(icu_context) = current_icu {
                                // Format the ICU placeholder value
                                let formatted = format_icu_placeholder(
                                    icu_placeholder.strings.as_slice(),
                                    icu_placeholder.expression_placeholders.as_slice(),
                                );

                                icu_placeholders_to_process.push((
                                    *view_xref,
                                    NonNull::from(op),
                                    icu_context,
                                    icu_placeholder.name.clone(),
                                    formatted,
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Apply ICU placeholder values to contexts and remove the ops
        for (view_xref, op_ptr, context_xref, name, formatted) in icu_placeholders_to_process {
            // Find the I18nContext and add the icu_placeholder_literal
            // Allocate the formatted string in the arena
            let formatted_atom = Atom::from(allocator.alloc_str(&formatted));
            for vx in &view_xrefs_for_icu {
                let view = if vx.0 == 0 { Some(&mut job.root) } else { job.view_mut(*vx) };

                if let Some(view) = view {
                    for op in view.create.iter_mut() {
                        if let CreateOp::I18nContext(ctx_op) = op {
                            if ctx_op.xref == context_xref {
                                ctx_op
                                    .icu_placeholder_literals
                                    .insert(name.clone(), formatted_atom.clone());
                                break;
                            }
                        }
                    }
                }
            }

            // Remove the IcuPlaceholder op
            if view_xref.0 == 0 {
                unsafe {
                    job.root.create.remove(op_ptr);
                }
            } else if let Some(view) = job.view_mut(view_xref) {
                unsafe {
                    view.create.remove(op_ptr);
                }
            }
        }
    }

    // Fifth pass: remove IcuStart/IcuEnd ops
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();
    for view_xref in view_xrefs {
        let mut ops_to_remove: Vec<NonNull<CreateOp<'_>>> = Vec::new();

        {
            let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

            if let Some(view) = view {
                for op in view.create.iter() {
                    if matches!(op, CreateOp::IcuStart(_) | CreateOp::IcuEnd(_)) {
                        ops_to_remove.push(NonNull::from(op));
                    }
                }
            }
        }

        for op_ptr in ops_to_remove {
            if view_xref.0 == 0 {
                unsafe {
                    job.root.create.remove(op_ptr);
                }
            } else if let Some(view) = job.view_mut(view_xref) {
                unsafe {
                    view.create.remove(op_ptr);
                }
            }
        }
    }
}

/// Formats a list of params into (placeholder, formatted_value) pairs.
/// Returns the formatted params and whether postprocessing is needed.
pub fn format_params(
    params: &[(Atom<'_>, Vec<I18nParamValue>)],
) -> (std::vec::Vec<(String, String)>, bool) {
    let mut formatted = std::vec::Vec::new();
    let mut needs_postprocessing = false;

    for (placeholder, values) in params {
        if values.len() > 1 {
            needs_postprocessing = true;
        }

        if let Some(serialized) = format_param_values(values) {
            formatted.push((placeholder.to_string(), serialized));
        }
    }

    (formatted, needs_postprocessing)
}

/// Formats an array of I18nParamValue into a string (or None for empty array).
pub fn format_param_values(values: &[I18nParamValue]) -> Option<String> {
    if values.is_empty() {
        return None;
    }

    let serialized: Vec<String> = values.iter().map(format_value).collect();

    match serialized.len() {
        1 => serialized.into_iter().next(),
        _ => Some(format!(
            "{}{}{}",
            LIST_START_MARKER,
            serialized.join(&LIST_DELIMITER.to_string()),
            LIST_END_MARKER
        )),
    }
}

/// Formats an ICU placeholder op into a string that interleaves static strings
/// with expression placeholders.
///
/// For example, if strings = ["Hello ", "!"] and expression_placeholders has one
/// entry with value 0, the output would be "Hello ${�0�}!" where the expression
/// placeholder is formatted using format_value.
///
/// Ported from Angular's `formatIcuPlaceholder` function.
pub fn format_icu_placeholder(
    strings: &[Atom<'_>],
    expression_placeholders: &[I18nParamValue],
) -> String {
    let mut result = String::new();
    for (i, s) in strings.iter().enumerate() {
        result.push_str(s.as_str());
        if let Some(expr_value) = expression_placeholders.get(i) {
            // Format as ${value} where value is the formatted expression placeholder
            result.push_str(&format!("${{{}}}", format_value(expr_value)));
        }
    }
    result
}

/// Formats a single I18nParamValue into a string.
fn format_value(value: &I18nParamValue) -> String {
    let flags = value.flags;

    // Element tags with a structural directive use a special form that concatenates the element and
    // template values.
    if flags.contains(I18nParamValueFlags::ELEMENT_TAG)
        && flags.contains(I18nParamValueFlags::TEMPLATE_TAG)
    {
        if let I18nParamValueContent::Compound { element, template } = value.value {
            let element_value = format_value(&I18nParamValue::new(
                I18nParamValueContent::Slot(element),
                value.sub_template_index,
                flags.without(I18nParamValueFlags::TEMPLATE_TAG),
            ));
            let template_value = format_value(&I18nParamValue::new(
                I18nParamValueContent::Slot(template),
                value.sub_template_index,
                flags.without(I18nParamValueFlags::ELEMENT_TAG),
            ));

            // Handle self-closing case
            if flags.contains(I18nParamValueFlags::OPEN_TAG)
                && flags.contains(I18nParamValueFlags::CLOSE_TAG)
            {
                return format!("{}{}{}", template_value, element_value, template_value);
            }

            // Flip order based on close/open tag
            return if flags.contains(I18nParamValueFlags::CLOSE_TAG) {
                format!("{}{}", element_value, template_value)
            } else {
                format!("{}{}", template_value, element_value)
            };
        }
    }

    // Self-closing tags use a special form that concatenates the start and close tag values.
    if flags.contains(I18nParamValueFlags::OPEN_TAG)
        && flags.contains(I18nParamValueFlags::CLOSE_TAG)
    {
        let open_value = format_value(&I18nParamValue::new(
            value.value,
            value.sub_template_index,
            flags.without(I18nParamValueFlags::CLOSE_TAG),
        ));
        let close_value = format_value(&I18nParamValue::new(
            value.value,
            value.sub_template_index,
            flags.without(I18nParamValueFlags::OPEN_TAG),
        ));
        return format!("{}{}", open_value, close_value);
    }

    // If there are no special flags, just return the raw value.
    if flags.bits() == 0 {
        return match value.value {
            I18nParamValueContent::Slot(slot) => slot.to_string(),
            I18nParamValueContent::Compound { element, .. } => element.to_string(),
        };
    }

    // Encode the remaining flags as part of the value.
    let tag_marker = if flags.contains(I18nParamValueFlags::ELEMENT_TAG) {
        ELEMENT_MARKER
    } else if flags.contains(I18nParamValueFlags::TEMPLATE_TAG) {
        TEMPLATE_MARKER
    } else {
        '\0' // No marker
    };

    let close_marker = if tag_marker != '\0' && flags.contains(I18nParamValueFlags::CLOSE_TAG) {
        TAG_CLOSE_MARKER
    } else {
        '\0'
    };

    let slot_value = match value.value {
        I18nParamValueContent::Slot(slot) => slot,
        I18nParamValueContent::Compound { element, .. } => element,
    };

    let context = match value.sub_template_index {
        Some(idx) => format!("{}{}", CONTEXT_MARKER, idx),
        None => String::new(),
    };

    if tag_marker != '\0' {
        if close_marker != '\0' {
            format!("{}{}{}{}{}{}", ESCAPE, close_marker, tag_marker, slot_value, context, ESCAPE)
        } else {
            format!("{}{}{}{}{}", ESCAPE, tag_marker, slot_value, context, ESCAPE)
        }
    } else {
        // Expression index case
        format!("{}{}{}{}", ESCAPE, slot_value, context, ESCAPE)
    }
}

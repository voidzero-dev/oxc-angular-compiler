//! Resolve i18n element placeholders phase.
//!
//! Resolve the element placeholders in i18n messages.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_i18n_element_placeholders.ts`.

use oxc_span::Ident;
use rustc_hash::FxHashMap;

use crate::ir::enums::{I18nParamValueFlags, TemplateKind};
use crate::ir::i18n_params::{I18nParamValue, I18nParamValueContent};
use crate::ir::ops::{CreateOp, I18nPlaceholder, SlotId, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Resolves element placeholders in i18n messages.
///
/// This phase:
/// 1. Records all element and i18n context ops
/// 2. Resolves element tag placeholders with slot indices
/// 3. Handles element tag opening/closing pairs
/// 4. Handles template/conditional/repeater tag placeholders for nested views
pub fn resolve_i18n_element_placeholders(job: &mut ComponentCompilationJob<'_>) {
    // Record all i18n context ops and element start ops
    let mut i18n_contexts: FxHashMap<XrefId, XrefId> = FxHashMap::default();
    let mut elements: FxHashMap<XrefId, ElementInfo> = FxHashMap::default();

    // Collect from all views
    for view in job.all_views() {
        for op in view.create.iter() {
            match op {
                CreateOp::I18nContext(ctx_op) => {
                    i18n_contexts.insert(ctx_op.xref, ctx_op.xref);
                }
                CreateOp::ElementStart(elem_op) => {
                    elements.insert(
                        elem_op.xref,
                        ElementInfo {
                            slot: elem_op.slot,
                            i18n_placeholder: elem_op.i18n_placeholder.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
    }

    // Process placeholders for root view
    resolve_placeholders_for_view(job, job.root.xref, &i18n_contexts, &elements, None);

    // Process placeholders for other views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs {
        resolve_placeholders_for_view(job, view_xref, &i18n_contexts, &elements, None);
    }
}

/// Information about an element for placeholder resolution.
#[derive(Clone)]
struct ElementInfo<'a> {
    slot: Option<SlotId>,
    i18n_placeholder: Option<I18nPlaceholder<'a>>,
}

/// Pending structural directive info for combined placeholder values.
#[derive(Clone, Copy)]
struct PendingStructuralDirective {
    slot: SlotId,
}

/// Current i18n block and context tracking.
struct CurrentI18nOps {
    i18n_context_xref: XrefId,
    sub_template_index: Option<u32>,
}

/// Recursively resolves element and template tag placeholders in the given view.
fn resolve_placeholders_for_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    i18n_contexts: &FxHashMap<XrefId, XrefId>,
    elements: &FxHashMap<XrefId, ElementInfo<'a>>,
    pending_structural_directive: Option<PendingStructuralDirective>,
) {
    let allocator = job.allocator;

    // Track the current i18n op and corresponding i18n context op
    let mut current_ops: Option<CurrentI18nOps> = None;
    let mut pending_structural_directive_closes: FxHashMap<XrefId, PendingStructuralDirective> =
        FxHashMap::default();
    let mut pending_structural = pending_structural_directive;

    // Collect operations and context info in first pass
    let mut operations: Vec<OpInfo> = Vec::new();
    let mut child_views_to_process: Vec<(XrefId, Option<PendingStructuralDirective>)> = Vec::new();

    {
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

        if let Some(view) = view {
            for op in view.create.iter() {
                match op {
                    CreateOp::I18nStart(i18n_op) => {
                        if let Some(context_xref) = i18n_op.context {
                            if i18n_contexts.contains_key(&context_xref) {
                                current_ops = Some(CurrentI18nOps {
                                    i18n_context_xref: context_xref,
                                    sub_template_index: i18n_op.sub_template_index,
                                });
                            }
                        }
                    }
                    CreateOp::I18nEnd(_) => {
                        current_ops = None;
                    }
                    CreateOp::ElementStart(elem_op) => {
                        if let Some(ref placeholder) = elem_op.i18n_placeholder {
                            if let Some(ref ops) = current_ops {
                                // Record element start using start_name
                                operations.push(OpInfo::ElementStart {
                                    slot: elem_op.slot.map(|s| s.0),
                                    start_name: placeholder.start_name.clone(),
                                    context_xref: ops.i18n_context_xref,
                                    sub_template_index: ops.sub_template_index,
                                    pending_structural,
                                    has_close_name: placeholder.close_name.is_some(),
                                });

                                // Save pending structural for closing tag
                                if let Some(structural) = pending_structural {
                                    pending_structural_directive_closes
                                        .insert(elem_op.xref, structural);
                                }
                                pending_structural = None;
                            }
                        }
                    }
                    CreateOp::ElementEnd(elem_end) => {
                        if let Some(elem_info) = elements.get(&elem_end.xref) {
                            // Use close_name for ElementEnd
                            if let Some(ref placeholder) = elem_info.i18n_placeholder {
                                if let Some(close_name) = &placeholder.close_name {
                                    if let Some(ref ops) = current_ops {
                                        let structural = pending_structural_directive_closes
                                            .get(&elem_end.xref)
                                            .copied();
                                        operations.push(OpInfo::ElementEnd {
                                            slot: elem_info.slot.map(|s| s.0),
                                            close_name: close_name.clone(),
                                            context_xref: ops.i18n_context_xref,
                                            sub_template_index: ops.sub_template_index,
                                            pending_structural: structural,
                                        });
                                        pending_structural_directive_closes.remove(&elem_end.xref);
                                    }
                                }
                            }
                        }
                    }
                    CreateOp::Projection(proj_op) => {
                        if let Some(ref placeholder) = proj_op.i18n_placeholder {
                            if let Some(ref ops) = current_ops {
                                // Record start for projection using start_name
                                if let Some(slot) = proj_op.slot {
                                    operations.push(OpInfo::ElementStart {
                                        slot: Some(slot.0),
                                        start_name: placeholder.start_name.clone(),
                                        context_xref: ops.i18n_context_xref,
                                        sub_template_index: ops.sub_template_index,
                                        pending_structural,
                                        has_close_name: placeholder.close_name.is_some(),
                                    });
                                    // Record end for projection using close_name if present
                                    if let Some(close_name) = &placeholder.close_name {
                                        operations.push(OpInfo::ElementEnd {
                                            slot: Some(slot.0),
                                            close_name: close_name.clone(),
                                            context_xref: ops.i18n_context_xref,
                                            sub_template_index: ops.sub_template_index,
                                            pending_structural,
                                        });
                                    }
                                }
                                pending_structural = None;
                            }
                        }

                        // Handle fallback view
                        if let Some(fallback_xref) = proj_op.fallback {
                            if let Some(ref fallback_placeholder) =
                                proj_op.fallback_i18n_placeholder
                            {
                                if let Some(ref ops) = current_ops {
                                    // Record template start/end for fallback view
                                    if let Some(slot) = proj_op.slot {
                                        operations.push(OpInfo::TemplateStart {
                                            view_xref: fallback_xref,
                                            slot: slot.0,
                                            start_name: fallback_placeholder.start_name.clone(),
                                            context_xref: ops.i18n_context_xref,
                                            sub_template_index: ops.sub_template_index,
                                            pending_structural,
                                            has_close_name: fallback_placeholder
                                                .close_name
                                                .is_some(),
                                        });
                                        if let Some(close_name) = &fallback_placeholder.close_name {
                                            operations.push(OpInfo::TemplateEnd {
                                                view_xref: fallback_xref,
                                                slot: slot.0,
                                                close_name: close_name.clone(),
                                                context_xref: ops.i18n_context_xref,
                                                pending_structural,
                                            });
                                        }
                                    }
                                }
                            }
                            child_views_to_process.push((fallback_xref, None));
                        }
                    }
                    CreateOp::Template(template_op) => {
                        let template_view_xref = template_op.xref;
                        if template_op.i18n_placeholder.is_none() {
                            // No i18n placeholder, just recurse
                            child_views_to_process.push((template_view_xref, None));
                        } else if let Some(ref placeholder) = template_op.i18n_placeholder {
                            if let Some(ref ops) = current_ops {
                                if template_op.template_kind == TemplateKind::Structural {
                                    // Structural directive - pass as pending
                                    if let Some(slot) = template_op.slot {
                                        child_views_to_process.push((
                                            template_view_xref,
                                            Some(PendingStructuralDirective { slot }),
                                        ));
                                    }
                                } else {
                                    // Non-structural template - record start and end
                                    if let Some(slot) = template_op.slot {
                                        operations.push(OpInfo::TemplateStart {
                                            view_xref: template_view_xref,
                                            slot: slot.0,
                                            start_name: placeholder.start_name.clone(),
                                            context_xref: ops.i18n_context_xref,
                                            sub_template_index: ops.sub_template_index,
                                            pending_structural,
                                            has_close_name: placeholder.close_name.is_some(),
                                        });
                                        child_views_to_process.push((template_view_xref, None));
                                        if let Some(close_name) = &placeholder.close_name {
                                            operations.push(OpInfo::TemplateEnd {
                                                view_xref: template_view_xref,
                                                slot: slot.0,
                                                close_name: close_name.clone(),
                                                context_xref: ops.i18n_context_xref,
                                                pending_structural,
                                            });
                                        }
                                    }
                                    pending_structural = None;
                                }
                            }
                        }
                    }
                    CreateOp::Conditional(cond_op) => {
                        let cond_view_xref = cond_op.xref;
                        if cond_op.i18n_placeholder.is_none() {
                            child_views_to_process.push((cond_view_xref, None));
                        } else if let Some(ref placeholder) = cond_op.i18n_placeholder {
                            if let Some(ref ops) = current_ops {
                                // Record conditional start/end
                                if let Some(slot) = cond_op.slot {
                                    operations.push(OpInfo::TemplateStart {
                                        view_xref: cond_view_xref,
                                        slot: slot.0,
                                        start_name: placeholder.start_name.clone(),
                                        context_xref: ops.i18n_context_xref,
                                        sub_template_index: ops.sub_template_index,
                                        pending_structural,
                                        has_close_name: placeholder.close_name.is_some(),
                                    });
                                    child_views_to_process.push((cond_view_xref, None));
                                    if let Some(close_name) = &placeholder.close_name {
                                        operations.push(OpInfo::TemplateEnd {
                                            view_xref: cond_view_xref,
                                            slot: slot.0,
                                            close_name: close_name.clone(),
                                            context_xref: ops.i18n_context_xref,
                                            pending_structural,
                                        });
                                    }
                                }
                                pending_structural = None;
                            }
                        }
                    }
                    CreateOp::RepeaterCreate(rep_op) => {
                        // RepeaterCreate has 3 slots: op itself, @for template, @empty template
                        let for_slot = rep_op.slot.map(|s| s.0 + 1).unwrap_or(0);
                        let for_view_xref = rep_op.body_view;

                        if rep_op.i18n_placeholder.is_none() {
                            child_views_to_process.push((for_view_xref, None));
                        } else if let Some(ref placeholder) = rep_op.i18n_placeholder {
                            if let Some(ref ops) = current_ops {
                                // Record @for template start/end
                                operations.push(OpInfo::TemplateStart {
                                    view_xref: for_view_xref,
                                    slot: for_slot,
                                    start_name: placeholder.start_name.clone(),
                                    context_xref: ops.i18n_context_xref,
                                    sub_template_index: ops.sub_template_index,
                                    pending_structural: None,
                                    has_close_name: placeholder.close_name.is_some(),
                                });
                                child_views_to_process.push((for_view_xref, None));
                                if let Some(close_name) = &placeholder.close_name {
                                    operations.push(OpInfo::TemplateEnd {
                                        view_xref: for_view_xref,
                                        slot: for_slot,
                                        close_name: close_name.clone(),
                                        context_xref: ops.i18n_context_xref,
                                        pending_structural: None,
                                    });
                                }
                            }
                        }

                        // Handle @empty template if present
                        if let Some(empty_view_xref) = rep_op.empty_view {
                            let empty_slot = rep_op.slot.map(|s| s.0 + 2).unwrap_or(0);
                            if rep_op.empty_i18n_placeholder.is_none() {
                                child_views_to_process.push((empty_view_xref, None));
                            } else if let Some(ref empty_placeholder) =
                                rep_op.empty_i18n_placeholder
                            {
                                if let Some(ref ops) = current_ops {
                                    operations.push(OpInfo::TemplateStart {
                                        view_xref: empty_view_xref,
                                        slot: empty_slot,
                                        start_name: empty_placeholder.start_name.clone(),
                                        context_xref: ops.i18n_context_xref,
                                        sub_template_index: ops.sub_template_index,
                                        pending_structural: None,
                                        has_close_name: empty_placeholder.close_name.is_some(),
                                    });
                                    child_views_to_process.push((empty_view_xref, None));
                                    if let Some(close_name) = &empty_placeholder.close_name {
                                        operations.push(OpInfo::TemplateEnd {
                                            view_xref: empty_view_xref,
                                            slot: empty_slot,
                                            close_name: close_name.clone(),
                                            context_xref: ops.i18n_context_xref,
                                            pending_structural: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Second pass: apply the operations to context params
    for op_info in operations {
        match op_info {
            OpInfo::ElementStart {
                slot,
                start_name,
                context_xref,
                sub_template_index,
                pending_structural,
                has_close_name,
            } => {
                if let Some(slot) = slot {
                    let mut flags =
                        I18nParamValueFlags::ELEMENT_TAG.with(I18nParamValueFlags::OPEN_TAG);

                    let value = if let Some(structural) = pending_structural {
                        flags = flags.with(I18nParamValueFlags::TEMPLATE_TAG);
                        I18nParamValueContent::Compound {
                            element: slot,
                            template: structural.slot.0,
                        }
                    } else {
                        I18nParamValueContent::Slot(slot)
                    };

                    // For self-closing tags, add CLOSE_TAG flag
                    if !has_close_name {
                        flags = flags.with(I18nParamValueFlags::CLOSE_TAG);
                    }

                    let param_value = I18nParamValue::new(value, sub_template_index, flags);
                    add_param_to_context(
                        job,
                        context_xref,
                        start_name.as_str(),
                        param_value,
                        allocator,
                    );
                }
            }
            OpInfo::ElementEnd {
                slot,
                close_name,
                context_xref,
                sub_template_index,
                pending_structural,
            } => {
                if let Some(slot) = slot {
                    let mut flags =
                        I18nParamValueFlags::ELEMENT_TAG.with(I18nParamValueFlags::CLOSE_TAG);

                    let value = if let Some(structural) = pending_structural {
                        flags = flags.with(I18nParamValueFlags::TEMPLATE_TAG);
                        I18nParamValueContent::Compound {
                            element: slot,
                            template: structural.slot.0,
                        }
                    } else {
                        I18nParamValueContent::Slot(slot)
                    };

                    let param_value = I18nParamValue::new(value, sub_template_index, flags);
                    add_param_to_context(
                        job,
                        context_xref,
                        close_name.as_str(),
                        param_value,
                        allocator,
                    );
                }
            }
            OpInfo::TemplateStart {
                view_xref,
                slot,
                start_name,
                context_xref,
                sub_template_index,
                pending_structural,
                has_close_name,
            } => {
                let mut flags =
                    I18nParamValueFlags::TEMPLATE_TAG.with(I18nParamValueFlags::OPEN_TAG);

                if !has_close_name {
                    flags = flags.with(I18nParamValueFlags::CLOSE_TAG);
                }

                // If associated with structural directive, record it first
                if let Some(structural) = pending_structural {
                    let structural_value = I18nParamValue::new(
                        I18nParamValueContent::Slot(structural.slot.0),
                        sub_template_index,
                        flags,
                    );
                    add_param_to_context(
                        job,
                        context_xref,
                        start_name.as_str(),
                        structural_value,
                        allocator,
                    );
                }

                // Record template start with proper sub-template index
                let template_sub_index =
                    get_sub_template_index_for_template_tag(job, sub_template_index, view_xref);
                let param_value = I18nParamValue::new(
                    I18nParamValueContent::Slot(slot),
                    template_sub_index,
                    flags,
                );
                add_param_to_context(
                    job,
                    context_xref,
                    start_name.as_str(),
                    param_value,
                    allocator,
                );
            }
            OpInfo::TemplateEnd {
                view_xref,
                slot,
                close_name,
                context_xref,
                pending_structural,
            } => {
                let flags = I18nParamValueFlags::TEMPLATE_TAG.with(I18nParamValueFlags::CLOSE_TAG);

                // Record template close with proper sub-template index
                let template_sub_index =
                    get_sub_template_index_for_template_tag(job, None, view_xref);
                let param_value = I18nParamValue::new(
                    I18nParamValueContent::Slot(slot),
                    template_sub_index,
                    flags,
                );
                add_param_to_context(
                    job,
                    context_xref,
                    close_name.as_str(),
                    param_value,
                    allocator,
                );

                // If associated with structural directive, record it after
                if let Some(structural) = pending_structural {
                    let structural_value = I18nParamValue::new(
                        I18nParamValueContent::Slot(structural.slot.0),
                        None, // Use current block's sub-template index
                        flags,
                    );
                    add_param_to_context(
                        job,
                        context_xref,
                        close_name.as_str(),
                        structural_value,
                        allocator,
                    );
                }
            }
        }
    }

    // Recursively process child views
    for (child_view_xref, child_pending_structural) in child_views_to_process {
        resolve_placeholders_for_view(
            job,
            child_view_xref,
            i18n_contexts,
            elements,
            child_pending_structural,
        );
    }
}

/// Get the subTemplateIndex for the given template op.
/// For template ops, use the subTemplateIndex of the child i18n block inside the template.
fn get_sub_template_index_for_template_tag(
    job: &ComponentCompilationJob<'_>,
    fallback_index: Option<u32>,
    view_xref: XrefId,
) -> Option<u32> {
    let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

    if let Some(view) = view {
        for op in view.create.iter() {
            if let CreateOp::I18nStart(i18n_op) = op {
                return i18n_op.sub_template_index;
            }
        }
    }

    fallback_index
}

/// Operation info collected for later processing.
enum OpInfo<'a> {
    ElementStart {
        slot: Option<u32>,
        /// The start placeholder name (e.g., "START_TAG_DIV").
        start_name: Ident<'a>,
        context_xref: XrefId,
        sub_template_index: Option<u32>,
        pending_structural: Option<PendingStructuralDirective>,
        /// Whether this element has a close_name (not void/self-closing).
        has_close_name: bool,
    },
    ElementEnd {
        slot: Option<u32>,
        /// The close placeholder name (e.g., "CLOSE_TAG_DIV").
        close_name: Ident<'a>,
        context_xref: XrefId,
        sub_template_index: Option<u32>,
        pending_structural: Option<PendingStructuralDirective>,
    },
    TemplateStart {
        view_xref: XrefId,
        slot: u32,
        /// The start placeholder name (e.g., "START_BLOCK_IF").
        start_name: Ident<'a>,
        context_xref: XrefId,
        sub_template_index: Option<u32>,
        pending_structural: Option<PendingStructuralDirective>,
        /// Whether this template has a close_name.
        has_close_name: bool,
    },
    TemplateEnd {
        view_xref: XrefId,
        slot: u32,
        /// The close placeholder name (e.g., "CLOSE_BLOCK_IF").
        close_name: Ident<'a>,
        context_xref: XrefId,
        pending_structural: Option<PendingStructuralDirective>,
    },
}

/// Add a param value to an i18n context's params map.
fn add_param_to_context<'a>(
    job: &mut ComponentCompilationJob<'a>,
    context_xref: XrefId,
    placeholder: &'a str,
    value: I18nParamValue,
    allocator: &'a oxc_allocator::Allocator,
) {
    let placeholder_atom = Ident::from(placeholder);

    // Try root view first
    for op in job.root.create.iter_mut() {
        if let CreateOp::I18nContext(ctx) = op {
            if ctx.xref == context_xref {
                add_to_params_map(&mut ctx.params, placeholder_atom, value, allocator);
                return;
            }
        }
    }

    // Try other views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            for op in view.create.iter_mut() {
                if let CreateOp::I18nContext(ctx) = op {
                    if ctx.xref == context_xref {
                        add_to_params_map(&mut ctx.params, placeholder_atom, value, allocator);
                        return;
                    }
                }
            }
        }
    }
}

/// Add a value to a params map, creating the list if needed.
fn add_to_params_map<'a>(
    params: &mut oxc_allocator::HashMap<'a, Ident<'a>, oxc_allocator::Vec<'a, I18nParamValue>>,
    placeholder: Ident<'a>,
    value: I18nParamValue,
    allocator: &'a oxc_allocator::Allocator,
) {
    if let Some(values) = params.get_mut(&placeholder) {
        values.push(value);
    } else {
        let mut values = oxc_allocator::Vec::new_in(allocator);
        values.push(value);
        params.insert(placeholder, values);
    }
}

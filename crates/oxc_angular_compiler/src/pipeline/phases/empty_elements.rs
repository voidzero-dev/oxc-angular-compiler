//! Empty element collapse phase.
//!
//! Replace sequences of mergable instructions (e.g. `ElementStart` and `ElementEnd`) with a
//! consolidated instruction (e.g. `Element`).
//!
//! Ported from Angular's `template/pipeline/src/phases/empty_elements.ts`.

use crate::ir::ops::{ContainerOp, CreateOp, CreateOpBase, ElementOp, I18nOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Type of merge to perform.
#[derive(Debug, Clone, Copy)]
enum MergeType {
    Element,
    Container,
    I18n,
}

/// Check if an operation should be ignored when looking for start/end pairs.
fn is_ignored_op(op: &CreateOp<'_>) -> bool {
    matches!(op, CreateOp::Pipe(_))
}

/// Collapses empty instructions by merging start/end pairs.
///
/// This phase looks for ElementStart/ElementEnd, ContainerStart/ContainerEnd,
/// and I18nStart/I18nEnd pairs that are adjacent (ignoring Pipe ops) and
/// converts them to the merged Element, Container, or I18n form.
pub fn collapse_empty_instructions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect all view xrefs
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            // Collect ops into a vec for easier manipulation
            let ops: std::vec::Vec<_> = view.create.iter().collect();

            // Find pairs to merge: (end_index, xref, merge_type)
            let mut merge_pairs: std::vec::Vec<(usize, XrefId, MergeType)> = std::vec::Vec::new();

            for (idx, op) in ops.iter().enumerate() {
                // Check if this is an End op
                let (xref, merge_type) = match op {
                    CreateOp::ElementEnd(end) => (end.xref, MergeType::Element),
                    CreateOp::ContainerEnd(end) => (end.xref, MergeType::Container),
                    CreateOp::I18nEnd(end) => (end.xref, MergeType::I18n),
                    _ => continue,
                };

                // Find the previous non-ignored op
                let mut prev_idx = idx.saturating_sub(1);
                while prev_idx > 0 && is_ignored_op(ops[prev_idx]) {
                    prev_idx = prev_idx.saturating_sub(1);
                }

                // Check if the previous op is the matching start
                let is_match = match (ops.get(prev_idx), merge_type) {
                    (Some(CreateOp::ElementStart(start)), MergeType::Element) => start.xref == xref,
                    (Some(CreateOp::ContainerStart(start)), MergeType::Container) => {
                        start.xref == xref
                    }
                    (Some(CreateOp::I18nStart(start)), MergeType::I18n) => start.xref == xref,
                    _ => false,
                };

                if is_match {
                    merge_pairs.push((idx, xref, merge_type));
                }
            }

            // Process merges in reverse order to preserve indices
            for (end_idx, xref, merge_type) in merge_pairs.into_iter().rev() {
                // First, remove the end op
                let mut cursor = view.create.cursor_front();
                let mut current_idx = 0;

                // Navigate to the end op
                while current_idx < end_idx && cursor.move_next() {
                    current_idx += 1;
                }

                // Remove the end op
                if cursor.current().is_some() {
                    cursor.remove_current();
                }

                // Find and convert the matching start op
                cursor = view.create.cursor_front();
                loop {
                    let should_convert = match cursor.current() {
                        Some(CreateOp::ElementStart(start))
                            if matches!(merge_type, MergeType::Element) =>
                        {
                            start.xref == xref
                        }
                        Some(CreateOp::ContainerStart(start))
                            if matches!(merge_type, MergeType::Container) =>
                        {
                            start.xref == xref
                        }
                        Some(CreateOp::I18nStart(start))
                            if matches!(merge_type, MergeType::I18n) =>
                        {
                            start.xref == xref
                        }
                        _ => false,
                    };

                    if should_convert {
                        // Replace the start op with the merged version
                        if let Some(op) = cursor.current_mut() {
                            match op {
                                CreateOp::ElementStart(start) => {
                                    let merged = ElementOp {
                                        base: CreateOpBase {
                                            prev: start.base.prev,
                                            next: start.base.next,
                                            source_span: start.base.source_span,
                                        },
                                        xref: start.xref,
                                        tag: start.tag.clone(),
                                        slot: start.slot,
                                        namespace: start.namespace,
                                        attribute_namespace: start.attribute_namespace.clone(),
                                        local_refs: std::mem::replace(
                                            &mut start.local_refs,
                                            oxc_allocator::Vec::new_in(&allocator),
                                        ),
                                        local_refs_index: start.local_refs_index,
                                        non_bindable: start.non_bindable,
                                        i18n_placeholder: start.i18n_placeholder.clone(),
                                        attributes: start.attributes,
                                    };
                                    *op = CreateOp::Element(merged);
                                }
                                CreateOp::ContainerStart(start) => {
                                    let merged = ContainerOp {
                                        base: CreateOpBase {
                                            prev: start.base.prev,
                                            next: start.base.next,
                                            source_span: start.base.source_span,
                                        },
                                        xref: start.xref,
                                        slot: start.slot,
                                        attributes: start.attributes,
                                        local_refs_index: start.local_refs_index,
                                        local_refs: std::mem::replace(
                                            &mut start.local_refs,
                                            oxc_allocator::Vec::new_in(&allocator),
                                        ),
                                        non_bindable: start.non_bindable,
                                        i18n_placeholder: start.i18n_placeholder.clone(),
                                    };
                                    *op = CreateOp::Container(merged);
                                }
                                CreateOp::I18nStart(start) => {
                                    let merged = I18nOp {
                                        base: CreateOpBase {
                                            prev: start.base.prev,
                                            next: start.base.next,
                                            source_span: start.base.source_span,
                                        },
                                        xref: start.xref,
                                        slot: start.slot,
                                        context: start.context,
                                        message: start.message,
                                        i18n_placeholder: start.i18n_placeholder.clone(),
                                        sub_template_index: start.sub_template_index,
                                        root: start.root,
                                        message_index: start.message_index,
                                    };
                                    *op = CreateOp::I18n(merged);
                                }
                                _ => {}
                            }
                        }
                        break;
                    }

                    if !cursor.move_next() {
                        break;
                    }
                }
            }
        }
    }
}

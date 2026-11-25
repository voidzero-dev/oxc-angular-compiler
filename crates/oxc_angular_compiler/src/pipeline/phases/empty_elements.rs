//! Empty element collapse phase.
//!
//! Replace sequences of mergable instructions (e.g. `ElementStart` and `ElementEnd`) with a
//! consolidated instruction (e.g. `Element`).
//!
//! Ported from Angular's `template/pipeline/src/phases/empty_elements.ts`.

use oxc_allocator::Allocator;

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

            // Find pairs to merge: (start_index, end_index). Indices are positions in the
            // original list, so the pairs come in increasing order and never overlap.
            let mut merge_pairs: std::vec::Vec<(usize, usize)> = std::vec::Vec::new();

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
                    merge_pairs.push((prev_idx, idx));
                }
            }

            if merge_pairs.is_empty() {
                continue;
            }

            // Apply all merges in one forward pass: turn each start op into its merged
            // form and remove the matching end op.
            let mut pairs = merge_pairs.into_iter().peekable();
            let mut cursor = view.create.cursor_front();
            let mut idx = 0;
            while let Some(&(start_idx, end_idx)) = pairs.peek() {
                if cursor.current().is_none() {
                    break;
                }
                if idx == start_idx {
                    if let Some(op) = cursor.current_mut() {
                        merge_start_op(op, allocator);
                    }
                } else if idx == end_idx {
                    // Removing moves the cursor to the next op.
                    cursor.remove_current();
                    idx += 1;
                    pairs.next();
                    continue;
                }
                cursor.move_next();
                idx += 1;
            }
        }
    }
}

/// Replaces a start op (`ElementStart`, `ContainerStart`, `I18nStart`) in place with its
/// merged form (`Element`, `Container`, `I18n`).
fn merge_start_op<'a>(op: &mut CreateOp<'a>, allocator: &'a Allocator) {
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

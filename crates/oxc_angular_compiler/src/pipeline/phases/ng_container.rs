//! Ng-container phase.
//!
//! Replaces `ElementStart`/`ElementEnd` operations for `ng-container` elements
//! with `ContainerStart`/`ContainerEnd` operations. This transformation is
//! necessary because ng-container elements don't render a DOM element - they're
//! logical containers only.
//!
//! Ported from Angular's `template/pipeline/src/phases/ng_container.ts`.

use oxc_allocator::Vec as ArenaVec;
use rustc_hash::FxHashSet;

use crate::ir::ops::{ContainerEndOp, ContainerStartOp, CreateOp, CreateOpBase};
use crate::pipeline::compilation::ComponentCompilationJob;

const CONTAINER_TAG: &str = "ng-container";

/// Replaces ElementStart/ElementEnd ops for ng-container with ContainerStart/ContainerEnd.
///
/// This phase identifies elements with tag "ng-container" and transforms them:
/// - `ElementStart` with tag "ng-container" → `ContainerStart`
/// - Corresponding `ElementEnd` → `ContainerEnd`
pub fn generate_ng_container_ops(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process all views
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            // Track which element xrefs have been converted
            let mut updated_element_xrefs = FxHashSet::default();

            // First pass: convert ElementStart to ContainerStart
            for op in view.create.iter_mut() {
                if let CreateOp::ElementStart(elem) = op {
                    if elem.tag.as_str() == CONTAINER_TAG {
                        // Store the xref for the second pass
                        updated_element_xrefs.insert(elem.xref);

                        // Create ContainerStart with the same properties
                        let container_start = ContainerStartOp {
                            base: CreateOpBase {
                                prev: elem.base.prev,
                                next: elem.base.next,
                                source_span: elem.base.source_span,
                            },
                            xref: elem.xref,
                            slot: elem.slot,
                            attributes: elem.attributes,
                            local_refs_index: elem.local_refs_index, // Copy from element
                            local_refs: std::mem::replace(
                                &mut elem.local_refs,
                                ArenaVec::new_in(&allocator),
                            ),
                            non_bindable: elem.non_bindable,
                            i18n_placeholder: elem.i18n_placeholder.clone(),
                        };

                        // Replace the op
                        *op = CreateOp::ContainerStart(container_start);
                    }
                }
            }

            // Second pass: convert ElementEnd to ContainerEnd for matching xrefs
            for op in view.create.iter_mut() {
                if let CreateOp::ElementEnd(elem_end) = op {
                    if updated_element_xrefs.contains(&elem_end.xref) {
                        let container_end = ContainerEndOp {
                            base: CreateOpBase {
                                prev: elem_end.base.prev,
                                next: elem_end.base.next,
                                source_span: elem_end.base.source_span,
                            },
                            xref: elem_end.xref,
                        };

                        *op = CreateOp::ContainerEnd(container_end);
                    }
                }
            }
        }
    }
}

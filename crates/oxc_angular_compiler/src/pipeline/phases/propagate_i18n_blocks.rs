//! Propagate i18n blocks phase.
//!
//! Propagate i18n blocks down through child templates that act as placeholders
//! in the root i18n message. Specifically, perform an in-order traversal of all
//! the views, and add i18nStart/i18nEnd op pairs into descending views. Also,
//! assign an increasing sub-template index to each descending view.
//!
//! Ported from Angular's `template/pipeline/src/phases/propagate_i18n_blocks.ts`.

use crate::ir::enums::OpKind;
use crate::ir::ops::{CreateOp, CreateOpBase, I18nEndOp, I18nStartOp, Op, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Propagates i18n block context to child templates.
pub fn propagate_i18n_blocks(job: &mut ComponentCompilationJob<'_>) {
    // Start with the root view at sub-template index 0
    propagate_i18n_blocks_to_templates(job, job.root.xref, 0);
}

/// Propagates i18n ops in the given view through to any child views recursively.
fn propagate_i18n_blocks_to_templates(
    job: &mut ComponentCompilationJob<'_>,
    view_xref: XrefId,
    mut sub_template_index: u32,
) -> u32 {
    // Collect operations we need to process
    // We need to do this in two passes to avoid borrow checker issues
    // The tuple contains: (kind, view_xref, has_placeholder, extra_view, extra_has_placeholder)
    let ops_info: Vec<(OpKind, XrefId, Option<bool>, Option<XrefId>, Option<bool>)> = {
        let view = match job.view(view_xref) {
            Some(v) => v,
            None => return sub_template_index,
        };

        view.create
            .iter()
            .map(|op| match op {
                CreateOp::I18nStart(i18n_op) => (OpKind::I18nStart, i18n_op.xref, None, None, None),
                CreateOp::I18nEnd(_) => (OpKind::I18nEnd, XrefId::new(0), None, None, None),
                CreateOp::Template(template_op) => (
                    OpKind::Template,
                    template_op.embedded_view,
                    Some(template_op.i18n_placeholder.is_some()),
                    None,
                    None,
                ),
                CreateOp::Conditional(cond_op) => (
                    OpKind::ConditionalCreate,
                    cond_op.xref,
                    Some(cond_op.i18n_placeholder.is_some()),
                    None,
                    None,
                ),
                CreateOp::ConditionalBranch(branch_op) => (
                    OpKind::ConditionalBranchCreate,
                    branch_op.xref,
                    Some(branch_op.i18n_placeholder.is_some()),
                    None,
                    None,
                ),
                CreateOp::RepeaterCreate(rep_op) => (
                    OpKind::RepeaterCreate,
                    rep_op.body_view,
                    Some(rep_op.i18n_placeholder.is_some()),
                    rep_op.empty_view,
                    rep_op.empty_i18n_placeholder.as_ref().map(|_| true),
                ),
                CreateOp::Projection(proj_op) => (
                    OpKind::Projection,
                    proj_op.xref,
                    None,
                    proj_op.fallback,
                    proj_op.fallback_i18n_placeholder.as_ref().map(|_| true),
                ),
                _ => (Op::kind(op), XrefId::new(0), None, None, None),
            })
            .collect()
    };

    // Track the current i18n block: (root_xref, message, sub_template_index)
    // root_xref is always the root of the i18n block tree (same for nested blocks)
    let mut i18n_block: Option<(XrefId, Option<u32>, Option<u32>)> = None;

    for (kind, xref, has_placeholder, extra_view, extra_has_placeholder) in ops_info {
        match kind {
            OpKind::I18nStart => {
                // Update sub_template_index on the I18nStart op
                if let Some(view) = job.view_mut(view_xref) {
                    for op in view.create.iter_mut() {
                        if let CreateOp::I18nStart(i18n_op) = op {
                            if i18n_op.xref == xref {
                                i18n_op.sub_template_index = if sub_template_index == 0 {
                                    None
                                } else {
                                    Some(sub_template_index)
                                };
                                // Get the root xref - use existing root or self for root-level
                                let root_xref = i18n_op.root.unwrap_or(xref);
                                i18n_block =
                                    Some((root_xref, i18n_op.message, i18n_op.sub_template_index));
                                break;
                            }
                        }
                    }
                }
            }
            OpKind::I18nEnd => {
                // When we exit a root-level i18n block, reset the sub-template index counter
                if let Some((_, _, sub_idx)) = &i18n_block {
                    if sub_idx.is_none() {
                        sub_template_index = 0;
                    }
                }
                i18n_block = None;
            }
            // ConditionalCreate, ConditionalBranchCreate, and Template are all handled the same way
            // - they use op.xref to get the view and op.i18n_placeholder for the placeholder.
            // Ported from Angular's propagateI18nBlocks in propagate_i18n_blocks.ts.
            OpKind::Template | OpKind::ConditionalCreate | OpKind::ConditionalBranchCreate => {
                if has_placeholder == Some(true) {
                    if let Some((root_xref, message, _)) = i18n_block {
                        sub_template_index += 1;
                        wrap_template_with_i18n(job, xref, root_xref, message);
                    }
                }
                sub_template_index =
                    propagate_i18n_blocks_to_templates(job, xref, sub_template_index);
            }
            OpKind::RepeaterCreate => {
                // Propagate to the @for template body
                if has_placeholder == Some(true) {
                    if let Some((root_xref, message, _)) = i18n_block {
                        sub_template_index += 1;
                        wrap_template_with_i18n(job, xref, root_xref, message);
                    }
                }
                sub_template_index =
                    propagate_i18n_blocks_to_templates(job, xref, sub_template_index);

                // Then if there's an @empty template, propagate for it as well
                if let Some(empty_view) = extra_view {
                    if extra_has_placeholder == Some(true) {
                        if let Some((root_xref, message, _)) = i18n_block {
                            sub_template_index += 1;
                            wrap_template_with_i18n(job, empty_view, root_xref, message);
                        }
                    }
                    sub_template_index =
                        propagate_i18n_blocks_to_templates(job, empty_view, sub_template_index);
                }
            }
            OpKind::Projection => {
                // Propagate to fallback view if it exists
                if let Some(fallback_view) = extra_view {
                    if extra_has_placeholder == Some(true) {
                        if let Some((root_xref, message, _)) = i18n_block {
                            sub_template_index += 1;
                            wrap_template_with_i18n(job, fallback_view, root_xref, message);
                        }
                    }
                    sub_template_index =
                        propagate_i18n_blocks_to_templates(job, fallback_view, sub_template_index);
                }
            }
            _ => {}
        }
    }

    sub_template_index
}

/// Wraps a template view with i18n start and end ops.
fn wrap_template_with_i18n(
    job: &mut ComponentCompilationJob<'_>,
    view_xref: XrefId,
    root_i18n_xref: XrefId,
    message: Option<u32>,
) {
    let view = match job.view(view_xref) {
        Some(v) => v,
        None => return,
    };

    // Only add i18n ops if they have not already been propagated
    let first_op_is_i18n_start =
        view.create.head().map(|op| matches!(op, CreateOp::I18nStart(_))).unwrap_or(false);

    if first_op_is_i18n_start {
        return;
    }

    // Allocate new xref for the nested i18n block
    let i18n_xref = job.allocate_xref_id();

    // Create i18n start and end ops
    // The root is always the root of the original i18n block (propagated through nesting)
    let i18n_start = CreateOp::I18nStart(I18nStartOp {
        base: CreateOpBase::default(),
        xref: i18n_xref,
        slot: None,
        context: None,
        message,
        i18n_placeholder: None,
        sub_template_index: None,
        root: Some(root_i18n_xref),
        message_index: None,
    });

    let i18n_end = CreateOp::I18nEnd(I18nEndOp { base: CreateOpBase::default(), xref: i18n_xref });

    // Insert at head and tail of the view's create list
    if let Some(view) = job.view_mut(view_xref) {
        view.create.push_front(i18n_start);
        view.create.push(i18n_end);
    }
}

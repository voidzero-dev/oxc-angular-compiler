//! Wrap ICUs phase.
//!
//! Wraps ICU expressions that do not already belong to an i18n block
//! in a new i18n block.
//!
//! Ported from Angular's `template/pipeline/src/phases/wrap_icus.ts`.

use std::ptr::NonNull;

use crate::ir::ops::{CreateOp, CreateOpBase, I18nEndOp, I18nStartOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Wraps standalone ICU expressions in i18n blocks.
pub fn wrap_i18n_icus(job: &mut ComponentCompilationJob<'_>) {
    // Process root view
    wrap_icus_in_view(job, job.root.xref);

    // Process all other views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for xref in view_xrefs {
        wrap_icus_in_view(job, xref);
    }
}

fn wrap_icus_in_view(job: &mut ComponentCompilationJob<'_>, view_xref: XrefId) {
    // First pass: collect information about ICUs that need wrapping
    let mut icus_to_wrap: Vec<(NonNull<CreateOp<'_>>, Option<u32>)> = Vec::new();

    {
        let view = match job.view(view_xref) {
            Some(v) => v,
            None => return,
        };

        let mut current_i18n_op: Option<XrefId> = None;

        for op in view.create.iter() {
            match op {
                CreateOp::I18nStart(i18n_op) => {
                    current_i18n_op = Some(i18n_op.xref);
                }
                CreateOp::I18nEnd(_) => {
                    current_i18n_op = None;
                }
                CreateOp::IcuStart(icu_op) => {
                    if current_i18n_op.is_none() {
                        // This ICU is not inside an i18n block, we need to wrap it
                        // We can't get the pointer directly, so we'll record the xref
                        icus_to_wrap.push((NonNull::from(op), icu_op.message));
                    }
                }
                _ => {}
            }
        }
    }

    // Second pass: wrap each ICU
    for (icu_ptr, message) in icus_to_wrap {
        let i18n_xref = job.allocate_xref_id();

        // Create i18n start op
        let i18n_start = CreateOp::I18nStart(I18nStartOp {
            base: CreateOpBase::default(),
            xref: i18n_xref,
            slot: None,
            context: None,
            message,
            i18n_placeholder: None,
            sub_template_index: None,
            root: None,
            message_index: None,
        });

        // Insert i18n start before the ICU
        if let Some(view) = job.view_mut(view_xref) {
            // SAFETY: icu_ptr is a valid pointer we obtained from iteration
            unsafe {
                view.create.insert_before(icu_ptr, i18n_start);
            }
        }

        // Find the matching IcuEnd and insert I18nEnd after it
        let mut found_icu_end: Option<NonNull<CreateOp<'_>>> = None;
        if let Some(view) = job.view(view_xref) {
            let mut in_target_icu = false;
            for op in view.create.iter() {
                if std::ptr::eq(op, unsafe { icu_ptr.as_ref() }) {
                    in_target_icu = true;
                    continue;
                }
                if in_target_icu {
                    if let CreateOp::IcuEnd(_) = op {
                        found_icu_end = Some(NonNull::from(op));
                        break;
                    }
                }
            }
        }

        if let Some(icu_end_ptr) = found_icu_end {
            let i18n_end =
                CreateOp::I18nEnd(I18nEndOp { base: CreateOpBase::default(), xref: i18n_xref });

            if let Some(view) = job.view_mut(view_xref) {
                // SAFETY: icu_end_ptr is a valid pointer we obtained from iteration
                unsafe {
                    view.create.insert_after(icu_end_ptr, i18n_end);
                }
            }
        }
    }
}

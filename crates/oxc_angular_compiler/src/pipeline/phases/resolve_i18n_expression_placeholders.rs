//! Resolve i18n expression placeholders phase.
//!
//! Resolve the i18n expression placeholders in i18n messages.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_i18n_expression_placeholders.ts`.

use oxc_span::Ident;
use rustc_hash::FxHashMap;

use crate::ir::enums::{I18nExpressionFor, I18nParamResolutionTime, I18nParamValueFlags};
use crate::ir::i18n_params::{I18nParamValue, I18nParamValueContent};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Resolves expression placeholders in i18n messages.
///
/// This phase assigns expression indices to i18n expression ops and updates
/// the i18n context params with the expression placeholder values.
pub fn resolve_i18n_expression_placeholders(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Record all of the i18n context ops, and the sub-template index for each i18n op.
    let mut sub_template_indices: FxHashMap<XrefId, Option<u32>> = FxHashMap::default();
    let mut i18n_context_xrefs: FxHashMap<XrefId, XrefId> = FxHashMap::default(); // context xref -> view xref
    let mut icu_placeholder_xrefs: FxHashMap<XrefId, XrefId> = FxHashMap::default(); // icu xref -> view xref

    // Collect from all views
    for view in job.all_views() {
        let view_xref = view.xref;
        for op in view.create.iter() {
            match op {
                CreateOp::I18nStart(i18n_op) => {
                    sub_template_indices.insert(i18n_op.xref, i18n_op.sub_template_index);
                }
                CreateOp::I18nContext(ctx_op) => {
                    i18n_context_xrefs.insert(ctx_op.xref, view_xref);
                }
                CreateOp::IcuPlaceholder(icu_op) => {
                    icu_placeholder_xrefs.insert(icu_op.xref, view_xref);
                }
                _ => {}
            }
        }
    }

    // Keep track of the next available expression index for each i18n message.
    let mut expression_indices: FxHashMap<XrefId, u32> = FxHashMap::default();

    // Collect expression info and prepare updates
    // (context_xref, placeholder_name, value, resolution_time, icu_placeholder_xref)
    struct ExprUpdate<'a> {
        context_xref: XrefId,
        placeholder: Option<Ident<'a>>,
        value: I18nParamValue,
        resolution_time: I18nParamResolutionTime,
        icu_placeholder: Option<XrefId>,
    }

    let mut updates: Vec<ExprUpdate<'_>> = Vec::new();

    for view in job.all_views() {
        for op in view.update.iter() {
            if let UpdateOp::I18nExpression(i18n_expr) = op {
                // Get the reference index - different for i18n text vs attributes
                // Child i18n blocks in templates don't get their own context, since they're rolled
                // into the translated message of the parent, but they may target a different slot.
                let reference_index = if i18n_expr.usage == I18nExpressionFor::I18nText {
                    i18n_expr.i18n_owner
                } else {
                    i18n_expr.context
                };

                let index = *expression_indices.get(&reference_index).unwrap_or(&0);
                expression_indices.insert(reference_index, index + 1);

                let sub_template_index =
                    sub_template_indices.get(&i18n_expr.i18n_owner).copied().flatten();

                let value = I18nParamValue::new(
                    I18nParamValueContent::Slot(index),
                    sub_template_index,
                    I18nParamValueFlags::EXPRESSION_INDEX,
                );

                updates.push(ExprUpdate {
                    context_xref: i18n_expr.context,
                    placeholder: i18n_expr.i18n_placeholder.clone(),
                    value,
                    resolution_time: i18n_expr.resolution_time,
                    icu_placeholder: i18n_expr.icu_placeholder,
                });
            }
        }
    }

    // Apply updates to I18nContext params
    for update in updates {
        // Update i18n context params if there's a placeholder
        if let Some(placeholder) = update.placeholder {
            if let Some(&view_xref) = i18n_context_xrefs.get(&update.context_xref) {
                let view =
                    if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(view_xref) };

                if let Some(view) = view {
                    for op in view.create.iter_mut() {
                        if let CreateOp::I18nContext(ctx_op) = op {
                            if ctx_op.xref == update.context_xref {
                                // Choose params or postprocessing_params based on resolution time
                                let params = if update.resolution_time
                                    == I18nParamResolutionTime::Creation
                                {
                                    &mut ctx_op.params
                                } else {
                                    &mut ctx_op.postprocessing_params
                                };

                                // Add to params map
                                let placeholder_atom = Ident::from(placeholder.as_str());
                                if let Some(values) = params.get_mut(&placeholder_atom) {
                                    values.push(update.value);
                                } else {
                                    let mut values = oxc_allocator::Vec::new_in(allocator);
                                    values.push(update.value);
                                    params.insert(placeholder_atom, values);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Update ICU placeholder if present
        if let Some(icu_xref) = update.icu_placeholder {
            if let Some(&view_xref) = icu_placeholder_xrefs.get(&icu_xref) {
                let view =
                    if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(view_xref) };

                if let Some(view) = view {
                    for op in view.create.iter_mut() {
                        if let CreateOp::IcuPlaceholder(icu_op) = op {
                            if icu_op.xref == icu_xref {
                                icu_op.expression_placeholders.push(update.value);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

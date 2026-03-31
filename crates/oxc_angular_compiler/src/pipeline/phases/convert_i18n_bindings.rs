//! Convert i18n bindings phase.
//!
//! Some binding instructions in the update block may actually correspond to i18n bindings.
//! In that case, they should be replaced with i18nExp instructions for the dynamic portions.
//!
//! Ported from Angular's `template/pipeline/src/phases/convert_i18n_bindings.ts`.

use std::ptr::NonNull;

use rustc_hash::FxHashMap;

use crate::ir::enums::{I18nExpressionFor, I18nParamResolutionTime};
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, I18nExpressionOp, I18nSlotHandle, UpdateOp, UpdateOpBase, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Converts i18n bindings to runtime i18n calls.
///
/// This phase processes Property and Attribute ops in the update block that have
/// an i18n context. If their expression is an interpolation, they are replaced
/// with I18nExpression ops for the dynamic portions.
pub fn convert_i18n_bindings(job: &mut ComponentCompilationJob<'_>) {
    // Build a map of I18nAttributes ops by target element
    let mut i18n_attributes_by_elem: FxHashMap<XrefId, I18nAttributesInfo> = FxHashMap::default();

    // Collect I18nAttributes from root view
    for op in job.root.create.iter() {
        if let CreateOp::I18nAttributes(i18n_attrs) = op {
            i18n_attributes_by_elem.insert(
                i18n_attrs.target,
                I18nAttributesInfo { xref: i18n_attrs.xref, handle: i18n_attrs.handle },
            );
        }
    }

    // Collect I18nAttributes from other views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in &view_xrefs {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                if let CreateOp::I18nAttributes(i18n_attrs) = op {
                    i18n_attributes_by_elem.insert(
                        i18n_attrs.target,
                        I18nAttributesInfo { xref: i18n_attrs.xref, handle: i18n_attrs.handle },
                    );
                }
            }
        }
    }

    // Process update ops in root view
    process_update_ops_for_view(job, job.root.xref, &i18n_attributes_by_elem);

    // Process update ops in other views
    for view_xref in view_xrefs {
        process_update_ops_for_view(job, view_xref, &i18n_attributes_by_elem);
    }
}

/// Information about an I18nAttributes op.
#[derive(Clone, Copy)]
struct I18nAttributesInfo {
    xref: XrefId,
    handle: I18nSlotHandle,
}

/// Information about an op that needs to be converted.
struct ConversionInfo<'a> {
    op_ptr: NonNull<UpdateOp<'a>>,
    target: XrefId,
    i18n_context: XrefId,
    name: oxc_span::Ident<'a>,
    source_span: Option<oxc_span::Span>,
    /// The expression (if it's an interpolation, we extract sub-expressions).
    expression: Option<InterpolationInfo<'a>>,
}

/// Information extracted from an Interpolation expression.
struct InterpolationInfo<'a> {
    expressions: Vec<IrExpression<'a>>,
    i18n_placeholders: Vec<oxc_span::Ident<'a>>,
}

/// Processes update ops for a single view, converting i18n bindings.
fn process_update_ops_for_view(
    job: &mut ComponentCompilationJob<'_>,
    view_xref: XrefId,
    i18n_attributes_by_elem: &FxHashMap<XrefId, I18nAttributesInfo>,
) {
    let allocator = job.allocator;

    // Collect ops that need to be converted
    let mut ops_to_convert: Vec<ConversionInfo<'_>> = Vec::new();

    {
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

        if let Some(view) = view {
            for op in view.update.iter() {
                match op {
                    UpdateOp::Property(prop_op) => {
                        if let Some(i18n_context) = prop_op.i18n_context {
                            // Check if expression is an interpolation
                            if let IrExpression::Interpolation(interp) = prop_op.expression.as_ref()
                            {
                                if i18n_attributes_by_elem.contains_key(&prop_op.target) {
                                    // Clone the expressions and placeholders
                                    let mut expressions = Vec::new();
                                    for expr in interp.expressions.iter() {
                                        expressions.push(expr.clone_in(allocator));
                                    }
                                    let i18n_placeholders: Vec<_> =
                                        interp.i18n_placeholders.iter().cloned().collect();

                                    ops_to_convert.push(ConversionInfo {
                                        op_ptr: NonNull::from(op),
                                        target: prop_op.target,
                                        i18n_context,
                                        name: prop_op.name.clone(),
                                        source_span: prop_op.base.source_span,
                                        expression: Some(InterpolationInfo {
                                            expressions,
                                            i18n_placeholders,
                                        }),
                                    });
                                }
                            }
                        }
                    }
                    UpdateOp::Attribute(attr_op) => {
                        if let Some(i18n_context) = attr_op.i18n_context {
                            // Check if expression is an interpolation
                            if let IrExpression::Interpolation(interp) = attr_op.expression.as_ref()
                            {
                                if i18n_attributes_by_elem.contains_key(&attr_op.target) {
                                    // Clone the expressions and placeholders
                                    let mut expressions = Vec::new();
                                    for expr in interp.expressions.iter() {
                                        expressions.push(expr.clone_in(allocator));
                                    }
                                    let i18n_placeholders: Vec<_> =
                                        interp.i18n_placeholders.iter().cloned().collect();

                                    ops_to_convert.push(ConversionInfo {
                                        op_ptr: NonNull::from(op),
                                        target: attr_op.target,
                                        i18n_context,
                                        name: attr_op.name.clone(),
                                        source_span: attr_op.base.source_span,
                                        expression: Some(InterpolationInfo {
                                            expressions,
                                            i18n_placeholders,
                                        }),
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Convert each op in place (replace with new ops at the same position).
    // This is important for maintaining the correct order of operations.
    // Angular's `ir.OpList.replaceWithMany` does this in place.
    for conversion in ops_to_convert {
        if let Some(interp_info) = &conversion.expression {
            if let Some(i18n_attrs) = i18n_attributes_by_elem.get(&conversion.target) {
                // Create I18nExpression ops for each expression in the interpolation
                let mut replacement_ops: Vec<UpdateOp<'_>> = Vec::new();
                for (i, expr) in interp_info.expressions.iter().enumerate() {
                    let i18n_placeholder = interp_info.i18n_placeholders.get(i).cloned();

                    let i18n_expr = UpdateOp::I18nExpression(I18nExpressionOp {
                        base: UpdateOpBase {
                            source_span: conversion.source_span,
                            ..Default::default()
                        },
                        i18n_owner: i18n_attrs.xref,
                        target: conversion.target,
                        context: conversion.i18n_context,
                        handle: i18n_attrs.handle,
                        expression: oxc_allocator::Box::new_in(expr.clone_in(allocator), allocator),
                        resolution_time: I18nParamResolutionTime::Creation,
                        usage: I18nExpressionFor::I18nAttribute,
                        name: conversion.name.clone(),
                        i18n_placeholder,
                        icu_placeholder: None,
                    });

                    replacement_ops.push(i18n_expr);
                }

                // Replace the original op in place with the new ops
                // Insert all new ops after the original, then remove the original
                if view_xref.0 == 0 {
                    // SAFETY: op_ptr is a valid pointer we obtained from iteration
                    unsafe {
                        // Insert new ops after the original (in reverse order to maintain order)
                        let mut insert_after_ptr = conversion.op_ptr;
                        for op in replacement_ops {
                            insert_after_ptr =
                                job.root.update.insert_after_returning_new(insert_after_ptr, op);
                        }
                        // Remove the original op
                        job.root.update.remove(conversion.op_ptr);
                    }
                } else if let Some(view) = job.view_mut(view_xref) {
                    unsafe {
                        let mut insert_after_ptr = conversion.op_ptr;
                        for op in replacement_ops {
                            insert_after_ptr =
                                view.update.insert_after_returning_new(insert_after_ptr, op);
                        }
                        view.update.remove(conversion.op_ptr);
                    }
                }
            }
        }
    }
}

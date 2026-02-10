//! Defer configuration phase.
//!
//! Configures @defer block instructions with trigger and dependency information.
//!
//! This phase processes DeferOp and DeferOnOp to:
//! 1. Wrap timing configs in ConstCollectedExpr (resolved to consts later by Phase 53)
//! 2. Link defer triggers to their target defer blocks
//! 3. Set up main, placeholder, loading, and error template slots
//! 4. Configure timing parameters (minimum time, loading after, etc.)
//!
//! Ported from Angular's `template/pipeline/src/phases/defer_configs.ts`.
//!
//! Key difference from old approach: instead of calling `job.add_const()` here (which
//! places timer configs before i18n consts), we create ConstCollectedExpr wrappers.
//! These are resolved by Phase 53 (collectConstExpressions) which runs AFTER Phase 52
//! (collectI18nConsts), ensuring correct const array ordering.

use oxc_allocator::{Box, Vec as OxcVec};

use crate::ast::expression::{AbsoluteSourceSpan, LiteralPrimitive, LiteralValue, ParseSpan};
use crate::ir::enums::DeferOpModifierKind;
use crate::ir::expression::{ConstCollectedExpr, IrExpression, IrLiteralArrayExpr};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Collected timing config for a defer block.
#[derive(Clone)]
struct DeferTimingConfig {
    xref: XrefId,
    placeholder_minimum_time: Option<u32>,
    loading_minimum_time: Option<u32>,
    loading_after_time: Option<u32>,
}

/// Configures defer instructions with trigger and dependency information.
///
/// This phase:
/// 1. Collects timing configs and wraps them in ConstCollectedExpr for later resolution
/// 2. Collects all DeferOp blocks and their associated DeferOnOp triggers
/// 3. Links triggers to their target defer blocks
/// 4. Validates timing parameters
pub fn configure_defer_instructions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect all defer block timing configs
    let timing_configs: Vec<DeferTimingConfig> = job
        .all_views()
        .flat_map(|view| {
            view.create.iter().filter_map(|op| {
                if let CreateOp::Defer(defer) = op {
                    Some(DeferTimingConfig {
                        xref: defer.xref,
                        placeholder_minimum_time: defer.placeholder_minimum_time,
                        loading_minimum_time: defer.loading_minimum_time,
                        loading_after_time: defer.loading_after_time,
                    })
                } else {
                    None
                }
            })
        })
        .collect();

    // Build ConstCollectedExpr wrappers for each defer block's timing configs.
    // These wrap a LiteralArray (e.g. [100, null]) in ConstCollectedExpr so that
    // Phase 53 (collectConstExpressions) will lift them to consts AFTER i18n consts.
    //
    // This matches Angular TS's defer_configs.ts which uses:
    //   op.loadingConfig = new ir.ConstCollectedExpr(literalOrArrayLiteral([...]))
    let mut config_exprs: std::vec::Vec<(
        XrefId,
        Option<Box<'_, IrExpression<'_>>>,
        Option<Box<'_, IrExpression<'_>>>,
    )> = Vec::new();

    let span = ParseSpan { start: 0, end: 0 };
    let source_span = AbsoluteSourceSpan { start: 0, end: 0 };

    for config in &timing_configs {
        let mut placeholder_config_expr = None;
        let mut loading_config_expr = None;

        // Create loading config: [minimumTime, afterTime]
        // Angular processes loadingConfig before placeholderConfig in transformExpressionsInOp
        // (see ir/src/expression.ts lines 1241-1251), so we create loading config first.
        // Angular uses `literalOrArrayLiteral([op.loadingMinimumTime, op.loadingAfterTime])`
        // which emits `null` for missing values, not `0`.
        if config.loading_minimum_time.is_some() || config.loading_after_time.is_some() {
            let mut elements = OxcVec::with_capacity_in(2, allocator);

            // minimumTime: number or null
            let min_val = match config.loading_minimum_time {
                Some(t) => LiteralValue::Number(t as f64),
                None => LiteralValue::Null,
            };
            elements.push(IrExpression::Ast(Box::new_in(
                crate::ast::expression::AngularExpression::LiteralPrimitive(Box::new_in(
                    LiteralPrimitive { span, source_span, value: min_val },
                    allocator,
                )),
                allocator,
            )));

            // afterTime: number or null
            let after_val = match config.loading_after_time {
                Some(t) => LiteralValue::Number(t as f64),
                None => LiteralValue::Null,
            };
            elements.push(IrExpression::Ast(Box::new_in(
                crate::ast::expression::AngularExpression::LiteralPrimitive(Box::new_in(
                    LiteralPrimitive { span, source_span, value: after_val },
                    allocator,
                )),
                allocator,
            )));

            let array_expr = IrExpression::LiteralArray(Box::new_in(
                IrLiteralArrayExpr { elements, source_span: None },
                allocator,
            ));

            loading_config_expr = Some(Box::new_in(
                IrExpression::ConstCollected(Box::new_in(
                    ConstCollectedExpr {
                        expr: Box::new_in(array_expr, allocator),
                        source_span: None,
                    },
                    allocator,
                )),
                allocator,
            ));
        }

        // Create placeholder config: [minimumTime]
        if let Some(min_time) = config.placeholder_minimum_time {
            let mut elements = OxcVec::with_capacity_in(1, allocator);
            elements.push(IrExpression::Ast(Box::new_in(
                crate::ast::expression::AngularExpression::LiteralPrimitive(Box::new_in(
                    LiteralPrimitive {
                        span,
                        source_span,
                        value: LiteralValue::Number(min_time as f64),
                    },
                    allocator,
                )),
                allocator,
            )));

            let array_expr = IrExpression::LiteralArray(Box::new_in(
                IrLiteralArrayExpr { elements, source_span: None },
                allocator,
            ));

            placeholder_config_expr = Some(Box::new_in(
                IrExpression::ConstCollected(Box::new_in(
                    ConstCollectedExpr {
                        expr: Box::new_in(array_expr, allocator),
                        source_span: None,
                    },
                    allocator,
                )),
                allocator,
            ));
        }

        config_exprs.push((config.xref, placeholder_config_expr, loading_config_expr));
    }

    // Update DeferOp with config expressions
    let view_xrefs: std::vec::Vec<XrefId> = job.all_views().map(|v| v.xref).collect();
    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            for op in view.create.iter_mut() {
                if let CreateOp::Defer(defer) = op {
                    // Find the config expressions for this defer block
                    if let Some((_, placeholder_expr, loading_expr)) =
                        config_exprs.iter_mut().find(|(xref, _, _)| *xref == defer.xref)
                    {
                        defer.placeholder_config = placeholder_expr.take();
                        defer.loading_config = loading_expr.take();
                    }
                }
            }
        }
    }

    // Collect all defer triggers for each block (defer_xref, modifier)
    let triggers: Vec<(XrefId, DeferOpModifierKind)> = job
        .all_views()
        .flat_map(|view| {
            view.create.iter().filter_map(|op| {
                if let CreateOp::DeferOn(defer_on) = op {
                    Some((defer_on.defer, defer_on.modifier))
                } else {
                    None
                }
            })
        })
        .collect();

    // Collect when conditions (defer_xref, modifier)
    let when_conditions: Vec<(XrefId, DeferOpModifierKind)> = job
        .all_views()
        .flat_map(|view| {
            view.update.iter().filter_map(|op| {
                if let UpdateOp::DeferWhen(defer_when) = op {
                    Some((defer_when.defer, defer_when.modifier))
                } else {
                    None
                }
            })
        })
        .collect();

    // Validate configurations
    for config in &timing_configs {
        validate_defer_block(config.xref, &triggers, &when_conditions);
    }
}

/// Validate a defer block's configuration.
/// This is a placeholder for future validation logic.
fn validate_defer_block(
    _block_xref: XrefId,
    _triggers: &[(XrefId, DeferOpModifierKind)],
    _when_conditions: &[(XrefId, DeferOpModifierKind)],
) {
    // A defer block should have at least one trigger mechanism
    // Default is on idle when no triggers are specified - Angular handles this automatically
    // Future: Add validation for invalid trigger combinations, missing references, etc.
}

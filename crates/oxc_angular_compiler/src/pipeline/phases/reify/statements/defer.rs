//! @defer block statement generation.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_span::Atom;

use crate::ir::enums::{DeferOpModifierKind, DeferTriggerKind};
use crate::output::ast::{
    LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

use super::super::utils::create_instruction_call_stmt;

/// Creates an ɵɵdefer() call statement.
///
/// The ɵɵdefer instruction takes:
/// - slot: The slot index for the defer block
/// - mainSlot: The slot for the main template
/// - resolverFn: Dependency resolver function expression (optional)
/// - loadingSlot: Loading template slot (optional)
/// - placeholderSlot: Placeholder template slot (optional)
/// - errorSlot: Error template slot (optional)
/// - loadingConfig: Loading timing config const index (optional)
/// - placeholderConfig: Placeholder timing config const index (optional)
/// - timerScheduling: Reference to ɵɵdeferEnableTimerScheduling when needed
/// - flags: Defer block flags (e.g., HasHydrateTriggers = 1)
pub fn create_defer_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    main_slot: Option<u32>,
    resolver_fn: Option<OutputExpression<'a>>,
    loading_slot: Option<u32>,
    placeholder_slot: Option<u32>,
    error_slot: Option<u32>,
    loading_config: Option<u32>,
    placeholder_config: Option<u32>,
    flags: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Main slot
    if let Some(ms) = main_slot {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(ms as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number((slot + 1) as f64), source_span: None },
            allocator,
        )));
    }

    // Resolver function expression (already a ReadVar or other expression)
    if let Some(resolver) = resolver_fn {
        args.push(resolver);
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Loading slot
    if let Some(ls) = loading_slot {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(ls as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Placeholder slot
    if let Some(ps) = placeholder_slot {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(ps as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Error slot
    if let Some(es) = error_slot {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(es as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Loading config - const pool index (points to [minimumTime, afterTime] in consts array)
    if let Some(config_idx) = loading_config {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(config_idx as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Placeholder config - const pool index (points to [minimumTime] in consts array)
    if let Some(config_idx) = placeholder_config {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(config_idx as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Timer scheduling - emit i0.ɵɵdeferEnableTimerScheduling when needed,
    // otherwise emit null. Angular trims trailing null arguments.
    let timer_scheduling = loading_config.is_some() || placeholder_config.is_some();
    if timer_scheduling {
        // Create: i0.ɵɵdeferEnableTimerScheduling (a property access, not a call)
        args.push(OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Atom::from("i0"), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: Atom::from(Identifiers::DEFER_ENABLE_TIMER_SCHEDULING),
                optional: false,
                source_span: None,
            },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Flags argument (10th argument)
    if let Some(f) = flags {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(f as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Trim trailing null arguments, matching Angular's behavior in instruction.ts
    while let Some(last) = args.last() {
        if matches!(last, OutputExpression::Literal(lit) if matches!(lit.value, LiteralValue::Null))
        {
            args.pop();
        } else {
            break;
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::DEFER, args)
}

/// Creates an ɵɵdeferOn*() call statement based on trigger kind.
///
/// Arguments vary by trigger kind:
/// - Idle, Immediate, Never: no arguments
/// - Timer: delay (in milliseconds)
/// - Viewport: target_slot, target_slot_view_steps (optional), options (optional)
/// - Interaction, Hover: target_slot, target_slot_view_steps (optional)
///
/// For hydrate modifier, Viewport/Interaction/Hover triggers don't support targets.
pub fn create_defer_on_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    trigger: DeferTriggerKind,
    target_slot: Option<u32>,
    target_slot_view_steps: Option<i32>,
    modifier: DeferOpModifierKind,
    delay: Option<u32>,
    options: Option<OutputExpression<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Build arguments based on trigger kind, matching Angular's reify.ts behavior
    match trigger {
        DeferTriggerKind::Never | DeferTriggerKind::Idle | DeferTriggerKind::Immediate => {
            // No arguments for these triggers
        }
        DeferTriggerKind::Timer => {
            // Timer trigger takes the delay as first argument
            if let Some(d) = delay {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(d as f64), source_span: None },
                    allocator,
                )));
            }
        }
        DeferTriggerKind::Viewport => {
            // Hydrate triggers don't support targets
            if modifier == DeferOpModifierKind::Hydrate {
                if let Some(opts) = options {
                    args.push(opts);
                }
            } else {
                // Always emit the first arg: slot number or null if unresolved.
                // Angular: o.literal(op.trigger.targetSlot?.slot ?? null)
                args.push(match target_slot {
                    Some(slot) => OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
                        allocator,
                    )),
                    None => OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Null, source_span: None },
                        allocator,
                    )),
                });

                let view_steps = target_slot_view_steps.unwrap_or(0);
                if view_steps != 0 {
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::Number(view_steps as f64),
                            source_span: None,
                        },
                        allocator,
                    )));
                } else if options.is_some() {
                    // Need to push null placeholder if options follow
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Null, source_span: None },
                        allocator,
                    )));
                }

                if let Some(opts) = options {
                    args.push(opts);
                }
            }
        }
        DeferTriggerKind::Interaction | DeferTriggerKind::Hover => {
            // Hydrate triggers don't support targets
            if modifier != DeferOpModifierKind::Hydrate {
                // Always emit the first arg: slot number or null if unresolved.
                // Angular: o.literal(op.trigger.targetSlot?.slot ?? null)
                args.push(match target_slot {
                    Some(slot) => OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
                        allocator,
                    )),
                    None => OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Null, source_span: None },
                        allocator,
                    )),
                });

                let view_steps = target_slot_view_steps.unwrap_or(0);
                if view_steps != 0 {
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::Number(view_steps as f64),
                            source_span: None,
                        },
                        allocator,
                    )));
                }
            }
        }
    }

    // Select instruction based on trigger kind and modifier
    let instruction = match (trigger, modifier) {
        // None modifier (regular defer)
        (DeferTriggerKind::Idle, DeferOpModifierKind::None) => Identifiers::DEFER_ON_IDLE,
        (DeferTriggerKind::Immediate, DeferOpModifierKind::None) => Identifiers::DEFER_ON_IMMEDIATE,
        (DeferTriggerKind::Timer, DeferOpModifierKind::None) => Identifiers::DEFER_ON_TIMER,
        (DeferTriggerKind::Viewport, DeferOpModifierKind::None) => Identifiers::DEFER_ON_VIEWPORT,
        (DeferTriggerKind::Interaction, DeferOpModifierKind::None) => {
            Identifiers::DEFER_ON_INTERACTION
        }
        (DeferTriggerKind::Hover, DeferOpModifierKind::None) => Identifiers::DEFER_ON_HOVER,
        (DeferTriggerKind::Never, DeferOpModifierKind::None) => Identifiers::DEFER_HYDRATE_NEVER,
        // Prefetch modifier
        (DeferTriggerKind::Idle, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_PREFETCH_ON_IDLE
        }
        (DeferTriggerKind::Immediate, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_PREFETCH_ON_IMMEDIATE
        }
        (DeferTriggerKind::Timer, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_PREFETCH_ON_TIMER
        }
        (DeferTriggerKind::Viewport, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_PREFETCH_ON_VIEWPORT
        }
        (DeferTriggerKind::Interaction, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_PREFETCH_ON_INTERACTION
        }
        (DeferTriggerKind::Hover, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_PREFETCH_ON_HOVER
        }
        (DeferTriggerKind::Never, DeferOpModifierKind::Prefetch) => {
            Identifiers::DEFER_HYDRATE_NEVER
        }
        // Hydrate modifier
        (DeferTriggerKind::Idle, DeferOpModifierKind::Hydrate) => {
            Identifiers::DEFER_HYDRATE_ON_IDLE
        }
        (DeferTriggerKind::Immediate, DeferOpModifierKind::Hydrate) => {
            Identifiers::DEFER_HYDRATE_ON_IMMEDIATE
        }
        (DeferTriggerKind::Timer, DeferOpModifierKind::Hydrate) => {
            Identifiers::DEFER_HYDRATE_ON_TIMER
        }
        (DeferTriggerKind::Viewport, DeferOpModifierKind::Hydrate) => {
            Identifiers::DEFER_HYDRATE_ON_VIEWPORT
        }
        (DeferTriggerKind::Interaction, DeferOpModifierKind::Hydrate) => {
            Identifiers::DEFER_HYDRATE_ON_INTERACTION
        }
        (DeferTriggerKind::Hover, DeferOpModifierKind::Hydrate) => {
            Identifiers::DEFER_HYDRATE_ON_HOVER
        }
        (DeferTriggerKind::Never, DeferOpModifierKind::Hydrate) => Identifiers::DEFER_HYDRATE_NEVER,
    };

    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an ɵɵdeferWhen() call statement.
///
/// The ɵɵdeferWhen instruction takes a condition expression that determines
/// when the deferred content should be loaded. For prefetch and hydrate modifiers,
/// it uses ɵɵdeferPrefetchWhen and ɵɵdeferHydrateWhen respectively.
pub fn create_defer_when_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    modifier: DeferOpModifierKind,
    condition: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(condition);

    let instruction = match modifier {
        DeferOpModifierKind::Prefetch => Identifiers::DEFER_PREFETCH_WHEN,
        DeferOpModifierKind::Hydrate => Identifiers::DEFER_HYDRATE_WHEN,
        DeferOpModifierKind::None => Identifiers::DEFER_WHEN,
    };

    create_instruction_call_stmt(allocator, instruction, args)
}

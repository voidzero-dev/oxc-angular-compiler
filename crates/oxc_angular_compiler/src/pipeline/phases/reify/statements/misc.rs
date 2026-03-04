//! Miscellaneous statement generation (listener, animation, pipe, projection, etc.).

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_span::Atom;

use crate::ir::enums::AnimationKind;
use crate::output::ast::{
    FnParam, FunctionExpr, LiteralExpr, LiteralValue, OutputExpression, OutputStatement,
};
use crate::r3::Identifiers;

use super::super::utils::create_instruction_call_stmt;

/// Global event target type for listener instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalEventTarget {
    /// Window object (window:event)
    Window,
    /// Document object (document:event)
    Document,
    /// Body element (body:event)
    Body,
}

impl GlobalEventTarget {
    /// Get the resolver instruction name for this global target.
    fn resolver_instruction(&self) -> &'static str {
        match self {
            GlobalEventTarget::Window => Identifiers::RESOLVE_WINDOW,
            GlobalEventTarget::Document => Identifiers::RESOLVE_DOCUMENT,
            GlobalEventTarget::Body => Identifiers::RESOLVE_BODY,
        }
    }

    /// Parse a string into a GlobalEventTarget.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "window" => Some(GlobalEventTarget::Window),
            "document" => Some(GlobalEventTarget::Document),
            "body" => Some(GlobalEventTarget::Body),
            _ => None,
        }
    }
}

/// Creates an ɵɵlistener() call statement with a handler function.
///
/// If an `event_target` is provided (window, document, or body), the listener
/// will include a resolver function as the third argument to target that global object.
///
/// The `use_capture` flag is used for legacy host animation listeners and adds
/// a fourth boolean argument to the listener call.
///
/// The `handler_fn_name` parameter sets the name of the handler function expression.
/// The `consumes_dollar_event` parameter controls whether the `$event` parameter is included.
pub fn create_listener_stmt_with_handler<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    handler_stmts: OxcVec<'a, OutputStatement<'a>>,
    event_target: Option<GlobalEventTarget>,
    use_capture: bool,
    handler_fn_name: Option<&Atom<'a>>,
    consumes_dollar_event: bool,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    // Event name
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));

    // Handler function: function name($event) { ... } or function name() { ... }
    let mut params = OxcVec::new_in(allocator);
    if consumes_dollar_event {
        params.push(FnParam { name: Atom::from("$event") });
    }

    let handler_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: handler_fn_name.cloned(),
            params,
            statements: handler_stmts,
            source_span: None,
        },
        allocator,
    ));
    args.push(handler_fn);

    // Optional global target resolver (e.g., ɵɵresolveWindow, ɵɵresolveDocument, ɵɵresolveBody)
    // These are Angular runtime functions and need the i0. namespace prefix
    if let Some(target) = event_target {
        args.push(OutputExpression::ReadProp(Box::new_in(
            crate::output::ast::ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        crate::output::ast::ReadVarExpr {
                            name: Atom::from("i0"),
                            source_span: None,
                        },
                        allocator,
                    )),
                    allocator,
                ),
                name: Atom::from(target.resolver_instruction()),
                optional: false,
                source_span: None,
            },
            allocator,
        )));
    } else if use_capture {
        // If we need use_capture but no event_target, add null as placeholder
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Add use_capture flag if true (for host animation listeners)
    if use_capture {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(true), source_span: None },
            allocator,
        )));
    }

    create_instruction_call_stmt(allocator, Identifiers::LISTENER, args)
}

/// Creates an ɵɵdomListener() call statement with a handler function.
///
/// Used in DomOnly mode when the component has no directive dependencies.
/// This is an optimized version that skips directive output matching.
///
/// The `handler_fn_name` parameter sets the name of the handler function expression.
/// The `consumes_dollar_event` parameter controls whether the `$event` parameter is included.
pub fn create_dom_listener_stmt_with_handler<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    handler_stmts: OxcVec<'a, OutputStatement<'a>>,
    event_target: Option<GlobalEventTarget>,
    handler_fn_name: Option<&Atom<'a>>,
    consumes_dollar_event: bool,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    // Event name
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));

    // Handler function: function name($event) { ... } or function name() { ... }
    let mut params = OxcVec::new_in(allocator);
    if consumes_dollar_event {
        params.push(FnParam { name: Atom::from("$event") });
    }

    let handler_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: handler_fn_name.cloned(),
            params,
            statements: handler_stmts,
            source_span: None,
        },
        allocator,
    ));
    args.push(handler_fn);

    // Optional global target resolver
    // These are Angular runtime functions and need the i0. namespace prefix
    if let Some(target) = event_target {
        args.push(OutputExpression::ReadProp(Box::new_in(
            crate::output::ast::ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        crate::output::ast::ReadVarExpr {
                            name: Atom::from("i0"),
                            source_span: None,
                        },
                        allocator,
                    )),
                    allocator,
                ),
                name: Atom::from(target.resolver_instruction()),
                optional: false,
                source_span: None,
            },
            allocator,
        )));
    }

    create_instruction_call_stmt(allocator, Identifiers::DOM_LISTENER, args)
}

/// Creates an ɵɵtwoWayListener() call statement with a handler function.
///
/// The `handler_fn_name` parameter sets the name of the handler function expression.
pub fn create_two_way_listener_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    handler_stmts: OxcVec<'a, OutputStatement<'a>>,
    handler_fn_name: Option<&Atom<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    // Event name (typically "{property}Change")
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));

    // Handler function: function name($event) { ... }
    // Two-way listeners always consume $event since they need the new value
    let mut params = OxcVec::new_in(allocator);
    params.push(FnParam { name: Atom::from("$event") });

    let handler_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: handler_fn_name.cloned(),
            params,
            statements: handler_stmts,
            source_span: None,
        },
        allocator,
    ));
    args.push(handler_fn);

    create_instruction_call_stmt(allocator, Identifiers::TWO_WAY_LISTENER, args)
}

/// Creates an ɵɵsyntheticHostListener() call statement for animation listeners.
///
/// Animation listeners have event names like "@trigger.start" or "@trigger.done".
/// The AnimationKind enum currently has Enter/Leave for direction, but animation
/// callbacks use start/done for timing. For now, we map Enter -> start, Leave -> done.
///
/// The `handler_fn_name` parameter sets the name of the handler function expression.
/// The `consumes_dollar_event` parameter controls whether the `$event` parameter is included.
pub fn create_animation_listener_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    phase: crate::ir::enums::AnimationKind,
    handler_stmts: OxcVec<'a, OutputStatement<'a>>,
    handler_fn_name: Option<&Atom<'a>>,
    consumes_dollar_event: bool,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Build the full event name: "@{name}.{phase}"
    // AnimationKind::Enter maps to "start", AnimationKind::Leave maps to "done"
    let phase_str = match phase {
        crate::ir::enums::AnimationKind::Enter => "start",
        crate::ir::enums::AnimationKind::Leave => "done",
    };
    let full_name = allocator.alloc_str(&format!("@{}.{}", name.as_str(), phase_str));
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Atom::from(full_name)), source_span: None },
        allocator,
    )));

    // Handler function: function name($event) { ... } or function name() { ... }
    let mut params = OxcVec::new_in(allocator);
    if consumes_dollar_event {
        params.push(FnParam { name: Atom::from("$event") });
    }

    let handler_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: handler_fn_name.cloned(),
            params,
            statements: handler_stmts,
            source_span: None,
        },
        allocator,
    ));
    args.push(handler_fn);

    create_instruction_call_stmt(allocator, Identifiers::SYNTHETIC_HOST_LISTENER, args)
}

/// Creates an ɵɵanimateEnter() or ɵɵanimateLeave() call statement for animation string bindings.
///
/// The instruction takes just the value (e.g., "slide" or "fade"), not the animation name.
/// Example: `ɵɵanimateEnter("slide")` for `<div animate.enter="slide">`
pub fn create_animation_string_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    animation_kind: AnimationKind,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(value);
    let instruction = match animation_kind {
        AnimationKind::Enter => Identifiers::ANIMATION_ENTER,
        AnimationKind::Leave => Identifiers::ANIMATION_LEAVE,
    };
    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an ɵɵsyntheticHostProperty() call statement for DomProperty animation bindings.
///
/// This is the simple form used for LegacyAnimation and Animation binding kinds on DomProperty.
/// It emits `syntheticHostProperty(name, value)`.
pub fn create_animation_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Animation name with @ prefix
    let full_name = allocator.alloc_str(&format!("@{}", name.as_str()));
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Atom::from(full_name)), source_span: None },
        allocator,
    )));
    args.push(value);

    create_instruction_call_stmt(allocator, Identifiers::SYNTHETIC_HOST_PROPERTY, args)
}

/// Creates an ɵɵanimateEnter() or ɵɵanimateLeave() call statement for animation CreateOp.
///
/// This is called for AnimationOp (CreateOp) which contains handler_ops with a return statement.
/// The handler function returns the animation value expression.
///
/// Corresponds to Angular's reify.ts handling of Animation ops:
/// ```typescript
/// const animationCallbackFn = reifyListenerHandler(...);
/// ng.animation(op.animationKind, animationCallbackFn, op.sanitizer, op.sourceSpan);
/// ```
///
/// Where ng.animation generates:
/// - `Identifiers.animationEnter` for AnimationKind::ENTER
/// - `Identifiers.animationLeave` for AnimationKind::LEAVE
pub fn create_animation_op_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    animation_kind: AnimationKind,
    handler_stmts: OxcVec<'a, OutputStatement<'a>>,
    handler_fn_name: Option<&Atom<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Handler function: function name() { return expr; }
    let handler_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: handler_fn_name.cloned(),
            params: OxcVec::new_in(allocator),
            statements: handler_stmts,
            source_span: None,
        },
        allocator,
    ));
    args.push(handler_fn);

    // Note: sanitizer would be pushed here if not null, but AnimationOp currently
    // doesn't include a sanitizer expression in the handler

    let instruction = match animation_kind {
        AnimationKind::Enter => Identifiers::ANIMATION_ENTER,
        AnimationKind::Leave => Identifiers::ANIMATION_LEAVE,
    };
    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an animation binding call statement.
pub fn create_animation_binding_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    create_instruction_call_stmt(allocator, Identifiers::SYNTHETIC_HOST_PROPERTY, args)
}

/// Creates a control binding call statement (ɵɵcontrol).
///
/// The control instruction takes:
/// - expression: The expression to evaluate for the control value
/// - name: The property name as a string literal
/// - sanitizer: Optional sanitizer (only if not null)
///
/// Note: Unlike property() which takes (name, expression), control() takes (expression, name).
/// Ported from Angular's `control()` in `instruction.ts` lines 598-614.
pub fn create_control_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    value: OutputExpression<'a>,
    name: &Atom<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(value);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    // Note: sanitizer would be pushed here if not null, but it's always null for ControlOp
    create_instruction_call_stmt(allocator, Identifiers::CONTROL, args)
}

/// Creates an ɵɵprojectionDef() call statement from a pre-built R3 def expression.
///
/// If `def` is None, no argument is passed (default single-wildcard case).
/// If `def` is Some, it's passed as the selector array argument.
pub fn create_projection_def_stmt_from_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    def: Option<&OutputExpression<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    if let Some(def_expr) = def {
        args.push(def_expr.clone_in(allocator));
    }

    create_instruction_call_stmt(allocator, Identifiers::PROJECTION_DEF, args)
}

/// Creates an ɵɵdisableBindings() call statement.
pub fn create_disable_bindings_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    create_instruction_call_stmt(
        allocator,
        Identifiers::DISABLE_BINDINGS,
        OxcVec::new_in(allocator),
    )
}

/// Creates an ɵɵenableBindings() call statement.
pub fn create_enable_bindings_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    create_instruction_call_stmt(allocator, Identifiers::ENABLE_BINDINGS, OxcVec::new_in(allocator))
}

/// Creates an ɵɵpipe() call statement.
pub fn create_pipe_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    name: &Atom<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    create_instruction_call_stmt(allocator, Identifiers::PIPE, args)
}

//! Shared utilities for the reify phase.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_span::Ident;

use crate::output::ast::{
    ExpressionStatement, InvokeFunctionExpr, OutputExpression, OutputStatement, ReadPropExpr,
    ReadVarExpr,
};

/// Strips a prefix from a binding name if present.
/// For example: "class.active" -> "active", "style.color" -> "color", "attr.aria-label" -> "aria-label"
pub fn strip_prefix<'a>(name: &Ident<'a>, prefix: &str) -> Ident<'a> {
    if name.as_str().starts_with(prefix) {
        Ident::from(&name.as_str()[prefix.len()..])
    } else {
        name.clone()
    }
}

/// Creates an instruction call statement.
pub fn create_instruction_call_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    instruction: &'a str,
    args: OxcVec<'a, OutputExpression<'a>>,
) -> OutputStatement<'a> {
    // Create: i0.ɵɵinstruction(args)
    let fn_expr = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(instruction),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let invoke = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    OutputStatement::Expression(Box::new_in(
        ExpressionStatement { expr: invoke, source_span: None },
        allocator,
    ))
}

/// Creates an instruction call expression (not statement).
pub fn create_instruction_call_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    instruction: &'a str,
    args: OxcVec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    let fn_expr = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(instruction),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates a value interpolation expression (ɵɵinterpolate1-8 or ɵɵinterpolateV).
///
/// This is used for property/attribute bindings with interpolation like `[title]="Hello {{name}}"`.
/// The args should be strings and expressions interleaved: [s0, v0, s1, v1, s2, ...]
///
/// Unlike textInterpolate which is a statement, this returns an expression that can be
/// passed to ɵɵproperty, ɵɵattribute, etc.
pub fn create_value_interpolate_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
) -> OutputExpression<'a> {
    use crate::r3::identifiers::get_interpolate_instruction;
    create_instruction_call_expr(allocator, get_interpolate_instruction(expr_count), args)
}

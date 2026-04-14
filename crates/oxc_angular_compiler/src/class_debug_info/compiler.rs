//! Class debug info compilation.
//!
//! Ported from Angular's `render3/r3_class_debug_info_compiler.ts`.
//!
//! Generates:
//! - `setClassDebugInfo(type, { className, filePath?, lineNumber?, forbidOrphanRendering? })`

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::metadata::R3ClassDebugInfo;
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, ExpressionStatement,
    InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue,
    OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr, TypeofExpr,
};
use crate::r3::Identifiers;

/// Compiles class debug info wrapped in a dev-only IIFE.
///
/// Generates:
/// ```javascript
/// (() => {
///   (typeof ngDevMode === "undefined" || ngDevMode) &&
///     i0.ɵsetClassDebugInfo(Type, {
///       className: "ClassName",
///       filePath: "path/to/file.ts",
///       lineNumber: N
///     });
/// })();
/// ```
///
/// Ported from Angular's `compileClassDebugInfo` function.
pub fn compile_class_debug_info<'a>(
    allocator: &'a Allocator,
    debug_info: &R3ClassDebugInfo<'a>,
) -> OutputExpression<'a> {
    let fn_call = internal_compile_class_debug_info(allocator, debug_info);
    let guarded = dev_only_guarded_expression(allocator, fn_call);
    let stmt = expr_stmt(allocator, guarded);
    create_arrow_iife(allocator, stmt)
}

/// Compiles the internal `setClassDebugInfo` call without wrappers.
fn internal_compile_class_debug_info<'a>(
    allocator: &'a Allocator,
    debug_info: &R3ClassDebugInfo<'a>,
) -> OutputExpression<'a> {
    let import = import_expr(allocator, Identifiers::SET_CLASS_DEBUG_INFO);

    // Build the debug info object
    // Always include className
    let mut entries = Vec::new_in(allocator);

    // className
    entries.push(LiteralMapEntry {
        key: Ident::from("className"),
        value: literal_string_atom(allocator, debug_info.class_name.clone()),
        quoted: false,
    });

    // Include filePath and lineNumber only if filePath is set
    // (matching Angular's behavior - if filePath is null, downstream consumers
    // will typically ignore lineNumber as well)
    if let Some(file_path) = &debug_info.file_path {
        entries.push(LiteralMapEntry {
            key: Ident::from("filePath"),
            value: literal_string_atom(allocator, file_path.clone()),
            quoted: false,
        });

        entries.push(LiteralMapEntry {
            key: Ident::from("lineNumber"),
            value: literal_number(allocator, debug_info.line_number),
            quoted: false,
        });
    }

    // Include forbidOrphanRendering only if it's true (to reduce generated code)
    if debug_info.forbid_orphan_rendering {
        entries.push(LiteralMapEntry {
            key: Ident::from("forbidOrphanRendering"),
            value: literal_bool(allocator, true),
            quoted: false,
        });
    }

    let debug_info_object = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ));

    let mut args = Vec::new_in(allocator);
    args.push(debug_info.r#type.clone_in(allocator));
    args.push(debug_info_object);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(import, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates: (typeof ngDevMode === "undefined" || ngDevMode) && expr
///
/// Ported from Angular's `devOnlyGuardedExpression` in `util.ts`.
fn dev_only_guarded_expression<'a>(
    allocator: &'a Allocator,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let guard_var = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from("ngDevMode"), source_span: None },
        allocator,
    ));

    // typeof ngDevMode
    let typeof_guard = OutputExpression::Typeof(Box::new_in(
        TypeofExpr {
            expr: Box::new_in(guard_var.clone_in(allocator), allocator),
            source_span: None,
        },
        allocator,
    ));

    // typeof ngDevMode === "undefined"
    let guard_not_defined = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Identical,
            lhs: Box::new_in(typeof_guard, allocator),
            rhs: Box::new_in(literal_string(allocator, "undefined"), allocator),
            source_span: None,
        },
        allocator,
    ));

    // typeof ngDevMode === "undefined" || ngDevMode
    let guard_undefined_or_true = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(guard_not_defined, allocator),
            rhs: Box::new_in(guard_var, allocator),
            source_span: None,
        },
        allocator,
    ));

    // (typeof ngDevMode === "undefined" || ngDevMode) && expr
    OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::And,
            lhs: Box::new_in(guard_undefined_or_true, allocator),
            rhs: Box::new_in(expr, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Creates an import expression: i0.identifier
fn import_expr<'a>(allocator: &'a Allocator, identifier: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(identifier),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates a string literal from a static str.
fn literal_string<'a>(allocator: &'a Allocator, value: &'static str) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(value)), source_span: None },
        allocator,
    ))
}

/// Creates a string literal from an Atom.
fn literal_string_atom<'a>(allocator: &'a Allocator, value: Ident<'a>) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(value), source_span: None },
        allocator,
    ))
}

/// Creates a number literal.
fn literal_number<'a>(allocator: &'a Allocator, value: u32) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(f64::from(value)), source_span: None },
        allocator,
    ))
}

/// Creates a boolean literal.
fn literal_bool<'a>(allocator: &'a Allocator, value: bool) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Boolean(value), source_span: None },
        allocator,
    ))
}

/// Creates an expression statement.
fn expr_stmt<'a>(allocator: &'a Allocator, expr: OutputExpression<'a>) -> OutputStatement<'a> {
    OutputStatement::Expression(Box::new_in(
        ExpressionStatement { expr, source_span: None },
        allocator,
    ))
}

/// Creates an immediately invoked arrow function: (() => { stmt })()
fn create_arrow_iife<'a>(
    allocator: &'a Allocator,
    stmt: OutputStatement<'a>,
) -> OutputExpression<'a> {
    let empty_params = Vec::new_in(allocator);
    let mut stmts = Vec::new_in(allocator);
    stmts.push(stmt);

    let arrow = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: empty_params,
            body: ArrowFunctionBody::Statements(stmts),
            source_span: None,
        },
        allocator,
    ));

    // Call the arrow function: arrow()
    let empty_args = Vec::new_in(allocator);
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(arrow, allocator),
            args: empty_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

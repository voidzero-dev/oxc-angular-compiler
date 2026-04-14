//! Property, style, class, and attribute binding statement generation.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_str::Ident;

use crate::output::ast::{
    LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
};
use crate::r3::{
    Identifiers, get_attribute_interpolate_instruction, get_class_map_interpolate_instruction,
    get_property_interpolate_instruction, get_style_map_interpolate_instruction,
    get_style_prop_interpolate_instruction, get_text_interpolate_instruction,
};

use super::super::utils::create_instruction_call_stmt;

/// The prefix for ARIA attributes.
const ARIA_PREFIX: &str = "aria-";

/// Checks if an attribute name is an ARIA attribute.
///
/// This is a heuristic based on whether name begins with and is longer than `aria-`.
/// For example, "aria-label" and "aria-hidden" are ARIA attributes.
pub fn is_aria_attribute(name: &str) -> bool {
    name.starts_with(ARIA_PREFIX) && name.len() > ARIA_PREFIX.len()
}

/// DOM properties that need to be remapped on the compiler side.
/// Note: this mapping has to be kept in sync with the equally named mapping in the Angular runtime.
/// See: Angular's `template/pipeline/src/phases/reify.ts`
fn remap_dom_property<'a>(name: &Ident<'a>) -> Ident<'a> {
    match name.as_str() {
        "class" => Ident::from("className"),
        "for" => Ident::from("htmlFor"),
        "formaction" => Ident::from("formAction"),
        "innerHtml" => Ident::from("innerHTML"),
        "readonly" => Ident::from("readOnly"),
        "tabindex" => Ident::from("tabIndex"),
        _ => name.clone(),
    }
}

/// Creates a sanitizer external reference expression.
///
/// The sanitizer name should be the raw function name like "ɵɵsanitizeHtml".
/// This creates an expression like `i0.ɵɵsanitizeHtml`.
fn create_sanitizer_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    sanitizer: &Ident<'a>,
) -> OutputExpression<'a> {
    // Create: i0.ɵɵsanitize* expression
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: sanitizer.clone(),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates an ɵɵproperty() call statement with expression value.
pub fn create_property_stmt_with_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
    sanitizer: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    if let Some(san) = sanitizer {
        args.push(create_sanitizer_expr(allocator, san));
    }
    create_instruction_call_stmt(allocator, Identifiers::PROPERTY, args)
}

/// Creates an ɵɵariaProperty() call statement for ARIA property binding.
///
/// ARIA properties (like `aria-label`, `aria-hidden`, etc.) use a specialized instruction
/// that sets the ARIA attribute rather than a DOM property.
pub fn create_aria_property_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    create_instruction_call_stmt(allocator, Identifiers::ARIA_PROPERTY, args)
}

/// Creates a generic binding statement with expression value.
pub fn create_binding_stmt_with_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    // This should be specialized by binding_specialization phase
    create_instruction_call_stmt(allocator, Identifiers::PROPERTY, args)
}

/// Creates an ɵɵstyleProp() call statement with expression.
pub fn create_style_prop_stmt_with_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
    unit: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    // Add unit suffix if present
    if let Some(unit_val) = unit {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(unit_val.clone()), source_span: None },
            allocator,
        )));
    }
    create_instruction_call_stmt(allocator, Identifiers::STYLE_PROP, args)
}

/// Creates an ɵɵclassProp() call statement with expression.
pub fn create_class_prop_stmt_with_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    create_instruction_call_stmt(allocator, Identifiers::CLASS_PROP, args)
}

/// Creates an ɵɵattribute() call statement with expression.
///
/// Arguments: name, expression, [sanitizer], [namespace]
/// If sanitizer is None but namespace is Some, emits null for sanitizer.
pub fn create_attribute_stmt_with_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
    sanitizer: Option<&Ident<'a>>,
    namespace: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    // Add sanitizer if present, or null if namespace is present
    if sanitizer.is_some() || namespace.is_some() {
        if let Some(san) = sanitizer {
            args.push(create_sanitizer_expr(allocator, san));
        } else {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Null, source_span: None },
                allocator,
            )));
        }
    }
    // Add namespace if present
    if let Some(ns) = namespace {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(ns.clone()), source_span: None },
            allocator,
        )));
    }
    create_instruction_call_stmt(allocator, Identifiers::ATTRIBUTE, args)
}

/// Creates an ɵɵtwoWayProperty() call statement.
pub fn create_two_way_property_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
    sanitizer: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    args.push(value);
    if let Some(san) = sanitizer {
        args.push(create_sanitizer_expr(allocator, san));
    }
    create_instruction_call_stmt(allocator, Identifiers::TWO_WAY_PROPERTY, args)
}

/// Creates an ɵɵdomProperty() call statement for DOM property binding.
///
/// This is an optimized version that avoids unnecessarily trying to bind
/// to directive inputs at runtime for views that don't import any directives.
/// The property name is remapped if necessary (e.g., `for` -> `htmlFor`).
pub fn create_dom_property_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
    sanitizer: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let remapped_name = remap_dom_property(name);
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(remapped_name), source_span: None },
        allocator,
    )));
    args.push(value);
    if let Some(san) = sanitizer {
        args.push(create_sanitizer_expr(allocator, san));
    }
    create_instruction_call_stmt(allocator, Identifiers::DOM_PROPERTY, args)
}

/// Creates an ɵɵstyleMap() call statement.
pub fn create_style_map_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(value);
    create_instruction_call_stmt(allocator, Identifiers::STYLE_MAP, args)
}

/// Creates an ɵɵclassMap() call statement.
pub fn create_class_map_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(value);
    create_instruction_call_stmt(allocator, Identifiers::CLASS_MAP, args)
}

/// Creates an ɵɵtextInterpolate() call statement with arguments.
pub fn create_text_interpolate_stmt_with_args<'a>(
    allocator: &'a oxc_allocator::Allocator,
    args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
) -> OutputStatement<'a> {
    // Choose the appropriate interpolate instruction based on expression count
    // Use helper function from r3::identifiers for the simple case adjustment
    let instruction = if expr_count == 1 && args.len() == 1 {
        // Simple case: just the value (no surrounding strings)
        Identifiers::TEXT_INTERPOLATE
    } else {
        get_text_interpolate_instruction(expr_count)
    };
    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an ɵɵpropertyInterpolate*() call statement (Angular 19).
///
/// For Angular 19, property bindings with interpolation use combined instructions:
/// `ɵɵpropertyInterpolate1("title", "Hello ", name, "")` instead of
/// `ɵɵproperty("title", ɵɵinterpolate1("Hello ", name, ""))`.
///
/// Arguments: name, [s0, v0, s1, v1, ..., sN], [sanitizer]
pub fn create_property_interpolate_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    interp_args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
    sanitizer: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    // Save length before consuming interp_args — the simple case check must use
    // the interpolation args count, not the final args count (which includes name
    // and sanitizer). Otherwise a singleton like `{{url}}` with a sanitizer would
    // mis-select propertyInterpolate1 instead of propertyInterpolate.
    let interp_args_len = interp_args.len();
    let mut args = OxcVec::new_in(allocator);
    // First arg: property name
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    // Then interleaved strings and expressions
    for arg in interp_args {
        args.push(arg);
    }
    // Optional sanitizer
    if let Some(san) = sanitizer {
        args.push(create_sanitizer_expr(allocator, san));
    }
    let instruction = if expr_count == 1 && interp_args_len == 1 {
        // Simple case: just name + value (no surrounding strings)
        // e.g. propertyInterpolate("src", url, sanitizerFn)
        Identifiers::PROPERTY_INTERPOLATE
    } else {
        get_property_interpolate_instruction(expr_count)
    };
    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an ɵɵattributeInterpolate*() call statement (Angular 19).
///
/// For Angular 19, attribute bindings with interpolation use combined instructions:
/// `ɵɵattributeInterpolate1("title", "Hello ", name, "")` instead of
/// `ɵɵattribute("title", ɵɵinterpolate1("Hello ", name, ""))`.
///
/// Arguments: name, [s0, v0, s1, v1, ..., sN], [sanitizer], [namespace]
pub fn create_attribute_interpolate_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    interp_args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
    sanitizer: Option<&Ident<'a>>,
    namespace: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    // Save length before consuming — same reason as create_property_interpolate_stmt.
    let interp_args_len = interp_args.len();
    let mut args = OxcVec::new_in(allocator);
    // First arg: attribute name
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    // Then interleaved strings and expressions
    for arg in interp_args {
        args.push(arg);
    }
    // Optional sanitizer, or null if namespace is present
    if sanitizer.is_some() || namespace.is_some() {
        if let Some(san) = sanitizer {
            args.push(create_sanitizer_expr(allocator, san));
        } else {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Null, source_span: None },
                allocator,
            )));
        }
    }
    // Optional namespace
    if let Some(ns) = namespace {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(ns.clone()), source_span: None },
            allocator,
        )));
    }
    let instruction = if expr_count == 1 && interp_args_len == 1 {
        Identifiers::ATTRIBUTE_INTERPOLATE
    } else {
        get_attribute_interpolate_instruction(expr_count)
    };
    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an ɵɵhostProperty() call statement (Angular 19).
///
/// For Angular 19, host/DomOnly property bindings use `ɵɵhostProperty` instead of `ɵɵdomProperty`.
pub fn create_host_property_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    value: OutputExpression<'a>,
    sanitizer: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let remapped_name = remap_dom_property(name);
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(remapped_name), source_span: None },
        allocator,
    )));
    args.push(value);
    if let Some(san) = sanitizer {
        args.push(create_sanitizer_expr(allocator, san));
    }
    create_instruction_call_stmt(allocator, Identifiers::HOST_PROPERTY, args)
}

/// Creates an ɵɵstylePropInterpolate*() call statement (Angular 19).
///
/// For Angular 19, style prop bindings with interpolation use combined instructions:
/// `ɵɵstylePropInterpolate1("width", "", expr, "px", "px")` instead of
/// `ɵɵstyleProp("width", ɵɵinterpolate1("", expr, "px"), "px")`.
///
/// Signature: `ɵɵstylePropInterpolateN(prop, s0, v0, ..., [unit])`
pub fn create_style_prop_interpolate_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Ident<'a>,
    interp_args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
    unit: Option<&Ident<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    // First arg: style property name
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
        allocator,
    )));
    // Then interleaved strings and expressions
    for arg in interp_args {
        args.push(arg);
    }
    // Optional unit suffix (valueSuffix)
    if let Some(unit_val) = unit {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(unit_val.clone()), source_span: None },
            allocator,
        )));
    }
    let instruction = get_style_prop_interpolate_instruction(expr_count);
    create_instruction_call_stmt(allocator, instruction, args)
}

/// Creates an ɵɵstyleMapInterpolate*() call statement (Angular 19).
///
/// For Angular 19, style map bindings with interpolation use combined instructions:
/// `ɵɵstyleMapInterpolate1("", expr, "")` instead of
/// `ɵɵstyleMap(ɵɵinterpolate1("", expr, ""))`.
///
/// Signature: `ɵɵstyleMapInterpolateN(s0, v0, ...)`
pub fn create_style_map_interpolate_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    interp_args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
) -> OutputStatement<'a> {
    let instruction = get_style_map_interpolate_instruction(expr_count);
    create_instruction_call_stmt(allocator, instruction, interp_args)
}

/// Creates an ɵɵclassMapInterpolate*() call statement (Angular 19).
///
/// For Angular 19, class map bindings with interpolation use combined instructions:
/// `ɵɵclassMapInterpolate1("", expr, "")` instead of
/// `ɵɵclassMap(ɵɵinterpolate1("", expr, ""))`.
///
/// Signature: `ɵɵclassMapInterpolateN(s0, v0, ...)`
pub fn create_class_map_interpolate_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    interp_args: OxcVec<'a, OutputExpression<'a>>,
    expr_count: usize,
) -> OutputStatement<'a> {
    let instruction = get_class_map_interpolate_instruction(expr_count);
    create_instruction_call_stmt(allocator, instruction, interp_args)
}

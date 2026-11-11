//! Expression unparser for converting Angular expression AST back to string representation.
//!
//! This module provides utilities for converting Angular expression AST nodes back into
//! their string representations. It supports two output formats:
//!
//! - **Standard format**: Used by expression_parser, ast_serializer, and html_parser tests.
//!   Wraps pipes in parentheses and uses trailing semicolons for chains.
//!
//! - **R3 format**: Used by r3_transform and ast_spans tests.
//!   Pipes are unwrapped, chains use "; " separator without trailing semicolon.

use oxc_angular_compiler::ast::expression::{
    AngularExpression, ArrowFunction, Binary, BinaryOperator, BindingPipe, Call, Chain,
    Conditional, Interpolation, KeyedRead, LiteralArray, LiteralMap, LiteralMapKey,
    LiteralPrimitive, LiteralValue, NonNullAssert, ParenthesizedExpression, PrefixNot,
    PropertyRead, RegularExpressionLiteral, SafeCall, SafeKeyedRead, SafePropertyRead,
    SpreadElement, TaggedTemplateLiteral, TemplateLiteral, TypeofExpression, Unary, VoidExpression,
};

/// Configuration for expression unparsing behavior
#[derive(Clone, Copy, Default)]
pub struct UnparseConfig {
    /// If true, wrap pipe expressions in parentheses: `(expr | pipe)`
    /// If false, output without parens: `expr | pipe`
    pub wrap_pipes_in_parens: bool,
    /// If true, use trailing semicolon for chains: `a; b;`
    /// If false, use separator only: `a; b`
    pub chain_trailing_semicolon: bool,
    /// If true, normalize whitespace in interpolation strings
    pub normalize_interpolation_whitespace: bool,
}

impl UnparseConfig {
    /// Standard configuration used by expression_parser, ast_serializer, html_parser
    pub const fn standard() -> Self {
        Self {
            wrap_pipes_in_parens: true,
            chain_trailing_semicolon: true,
            normalize_interpolation_whitespace: false,
        }
    }

    /// R3 configuration used by r3_transform, ast_spans
    pub const fn r3() -> Self {
        Self {
            wrap_pipes_in_parens: false,
            chain_trailing_semicolon: false,
            normalize_interpolation_whitespace: true,
        }
    }
}

/// Converts an Angular expression AST back into a string representation.
/// Uses standard configuration (pipes wrapped in parens, trailing semicolons).
pub fn unparse_expression(ast: &AngularExpression<'_>) -> String {
    unparse_expression_with_config(ast, UnparseConfig::standard())
}

/// Converts an Angular expression AST back into a string representation.
/// Uses R3 configuration (no pipe parens, no trailing semicolons, normalized whitespace).
pub fn unparse_expression_r3(ast: &AngularExpression<'_>) -> String {
    unparse_expression_with_config(ast, UnparseConfig::r3())
}

/// Converts an Angular expression AST back into a string representation
/// with the given configuration.
pub fn unparse_expression_with_config(
    ast: &AngularExpression<'_>,
    config: UnparseConfig,
) -> String {
    let mut output = String::new();
    visit_expression(ast, &mut output, &config);
    output
}

fn visit_expression(ast: &AngularExpression<'_>, out: &mut String, config: &UnparseConfig) {
    match ast {
        AngularExpression::Empty(_) | AngularExpression::ImplicitReceiver(_) => {}
        AngularExpression::ThisReceiver(_) => {
            out.push_str("this");
        }
        AngularExpression::Chain(chain) => visit_chain(chain, out, config),
        AngularExpression::Conditional(cond) => visit_conditional(cond, out, config),
        AngularExpression::PropertyRead(prop) => visit_property_read(prop, out, config),
        AngularExpression::SafePropertyRead(prop) => visit_safe_property_read(prop, out, config),
        AngularExpression::KeyedRead(keyed) => visit_keyed_read(keyed, out, config),
        AngularExpression::SafeKeyedRead(keyed) => visit_safe_keyed_read(keyed, out, config),
        AngularExpression::BindingPipe(pipe) => visit_pipe(pipe, out, config),
        AngularExpression::LiteralPrimitive(lit) => visit_literal_primitive(lit, out),
        AngularExpression::LiteralArray(arr) => visit_literal_array(arr, out, config),
        AngularExpression::LiteralMap(map) => visit_literal_map(map, out, config),
        AngularExpression::Interpolation(interp) => visit_interpolation(interp, out, config),
        AngularExpression::Binary(bin) => visit_binary(bin, out, config),
        AngularExpression::Unary(unary) => visit_unary(unary, out, config),
        AngularExpression::PrefixNot(not) => visit_prefix_not(not, out, config),
        AngularExpression::TypeofExpression(typeof_expr) => visit_typeof(typeof_expr, out, config),
        AngularExpression::VoidExpression(void_expr) => visit_void(void_expr, out, config),
        AngularExpression::NonNullAssert(assert) => visit_non_null_assert(assert, out, config),
        AngularExpression::Call(call) => visit_call(call, out, config),
        AngularExpression::SafeCall(call) => visit_safe_call(call, out, config),
        AngularExpression::TaggedTemplateLiteral(tagged) => {
            visit_tagged_template(tagged, out, config);
        }
        AngularExpression::TemplateLiteral(tpl) => visit_template_literal(tpl, out, config),
        AngularExpression::ParenthesizedExpression(paren) => {
            visit_parenthesized(paren, out, config);
        }
        AngularExpression::RegularExpressionLiteral(regex) => visit_regex(regex, out),
        AngularExpression::SpreadElement(spread) => visit_spread_element(spread, out, config),
        AngularExpression::ArrowFunction(arrow) => visit_arrow_function(arrow, out, config),
    }
}

fn is_implicit_receiver(expr: &AngularExpression<'_>) -> bool {
    matches!(
        expr,
        AngularExpression::ImplicitReceiver(_)
            | AngularExpression::ThisReceiver(_)
            | AngularExpression::Empty(_)
    )
}

fn visit_chain(chain: &Chain<'_>, out: &mut String, config: &UnparseConfig) {
    let len = chain.expressions.len();
    for (i, expr) in chain.expressions.iter().enumerate() {
        visit_expression(expr, out, config);
        if config.chain_trailing_semicolon {
            // Standard format: trailing semicolon for last expression
            if i == len - 1 {
                out.push(';');
            } else {
                out.push_str("; ");
            }
        } else {
            // R3 format: semicolon separator only, no trailing
            if i < len - 1 {
                out.push_str("; ");
            }
        }
    }
}

fn visit_conditional(cond: &Conditional<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&cond.condition, out, config);
    out.push_str(" ? ");
    visit_expression(&cond.true_exp, out, config);
    out.push_str(" : ");
    visit_expression(&cond.false_exp, out, config);
}

fn visit_property_read(prop: &PropertyRead<'_>, out: &mut String, config: &UnparseConfig) {
    if is_implicit_receiver(&prop.receiver) {
        out.push_str(prop.name.as_str());
    } else {
        visit_expression(&prop.receiver, out, config);
        out.push('.');
        out.push_str(prop.name.as_str());
    }
}

fn visit_safe_property_read(prop: &SafePropertyRead<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&prop.receiver, out, config);
    out.push_str("?.");
    out.push_str(prop.name.as_str());
}

fn visit_keyed_read(keyed: &KeyedRead<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&keyed.receiver, out, config);
    out.push('[');
    visit_expression(&keyed.key, out, config);
    out.push(']');
}

fn visit_safe_keyed_read(keyed: &SafeKeyedRead<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&keyed.receiver, out, config);
    out.push_str("?.[");
    visit_expression(&keyed.key, out, config);
    out.push(']');
}

fn visit_pipe(pipe: &BindingPipe<'_>, out: &mut String, config: &UnparseConfig) {
    if config.wrap_pipes_in_parens {
        out.push('(');
    }
    visit_expression(&pipe.exp, out, config);
    out.push_str(" | ");
    out.push_str(pipe.name.as_str());
    for arg in &pipe.args {
        out.push(':');
        visit_expression(arg, out, config);
    }
    if config.wrap_pipes_in_parens {
        out.push(')');
    }
}

fn visit_literal_primitive(lit: &LiteralPrimitive, out: &mut String) {
    match &lit.value {
        LiteralValue::Null => out.push_str("null"),
        LiteralValue::Undefined => out.push_str("undefined"),
        LiteralValue::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        LiteralValue::Number(n) => out.push_str(&n.to_string()),
        LiteralValue::String(s) => {
            // Angular's unparse() uses double quotes with escaping
            out.push('"');
            for c in s.as_str().chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }
    }
}

fn visit_literal_array(arr: &LiteralArray<'_>, out: &mut String, config: &UnparseConfig) {
    out.push('[');
    for (i, expr) in arr.expressions.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        visit_expression(expr, out, config);
    }
    out.push(']');
}

fn visit_literal_map(map: &LiteralMap<'_>, out: &mut String, config: &UnparseConfig) {
    out.push('{');
    for (i, (key, value)) in map.keys.iter().zip(map.values.iter()).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match key {
            LiteralMapKey::Property(prop) => {
                if prop.quoted {
                    out.push('"');
                    out.push_str(prop.key.as_str());
                    out.push('"');
                } else {
                    out.push_str(prop.key.as_str());
                }
                out.push_str(": ");
                visit_expression(value, out, config);
            }
            LiteralMapKey::Spread(_) => {
                out.push_str("...");
                visit_expression(value, out, config);
            }
        }
    }
    out.push('}');
}

fn visit_interpolation(interp: &Interpolation<'_>, out: &mut String, config: &UnparseConfig) {
    for (i, expr) in interp.expressions.iter().enumerate() {
        if i < interp.strings.len() {
            if config.normalize_interpolation_whitespace {
                out.push_str(&normalize_whitespace(&interp.strings[i]));
            } else {
                out.push_str(&interp.strings[i]);
            }
        }
        out.push_str("{{ ");
        visit_expression(expr, out, config);
        out.push_str(" }}");
    }
    if interp.strings.len() > interp.expressions.len() {
        if config.normalize_interpolation_whitespace {
            out.push_str(&normalize_whitespace(&interp.strings[interp.expressions.len()]));
        } else {
            out.push_str(&interp.strings[interp.expressions.len()]);
        }
    }
}

fn visit_binary(bin: &Binary<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&bin.left, out, config);
    out.push(' ');
    out.push_str(binary_op_str(bin.operation));
    out.push(' ');
    visit_expression(&bin.right, out, config);
}

/// Convert a binary operator to its string representation
pub fn binary_op_str(op: BinaryOperator) -> &'static str {
    match op {
        BinaryOperator::Equal => "==",
        BinaryOperator::NotEqual => "!=",
        BinaryOperator::StrictEqual => "===",
        BinaryOperator::StrictNotEqual => "!==",
        BinaryOperator::LessThan => "<",
        BinaryOperator::GreaterThan => ">",
        BinaryOperator::LessThanOrEqual => "<=",
        BinaryOperator::GreaterThanOrEqual => ">=",
        BinaryOperator::And => "&&",
        BinaryOperator::Or => "||",
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Modulo => "%",
        BinaryOperator::Power => "**",
        BinaryOperator::NullishCoalescing => "??",
        BinaryOperator::In => "in",
        BinaryOperator::Instanceof => "instanceof",
        // Assignment operators
        BinaryOperator::Assign => "=",
        BinaryOperator::AddAssign => "+=",
        BinaryOperator::SubtractAssign => "-=",
        BinaryOperator::MultiplyAssign => "*=",
        BinaryOperator::DivideAssign => "/=",
        BinaryOperator::ModuloAssign => "%=",
        BinaryOperator::PowerAssign => "**=",
        BinaryOperator::AndAssign => "&&=",
        BinaryOperator::OrAssign => "||=",
        BinaryOperator::NullishCoalescingAssign => "??=",
    }
}

fn visit_unary(unary: &Unary<'_>, out: &mut String, config: &UnparseConfig) {
    let mut inner = String::new();
    visit_expression(&unary.expr, &mut inner, config);
    let op = unary.operator.as_str();
    if unary.is_prefix {
        out.push_str(op);
        // Keep adjacent operators apart, e.g. `- -foo`.
        if inner.starts_with(&op[..1]) {
            out.push(' ');
        }
        out.push_str(&inner);
    } else {
        out.push_str(&inner);
        out.push_str(op);
    }
}

fn visit_prefix_not(not: &PrefixNot<'_>, out: &mut String, config: &UnparseConfig) {
    out.push('!');
    visit_expression(&not.expression, out, config);
}

fn visit_spread_element(spread: &SpreadElement<'_>, out: &mut String, config: &UnparseConfig) {
    out.push_str("...");
    visit_expression(&spread.expression, out, config);
}

fn visit_typeof(typeof_expr: &TypeofExpression<'_>, out: &mut String, config: &UnparseConfig) {
    out.push_str("typeof ");
    visit_expression(&typeof_expr.expression, out, config);
}

fn visit_void(void_expr: &VoidExpression<'_>, out: &mut String, config: &UnparseConfig) {
    out.push_str("void ");
    visit_expression(&void_expr.expression, out, config);
}

fn visit_non_null_assert(assert: &NonNullAssert<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&assert.expression, out, config);
    out.push('!');
}

fn visit_call(call: &Call<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&call.receiver, out, config);
    out.push('(');
    for (i, arg) in call.args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        visit_expression(arg, out, config);
    }
    out.push(')');
}

fn visit_safe_call(call: &SafeCall<'_>, out: &mut String, config: &UnparseConfig) {
    visit_expression(&call.receiver, out, config);
    out.push_str("?.(");
    for (i, arg) in call.args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        visit_expression(arg, out, config);
    }
    out.push(')');
}

fn visit_tagged_template(
    tagged: &TaggedTemplateLiteral<'_>,
    out: &mut String,
    config: &UnparseConfig,
) {
    visit_expression(&tagged.tag, out, config);
    visit_template_literal(&tagged.template, out, config);
}

fn visit_template_literal(tpl: &TemplateLiteral<'_>, out: &mut String, config: &UnparseConfig) {
    out.push('`');
    for (i, element) in tpl.elements.iter().enumerate() {
        out.push_str(element.text.as_str());
        if i < tpl.expressions.len() {
            out.push_str("${");
            visit_expression(&tpl.expressions[i], out, config);
            out.push('}');
        }
    }
    out.push('`');
}

fn visit_parenthesized(
    paren: &ParenthesizedExpression<'_>,
    out: &mut String,
    config: &UnparseConfig,
) {
    out.push('(');
    visit_expression(&paren.expression, out, config);
    out.push(')');
}

fn visit_regex(regex: &RegularExpressionLiteral<'_>, out: &mut String) {
    out.push('/');
    out.push_str(regex.body.as_str());
    out.push('/');
    if let Some(flags) = &regex.flags {
        out.push_str(flags.as_str());
    }
}

fn visit_arrow_function(arrow: &ArrowFunction<'_>, out: &mut String, config: &UnparseConfig) {
    // Single parameter without parens, otherwise with parens
    if arrow.parameters.len() == 1 {
        out.push_str(arrow.parameters[0].name.as_str());
    } else {
        out.push('(');
        for (i, param) in arrow.parameters.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(param.name.as_str());
        }
        out.push(')');
    }
    out.push_str(" => ");
    visit_expression(&arrow.body, out, config);
}

/// Normalize whitespace by collapsing consecutive whitespace characters to single spaces.
/// This matches Angular's behavior for text normalization.
pub fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_whitespace = false;

    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_was_whitespace {
                result.push(' ');
            }
            prev_was_whitespace = true;
        } else {
            result.push(c);
            prev_was_whitespace = false;
        }
    }

    result
}

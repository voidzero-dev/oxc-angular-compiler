//! AST to string conversion (Unparser).
//!
//! This module provides functionality to convert an Angular expression AST
//! back into a string representation. This is used for testing to verify
//! that expressions are parsed correctly.
//!
//! Ported from Angular's `test/expression_parser/utils/unparser.ts`.

use oxc_angular_compiler::ast::expression::{
    AngularExpression, ArrowFunction, Binary, BinaryOperator, BindingPipe, Call, Chain,
    Conditional, Interpolation, KeyedRead, LiteralArray, LiteralMap, LiteralMapKey,
    LiteralPrimitive, LiteralValue, NonNullAssert, ParenthesizedExpression, PrefixNot,
    PropertyRead, RegularExpressionLiteral, SafeCall, SafeKeyedRead, SafePropertyRead,
    SpreadElement, TaggedTemplateLiteral, TemplateLiteral, TypeofExpression, Unary, VoidExpression,
};

/// Converts an Angular expression AST back into a string representation.
pub fn unparse(ast: &AngularExpression<'_>) -> String {
    let mut output = String::new();
    visit_expression(ast, &mut output);
    output
}

fn visit_expression(ast: &AngularExpression<'_>, out: &mut String) {
    match ast {
        AngularExpression::Empty(_) => {}
        AngularExpression::ImplicitReceiver(_) => {}
        AngularExpression::ThisReceiver(_) => {
            // Angular's serialize() outputs "this" for ThisReceiver
            out.push_str("this");
        }
        AngularExpression::Chain(chain) => visit_chain(chain, out),
        AngularExpression::Conditional(cond) => visit_conditional(cond, out),
        AngularExpression::PropertyRead(prop) => visit_property_read(prop, out),
        AngularExpression::SafePropertyRead(prop) => visit_safe_property_read(prop, out),
        AngularExpression::KeyedRead(keyed) => visit_keyed_read(keyed, out),
        AngularExpression::SafeKeyedRead(keyed) => visit_safe_keyed_read(keyed, out),
        AngularExpression::BindingPipe(pipe) => visit_pipe(pipe, out),
        AngularExpression::LiteralPrimitive(lit) => visit_literal_primitive(lit, out),
        AngularExpression::LiteralArray(arr) => visit_literal_array(arr, out),
        AngularExpression::LiteralMap(map) => visit_literal_map(map, out),
        AngularExpression::Interpolation(interp) => visit_interpolation(interp, out),
        AngularExpression::Binary(bin) => visit_binary(bin, out),
        AngularExpression::Unary(unary) => visit_unary(unary, out),
        AngularExpression::PrefixNot(not) => visit_prefix_not(not, out),
        AngularExpression::TypeofExpression(typeof_expr) => visit_typeof(typeof_expr, out),
        AngularExpression::VoidExpression(void_expr) => visit_void(void_expr, out),
        AngularExpression::NonNullAssert(assert) => visit_non_null_assert(assert, out),
        AngularExpression::Call(call) => visit_call(call, out),
        AngularExpression::SafeCall(call) => visit_safe_call(call, out),
        AngularExpression::TaggedTemplateLiteral(tagged) => visit_tagged_template(tagged, out),
        AngularExpression::TemplateLiteral(tpl) => visit_template_literal(tpl, out),
        AngularExpression::ParenthesizedExpression(paren) => visit_parenthesized(paren, out),
        AngularExpression::RegularExpressionLiteral(regex) => visit_regex(regex, out),
        AngularExpression::SpreadElement(spread) => visit_spread_element(spread, out),
        AngularExpression::ArrowFunction(arrow) => visit_arrow_function(arrow, out),
    }
}

fn is_implicit_receiver(expr: &AngularExpression<'_>) -> bool {
    // Both ImplicitReceiver and ThisReceiver are treated as "implicit" for property access
    // In Angular's TypeScript, ThisReceiver extends ImplicitReceiver, so instanceof checks match both
    // This means `this.a` serializes to just "a" (not "this.a")
    matches!(expr, AngularExpression::ImplicitReceiver(_) | AngularExpression::ThisReceiver(_))
}

fn visit_property_read(prop: &PropertyRead<'_>, out: &mut String) {
    // Angular's serialize(): if receiver is implicit, just output the name
    // Don't visit the receiver at all for implicit receivers
    if is_implicit_receiver(&prop.receiver) {
        out.push_str(prop.name.as_str());
    } else {
        visit_expression(&prop.receiver, out);
        out.push('.');
        out.push_str(prop.name.as_str());
    }
}

fn visit_safe_property_read(prop: &SafePropertyRead<'_>, out: &mut String) {
    visit_expression(&prop.receiver, out);
    out.push_str("?.");
    out.push_str(prop.name.as_str());
}

fn visit_unary(unary: &Unary<'_>, out: &mut String) {
    let mut inner = String::new();
    visit_expression(&unary.expr, &mut inner);
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

fn visit_binary(bin: &Binary<'_>, out: &mut String) {
    visit_expression(&bin.left, out);
    out.push(' ');
    out.push_str(binary_op_str(bin.operation));
    out.push(' ');
    visit_expression(&bin.right, out);
}

fn binary_op_str(op: BinaryOperator) -> &'static str {
    match op {
        BinaryOperator::Equal => "==",
        BinaryOperator::NotEqual => "!=",
        BinaryOperator::StrictEqual => "===",
        BinaryOperator::StrictNotEqual => "!==",
        BinaryOperator::LessThan => "<",
        BinaryOperator::GreaterThan => ">",
        BinaryOperator::LessThanOrEqual => "<=",
        BinaryOperator::GreaterThanOrEqual => ">=",
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Modulo => "%",
        BinaryOperator::Power => "**",
        BinaryOperator::And => "&&",
        BinaryOperator::Or => "||",
        BinaryOperator::NullishCoalescing => "??",
        BinaryOperator::In => "in",
        BinaryOperator::Instanceof => "instanceof",
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

fn visit_chain(chain: &Chain<'_>, out: &mut String) {
    // Angular's serialize() does NOT add a trailing semicolon for chain expressions
    let len = chain.expressions.len();
    for (i, expr) in chain.expressions.iter().enumerate() {
        visit_expression(expr, out);
        if i < len - 1 {
            out.push_str("; ");
        }
    }
}

fn visit_conditional(cond: &Conditional<'_>, out: &mut String) {
    // TS: Always outputs full ternary (no incomplete ternary handling)
    visit_expression(&cond.condition, out);
    out.push_str(" ? ");
    visit_expression(&cond.true_exp, out);
    out.push_str(" : ");
    visit_expression(&cond.false_exp, out);
}

fn visit_pipe(pipe: &BindingPipe<'_>, out: &mut String) {
    // Angular's serialize() does NOT wrap pipes in parentheses
    visit_expression(&pipe.exp, out);
    out.push_str(" | ");
    out.push_str(pipe.name.as_str());
    for arg in &pipe.args {
        out.push(':');
        visit_expression(arg, out);
    }
}

fn visit_call(call: &Call<'_>, out: &mut String) {
    visit_expression(&call.receiver, out);
    out.push('(');
    for (i, arg) in call.args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        visit_expression(arg, out);
    }
    out.push(')');
}

fn visit_safe_call(call: &SafeCall<'_>, out: &mut String) {
    visit_expression(&call.receiver, out);
    out.push_str("?.(");
    for (i, arg) in call.args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        visit_expression(arg, out);
    }
    out.push(')');
}

fn visit_interpolation(interp: &Interpolation<'_>, out: &mut String) {
    for (i, s) in interp.strings.iter().enumerate() {
        out.push_str(s.as_str());
        if i < interp.expressions.len() {
            out.push_str("{{ ");
            visit_expression(&interp.expressions[i], out);
            out.push_str(" }}");
        }
    }
}

fn visit_keyed_read(keyed: &KeyedRead<'_>, out: &mut String) {
    // TS: ALWAYS visits receiver (no implicit receiver skip)
    visit_expression(&keyed.receiver, out);
    out.push('[');
    visit_expression(&keyed.key, out);
    out.push(']');
}

fn visit_safe_keyed_read(keyed: &SafeKeyedRead<'_>, out: &mut String) {
    // TS: ALWAYS visits receiver (no implicit receiver skip)
    visit_expression(&keyed.receiver, out);
    out.push_str("?.[");
    visit_expression(&keyed.key, out);
    out.push(']');
}

fn visit_literal_array(arr: &LiteralArray<'_>, out: &mut String) {
    out.push('[');
    for (i, expr) in arr.expressions.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        visit_expression(expr, out);
    }
    out.push(']');
}

fn visit_literal_map(map: &LiteralMap<'_>, out: &mut String) {
    out.push('{');
    for (i, (key, value)) in map.keys.iter().zip(map.values.iter()).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match key {
            LiteralMapKey::Property(prop) => {
                if prop.quoted {
                    // Angular's serialize() uses single quotes for quoted map keys
                    out.push('\'');
                    for c in prop.key.as_str().chars() {
                        match c {
                            '\\' => out.push_str("\\\\"),
                            '\'' => out.push_str("\\'"),
                            _ => out.push(c),
                        }
                    }
                    out.push('\'');
                } else {
                    out.push_str(prop.key.as_str());
                }
                out.push_str(": ");
                visit_expression(value, out);
            }
            LiteralMapKey::Spread(_) => {
                out.push_str("...");
                visit_expression(value, out);
            }
        }
    }
    out.push('}');
}

fn visit_spread_element(spread: &SpreadElement<'_>, out: &mut String) {
    out.push_str("...");
    visit_expression(&spread.expression, out);
}

fn visit_literal_primitive(lit: &LiteralPrimitive<'_>, out: &mut String) {
    match &lit.value {
        LiteralValue::Null => out.push_str("null"),
        LiteralValue::Undefined => out.push_str("undefined"),
        LiteralValue::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        LiteralValue::Number(n) => {
            // Format number without trailing .0 for integers
            if n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{n}"));
            }
        }
        LiteralValue::String(s) => {
            // Angular's serialize() uses single quotes with proper escaping
            out.push('\'');
            for c in s.as_str().chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\'' => out.push_str("\\'"),
                    _ => out.push(c),
                }
            }
            out.push('\'');
        }
    }
}

fn visit_prefix_not(not: &PrefixNot<'_>, out: &mut String) {
    out.push('!');
    visit_expression(&not.expression, out);
}

fn visit_typeof(typeof_expr: &TypeofExpression<'_>, out: &mut String) {
    out.push_str("typeof ");
    visit_expression(&typeof_expr.expression, out);
}

fn visit_void(void_expr: &VoidExpression<'_>, out: &mut String) {
    out.push_str("void ");
    visit_expression(&void_expr.expression, out);
}

fn visit_non_null_assert(assert: &NonNullAssert<'_>, out: &mut String) {
    visit_expression(&assert.expression, out);
    out.push('!');
}

fn visit_template_literal(tpl: &TemplateLiteral<'_>, out: &mut String) {
    out.push('`');
    for (i, elem) in tpl.elements.iter().enumerate() {
        out.push_str(elem.text.as_str());
        if i < tpl.expressions.len() {
            out.push_str("${");
            visit_expression(&tpl.expressions[i], out);
            out.push('}');
        }
    }
    out.push('`');
}

fn visit_tagged_template(tagged: &TaggedTemplateLiteral<'_>, out: &mut String) {
    visit_expression(&tagged.tag, out);
    visit_template_literal(&tagged.template, out);
}

fn visit_parenthesized(paren: &ParenthesizedExpression<'_>, out: &mut String) {
    out.push('(');
    visit_expression(&paren.expression, out);
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

fn visit_arrow_function(arrow: &ArrowFunction<'_>, out: &mut String) {
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
    visit_expression(&arrow.body, out);
}

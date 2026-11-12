use oxc_allocator::Allocator;
use oxc_angular_compiler::ast::expression::AngularExpression;
use oxc_angular_compiler::parser::expression::Parser;

use super::SubsystemRunner;
use crate::test_case::{TestAssertion, TestResult};

/// Runner for Angular expression serializer conformance tests
/// Tests the serialize(parse(...)) pattern
pub struct ExpressionSerializerRunner;

impl ExpressionSerializerRunner {
    pub fn new() -> Self {
        Self
    }

    /// Parse and serialize an expression
    /// Tries parse_simple_binding first (for pipes), falls back to parse_action (for assignments)
    fn parse_and_serialize(&self, text: &str) -> Result<String, String> {
        // First try parse_simple_binding which allows pipes
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, text);
        let result = parser.parse_simple_binding();

        if result.errors.is_empty() {
            return Ok(serialize_expression(&result.ast));
        }

        // If binding fails, try action mode (allows assignments)
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, text);
        let result = parser.parse_action();

        if !result.errors.is_empty() {
            return Err(format!(
                "Parse errors: {:?}",
                result.errors.iter().map(|e| e.msg.clone()).collect::<Vec<_>>()
            ));
        }

        Ok(serialize_expression(&result.ast))
    }
}

/// Angular-compatible expression serializer
/// Matches the output format from angular/src/expression_parser/serializer.ts
fn serialize_expression(ast: &AngularExpression<'_>) -> String {
    use oxc_angular_compiler::ast::expression::{
        BinaryOperator, LiteralMapKey, LiteralValue, TemplateLiteral,
    };

    fn is_implicit_receiver(expr: &AngularExpression<'_>) -> bool {
        matches!(expr, AngularExpression::ImplicitReceiver(_) | AngularExpression::ThisReceiver(_))
    }

    fn serialize(ast: &AngularExpression<'_>) -> String {
        match ast {
            AngularExpression::Empty(_) | AngularExpression::ImplicitReceiver(_) => String::new(),
            AngularExpression::ThisReceiver(_) => "this".to_string(),
            AngularExpression::Chain(chain) => {
                chain.expressions.iter().map(serialize).collect::<Vec<_>>().join("; ")
            }
            AngularExpression::Conditional(cond) => {
                format!(
                    "{} ? {} : {}",
                    serialize(&cond.condition),
                    serialize(&cond.true_exp),
                    serialize(&cond.false_exp)
                )
            }
            AngularExpression::PropertyRead(prop) => {
                if is_implicit_receiver(&prop.receiver) {
                    prop.name.to_string()
                } else {
                    format!("{}.{}", serialize(&prop.receiver), prop.name)
                }
            }
            AngularExpression::SafePropertyRead(prop) => {
                format!("{}?.{}", serialize(&prop.receiver), prop.name)
            }
            AngularExpression::KeyedRead(keyed) => {
                format!("{}[{}]", serialize(&keyed.receiver), serialize(&keyed.key))
            }
            AngularExpression::SafeKeyedRead(keyed) => {
                format!("{}?.[{}]", serialize(&keyed.receiver), serialize(&keyed.key))
            }
            // Pipes: Angular uses "exp | name" without parentheses
            AngularExpression::BindingPipe(pipe) => {
                let mut result = format!("{} | {}", serialize(&pipe.exp), pipe.name);
                for arg in &pipe.args {
                    result.push(':');
                    result.push_str(&serialize(arg));
                }
                result
            }
            // Strings: Angular uses single quotes
            AngularExpression::LiteralPrimitive(lit) => match &lit.value {
                LiteralValue::Null => "null".to_string(),
                LiteralValue::Undefined => "undefined".to_string(),
                LiteralValue::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
                LiteralValue::Number(n) => n.to_string(),
                LiteralValue::String(s) => {
                    // Escape single quotes and wrap in single quotes
                    format!("'{}'", s.replace('\'', "\\'"))
                }
            },
            AngularExpression::LiteralArray(arr) => {
                format!(
                    "[{}]",
                    arr.expressions.iter().map(serialize).collect::<Vec<_>>().join(", ")
                )
            }
            AngularExpression::LiteralMap(map) => {
                let entries: Vec<String> = map
                    .keys
                    .iter()
                    .zip(map.values.iter())
                    .map(|(k, v)| match k {
                        LiteralMapKey::Property(prop) => {
                            let key_str = if prop.quoted {
                                format!("'{}'", prop.key)
                            } else {
                                prop.key.to_string()
                            };
                            format!("{}: {}", key_str, serialize(v))
                        }
                        LiteralMapKey::Spread(_) => {
                            format!("...{}", serialize(v))
                        }
                    })
                    .collect();
                format!("{{{}}}", entries.join(", "))
            }
            AngularExpression::SpreadElement(spread) => {
                format!("...{}", serialize(&spread.expression))
            }
            AngularExpression::Interpolation(interp) => {
                let mut result = String::new();
                for i in 0..interp.expressions.len() {
                    if i < interp.strings.len() {
                        result.push_str(&interp.strings[i]);
                    }
                    result.push_str(&serialize(&interp.expressions[i]));
                }
                if interp.strings.len() > interp.expressions.len() {
                    result.push_str(&interp.strings[interp.expressions.len()]);
                }
                result
            }
            AngularExpression::Binary(bin) => {
                let op = match bin.operation {
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
                };
                format!("{} {} {}", serialize(&bin.left), op, serialize(&bin.right))
            }
            AngularExpression::Unary(unary) => {
                let op = unary.operator.as_str();
                let inner = serialize(&unary.expr);
                if unary.is_prefix {
                    // Keep adjacent operators apart, e.g. `+ ++foo`.
                    let space = if inner.starts_with(&op[..1]) { " " } else { "" };
                    format!("{op}{space}{inner}")
                } else {
                    format!("{inner}{op}")
                }
            }
            AngularExpression::PrefixNot(not) => {
                format!("!{}", serialize(&not.expression))
            }
            AngularExpression::TypeofExpression(t) => {
                format!("typeof {}", serialize(&t.expression))
            }
            AngularExpression::VoidExpression(v) => {
                format!("void {}", serialize(&v.expression))
            }
            AngularExpression::NonNullAssert(a) => {
                format!("{}!", serialize(&a.expression))
            }
            AngularExpression::Call(call) => {
                format!(
                    "{}({})",
                    serialize(&call.receiver),
                    call.args.iter().map(serialize).collect::<Vec<_>>().join(", ")
                )
            }
            AngularExpression::SafeCall(call) => {
                format!(
                    "{}?.({})",
                    serialize(&call.receiver),
                    call.args.iter().map(serialize).collect::<Vec<_>>().join(", ")
                )
            }
            AngularExpression::TaggedTemplateLiteral(tagged) => {
                format!("{}{}", serialize(&tagged.tag), serialize_template(&tagged.template))
            }
            AngularExpression::TemplateLiteral(tpl) => serialize_template(tpl),
            AngularExpression::ParenthesizedExpression(paren) => {
                format!("({})", serialize(&paren.expression))
            }
            AngularExpression::RegularExpressionLiteral(regex) => {
                if let Some(flags) = &regex.flags {
                    format!("/{}/{}", regex.body, flags)
                } else {
                    format!("/{}/", regex.body)
                }
            }
            AngularExpression::ArrowFunction(arrow) => {
                let params = if arrow.parameters.len() == 1 {
                    arrow.parameters[0].name.to_string()
                } else {
                    format!(
                        "({})",
                        arrow
                            .parameters
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!("{} => {}", params, serialize(&arrow.body))
            }
        }
    }

    fn serialize_template(tpl: &TemplateLiteral<'_>) -> String {
        let mut result = String::from("`");
        for (i, element) in tpl.elements.iter().enumerate() {
            result.push_str(&element.text);
            if i < tpl.expressions.len() {
                result.push_str("${");
                result.push_str(&serialize(&tpl.expressions[i]));
                result.push('}');
            }
        }
        result.push('`');
        result
    }

    serialize(ast)
}

impl Default for ExpressionSerializerRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SubsystemRunner for ExpressionSerializerRunner {
    fn name(&self) -> &'static str {
        "expression_serializer"
    }

    fn description(&self) -> &'static str {
        "Angular expression serializer (serialize/unparse)"
    }

    fn can_handle(&self, assertion: &TestAssertion) -> bool {
        matches!(assertion, TestAssertion::SerializeExpression { .. })
    }

    fn run_assertion(&self, assertion: &TestAssertion) -> TestResult {
        match assertion {
            TestAssertion::SerializeExpression { input, expected } => {
                match self.parse_and_serialize(input) {
                    Ok(result) => {
                        if result == *expected {
                            TestResult::Passed
                        } else {
                            TestResult::Failed {
                                expected: expected.clone(),
                                actual: result,
                                diff: None,
                            }
                        }
                    }
                    Err(e) => TestResult::Error { message: e },
                }
            }
            _ => TestResult::Skipped {
                reason: "Not handled by expression serializer runner".to_string(),
            },
        }
    }
}

//! OXC Expression to OutputExpression converter.
//!
//! This module converts OXC AST expressions (from the TypeScript/JavaScript parser)
//! to Angular's OutputExpression for use in component definitions.
//!
//! This is needed because Angular decorator properties like `animations`, `providers`,
//! and `viewProviders` can contain complex expressions that need to be preserved
//! in the compiled output.
//!
//! Reference: The TypeScript Angular compiler uses `WrappedNodeExpr<ts.Expression>`
//! to wrap the original AST node. Since we cannot pass OXC AST nodes through our
//! output pipeline, we convert them directly to OutputExpression.

use oxc_allocator::{Allocator, Box, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, Expression, ObjectPropertyKind, PropertyKey,
    UnaryOperator as OxcUnaryOperator,
};
use oxc_span::{Ident, Span};

use super::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, CommaExpr,
    ConditionalExpr, FnParam, InstantiateExpr, InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr,
    LiteralMapEntry, LiteralMapExpr, LiteralValue, NotExpr, OutputExpression, OutputStatement,
    ParenthesizedExpr, RawSourceExpr, ReadKeyExpr, ReadPropExpr, ReadVarExpr, ReturnStatement,
    SpreadElementExpr, TemplateLiteralElement, TemplateLiteralExpr, TypeofExpr, UnaryOperator,
    UnaryOperatorExpr, VoidExpr,
};

// ============================================================================
// Main Conversion Function
// ============================================================================

/// Convert an OXC Expression to an OutputExpression.
///
/// This handles common expression types used in Angular decorator properties.
/// Returns `None` if the expression type is not supported.
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `expr` - The OXC expression to convert
/// * `source_text` - Optional source text for falling back to raw source when
///   complex expressions (e.g., block-body arrow functions with unsupported
///   statement types) cannot be fully represented in the output AST.
///
/// # Returns
/// `Some(OutputExpression)` if conversion succeeded, `None` otherwise.
pub fn convert_oxc_expression<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    match expr {
        // Literals
        Expression::BooleanLiteral(lit) => Some(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(lit.value.into()), source_span: None },
            allocator,
        ))),

        Expression::NullLiteral(_) => Some(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        ))),

        Expression::NumericLiteral(lit) => Some(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(lit.value.into()), source_span: None },
            allocator,
        ))),

        Expression::StringLiteral(lit) => Some(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::String(lit.value.clone().into()),
                source_span: None,
            },
            allocator,
        ))),

        // Identifiers
        Expression::Identifier(id) => Some(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: id.name.clone().into(), source_span: None },
            allocator,
        ))),

        // Array expressions
        Expression::ArrayExpression(arr) => convert_array_expression(allocator, arr, source_text),

        // Object expressions
        Expression::ObjectExpression(obj) => convert_object_expression(allocator, obj, source_text),

        // Call expressions
        Expression::CallExpression(call) => convert_call_expression(allocator, call, source_text),

        // New expressions
        Expression::NewExpression(new_expr) => {
            convert_new_expression(allocator, new_expr, source_text)
        }

        // Arrow function expressions
        Expression::ArrowFunctionExpression(arrow) => {
            convert_arrow_function_expression(allocator, arrow, source_text)
        }

        // Function expressions - fall back to raw source if available
        Expression::FunctionExpression(func) => make_raw_source(allocator, source_text, func.span),

        // Member expressions
        Expression::StaticMemberExpression(member) => {
            let receiver = convert_oxc_expression(allocator, &member.object, source_text)?;
            Some(OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(receiver, allocator),
                    name: member.property.name.clone().into(),
                    optional: member.optional,
                    source_span: None,
                },
                allocator,
            )))
        }

        Expression::ComputedMemberExpression(member) => {
            let receiver = convert_oxc_expression(allocator, &member.object, source_text)?;
            let index = convert_oxc_expression(allocator, &member.expression, source_text)?;
            Some(OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver, allocator),
                    index: Box::new_in(index, allocator),
                    optional: member.optional,
                    source_span: None,
                },
                allocator,
            )))
        }

        // Chain expression (optional chaining)
        Expression::ChainExpression(chain) => {
            convert_chain_element(allocator, &chain.expression, source_text)
        }

        // Binary expressions
        Expression::BinaryExpression(bin) => {
            let lhs = convert_oxc_expression(allocator, &bin.left, source_text)?;
            let rhs = convert_oxc_expression(allocator, &bin.right, source_text)?;
            let operator = convert_oxc_binary_operator(bin.operator)?;
            Some(OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(lhs, allocator),
                    rhs: Box::new_in(rhs, allocator),
                    source_span: None,
                },
                allocator,
            )))
        }

        // Logical expressions (&&, ||, ??)
        Expression::LogicalExpression(logical) => {
            let lhs = convert_oxc_expression(allocator, &logical.left, source_text)?;
            let rhs = convert_oxc_expression(allocator, &logical.right, source_text)?;
            let operator = match logical.operator {
                oxc_ast::ast::LogicalOperator::And => BinaryOperator::And,
                oxc_ast::ast::LogicalOperator::Or => BinaryOperator::Or,
                oxc_ast::ast::LogicalOperator::Coalesce => BinaryOperator::NullishCoalesce,
            };
            Some(OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(lhs, allocator),
                    rhs: Box::new_in(rhs, allocator),
                    source_span: None,
                },
                allocator,
            )))
        }

        // Unary expressions
        Expression::UnaryExpression(unary) => {
            convert_unary_expression(allocator, unary, source_text)
        }

        // Conditional expressions (ternary)
        Expression::ConditionalExpression(cond) => {
            let condition = convert_oxc_expression(allocator, &cond.test, source_text)?;
            let true_case = convert_oxc_expression(allocator, &cond.consequent, source_text)?;
            let false_case = convert_oxc_expression(allocator, &cond.alternate, source_text)?;
            Some(OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(condition, allocator),
                    true_case: Box::new_in(true_case, allocator),
                    false_case: Some(Box::new_in(false_case, allocator)),
                    source_span: None,
                },
                allocator,
            )))
        }

        // Template literals
        Expression::TemplateLiteral(tpl) => convert_template_literal(allocator, tpl, source_text),

        // Parenthesized expressions
        Expression::ParenthesizedExpression(paren) => {
            let inner = convert_oxc_expression(allocator, &paren.expression, source_text)?;
            Some(OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr { expr: Box::new_in(inner, allocator), source_span: None },
                allocator,
            )))
        }

        // Sequence expressions (comma operator)
        Expression::SequenceExpression(seq) => {
            let mut parts = OxcVec::with_capacity_in(seq.expressions.len(), allocator);
            for expr in &seq.expressions {
                parts.push(convert_oxc_expression(allocator, expr, source_text)?);
            }
            Some(OutputExpression::Comma(Box::new_in(
                CommaExpr { parts, source_span: None },
                allocator,
            )))
        }

        // This expression
        Expression::ThisExpression(_) => Some(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("this"), source_span: None },
            allocator,
        ))),

        // TypeScript type expressions - unwrap the inner expression
        // These don't affect runtime behavior, just compile-time type checking
        Expression::TSAsExpression(ts_as) => {
            convert_oxc_expression(allocator, &ts_as.expression, source_text)
        }
        Expression::TSTypeAssertion(ts_assert) => {
            convert_oxc_expression(allocator, &ts_assert.expression, source_text)
        }
        Expression::TSSatisfiesExpression(ts_satisfies) => {
            convert_oxc_expression(allocator, &ts_satisfies.expression, source_text)
        }
        Expression::TSNonNullExpression(ts_non_null) => {
            convert_oxc_expression(allocator, &ts_non_null.expression, source_text)
        }
        Expression::TSInstantiationExpression(ts_inst) => {
            convert_oxc_expression(allocator, &ts_inst.expression, source_text)
        }

        // Unsupported expressions - return None
        _ => None,
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert an OXC array expression to an OutputExpression.
fn convert_array_expression<'a>(
    allocator: &'a Allocator,
    arr: &oxc_ast::ast::ArrayExpression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    let mut entries = OxcVec::with_capacity_in(arr.elements.len(), allocator);

    for element in &arr.elements {
        match element {
            ArrayExpressionElement::SpreadElement(spread) => {
                // Convert the inner expression and wrap it in SpreadElement
                let inner_expr = convert_oxc_expression(allocator, &spread.argument, source_text)?;
                let spread_expr = OutputExpression::SpreadElement(Box::new_in(
                    SpreadElementExpr {
                        expr: Box::new_in(inner_expr, allocator),
                        source_span: None,
                    },
                    allocator,
                ));
                entries.push(spread_expr);
            }
            ArrayExpressionElement::Elision(_) => {
                // Elision (empty slot) - push undefined
                entries.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                    allocator,
                )));
            }
            _ => {
                // Regular expression element
                let expr_ref = element.to_expression();
                let converted = convert_oxc_expression(allocator, expr_ref, source_text)?;
                entries.push(converted);
            }
        }
    }

    Some(OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    )))
}

/// Convert an OXC object expression to an OutputExpression.
fn convert_object_expression<'a>(
    allocator: &'a Allocator,
    obj: &oxc_ast::ast::ObjectExpression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    let mut entries = OxcVec::with_capacity_in(obj.properties.len(), allocator);

    for prop in &obj.properties {
        match prop {
            ObjectPropertyKind::ObjectProperty(p) => {
                // Get the property key
                let (key, quoted) = match &p.key {
                    PropertyKey::StaticIdentifier(id) => (id.name.clone().into(), false),
                    PropertyKey::StringLiteral(lit) => (lit.value.clone().into(), true),
                    PropertyKey::NumericLiteral(lit) => {
                        (Ident::from(allocator.alloc_str(&lit.value.to_string())), true)
                    }
                    PropertyKey::PrivateIdentifier(_) => return None, // Private fields not supported
                    _ => {
                        // Computed property key - try to convert it
                        // For now, skip computed properties
                        continue;
                    }
                };

                // Convert the value
                let value = convert_oxc_expression(allocator, &p.value, source_text)?;

                entries.push(LiteralMapEntry { key, value, quoted });
            }
            ObjectPropertyKind::SpreadProperty(_) => {
                // Spread properties are not directly supported in LiteralMap
                // Skip for now
                continue;
            }
        }
    }

    Some(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    )))
}

/// Convert an OXC call expression to an OutputExpression.
fn convert_call_expression<'a>(
    allocator: &'a Allocator,
    call: &oxc_ast::ast::CallExpression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    convert_call_expression_with_optional(allocator, call, call.optional, source_text)
}

/// Convert an OXC call expression to an OutputExpression with explicit optional flag.
fn convert_call_expression_with_optional<'a>(
    allocator: &'a Allocator,
    call: &oxc_ast::ast::CallExpression<'a>,
    optional: bool,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    // Convert the callee
    let fn_expr = convert_oxc_expression(allocator, &call.callee, source_text)?;

    // Convert arguments
    let mut args = OxcVec::with_capacity_in(call.arguments.len(), allocator);
    for arg in &call.arguments {
        match arg {
            Argument::SpreadElement(spread) => {
                // Handle spread arguments
                let expr = convert_oxc_expression(allocator, &spread.argument, source_text)?;
                args.push(expr);
            }
            _ => {
                let expr = arg.to_expression();
                let converted = convert_oxc_expression(allocator, expr, source_text)?;
                args.push(converted);
            }
        }
    }

    Some(OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional,
            source_span: None,
        },
        allocator,
    )))
}

/// Convert an OXC new expression to an OutputExpression.
fn convert_new_expression<'a>(
    allocator: &'a Allocator,
    new_expr: &oxc_ast::ast::NewExpression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    // Convert the callee (class expression)
    let class_expr = convert_oxc_expression(allocator, &new_expr.callee, source_text)?;

    // Convert arguments
    let mut args = OxcVec::with_capacity_in(new_expr.arguments.len(), allocator);
    for arg in &new_expr.arguments {
        match arg {
            Argument::SpreadElement(spread) => {
                let expr = convert_oxc_expression(allocator, &spread.argument, source_text)?;
                args.push(expr);
            }
            _ => {
                let expr = arg.to_expression();
                let converted = convert_oxc_expression(allocator, expr, source_text)?;
                args.push(converted);
            }
        }
    }

    Some(OutputExpression::Instantiate(Box::new_in(
        InstantiateExpr { class_expr: Box::new_in(class_expr, allocator), args, source_span: None },
        allocator,
    )))
}

/// Convert an OXC arrow function expression to an OutputExpression.
fn convert_arrow_function_expression<'a>(
    allocator: &'a Allocator,
    arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    // Convert parameters
    let mut params = OxcVec::with_capacity_in(arrow.params.items.len(), allocator);
    for param in &arrow.params.items {
        // For simplicity, only handle identifier patterns
        if let BindingPattern::BindingIdentifier(id) = &param.pattern {
            params.push(FnParam { name: id.name.clone().into() });
        } else {
            // Complex patterns not supported — fall back to raw source if available
            return make_raw_source(allocator, source_text, arrow.span);
        }
    }

    // Convert body
    let body = if arrow.expression {
        // Expression body: () => expr
        let expr_body = arrow.body.statements.first()?;
        if let oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) = expr_body {
            match convert_oxc_expression(allocator, &expr_stmt.expression, source_text) {
                Some(converted) => ArrowFunctionBody::Expression(Box::new_in(converted, allocator)),
                None => {
                    // Unsupported expression (e.g., await, yield, class) —
                    // fall back to raw source for the entire arrow
                    return make_raw_source(allocator, source_text, arrow.span);
                }
            }
        } else {
            return None;
        }
    } else {
        // Block body: () => { ... }
        // If any statement cannot be converted, fall back to raw source to avoid
        // silently dropping statements (which corrupts the function body).
        let mut statements = OxcVec::with_capacity_in(arrow.body.statements.len(), allocator);
        for stmt in &arrow.body.statements {
            match convert_statement(allocator, stmt, source_text) {
                Some(output_stmt) => statements.push(output_stmt),
                None => {
                    // Unsupported statement type — fall back to raw source
                    return make_raw_source(allocator, source_text, arrow.span);
                }
            }
        }
        ArrowFunctionBody::Statements(statements)
    };

    Some(OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr { params, body, source_span: None },
        allocator,
    )))
}

/// Convert an OXC statement to an OutputStatement (limited support).
///
/// Returns `None` for unsupported statement types (e.g., variable declarations,
/// if/else, for loops, try/catch, etc.). The caller should fall back to raw
/// source preservation when this returns `None`.
fn convert_statement<'a>(
    allocator: &'a Allocator,
    stmt: &oxc_ast::ast::Statement<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputStatement<'a>> {
    match stmt {
        oxc_ast::ast::Statement::ReturnStatement(ret) => {
            // ReturnStatement.value is required, so we need to convert or use undefined
            let value = ret
                .argument
                .as_ref()
                .and_then(|expr| convert_oxc_expression(allocator, expr, source_text))
                .unwrap_or_else(|| {
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                        allocator,
                    ))
                });
            Some(OutputStatement::Return(Box::new_in(
                ReturnStatement { value, source_span: None },
                allocator,
            )))
        }
        oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) => {
            let expr = convert_oxc_expression(allocator, &expr_stmt.expression, source_text)?;
            Some(OutputStatement::Expression(Box::new_in(
                super::ast::ExpressionStatement { expr, source_span: None },
                allocator,
            )))
        }
        // Other statement types not supported — caller should fall back to raw source
        _ => None,
    }
}

/// Convert an OXC unary expression to an OutputExpression.
fn convert_unary_expression<'a>(
    allocator: &'a Allocator,
    unary: &oxc_ast::ast::UnaryExpression<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    let expr = convert_oxc_expression(allocator, &unary.argument, source_text)?;

    match unary.operator {
        OxcUnaryOperator::LogicalNot => Some(OutputExpression::Not(Box::new_in(
            NotExpr { condition: Box::new_in(expr, allocator), source_span: None },
            allocator,
        ))),
        OxcUnaryOperator::Typeof => Some(OutputExpression::Typeof(Box::new_in(
            TypeofExpr { expr: Box::new_in(expr, allocator), source_span: None },
            allocator,
        ))),
        OxcUnaryOperator::Void => Some(OutputExpression::Void(Box::new_in(
            VoidExpr { expr: Box::new_in(expr, allocator), source_span: None },
            allocator,
        ))),
        OxcUnaryOperator::UnaryPlus => Some(OutputExpression::UnaryOperator(Box::new_in(
            UnaryOperatorExpr {
                operator: UnaryOperator::Plus,
                expr: Box::new_in(expr, allocator),
                parens: false,
                source_span: None,
            },
            allocator,
        ))),
        OxcUnaryOperator::UnaryNegation => Some(OutputExpression::UnaryOperator(Box::new_in(
            UnaryOperatorExpr {
                operator: UnaryOperator::Minus,
                expr: Box::new_in(expr, allocator),
                parens: false,
                source_span: None,
            },
            allocator,
        ))),
        // BitwiseNot and Delete operators not directly supported in OutputExpression
        OxcUnaryOperator::BitwiseNot | OxcUnaryOperator::Delete => None,
    }
}

/// Convert an OXC template literal to an OutputExpression.
fn convert_template_literal<'a>(
    allocator: &'a Allocator,
    tpl: &oxc_ast::ast::TemplateLiteral<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    // Convert quasis to template literal elements
    let mut elements = OxcVec::with_capacity_in(tpl.quasis.len(), allocator);
    for quasi in &tpl.quasis {
        let text = quasi
            .value
            .cooked
            .as_ref()
            .map_or_else(|| quasi.value.raw.clone(), |cooked| cooked.clone());
        let raw_text = quasi.value.raw.clone();

        elements.push(TemplateLiteralElement {
            text: text.into(),
            raw_text: raw_text.into(),
            source_span: None,
        });
    }

    // Convert expressions
    let mut expressions = OxcVec::with_capacity_in(tpl.expressions.len(), allocator);
    for expr in &tpl.expressions {
        expressions.push(convert_oxc_expression(allocator, expr, source_text)?);
    }

    Some(OutputExpression::TemplateLiteral(Box::new_in(
        TemplateLiteralExpr { elements, expressions, source_span: None },
        allocator,
    )))
}

/// Convert an OXC ChainElement to an OutputExpression.
///
/// ChainElement is used in optional chaining expressions (`a?.b`, `a?.[b]`, `a?.()`)
/// and can be a CallExpression, MemberExpression, or TSNonNullExpression.
fn convert_chain_element<'a>(
    allocator: &'a Allocator,
    element: &oxc_ast::ast::ChainElement<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    use oxc_ast::ast::ChainElement;

    match element {
        ChainElement::CallExpression(call) => {
            // For call expressions within a chain, the optional flag is already set on the call
            convert_call_expression_with_optional(allocator, call, call.optional, source_text)
        }
        ChainElement::TSNonNullExpression(ts_non_null) => {
            // TypeScript non-null assertion (!) - just convert the inner expression
            convert_oxc_expression(allocator, &ts_non_null.expression, source_text)
        }
        ChainElement::ComputedMemberExpression(member) => {
            let receiver = convert_oxc_expression(allocator, &member.object, source_text)?;
            let index = convert_oxc_expression(allocator, &member.expression, source_text)?;
            Some(OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver, allocator),
                    index: Box::new_in(index, allocator),
                    optional: member.optional,
                    source_span: None,
                },
                allocator,
            )))
        }
        ChainElement::StaticMemberExpression(member) => {
            let receiver = convert_oxc_expression(allocator, &member.object, source_text)?;
            Some(OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(receiver, allocator),
                    name: member.property.name.clone().into(),
                    optional: member.optional,
                    source_span: None,
                },
                allocator,
            )))
        }
        ChainElement::PrivateFieldExpression(_) => {
            // Private fields (#field) are not supported
            None
        }
    }
}

/// Convert an OXC binary operator to an OutputExpression binary operator.
fn convert_oxc_binary_operator(op: oxc_ast::ast::BinaryOperator) -> Option<BinaryOperator> {
    use oxc_ast::ast::BinaryOperator as OxcBinOp;
    match op {
        OxcBinOp::Equality => Some(BinaryOperator::Equals),
        OxcBinOp::Inequality => Some(BinaryOperator::NotEquals),
        OxcBinOp::StrictEquality => Some(BinaryOperator::Identical),
        OxcBinOp::StrictInequality => Some(BinaryOperator::NotIdentical),
        OxcBinOp::LessThan => Some(BinaryOperator::Lower),
        OxcBinOp::LessEqualThan => Some(BinaryOperator::LowerEquals),
        OxcBinOp::GreaterThan => Some(BinaryOperator::Bigger),
        OxcBinOp::GreaterEqualThan => Some(BinaryOperator::BiggerEquals),
        OxcBinOp::Addition => Some(BinaryOperator::Plus),
        OxcBinOp::Subtraction => Some(BinaryOperator::Minus),
        OxcBinOp::Multiplication => Some(BinaryOperator::Multiply),
        OxcBinOp::Division => Some(BinaryOperator::Divide),
        OxcBinOp::Remainder => Some(BinaryOperator::Modulo),
        OxcBinOp::Exponential => Some(BinaryOperator::Exponentiation),
        OxcBinOp::BitwiseAnd => Some(BinaryOperator::BitwiseAnd),
        OxcBinOp::BitwiseOR => Some(BinaryOperator::BitwiseOr),
        OxcBinOp::BitwiseXOR => Some(BinaryOperator::BitwiseXor),
        OxcBinOp::ShiftLeft => Some(BinaryOperator::LeftShift),
        OxcBinOp::ShiftRight => Some(BinaryOperator::RightShift),
        OxcBinOp::ShiftRightZeroFill => Some(BinaryOperator::UnsignedRightShift),
        OxcBinOp::In => Some(BinaryOperator::In),
        OxcBinOp::Instanceof => Some(BinaryOperator::Instanceof),
    }
}

/// Create a `RawSource` expression from source text and span.
///
/// The source text is the original TypeScript source, so this function
/// strips TypeScript type annotations by parsing, transforming, and
/// re-generating the expression as JavaScript.
///
/// Returns `None` if `source_text` is not available.
fn make_raw_source<'a>(
    allocator: &'a Allocator,
    source_text: Option<&'a str>,
    span: Span,
) -> Option<OutputExpression<'a>> {
    let source = source_text?;
    let raw = &source[span.start as usize..span.end as usize];
    let js = strip_expression_types(raw);
    Some(OutputExpression::RawSource(Box::new_in(
        RawSourceExpr { source: Ident::from(allocator.alloc_str(&js)), source_span: None },
        allocator,
    )))
}

/// Strip TypeScript type annotations from an expression source string.
///
/// Fast path: tries parsing as ESM JavaScript first. If the expression is
/// already valid JS (no type annotations), returns it as-is without running
/// the heavier semantic/transform/codegen pipeline.
///
/// Slow path: if JS parsing fails (likely due to TS syntax), wraps the
/// expression, parses as TypeScript module, strips types via transformer,
/// and codegens to JavaScript.
fn strip_expression_types(expr_source: &str) -> String {
    use std::path::Path;

    // Fast path: try parsing as JS module. If it succeeds, the expression
    // is already valid JavaScript — return as-is without transformation.
    {
        let allocator = oxc_allocator::Allocator::default();
        let wrapped = format!("0,({expr_source})");
        let source_type = oxc_span::SourceType::mjs();
        let parser_ret = oxc_parser::Parser::new(&allocator, &wrapped, source_type).parse();
        if !parser_ret.panicked && parser_ret.errors.is_empty() {
            return expr_source.to_string();
        }
    }

    // Slow path: expression contains TypeScript syntax — run full pipeline.
    let allocator = oxc_allocator::Allocator::default();
    let wrapped = format!("0,({expr_source})");
    // Use module TypeScript so import.meta and ESM syntax are valid.
    let source_type = oxc_span::SourceType::ts().with_module(true);
    let parser_ret = oxc_parser::Parser::new(&allocator, &wrapped, source_type).parse();

    if parser_ret.panicked {
        return expr_source.to_string();
    }

    let mut program = parser_ret.program;
    let semantic_ret =
        oxc_semantic::SemanticBuilder::new().with_excess_capacity(2.0).build(&program);

    let transform_options = oxc_transformer::TransformOptions {
        typescript: oxc_transformer::TypeScriptOptions::default(),
        ..Default::default()
    };
    let transformer =
        oxc_transformer::Transformer::new(&allocator, Path::new("_.mts"), &transform_options);
    transformer.build_with_scoping(semantic_ret.semantic.into_scoping(), &mut program);

    let codegen_ret = oxc_codegen::Codegen::new().with_source_text(&wrapped).build(&program);

    // Strip the wrapper: codegen produces "0, (expr);\n" → extract "expr"
    let code = codegen_ret.code.trim_end();
    if let Some(rest) = code.strip_prefix("0, (").or_else(|| code.strip_prefix("0,(")) {
        if let Some(inner) = rest.strip_suffix(");") {
            return inner.to_string();
        }
    }

    // Fallback: return original
    expr_source.to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn parse_expression<'a>(allocator: &'a Allocator, source: &'a str) -> Expression<'a> {
        let source_type = SourceType::mjs();
        let parser = Parser::new(allocator, source, source_type);
        parser.parse_expression().unwrap_or_else(|_| panic!("Failed to parse expression: {source}"))
    }

    #[test]
    fn test_convert_string_literal() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, r#""hello""#);
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::Literal(lit)) = result {
            assert!(matches!(lit.value, LiteralValue::String(_)));
        } else {
            panic!("Expected Literal expression");
        }
    }

    #[test]
    fn test_convert_number_literal() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "42");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::Literal(lit)) = result {
            if let LiteralValue::Number(n) = lit.value {
                assert!((n - 42.0).abs() < f64::EPSILON);
            } else {
                panic!("Expected Number literal");
            }
        } else {
            panic!("Expected Literal expression");
        }
    }

    #[test]
    fn test_convert_identifier() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "myVar");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::ReadVar(var)) = result {
            assert_eq!(var.name.as_str(), "myVar");
        } else {
            panic!("Expected ReadVar expression");
        }
    }

    #[test]
    fn test_convert_array_expression() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "[1, 2, 3]");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::LiteralArray(arr)) = result {
            assert_eq!(arr.entries.len(), 3);
        } else {
            panic!("Expected LiteralArray expression");
        }
    }

    #[test]
    fn test_convert_object_expression() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "{ a: 1, b: 2 }");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::LiteralMap(map)) = result {
            assert_eq!(map.entries.len(), 2);
        } else {
            panic!("Expected LiteralMap expression");
        }
    }

    #[test]
    fn test_convert_call_expression() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "foo(1, 2)");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::InvokeFunction(call)) = result {
            assert_eq!(call.args.len(), 2);
        } else {
            panic!("Expected InvokeFunction expression");
        }
    }

    #[test]
    fn test_convert_member_expression() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "obj.prop");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::ReadProp(prop)) = result {
            assert_eq!(prop.name.as_str(), "prop");
        } else {
            panic!("Expected ReadProp expression");
        }
    }

    #[test]
    fn test_convert_spread_in_array() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "[...arr, 1, 2]");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::LiteralArray(arr)) = result {
            assert_eq!(arr.entries.len(), 3);
            // First element should be a SpreadElement
            assert!(
                matches!(arr.entries.first(), Some(OutputExpression::SpreadElement(_))),
                "Expected first element to be SpreadElement"
            );
            // Verify the spread contains the correct identifier
            if let Some(OutputExpression::SpreadElement(spread)) = arr.entries.first() {
                assert!(
                    matches!(spread.expr.as_ref(), OutputExpression::ReadVar(v) if v.name.as_str() == "arr"),
                    "Expected spread to contain 'arr' identifier"
                );
            }
        } else {
            panic!("Expected LiteralArray expression");
        }
    }

    #[test]
    fn test_convert_multiple_spreads_in_array() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "[...a, ...b, c]");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::LiteralArray(arr)) = result {
            assert_eq!(arr.entries.len(), 3);
            // First two elements should be SpreadElements
            assert!(matches!(arr.entries.get(0), Some(OutputExpression::SpreadElement(_))));
            assert!(matches!(arr.entries.get(1), Some(OutputExpression::SpreadElement(_))));
            // Third element should be a regular identifier
            assert!(matches!(arr.entries.get(2), Some(OutputExpression::ReadVar(_))));
        } else {
            panic!("Expected LiteralArray expression");
        }
    }

    #[test]
    fn test_convert_optional_chaining_property() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "obj?.prop");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::ReadProp(prop)) = result {
            assert_eq!(prop.name.as_str(), "prop");
            assert!(prop.optional, "Expected optional to be true for ?.");
        } else {
            panic!("Expected ReadProp expression");
        }
    }

    #[test]
    fn test_convert_optional_chaining_computed() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "obj?.[key]");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::ReadKey(key)) = result {
            assert!(key.optional, "Expected optional to be true for ?.[");
        } else {
            panic!("Expected ReadKey expression");
        }
    }

    #[test]
    fn test_convert_optional_chaining_call() {
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "fn?.()");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some());
        if let Some(OutputExpression::InvokeFunction(call)) = result {
            assert!(call.optional, "Expected optional to be true for ?.()");
        } else {
            panic!("Expected InvokeFunction expression");
        }
    }

    #[test]
    fn test_convert_optional_chaining_chain() {
        // Test a longer optional chain like: val?.trim().toLowerCase()
        let allocator = Allocator::default();
        let expr = parse_expression(&allocator, "val?.trim().toLowerCase()");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some(), "Failed to convert optional chain expression");

        // The expression structure is:
        // InvokeFunction { fn_expr: ReadProp(toLowerCase), args: [] }
        // where ReadProp.receiver is InvokeFunction { fn_expr: ReadProp(trim, optional=true), args: [] }
        // where ReadProp.receiver is ReadVar(val)
        if let Some(OutputExpression::InvokeFunction(_)) = result {
            // Successfully converted the complex optional chain
        } else {
            panic!("Expected InvokeFunction expression for val?.trim().toLowerCase()");
        }
    }

    #[test]
    fn test_convert_arrow_with_optional_chain() {
        // Test the specific case from the issue: (val: string) => val?.trim().toLowerCase()
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let parser = Parser::new(&allocator, "(val) => val?.trim().toLowerCase()", source_type);
        let expr = parser.parse_expression().expect("Failed to parse expression");
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_some(), "Failed to convert arrow function with optional chain");
    }

    #[test]
    fn test_block_body_arrow_with_unsupported_stmts_without_source_text() {
        // Without source_text, a block-body arrow with unsupported statements returns None
        let allocator = Allocator::default();
        let source = "() => { const x = 1; return x; }";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, None);
        // Without source_text, conversion fails (returns None) because
        // the const declaration cannot be represented
        assert!(result.is_none(), "Should return None without source_text for unsupported stmts");
    }

    #[test]
    fn test_block_body_arrow_with_unsupported_stmts_with_source_text() {
        // With source_text, a block-body arrow with unsupported statements falls back to RawSource
        let allocator = Allocator::default();
        let source = "() => { const x = 1; return x; }";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, Some(source));
        assert!(result.is_some(), "Should succeed with source_text");
        if let Some(OutputExpression::RawSource(raw)) = result {
            let raw_str = raw.source.as_str();
            assert!(
                raw_str.contains("const x = 1") && raw_str.contains("return x"),
                "Should preserve the arrow function body. Got: {raw_str}"
            );
        } else {
            panic!("Expected RawSource expression, got {result:?}");
        }
    }

    #[test]
    fn test_block_body_arrow_with_if_statement() {
        // The exact case from the issue: useFactory with if/const
        let allocator = Allocator::default();
        let source = "() => { const config = inject(AppConfig); if (config.useMock) { return new MockService(); } return new RealService(config); }";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, Some(source));
        assert!(result.is_some(), "Should succeed with source_text for complex arrow");
        assert!(
            matches!(result, Some(OutputExpression::RawSource(_))),
            "Should be RawSource for arrow with if statement"
        );
    }

    #[test]
    fn test_block_body_arrow_with_only_return_still_converts() {
        // A block-body arrow with only return and expression statements should still
        // convert normally (not fall back to RawSource)
        let allocator = Allocator::default();
        let source = "() => { return 42; }";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, Some(source));
        assert!(result.is_some());
        // Should NOT be RawSource since return statements are supported
        assert!(
            matches!(result, Some(OutputExpression::ArrowFunction(_))),
            "Should be ArrowFunction, not RawSource, for simple block body"
        );
    }

    #[test]
    fn test_function_expression_falls_back_to_raw_source() {
        // Function expressions should fall back to RawSource
        let allocator = Allocator::default();
        let source = "function() { return 42; }";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, Some(source));
        assert!(result.is_some(), "Should succeed with source_text for function expression");
        if let Some(OutputExpression::RawSource(raw)) = result {
            let raw_str = raw.source.as_str();
            assert!(
                raw_str.contains("function()") && raw_str.contains("return 42"),
                "Should preserve function expression. Got: {raw_str}"
            );
        } else {
            panic!("Expected RawSource expression for function expression");
        }
    }

    #[test]
    fn test_function_expression_without_source_text_returns_none() {
        // Function expressions without source_text return None
        let allocator = Allocator::default();
        let source = "function() { return 42; }";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, None);
        assert!(result.is_none(), "Should return None without source_text");
    }

    #[test]
    fn test_expression_body_arrow_unaffected_by_source_text() {
        // Expression-body arrows should still work normally
        let allocator = Allocator::default();
        let source = "() => 42";
        let expr = parse_expression(&allocator, source);
        let result = convert_oxc_expression(&allocator, &expr, Some(source));
        assert!(result.is_some());
        assert!(
            matches!(result, Some(OutputExpression::ArrowFunction(_))),
            "Expression-body arrow should still be ArrowFunction"
        );
    }

    #[test]
    fn test_raw_source_strips_typescript_types() {
        // RawSource should strip TypeScript type annotations to produce valid JS
        let allocator = Allocator::default();
        // Parse as TypeScript (not mjs) to include type annotations
        let source_type = SourceType::ts();
        let source =
            "(dep: SomeType) => { const x: MyType = dep.getValue(); return new Service(x); }";
        let parser = Parser::new(&allocator, source, source_type);
        let expr = parser.parse_expression().expect("Failed to parse expression");
        let result = convert_oxc_expression(&allocator, &expr, Some(source));
        assert!(result.is_some(), "Should succeed with source_text for typed arrow");
        if let Some(OutputExpression::RawSource(raw)) = result {
            let raw_str = raw.source.as_str();
            // Type annotations should be stripped
            assert!(
                !raw_str.contains(": SomeType") && !raw_str.contains(": MyType"),
                "TypeScript type annotations should be stripped. Got: {raw_str}"
            );
            // But the actual code should be preserved
            assert!(
                raw_str.contains("dep.getValue()") && raw_str.contains("return new Service(x)"),
                "JavaScript code should be preserved. Got: {raw_str}"
            );
        } else {
            panic!("Expected RawSource expression, got {result:?}");
        }
    }

    #[test]
    fn test_strip_expression_types_basic() {
        let result = strip_expression_types("(x: number) => x + 1");
        assert!(!result.contains(": number"), "Should strip type annotation. Got: {result}");
        assert!(result.contains("=> x + 1"), "Should preserve expression. Got: {result}");
    }
}

//! Angular AST expression to Output AST conversion.

use std::cell::RefCell;

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_str::Ident;

use crate::ast::expression::AngularExpression;
use crate::ir::ops::XrefId;
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, ConditionalExpr,
    FnParam, InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, NotExpr, OutputExpression, ParenthesizedExpr, ReadKeyExpr, ReadPropExpr,
    ReadVarExpr, RegularExpressionLiteralExpr, SpreadElementExpr, TaggedTemplateLiteralExpr,
    TemplateLiteralElement, TemplateLiteralExpr, TypeofExpr, UnaryOperator, UnaryOperatorExpr,
    VoidExpr,
};

/// Context for safe navigation expression conversion, providing temp variable allocation.
struct SafeConversionContext<'a> {
    allocator: &'a oxc_allocator::Allocator,
    /// Counter for generating unique temporary variable names.
    /// Uses format `tmp_{op_index}_{count}` where op_index is 0 for angular_expression conversion.
    temp_count: RefCell<u32>,
}

impl<'a> SafeConversionContext<'a> {
    fn new(allocator: &'a oxc_allocator::Allocator) -> Self {
        Self { allocator, temp_count: RefCell::new(0) }
    }

    /// Allocates a new temporary variable name.
    fn allocate_temp_name(&self) -> Ident<'a> {
        let mut count = self.temp_count.borrow_mut();
        // Use op_index 0 since angular_expression conversion happens during reify
        // The actual op_index will be adjusted by temporary_variables phase if needed
        let name = format!("tmp_0_{}", *count);
        *count += 1;
        Ident::from(self.allocator.alloc_str(&name))
    }
}

/// Checks if an AST expression requires a temporary variable to avoid re-evaluation.
///
/// Returns true for expressions with side effects:
/// - Function calls/invocations
/// - Safe function calls
/// - Pipe bindings
/// - Array and object literals
/// - Parenthesized expressions containing the above
///
/// Ported from Angular's `needsTemporaryInSafeAccess` in `expand_safe_reads.ts`.
fn needs_temporary_in_safe_access(expr: &AngularExpression<'_>) -> bool {
    match expr {
        // Function calls always need temporaries
        AngularExpression::Call(_) => true,
        AngularExpression::SafeCall(_) => true,
        // Pipe bindings need temporaries
        AngularExpression::BindingPipe(_) => true,
        // Array and object literals need temporaries
        AngularExpression::LiteralArray(_) => true,
        AngularExpression::LiteralMap(_) => true,
        // Parenthesized expressions need to check their inner expression
        AngularExpression::ParenthesizedExpression(paren) => {
            needs_temporary_in_safe_access(&paren.expression)
        }
        // Unary operators need to check operand
        AngularExpression::Unary(unary) => needs_temporary_in_safe_access(&unary.expr),
        AngularExpression::PrefixNot(not) => needs_temporary_in_safe_access(&not.expression),
        // Binary operators need to check both operands
        AngularExpression::Binary(binary) => {
            needs_temporary_in_safe_access(&binary.left)
                || needs_temporary_in_safe_access(&binary.right)
        }
        // Conditional needs to check all branches
        AngularExpression::Conditional(cond) => {
            needs_temporary_in_safe_access(&cond.condition)
                || needs_temporary_in_safe_access(&cond.true_exp)
                || needs_temporary_in_safe_access(&cond.false_exp)
        }
        // Property reads need to check their receiver
        AngularExpression::PropertyRead(prop) => needs_temporary_in_safe_access(&prop.receiver),
        AngularExpression::SafePropertyRead(prop) => needs_temporary_in_safe_access(&prop.receiver),
        // Keyed reads need to check receiver and key
        AngularExpression::KeyedRead(keyed) => {
            needs_temporary_in_safe_access(&keyed.receiver)
                || needs_temporary_in_safe_access(&keyed.key)
        }
        AngularExpression::SafeKeyedRead(keyed) => {
            needs_temporary_in_safe_access(&keyed.receiver)
                || needs_temporary_in_safe_access(&keyed.key)
        }
        // Simple expressions don't need temporaries
        _ => false,
    }
}

/// Converts an Angular AST expression to an output expression.
pub fn convert_angular_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &AngularExpression<'a>,
    root_xref: XrefId,
) -> OutputExpression<'a> {
    let ctx = SafeConversionContext::new(allocator);
    convert_angular_expression_with_ctx(allocator, expr, root_xref, &ctx)
}

/// Internal function that does the actual conversion with context.
fn convert_angular_expression_with_ctx<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &AngularExpression<'a>,
    root_xref: XrefId,
    ctx: &SafeConversionContext<'a>,
) -> OutputExpression<'a> {
    match expr {
        AngularExpression::LiteralPrimitive(lit) => {
            let value = match &lit.value {
                crate::ast::expression::LiteralValue::String(s) => LiteralValue::String(s.clone()),
                crate::ast::expression::LiteralValue::Number(n) => LiteralValue::Number(*n),
                crate::ast::expression::LiteralValue::Boolean(b) => LiteralValue::Boolean(*b),
                crate::ast::expression::LiteralValue::Null => LiteralValue::Null,
                crate::ast::expression::LiteralValue::Undefined => LiteralValue::Undefined,
            };
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value, source_span: Some(lit.source_span.to_span()) },
                allocator,
            ))
        }

        AngularExpression::ImplicitReceiver(ir) => {
            // Implicit receiver becomes ctx
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr {
                    name: Ident::from("ctx"),
                    source_span: Some(ir.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::ThisReceiver(tr) => {
            // This receiver becomes ctx
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr {
                    name: Ident::from("ctx"),
                    source_span: Some(tr.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::PropertyRead(prop) => {
            // Special case: $event on implicit receiver becomes a local variable reference.
            // This is used in event handlers where $event is the event parameter.
            if prop.name.as_str() == "$event" {
                if matches!(&prop.receiver, AngularExpression::ImplicitReceiver(_)) {
                    return OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr {
                            name: Ident::from("$event"),
                            source_span: Some(prop.source_span.to_span()),
                        },
                        allocator,
                    ));
                }
            }
            let receiver =
                convert_angular_expression_with_ctx(allocator, &prop.receiver, root_xref, ctx);
            OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(receiver, allocator),
                    name: prop.name.clone(),
                    optional: false,
                    source_span: Some(prop.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::KeyedRead(keyed) => {
            let receiver =
                convert_angular_expression_with_ctx(allocator, &keyed.receiver, root_xref, ctx);
            let key = convert_angular_expression_with_ctx(allocator, &keyed.key, root_xref, ctx);
            OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver, allocator),
                    index: Box::new_in(key, allocator),
                    optional: false,
                    source_span: Some(keyed.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::Call(call) => {
            let receiver =
                convert_angular_expression_with_ctx(allocator, &call.receiver, root_xref, ctx);
            let mut args = OxcVec::new_in(allocator);
            for arg in call.args.iter() {
                args.push(convert_angular_expression_with_ctx(allocator, arg, root_xref, ctx));
            }
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(receiver, allocator),
                    args,
                    pure: false,
                    optional: false,
                    source_span: Some(call.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::Binary(bin) => {
            let left = convert_angular_expression_with_ctx(allocator, &bin.left, root_xref, ctx);
            let right = convert_angular_expression_with_ctx(allocator, &bin.right, root_xref, ctx);
            let op = match bin.operation.as_str() {
                "+" => BinaryOperator::Plus,
                "-" => BinaryOperator::Minus,
                "*" => BinaryOperator::Multiply,
                "/" => BinaryOperator::Divide,
                "%" => BinaryOperator::Modulo,
                "**" => BinaryOperator::Exponentiation,
                "==" => BinaryOperator::Equals,
                "!=" => BinaryOperator::NotEquals,
                "===" => BinaryOperator::Identical,
                "!==" => BinaryOperator::NotIdentical,
                "<" => BinaryOperator::Lower,
                ">" => BinaryOperator::Bigger,
                "<=" => BinaryOperator::LowerEquals,
                ">=" => BinaryOperator::BiggerEquals,
                "&&" => BinaryOperator::And,
                "||" => BinaryOperator::Or,
                "??" => BinaryOperator::NullishCoalesce,
                "in" => BinaryOperator::In,
                // Assignment operators (only valid in event handlers)
                "=" => BinaryOperator::Assign,
                "+=" => BinaryOperator::AdditionAssignment,
                "-=" => BinaryOperator::SubtractionAssignment,
                "*=" => BinaryOperator::MultiplicationAssignment,
                "/=" => BinaryOperator::DivisionAssignment,
                "%=" => BinaryOperator::RemainderAssignment,
                "**=" => BinaryOperator::ExponentiationAssignment,
                "&&=" => BinaryOperator::AndAssignment,
                "||=" => BinaryOperator::OrAssignment,
                "??=" => BinaryOperator::NullishCoalesceAssignment,
                _ => BinaryOperator::Plus,
            };
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: op,
                    lhs: Box::new_in(left, allocator),
                    rhs: Box::new_in(right, allocator),
                    source_span: Some(bin.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::Unary(unary) => {
            let operand =
                convert_angular_expression_with_ctx(allocator, &unary.expr, root_xref, ctx);
            let op = match unary.operator {
                crate::ast::expression::UnaryOperator::Minus => UnaryOperator::Minus,
                crate::ast::expression::UnaryOperator::Plus => UnaryOperator::Plus,
            };
            OutputExpression::UnaryOperator(Box::new_in(
                UnaryOperatorExpr {
                    operator: op,
                    expr: Box::new_in(operand, allocator),
                    parens: false,
                    source_span: Some(unary.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::PrefixNot(not) => {
            let operand =
                convert_angular_expression_with_ctx(allocator, &not.expression, root_xref, ctx);
            OutputExpression::Not(Box::new_in(
                NotExpr {
                    condition: Box::new_in(operand, allocator),
                    source_span: Some(not.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::Conditional(cond) => {
            let condition =
                convert_angular_expression_with_ctx(allocator, &cond.condition, root_xref, ctx);
            let true_case =
                convert_angular_expression_with_ctx(allocator, &cond.true_exp, root_xref, ctx);
            let false_case =
                convert_angular_expression_with_ctx(allocator, &cond.false_exp, root_xref, ctx);
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(condition, allocator),
                    true_case: Box::new_in(true_case, allocator),
                    false_case: Some(Box::new_in(false_case, allocator)),
                    source_span: Some(cond.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::LiteralArray(arr) => {
            let mut entries = OxcVec::new_in(allocator);
            for entry in arr.expressions.iter() {
                if let AngularExpression::SpreadElement(spread) = entry {
                    let inner = convert_angular_expression_with_ctx(
                        allocator,
                        &spread.expression,
                        root_xref,
                        ctx,
                    );
                    entries.push(OutputExpression::SpreadElement(Box::new_in(
                        SpreadElementExpr {
                            expr: Box::new_in(inner, allocator),
                            source_span: None,
                        },
                        allocator,
                    )));
                } else {
                    entries.push(convert_angular_expression_with_ctx(
                        allocator, entry, root_xref, ctx,
                    ));
                }
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: Some(arr.source_span.to_span()) },
                allocator,
            ))
        }

        AngularExpression::LiteralMap(map) => {
            use crate::ast::expression::LiteralMapKey;
            let mut entries = OxcVec::new_in(allocator);
            for (i, key) in map.keys.iter().enumerate() {
                if i < map.values.len() {
                    match key {
                        LiteralMapKey::Property(prop) => {
                            entries.push(LiteralMapEntry::new(
                                prop.key.clone(),
                                convert_angular_expression_with_ctx(
                                    allocator,
                                    &map.values[i],
                                    root_xref,
                                    ctx,
                                ),
                                prop.quoted,
                            ));
                        }
                        LiteralMapKey::Spread(_) => {
                            entries.push(LiteralMapEntry::spread(
                                convert_angular_expression_with_ctx(
                                    allocator,
                                    &map.values[i],
                                    root_xref,
                                    ctx,
                                ),
                            ));
                        }
                    }
                }
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: Some(map.source_span.to_span()) },
                allocator,
            ))
        }

        AngularExpression::Empty(empty) => OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Undefined,
                source_span: Some(empty.source_span.to_span()),
            },
            allocator,
        )),

        AngularExpression::BindingPipe(pipe) => {
            // Pipes are handled by the PipeBinding IR expression after pipe_creation phase
            // At this point, we just convert the expression through the pipe
            // In a full implementation, this would be converted earlier in the pipeline
            convert_angular_expression_with_ctx(allocator, &pipe.exp, root_xref, ctx)
        }

        AngularExpression::SafePropertyRead(safe) => {
            // Convert obj?.prop to:
            // - If receiver needs temp: ((tmp = receiver) == null ? null : tmp.prop)
            // - Otherwise: (receiver == null ? null : receiver.prop)
            let span = Some(safe.source_span.to_span());

            if needs_temporary_in_safe_access(&safe.receiver) {
                // Allocate a temp variable name
                let temp_name = ctx.allocate_temp_name();
                let temp_var_read = || {
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: temp_name.clone(), source_span: None },
                        allocator,
                    ))
                };

                // Convert the receiver once
                let receiver =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);

                // Create: (tmp = receiver)
                let assign = OutputExpression::BinaryOperator(Box::new_in(
                    BinaryOperatorExpr {
                        operator: BinaryOperator::Assign,
                        lhs: Box::new_in(temp_var_read(), allocator),
                        rhs: Box::new_in(receiver, allocator),
                        source_span: None,
                    },
                    allocator,
                ));

                // Create: ((tmp = receiver) == null ? null : tmp.prop)
                let ternary = safe_ternary_with_temp(allocator, assign, || {
                    // Expr: tmp.prop
                    OutputExpression::ReadProp(Box::new_in(
                        ReadPropExpr {
                            receiver: Box::new_in(temp_var_read(), allocator),
                            name: safe.name.clone(),
                            optional: false,
                            source_span: span,
                        },
                        allocator,
                    ))
                });

                // Wrap in parentheses for proper grouping
                OutputExpression::Parenthesized(Box::new_in(
                    ParenthesizedExpr { expr: Box::new_in(ternary, allocator), source_span: span },
                    allocator,
                ))
            } else {
                // Simple case: convert receiver twice (safe for simple expressions)
                let receiver_for_check =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);
                let receiver_for_prop =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);

                let null_check = OutputExpression::BinaryOperator(Box::new_in(
                    BinaryOperatorExpr {
                        operator: BinaryOperator::Equals,
                        lhs: Box::new_in(receiver_for_check, allocator),
                        rhs: Box::new_in(
                            OutputExpression::Literal(Box::new_in(
                                LiteralExpr { value: LiteralValue::Null, source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        source_span: None,
                    },
                    allocator,
                ));
                let prop_read = OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(receiver_for_prop, allocator),
                        name: safe.name.clone(),
                        optional: false,
                        source_span: span,
                    },
                    allocator,
                ));
                let conditional = OutputExpression::Conditional(Box::new_in(
                    ConditionalExpr {
                        condition: Box::new_in(null_check, allocator),
                        true_case: Box::new_in(
                            OutputExpression::Literal(Box::new_in(
                                LiteralExpr { value: LiteralValue::Null, source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        false_case: Some(Box::new_in(prop_read, allocator)),
                        source_span: span,
                    },
                    allocator,
                ));
                // Wrap in parentheses for correct operator precedence
                OutputExpression::Parenthesized(Box::new_in(
                    ParenthesizedExpr {
                        expr: Box::new_in(conditional, allocator),
                        source_span: span,
                    },
                    allocator,
                ))
            }
        }

        AngularExpression::SafeKeyedRead(safe) => {
            // Convert obj?.[key] to:
            // - If receiver needs temp: ((tmp = receiver) == null ? null : tmp[key])
            // - Otherwise: (receiver == null ? null : receiver[key])
            let span = Some(safe.source_span.to_span());
            let key = convert_angular_expression_with_ctx(allocator, &safe.key, root_xref, ctx);

            if needs_temporary_in_safe_access(&safe.receiver) {
                // Allocate a temp variable name
                let temp_name = ctx.allocate_temp_name();
                let temp_var_read = || {
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: temp_name.clone(), source_span: None },
                        allocator,
                    ))
                };

                // Convert the receiver once
                let receiver =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);

                // Create: (tmp = receiver)
                let assign = OutputExpression::BinaryOperator(Box::new_in(
                    BinaryOperatorExpr {
                        operator: BinaryOperator::Assign,
                        lhs: Box::new_in(temp_var_read(), allocator),
                        rhs: Box::new_in(receiver, allocator),
                        source_span: None,
                    },
                    allocator,
                ));

                // Create: ((tmp = receiver) == null ? null : tmp[key])
                let ternary = safe_ternary_with_temp(allocator, assign, || {
                    // Expr: tmp[key]
                    OutputExpression::ReadKey(Box::new_in(
                        ReadKeyExpr {
                            receiver: Box::new_in(temp_var_read(), allocator),
                            index: Box::new_in(key.clone_in(allocator), allocator),
                            optional: false,
                            source_span: span,
                        },
                        allocator,
                    ))
                });

                // Wrap in parentheses for proper grouping
                OutputExpression::Parenthesized(Box::new_in(
                    ParenthesizedExpr { expr: Box::new_in(ternary, allocator), source_span: span },
                    allocator,
                ))
            } else {
                // Simple case: convert receiver twice (safe for simple expressions)
                let receiver_for_check =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);
                let receiver_for_access =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);

                let conditional = OutputExpression::Conditional(Box::new_in(
                    ConditionalExpr {
                        condition: Box::new_in(
                            OutputExpression::BinaryOperator(Box::new_in(
                                BinaryOperatorExpr {
                                    operator: BinaryOperator::Equals,
                                    lhs: Box::new_in(receiver_for_check, allocator),
                                    rhs: Box::new_in(
                                        OutputExpression::Literal(Box::new_in(
                                            LiteralExpr {
                                                value: LiteralValue::Null,
                                                source_span: None,
                                            },
                                            allocator,
                                        )),
                                        allocator,
                                    ),
                                    source_span: None,
                                },
                                allocator,
                            )),
                            allocator,
                        ),
                        true_case: Box::new_in(
                            OutputExpression::Literal(Box::new_in(
                                LiteralExpr { value: LiteralValue::Null, source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        false_case: Some(Box::new_in(
                            OutputExpression::ReadKey(Box::new_in(
                                ReadKeyExpr {
                                    receiver: Box::new_in(receiver_for_access, allocator),
                                    index: Box::new_in(key, allocator),
                                    optional: false,
                                    source_span: span,
                                },
                                allocator,
                            )),
                            allocator,
                        )),
                        source_span: span,
                    },
                    allocator,
                ));
                // Wrap in parentheses for correct operator precedence
                OutputExpression::Parenthesized(Box::new_in(
                    ParenthesizedExpr {
                        expr: Box::new_in(conditional, allocator),
                        source_span: span,
                    },
                    allocator,
                ))
            }
        }

        AngularExpression::SafeCall(safe) => {
            // Convert fn?.() to:
            // - If receiver needs temp: ((tmp = receiver) == null ? null : tmp())
            // - Otherwise: (receiver == null ? null : receiver())
            let span = Some(safe.source_span.to_span());

            if needs_temporary_in_safe_access(&safe.receiver) {
                // Allocate a temp variable name
                let temp_name = ctx.allocate_temp_name();
                let temp_var_read = || {
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: temp_name.clone(), source_span: None },
                        allocator,
                    ))
                };

                // Convert the receiver once
                let receiver =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);

                // Create: (tmp = receiver)
                let assign = OutputExpression::BinaryOperator(Box::new_in(
                    BinaryOperatorExpr {
                        operator: BinaryOperator::Assign,
                        lhs: Box::new_in(temp_var_read(), allocator),
                        rhs: Box::new_in(receiver, allocator),
                        source_span: None,
                    },
                    allocator,
                ));

                // Convert args for the call inside the ternary
                let mut args = OxcVec::new_in(allocator);
                for arg in safe.args.iter() {
                    args.push(convert_angular_expression_with_ctx(allocator, arg, root_xref, ctx));
                }

                // Create: ((tmp = receiver) == null ? null : tmp())
                let call = OutputExpression::InvokeFunction(Box::new_in(
                    InvokeFunctionExpr {
                        fn_expr: Box::new_in(temp_var_read(), allocator),
                        args,
                        pure: false,
                        optional: false,
                        source_span: span,
                    },
                    allocator,
                ));
                let ternary = safe_ternary_with_temp(allocator, assign, || call);

                // Wrap in parentheses for proper grouping
                OutputExpression::Parenthesized(Box::new_in(
                    ParenthesizedExpr { expr: Box::new_in(ternary, allocator), source_span: span },
                    allocator,
                ))
            } else {
                // Simple case: convert receiver twice (safe for simple expressions)
                let receiver_for_check =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);
                let receiver_for_call =
                    convert_angular_expression_with_ctx(allocator, &safe.receiver, root_xref, ctx);

                // Convert args
                let mut args = OxcVec::new_in(allocator);
                for arg in safe.args.iter() {
                    args.push(convert_angular_expression_with_ctx(allocator, arg, root_xref, ctx));
                }

                let call = OutputExpression::InvokeFunction(Box::new_in(
                    InvokeFunctionExpr {
                        fn_expr: Box::new_in(receiver_for_call, allocator),
                        args,
                        pure: false,
                        optional: false,
                        source_span: span,
                    },
                    allocator,
                ));
                let conditional = OutputExpression::Conditional(Box::new_in(
                    ConditionalExpr {
                        condition: Box::new_in(
                            OutputExpression::BinaryOperator(Box::new_in(
                                BinaryOperatorExpr {
                                    operator: BinaryOperator::Equals,
                                    lhs: Box::new_in(receiver_for_check, allocator),
                                    rhs: Box::new_in(
                                        OutputExpression::Literal(Box::new_in(
                                            LiteralExpr {
                                                value: LiteralValue::Null,
                                                source_span: None,
                                            },
                                            allocator,
                                        )),
                                        allocator,
                                    ),
                                    source_span: None,
                                },
                                allocator,
                            )),
                            allocator,
                        ),
                        true_case: Box::new_in(
                            OutputExpression::Literal(Box::new_in(
                                LiteralExpr { value: LiteralValue::Null, source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        false_case: Some(Box::new_in(call, allocator)),
                        source_span: span,
                    },
                    allocator,
                ));
                // Wrap in parentheses for correct operator precedence
                OutputExpression::Parenthesized(Box::new_in(
                    ParenthesizedExpr {
                        expr: Box::new_in(conditional, allocator),
                        source_span: span,
                    },
                    allocator,
                ))
            }
        }

        AngularExpression::NonNullAssert(nna) => {
            // Non-null assertion is a TypeScript type-only construct
            // At runtime, just return the expression with the NNA span
            // Note: The inner expression's span will be preserved via recursion
            convert_angular_expression_with_ctx(allocator, &nna.expression, root_xref, ctx)
        }

        AngularExpression::Chain(chain) => {
            // Chain of expressions - return the last one
            // In Angular templates, chains like `a; b; c` evaluate all but return last
            if let Some(last) = chain.expressions.last() {
                convert_angular_expression_with_ctx(allocator, last, root_xref, ctx)
            } else {
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::Undefined,
                        source_span: Some(chain.source_span.to_span()),
                    },
                    allocator,
                ))
            }
        }

        AngularExpression::Interpolation(interp) => {
            // For interpolation, if there's exactly one expression with no surrounding strings,
            // just return that expression. Otherwise, we'd need to use textInterpolateN.
            // For now, just return the first expression if available.
            if let Some(first) = interp.expressions.first() {
                convert_angular_expression_with_ctx(allocator, first, root_xref, ctx)
            } else {
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from("")),
                        source_span: Some(interp.source_span.to_span()),
                    },
                    allocator,
                ))
            }
        }

        AngularExpression::TypeofExpression(te) => {
            let operand =
                convert_angular_expression_with_ctx(allocator, &te.expression, root_xref, ctx);
            OutputExpression::Typeof(Box::new_in(
                TypeofExpr {
                    expr: Box::new_in(operand, allocator),
                    source_span: Some(te.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::VoidExpression(ve) => {
            let operand =
                convert_angular_expression_with_ctx(allocator, &ve.expression, root_xref, ctx);
            OutputExpression::Void(Box::new_in(
                VoidExpr {
                    expr: Box::new_in(operand, allocator),
                    source_span: Some(ve.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::ParenthesizedExpression(pe) => {
            // Just unwrap parentheses and convert inner expression
            // The inner expression's span will be preserved via recursion
            convert_angular_expression_with_ctx(allocator, &pe.expression, root_xref, ctx)
        }

        AngularExpression::TemplateLiteral(tl) => {
            // Convert template literal: `text ${expr} more text`
            let mut elements = OxcVec::new_in(allocator);
            let mut expressions = OxcVec::new_in(allocator);

            for elem in tl.elements.iter() {
                elements.push(TemplateLiteralElement {
                    text: elem.text.clone(),
                    raw_text: elem.text.clone(), // Use same text for raw
                    source_span: Some(elem.source_span.to_span()),
                });
            }

            for expr in tl.expressions.iter() {
                expressions
                    .push(convert_angular_expression_with_ctx(allocator, expr, root_xref, ctx));
            }

            OutputExpression::TemplateLiteral(Box::new_in(
                TemplateLiteralExpr {
                    elements,
                    expressions,
                    source_span: Some(tl.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::TaggedTemplateLiteral(ttl) => {
            // Convert tagged template literal: tag`text ${expr}`
            let tag = convert_angular_expression_with_ctx(allocator, &ttl.tag, root_xref, ctx);

            let mut elements = OxcVec::new_in(allocator);
            let mut expressions = OxcVec::new_in(allocator);

            for elem in ttl.template.elements.iter() {
                elements.push(TemplateLiteralElement {
                    text: elem.text.clone(),
                    raw_text: elem.text.clone(),
                    source_span: Some(elem.source_span.to_span()),
                });
            }

            for expr in ttl.template.expressions.iter() {
                expressions
                    .push(convert_angular_expression_with_ctx(allocator, expr, root_xref, ctx));
            }

            let template = TemplateLiteralExpr {
                elements,
                expressions,
                source_span: Some(ttl.template.source_span.to_span()),
            };

            OutputExpression::TaggedTemplateLiteral(Box::new_in(
                TaggedTemplateLiteralExpr {
                    tag: Box::new_in(tag, allocator),
                    template: Box::new_in(template, allocator),
                    source_span: Some(ttl.source_span.to_span()),
                },
                allocator,
            ))
        }

        AngularExpression::RegularExpressionLiteral(re) => {
            // Convert regex literal: /pattern/flags
            OutputExpression::RegularExpressionLiteral(Box::new_in(
                RegularExpressionLiteralExpr {
                    body: re.body.clone(),
                    flags: re.flags.clone(),
                    source_span: Some(re.source_span.to_span()),
                },
                allocator,
            ))
        }

        // Spread element - convert the inner expression
        AngularExpression::SpreadElement(spread) => {
            convert_angular_expression_with_ctx(allocator, &spread.expression, root_xref, ctx)
        }

        // Arrow function - convert to OutputExpression::ArrowFunction
        AngularExpression::ArrowFunction(arrow) => {
            let mut params = OxcVec::new_in(allocator);
            for p in arrow.parameters.iter() {
                params.push(FnParam { name: p.name.clone() });
            }
            let body = convert_angular_expression_with_ctx(allocator, &arrow.body, root_xref, ctx);

            // Use expression body for arrow functions
            OutputExpression::ArrowFunction(Box::new_in(
                ArrowFunctionExpr {
                    params,
                    body: ArrowFunctionBody::Expression(Box::new_in(body, allocator)),
                    source_span: Some(arrow.source_span.to_span()),
                },
                allocator,
            ))
        }
    }
}

/// Creates a safe ternary expression with temporary variable:
/// `(guard == null ? null : expr)`
///
/// This is used when the guard expression has already been assigned to a temp variable,
/// so the guard passed here is the assignment expression `(tmp = originalGuard)`.
fn safe_ternary_with_temp<'a, F>(
    allocator: &'a oxc_allocator::Allocator,
    guard: OutputExpression<'a>,
    make_expr: F,
) -> OutputExpression<'a>
where
    F: FnOnce() -> OutputExpression<'a>,
{
    // Create: guard == null ? null : expr
    OutputExpression::Conditional(Box::new_in(
        ConditionalExpr {
            condition: Box::new_in(
                OutputExpression::BinaryOperator(Box::new_in(
                    BinaryOperatorExpr {
                        operator: BinaryOperator::Equals,
                        lhs: Box::new_in(guard, allocator),
                        rhs: Box::new_in(
                            OutputExpression::Literal(Box::new_in(
                                LiteralExpr { value: LiteralValue::Null, source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        source_span: None,
                    },
                    allocator,
                )),
                allocator,
            ),
            true_case: Box::new_in(
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Null, source_span: None },
                    allocator,
                )),
                allocator,
            ),
            false_case: Some(Box::new_in(make_expr(), allocator)),
            source_span: None,
        },
        allocator,
    ))
}

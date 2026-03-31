//! IR expression to Output AST conversion.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_span::Ident;

use crate::ast::expression::AngularExpression;
use crate::ir::expression::{IrExpression, TwoWayBindingSetExpr};
use crate::ir::ops::XrefId;
use crate::output::ast::{
    BinaryOperator, BinaryOperatorExpr, ConditionalExpr, InvokeFunctionExpr, LiteralArrayExpr,
    LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue, OutputExpression,
    ParenthesizedExpr, ReadKeyExpr, ReadPropExpr, ReadVarExpr,
};
use crate::pipeline::expression_store::ExpressionStore;
use crate::r3::{Identifiers, get_pipe_bind_instruction, get_pure_function_instruction};

use super::angular_expression::convert_angular_expression;
use super::utils::{create_instruction_call_expr, create_value_interpolate_expr};

/// Converts an IR expression to an output expression.
///
/// This is the key transformation that makes the generated code functional.
/// It converts IR expressions (which reference variables, context, etc.)
/// to output expressions that will be emitted as JavaScript.
pub fn convert_ir_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &IrExpression<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
) -> OutputExpression<'a> {
    match expr {
        IrExpression::ReadVariable(var) => {
            // Read a variable that was resolved during resolve_names
            // This becomes a reference to a local variable in the generated code
            // The naming phase should have set the name; if missing, use a debuggable fallback
            let var_name = match &var.name {
                Some(name) => name.clone(),
                None => {
                    // Fallback to a debuggable name using xref (should not happen after naming phase)
                    let fallback = format!("_unnamed_{}", var.xref.0);
                    let fallback_str = allocator.alloc_str(&fallback);
                    Ident::from(fallback_str)
                }
            };
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: var_name, source_span: var.source_span },
                allocator,
            ))
        }

        IrExpression::Context(ctx) => {
            // Reference to the component context
            // This becomes `ctx` in the generated code
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from("ctx"), source_span: ctx.source_span },
                allocator,
            ))
        }

        IrExpression::LexicalRead(lexical) => {
            // Special cases for function parameters or context reference
            // $event is a function parameter in event handlers
            // ctx is the context parameter itself (from resolve_contexts for conditional aliases)
            if lexical.name.as_str() == "$event" || lexical.name.as_str() == "ctx" {
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: lexical.name.clone(), source_span: lexical.source_span },
                    allocator,
                ))
            } else {
                // If we still have a lexical read at this point, it's a component property
                // This becomes ctx.propertyName
                OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(
                            OutputExpression::ReadVar(Box::new_in(
                                ReadVarExpr { name: Ident::from("ctx"), source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        name: lexical.name.clone(),
                        optional: false,
                        source_span: lexical.source_span,
                    },
                    allocator,
                ))
            }
        }

        IrExpression::Ast(ast_expr) => {
            // Convert AST expression to output expression
            convert_angular_expression(allocator, ast_expr, root_xref)
        }

        IrExpression::ExpressionRef(id) => {
            // Look up the expression in the store and convert it
            let angular_expr = expressions.get(*id);
            convert_angular_expression(allocator, angular_expr, root_xref)
        }

        IrExpression::Empty(empty) => {
            // Empty expression becomes undefined
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Undefined, source_span: empty.source_span },
                allocator,
            ))
        }

        IrExpression::NextContext(ctx) => {
            // i0.ɵɵnextContext(steps)
            let mut args = OxcVec::new_in(allocator);
            if ctx.steps > 1 {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::Number(ctx.steps as f64),
                        source_span: None,
                    },
                    allocator,
                )));
            }
            create_instruction_call_expr(allocator, Identifiers::NEXT_CONTEXT, args)
        }

        IrExpression::Reference(ref_expr) => {
            // i0.ɵɵreference(slot + 1 + offset)
            // TypeScript: ng.reference(expr.targetSlot.slot! + 1 + expr.offset)
            let mut args = OxcVec::new_in(allocator);
            if let Some(slot) = ref_expr.target_slot.slot {
                let slot_value = slot.0 as i32 + 1 + ref_expr.offset;
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::Number(slot_value as f64),
                        source_span: None,
                    },
                    allocator,
                )));
            }
            create_instruction_call_expr(allocator, Identifiers::REFERENCE, args)
        }

        IrExpression::RestoreView(rv) => {
            // i0.ɵɵrestoreView(savedView)
            let mut args = OxcVec::new_in(allocator);
            // The view should have been resolved to a variable during resolve_names phase
            match &rv.view {
                crate::ir::expression::RestoreViewTarget::Dynamic(inner_expr) => {
                    // The view was resolved to a variable reference
                    args.push(convert_ir_expression(allocator, inner_expr, expressions, root_xref));
                }
                crate::ir::expression::RestoreViewTarget::Static(_) => {
                    // Fallback: use _r if not resolved (shouldn't happen in correct flow)
                    args.push(OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Ident::from("_r"), source_span: None },
                        allocator,
                    )));
                }
            }
            create_instruction_call_expr(allocator, Identifiers::RESTORE_VIEW, args)
        }

        IrExpression::GetCurrentView(_) => {
            // i0.ɵɵgetCurrentView()
            create_instruction_call_expr(
                allocator,
                Identifiers::GET_CURRENT_VIEW,
                OxcVec::new_in(allocator),
            )
        }

        IrExpression::ResetView(rv) => {
            // i0.ɵɵresetView(expr)
            let mut args = OxcVec::new_in(allocator);
            args.push(convert_ir_expression(allocator, &rv.expr, expressions, root_xref));
            create_instruction_call_expr(allocator, Identifiers::RESET_VIEW, args)
        }

        IrExpression::PipeBinding(pipe) => {
            // i0.ɵɵpipeBind1/2/3/4/V(slot, varOffset, args...)
            let mut args = OxcVec::new_in(allocator);
            // Add pipe slot (targetSlot.slot - must always be present after slot allocation)
            let slot_value = pipe.target_slot.slot.map_or(0, |s| s.0);
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(slot_value as f64), source_span: None },
                allocator,
            )));
            // Add var offset (assigned by var_counting phase)
            let var_offset = pipe.var_offset.unwrap_or(0);
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(var_offset as f64), source_span: None },
                allocator,
            )));
            // Add pipe arguments
            for arg in pipe.args.iter() {
                args.push(convert_ir_expression(allocator, arg, expressions, root_xref));
            }
            let instruction = get_pipe_bind_instruction(pipe.args.len());
            create_instruction_call_expr(allocator, instruction, args)
        }

        IrExpression::PipeBindingVariadic(pipe) => {
            // i0.ɵɵpipeBindV(slot, varOffset, pureFunctionExpr)
            // For variadic pipes (>4 args), the args field contains a PureFunction
            // expression that wraps all the pipe arguments.
            let mut args = OxcVec::new_in(allocator);
            // Add pipe slot
            let slot_value = pipe.target_slot.slot.map_or(0, |s| s.0);
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(slot_value as f64), source_span: None },
                allocator,
            )));
            // Add var offset
            let var_offset = pipe.var_offset.unwrap_or(0);
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(var_offset as f64), source_span: None },
                allocator,
            )));
            // Add the pure function expression containing all arguments
            args.push(convert_ir_expression(allocator, &pipe.args, expressions, root_xref));
            create_instruction_call_expr(allocator, Identifiers::PIPE_BIND_V, args)
        }

        IrExpression::PureFunction(pf) => {
            // i0.ɵɵpureFunction1/2/etc(offset, fn, args...)
            // Signature: pureFunction(varOffset, fn, ...args)
            // - varOffset: LView slot for caching
            // - fn: reference to the pooled constant function (e.g., _c3)
            // - args: runtime arguments to the pure function
            //
            // For variadic (arg count > 8): pureFunctionV(offset, fn, [args...])
            // The args are wrapped in an array literal for the variadic case.
            let mut args = OxcVec::new_in(allocator);
            // Add var offset (assigned by var_counting phase)
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(pf.var_offset.unwrap_or(0) as f64),
                    source_span: None,
                },
                allocator,
            )));
            // Add function reference (the pure function constant)
            // This should always be set by the pure_function_extraction phase.
            // TypeScript throws an error here if fn is null, but we use a null
            // placeholder to avoid panicking (will cause runtime error instead).
            match &pf.fn_ref {
                Some(fn_ref) => {
                    args.push(convert_ir_expression(allocator, fn_ref, expressions, root_xref));
                }
                None => {
                    // fn_ref should have been set by pure_function_extraction phase
                    // Output null as placeholder - this will cause a runtime error
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Null, source_span: None },
                        allocator,
                    )));
                }
            }
            // Add arguments - for variadic (>8 args), wrap in array literal
            let is_variadic = pf.args.len() > 8;
            if is_variadic {
                // Variadic calling pattern: pureFunctionV(offset, fn, [args...])
                let mut array_entries = OxcVec::with_capacity_in(pf.args.len(), allocator);
                for arg in pf.args.iter() {
                    array_entries.push(convert_ir_expression(
                        allocator,
                        arg,
                        expressions,
                        root_xref,
                    ));
                }
                args.push(OutputExpression::LiteralArray(Box::new_in(
                    LiteralArrayExpr { entries: array_entries, source_span: None },
                    allocator,
                )));
            } else {
                // Constant calling pattern: pureFunction1/2/etc(offset, fn, arg1, arg2, ...)
                for arg in pf.args.iter() {
                    args.push(convert_ir_expression(allocator, arg, expressions, root_xref));
                }
            }
            let instruction = get_pure_function_instruction(pf.args.len());
            create_instruction_call_expr(allocator, instruction, args)
        }

        IrExpression::SlotLiteral(slot_lit) => {
            // Just output the slot number as a literal (-1.0 if no slot)
            let value = if let Some(slot) = slot_lit.slot.slot { slot.0 as f64 } else { -1.0 };
            OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(value),
                    source_span: slot_lit.source_span,
                },
                allocator,
            ))
        }

        IrExpression::StoreLet(store) => {
            // i0.ɵɵstoreLet(value)
            let mut args = OxcVec::new_in(allocator);
            args.push(convert_ir_expression(allocator, &store.value, expressions, root_xref));
            create_instruction_call_expr(allocator, Identifiers::STORE_LET, args)
        }

        IrExpression::ContextLetReference(ctx_let) => {
            // i0.ɵɵreadContextLet(slot)
            let mut args = OxcVec::new_in(allocator);
            if let Some(slot) = ctx_let.target_slot.slot {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(slot.0 as f64), source_span: None },
                    allocator,
                )));
            }
            create_instruction_call_expr(allocator, Identifiers::READ_CONTEXT_LET, args)
        }

        IrExpression::TrackContext(_) => {
            // Reference to `this` for track functions
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from("this"), source_span: None },
                allocator,
            ))
        }

        IrExpression::ReadTemporary(tmp) => {
            // Read a temporary variable
            let var_name = tmp.name.clone().unwrap_or_else(|| Ident::from("_tmp"));
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: var_name, source_span: None },
                allocator,
            ))
        }

        IrExpression::AssignTemporary(assign) => {
            // Assign to a temporary variable: _tmp = expr
            let var_name = assign.name.clone().unwrap_or_else(|| Ident::from("_tmp"));
            let value = convert_ir_expression(allocator, &assign.expr, expressions, root_xref);
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: var_name, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    rhs: Box::new_in(value, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        IrExpression::SafeTernary(st) => {
            // Safe ternary for null-safe access: (guard == null ? null : expr)
            // Angular uses the `== null` pattern for consistency with optional chaining.
            // The expression is wrapped in parentheses to ensure correct operator precedence
            // when used in a larger expression context.
            let guard = convert_ir_expression(allocator, &st.guard, expressions, root_xref);
            let true_case = convert_ir_expression(allocator, &st.expr, expressions, root_xref);
            // Build: guard == null ? null : expr
            let null_check = OutputExpression::BinaryOperator(Box::new_in(
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
                    false_case: Some(Box::new_in(true_case, allocator)),
                    source_span: None,
                },
                allocator,
            ));
            // Wrap in parentheses for correct precedence
            OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr { expr: Box::new_in(conditional, allocator), source_span: None },
                allocator,
            ))
        }

        IrExpression::Interpolation(interp) => {
            // For interpolation in property/attribute bindings, we need to generate
            // ɵɵinterpolate1-8 or ɵɵinterpolateV calls with interleaved strings and expressions.
            //
            // Example: [title]="Hello {{name}}, welcome!"
            // Generates: ɵɵinterpolate1("Hello ", ctx.name, ", welcome!")
            //
            // Special case: [title]="{{expr}}" (single expression, all empty strings)
            // Generates: ɵɵinterpolate(expr) - uses the simple form that just stringifies
            let expr_count = interp.expressions.len();
            let mut args = OxcVec::new_in(allocator);

            // For single expression with empty surrounding strings, use ɵɵinterpolate(expr)
            // This is the simple form that just stringifies the value without prefix/suffix.
            // Note: we use expr_count=0 to select ɵɵinterpolate instead of ɵɵinterpolate1.
            if expr_count == 1 && interp.strings.iter().all(|s| s.is_empty()) {
                args.push(convert_ir_expression(
                    allocator,
                    &interp.expressions[0],
                    expressions,
                    root_xref,
                ));
                // Pass 0 as expr_count to get ɵɵinterpolate (not ɵɵinterpolate1)
                create_value_interpolate_expr(allocator, args, 0)
            } else {
                // Build args: [s0, v0, s1, v1, s2, ...] (strings and expressions interleaved)
                for (i, ir_expr) in interp.expressions.iter().enumerate() {
                    if i < interp.strings.len() {
                        args.push(OutputExpression::Literal(Box::new_in(
                            LiteralExpr {
                                value: LiteralValue::String(interp.strings[i].clone()),
                                source_span: None,
                            },
                            allocator,
                        )));
                    }
                    args.push(convert_ir_expression(allocator, ir_expr, expressions, root_xref));
                }
                // Add trailing string if present (and not empty)
                // TypeScript drops trailing empty strings - the runtime handles it.
                if interp.strings.len() > interp.expressions.len() {
                    if let Some(trailing) = interp.strings.last() {
                        if !trailing.is_empty() {
                            args.push(OutputExpression::Literal(Box::new_in(
                                LiteralExpr {
                                    value: LiteralValue::String(trailing.clone()),
                                    source_span: None,
                                },
                                allocator,
                            )));
                        }
                    }
                }
                create_value_interpolate_expr(allocator, args, expr_count)
            }
        }

        IrExpression::TwoWayBindingSet(tbs) => {
            // Two-way binding: generate `i0.ɵɵtwoWayBindingSet(target, value) || (target = value)`
            // For ReadVariable targets, just generate `i0.ɵɵtwoWayBindingSet(target, value)`
            convert_two_way_binding_set(allocator, tbs, expressions, root_xref)
        }

        IrExpression::ConstReference(cr) => {
            // Reference to a const array index - emit as a literal number
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(cr.index as f64), source_span: None },
                allocator,
            ))
        }

        IrExpression::Binary(binary) => {
            // Convert binary expression for @for loop computed variables
            // and for expressions containing nested pipes like `a ?? (b | pipe)`
            let lhs = convert_ir_expression(allocator, &binary.lhs, expressions, root_xref);
            let rhs = convert_ir_expression(allocator, &binary.rhs, expressions, root_xref);
            let operator = convert_ir_binary_operator(binary.operator);
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(lhs, allocator),
                    rhs: Box::new_in(rhs, allocator),
                    source_span: binary.source_span,
                },
                allocator,
            ))
        }

        IrExpression::ResolvedPropertyRead(resolved) => {
            // Property read where the receiver was resolved during name resolution
            // Convert: ResolvedPropertyRead { receiver: ReadVariable(item), name: "title" }
            // To: item_i4.title
            let receiver =
                convert_ir_expression(allocator, &resolved.receiver, expressions, root_xref);
            OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(receiver, allocator),
                    name: resolved.name.clone(),
                    optional: false,
                    source_span: resolved.source_span,
                },
                allocator,
            ))
        }

        IrExpression::ResolvedBinary(resolved) => {
            // Binary expression where sub-expressions were resolved during name resolution
            // Convert: ResolvedBinary { operator: Assign, left: ResolvedPropertyRead(...), right: Ast($event) }
            // To: todo_i7.done = $event
            let left = convert_ir_expression(allocator, &resolved.left, expressions, root_xref);
            let right = convert_ir_expression(allocator, &resolved.right, expressions, root_xref);

            // Map Angular binary operator to output binary operator
            use crate::ast::expression::BinaryOperator as AngularOp;
            let operator = match resolved.operator {
                // Arithmetic
                AngularOp::Add => BinaryOperator::Plus,
                AngularOp::Subtract => BinaryOperator::Minus,
                AngularOp::Multiply => BinaryOperator::Multiply,
                AngularOp::Divide => BinaryOperator::Divide,
                AngularOp::Modulo => BinaryOperator::Modulo,
                AngularOp::Power => BinaryOperator::Exponentiation,
                // Comparison
                AngularOp::Equal => BinaryOperator::Equals,
                AngularOp::NotEqual => BinaryOperator::NotEquals,
                AngularOp::StrictEqual => BinaryOperator::Identical,
                AngularOp::StrictNotEqual => BinaryOperator::NotIdentical,
                AngularOp::LessThan => BinaryOperator::Lower,
                AngularOp::LessThanOrEqual => BinaryOperator::LowerEquals,
                AngularOp::GreaterThan => BinaryOperator::Bigger,
                AngularOp::GreaterThanOrEqual => BinaryOperator::BiggerEquals,
                AngularOp::In => BinaryOperator::In,
                AngularOp::Instanceof => BinaryOperator::Instanceof,
                // Logical
                AngularOp::And => BinaryOperator::And,
                AngularOp::Or => BinaryOperator::Or,
                AngularOp::NullishCoalescing => BinaryOperator::NullishCoalesce,
                // Assignment operators
                AngularOp::Assign => BinaryOperator::Assign,
                AngularOp::AddAssign => BinaryOperator::AdditionAssignment,
                AngularOp::SubtractAssign => BinaryOperator::SubtractionAssignment,
                AngularOp::MultiplyAssign => BinaryOperator::MultiplicationAssignment,
                AngularOp::DivideAssign => BinaryOperator::DivisionAssignment,
                AngularOp::ModuloAssign => BinaryOperator::RemainderAssignment,
                AngularOp::PowerAssign => BinaryOperator::ExponentiationAssignment,
                AngularOp::AndAssign => BinaryOperator::AndAssignment,
                AngularOp::OrAssign => BinaryOperator::OrAssignment,
                AngularOp::NullishCoalescingAssign => BinaryOperator::NullishCoalesceAssignment,
            };

            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(left, allocator),
                    rhs: Box::new_in(right, allocator),
                    source_span: resolved.source_span,
                },
                allocator,
            ))
        }

        IrExpression::ResolvedCall(resolved) => {
            // Function call where receiver and/or arguments were resolved during name resolution
            // Convert: ResolvedCall { receiver: Ast(ctx.removeTodo), args: [ReadVariable(todo_i8)] }
            // To: ctx.removeTodo(todo_i8)
            let receiver =
                convert_ir_expression(allocator, &resolved.receiver, expressions, root_xref);
            let mut args = OxcVec::new_in(allocator);
            for arg in resolved.args.iter() {
                args.push(convert_ir_expression(allocator, arg, expressions, root_xref));
            }
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(receiver, allocator),
                    args,
                    pure: false,
                    optional: false,
                    source_span: resolved.source_span,
                },
                allocator,
            ))
        }

        IrExpression::ResolvedKeyedRead(resolved) => {
            // Keyed read where the receiver was resolved during name resolution
            // Convert: ResolvedKeyedRead { receiver: ReadVariable(item), key: Ast(0) }
            // To: item_i4[0]
            let receiver =
                convert_ir_expression(allocator, &resolved.receiver, expressions, root_xref);
            let index = convert_ir_expression(allocator, &resolved.key, expressions, root_xref);
            OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver, allocator),
                    index: Box::new_in(index, allocator),
                    optional: false,
                    source_span: resolved.source_span,
                },
                allocator,
            ))
        }

        IrExpression::ResolvedSafePropertyRead(resolved) => {
            // Safe property read where the receiver was resolved during name resolution
            // Convert: ResolvedSafePropertyRead { receiver: ReadVariable(item), name: "title" }
            // This should be handled by the expand_safe_reads phase before reification,
            // but we handle it here as a fallback to avoid errors.
            // Output: receiver?.name (using conditional access pattern)
            let receiver =
                convert_ir_expression(allocator, &resolved.receiver, expressions, root_xref);
            // For now, output as a regular property read - the expand_safe_reads phase
            // should have already transformed this into a safe ternary pattern
            OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(receiver, allocator),
                    name: resolved.name.clone(),
                    optional: false,
                    source_span: resolved.source_span,
                },
                allocator,
            ))
        }

        IrExpression::OutputExpr(output) => {
            // Already an output expression - clone it
            output.clone_in(allocator)
        }

        // Safe property read that wasn't expanded by expand_safe_reads phase
        // This should not normally happen, but handle as fallback
        IrExpression::SafePropertyRead(safe) => {
            // Convert to: (receiver == null ? null : receiver.prop)
            // Note: This is a simplified fallback - the expand_safe_reads phase
            // should have already transformed this with proper temp handling
            let span = safe.source_span;
            let receiver = convert_ir_expression(allocator, &safe.receiver, expressions, root_xref);
            let receiver_clone = receiver.clone_in(allocator);
            let null_check = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Equals,
                    lhs: Box::new_in(receiver, allocator),
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
                    receiver: Box::new_in(receiver_clone, allocator),
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
                ParenthesizedExpr { expr: Box::new_in(conditional, allocator), source_span: span },
                allocator,
            ))
        }

        // Safe keyed read that wasn't expanded
        IrExpression::SafeKeyedRead(safe) => {
            // Convert to: (receiver == null ? null : receiver[key])
            let span = safe.source_span;
            let receiver = convert_ir_expression(allocator, &safe.receiver, expressions, root_xref);
            let receiver_clone = receiver.clone_in(allocator);
            let index = convert_ir_expression(allocator, &safe.index, expressions, root_xref);
            let null_check = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Equals,
                    lhs: Box::new_in(receiver, allocator),
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
            let keyed_read = OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver_clone, allocator),
                    index: Box::new_in(index, allocator),
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
                    false_case: Some(Box::new_in(keyed_read, allocator)),
                    source_span: span,
                },
                allocator,
            ));
            // Wrap in parentheses for correct operator precedence
            OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr { expr: Box::new_in(conditional, allocator), source_span: span },
                allocator,
            ))
        }

        // Safe function call that wasn't expanded
        IrExpression::SafeInvokeFunction(safe) => {
            // Convert to: (receiver == null ? null : receiver())
            let span = safe.source_span;
            let receiver = convert_ir_expression(allocator, &safe.receiver, expressions, root_xref);
            let receiver_clone = receiver.clone_in(allocator);
            let mut args = OxcVec::new_in(allocator);
            for arg in safe.args.iter() {
                args.push(convert_ir_expression(allocator, arg, expressions, root_xref));
            }
            let null_check = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Equals,
                    lhs: Box::new_in(receiver, allocator),
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
            let call = OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(receiver_clone, allocator),
                    args,
                    pure: false,
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
                    false_case: Some(Box::new_in(call, allocator)),
                    source_span: span,
                },
                allocator,
            ));
            // Wrap in parentheses for correct operator precedence
            OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr { expr: Box::new_in(conditional, allocator), source_span: span },
                allocator,
            ))
        }

        // Ternary expression - convert to conditional output expression
        IrExpression::Ternary(ternary) => {
            let condition =
                convert_ir_expression(allocator, &ternary.condition, expressions, root_xref);
            let true_case =
                convert_ir_expression(allocator, &ternary.true_expr, expressions, root_xref);
            let false_case =
                convert_ir_expression(allocator, &ternary.false_expr, expressions, root_xref);
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(condition, allocator),
                    true_case: Box::new_in(true_case, allocator),
                    false_case: Some(Box::new_in(false_case, allocator)),
                    source_span: ternary.source_span,
                },
                allocator,
            ))
        }

        IrExpression::LiteralArray(arr) => {
            let mut entries = OxcVec::with_capacity_in(arr.elements.len(), allocator);
            for elem in arr.elements.iter() {
                entries.push(convert_ir_expression(allocator, elem, expressions, root_xref));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: arr.source_span },
                allocator,
            ))
        }

        IrExpression::LiteralMap(map) => {
            let mut entries = OxcVec::with_capacity_in(map.values.len(), allocator);
            for (i, value) in map.values.iter().enumerate() {
                let key = map.keys.get(i).cloned().unwrap_or_else(|| Ident::from(""));
                let quoted = map.quoted.get(i).copied().unwrap_or(false);
                let converted_value =
                    convert_ir_expression(allocator, value, expressions, root_xref);
                entries.push(LiteralMapEntry { key, value: converted_value, quoted });
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: map.source_span },
                allocator,
            ))
        }

        IrExpression::DerivedLiteralArray(arr) => {
            let mut entries = OxcVec::with_capacity_in(arr.entries.len(), allocator);
            for entry in arr.entries.iter() {
                entries.push(convert_ir_expression(allocator, entry, expressions, root_xref));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: arr.source_span },
                allocator,
            ))
        }

        IrExpression::DerivedLiteralMap(map) => {
            let mut entries = OxcVec::with_capacity_in(map.values.len(), allocator);
            for (i, value) in map.values.iter().enumerate() {
                let key = map.keys.get(i).cloned().unwrap_or_else(|| Ident::from(""));
                let quoted = map.quoted.get(i).copied().unwrap_or(false);
                let converted_value =
                    convert_ir_expression(allocator, value, expressions, root_xref);
                entries.push(LiteralMapEntry { key, value: converted_value, quoted });
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: map.source_span },
                allocator,
            ))
        }

        // Logical NOT expression (!expr)
        IrExpression::Not(not) => {
            let expr = convert_ir_expression(allocator, &not.expr, expressions, root_xref);
            OutputExpression::Not(Box::new_in(
                crate::output::ast::NotExpr {
                    condition: Box::new_in(expr, allocator),
                    source_span: not.source_span,
                },
                allocator,
            ))
        }

        // Unary expression (+expr or -expr)
        IrExpression::Unary(unary) => {
            let expr = convert_ir_expression(allocator, &unary.expr, expressions, root_xref);
            let operator = match unary.operator {
                crate::ir::expression::IrUnaryOperator::Plus => {
                    crate::output::ast::UnaryOperator::Plus
                }
                crate::ir::expression::IrUnaryOperator::Minus => {
                    crate::output::ast::UnaryOperator::Minus
                }
            };
            OutputExpression::UnaryOperator(Box::new_in(
                crate::output::ast::UnaryOperatorExpr {
                    operator,
                    expr: Box::new_in(expr, allocator),
                    parens: false,
                    source_span: unary.source_span,
                },
                allocator,
            ))
        }

        // Typeof expression (typeof expr)
        IrExpression::Typeof(typeof_expr) => {
            let expr = convert_ir_expression(allocator, &typeof_expr.expr, expressions, root_xref);
            OutputExpression::Typeof(Box::new_in(
                crate::output::ast::TypeofExpr {
                    expr: Box::new_in(expr, allocator),
                    source_span: typeof_expr.source_span,
                },
                allocator,
            ))
        }

        // Void expression (void expr)
        IrExpression::Void(void_expr) => {
            let expr = convert_ir_expression(allocator, &void_expr.expr, expressions, root_xref);
            OutputExpression::Void(Box::new_in(
                crate::output::ast::VoidExpr {
                    expr: Box::new_in(expr, allocator),
                    source_span: void_expr.source_span,
                },
                allocator,
            ))
        }

        // Resolved template literal (template literal with resolved expressions)
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            // Convert to OutputExpression::TemplateLiteral
            let mut elements = OxcVec::new_in(allocator);
            let mut output_expressions = OxcVec::new_in(allocator);

            for elem in rtl.elements.iter() {
                elements.push(crate::output::ast::TemplateLiteralElement {
                    text: elem.text.clone(),
                    raw_text: elem.text.clone(),
                    source_span: elem.source_span,
                });
            }

            for expr in rtl.expressions.iter() {
                output_expressions.push(convert_ir_expression(
                    allocator,
                    expr,
                    expressions,
                    root_xref,
                ));
            }

            OutputExpression::TemplateLiteral(Box::new_in(
                crate::output::ast::TemplateLiteralExpr {
                    elements,
                    expressions: output_expressions,
                    source_span: rtl.source_span,
                },
                allocator,
            ))
        }

        IrExpression::Parenthesized(paren) => {
            let inner = convert_ir_expression(allocator, &paren.expr, expressions, root_xref);
            OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr {
                    expr: Box::new_in(inner, allocator),
                    source_span: paren.source_span,
                },
                allocator,
            ))
        }

        // For any remaining IR expressions, return null placeholder
        _ => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )),
    }
}

/// Converts a TwoWayBindingSet expression to the appropriate output expression.
///
/// For PropertyRead/KeyedRead targets:
///   `i0.ɵɵtwoWayBindingSet(target, value) || (target = value)`
///
/// For ReadVariable targets:
///   `i0.ɵɵtwoWayBindingSet(target, value)`
fn convert_two_way_binding_set<'a>(
    allocator: &'a oxc_allocator::Allocator,
    tbs: &TwoWayBindingSetExpr<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
) -> OutputExpression<'a> {
    let target = convert_ir_expression(allocator, &tbs.target, expressions, root_xref);
    let value = convert_ir_expression(allocator, &tbs.value, expressions, root_xref);

    // Determine if target is a settable property/keyed expression or a variable
    let is_property_or_keyed = is_settable_property_target(&tbs.target);

    if is_property_or_keyed {
        // For property/keyed targets: twoWayBindingSet(target, value) || (target = value)
        // Create the assignment: target = value
        let assignment = create_assignment(allocator, &target, &value);

        // Wrap assignment in parentheses
        let parens_assignment = OutputExpression::Parenthesized(Box::new_in(
            ParenthesizedExpr { expr: Box::new_in(assignment, allocator), source_span: None },
            allocator,
        ));

        // Create args using fresh clones for the instruction call
        let mut args = OxcVec::new_in(allocator);
        args.push(target);
        args.push(value);
        let instruction_call =
            create_instruction_call_expr(allocator, Identifiers::TWO_WAY_BINDING_SET, args);

        // Create the OR expression: instruction_call || (target = value)
        OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: BinaryOperator::Or,
                lhs: Box::new_in(instruction_call, allocator),
                rhs: Box::new_in(parens_assignment, allocator),
                source_span: None,
            },
            allocator,
        ))
    } else {
        // For variable targets: just the instruction call
        let mut args = OxcVec::new_in(allocator);
        args.push(target);
        args.push(value);
        create_instruction_call_expr(allocator, Identifiers::TWO_WAY_BINDING_SET, args)
    }
}

/// Checks if the target is a settable property or keyed expression.
fn is_settable_property_target(target: &IrExpression<'_>) -> bool {
    match target {
        // ExpressionRef typically points to a PropertyRead for two-way bindings
        IrExpression::ExpressionRef(_) => true,
        // ResolvedPropertyRead/ResolvedKeyedRead are resolved versions of PropertyRead/KeyedRead
        IrExpression::ResolvedPropertyRead(_) => true,
        IrExpression::ResolvedKeyedRead(_) => true,
        IrExpression::ResolvedSafePropertyRead(_) => true,
        IrExpression::Ast(ast_expr) => matches!(
            ast_expr.as_ref(),
            AngularExpression::PropertyRead(_)
                | AngularExpression::KeyedRead(_)
                | AngularExpression::SafePropertyRead(_)
                | AngularExpression::SafeKeyedRead(_)
        ),
        IrExpression::LexicalRead(_) => true, // Property on implicit receiver
        _ => false,
    }
}

/// Creates an assignment expression (target = value).
fn create_assignment<'a>(
    allocator: &'a oxc_allocator::Allocator,
    target: &OutputExpression<'a>,
    value: &OutputExpression<'a>,
) -> OutputExpression<'a> {
    OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Assign,
            lhs: Box::new_in(clone_output_expression(allocator, target), allocator),
            rhs: Box::new_in(clone_output_expression(allocator, value), allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Clones an output expression (shallow clone for use in multiple places).
fn clone_output_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &OutputExpression<'a>,
) -> OutputExpression<'a> {
    match expr {
        OutputExpression::ReadVar(rv) => OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: rv.name.clone(), source_span: rv.source_span },
            allocator,
        )),
        OutputExpression::ReadProp(rp) => OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(clone_output_expression(allocator, &rp.receiver), allocator),
                name: rp.name.clone(),
                optional: false,
                source_span: rp.source_span,
            },
            allocator,
        )),
        OutputExpression::ReadKey(rk) => OutputExpression::ReadKey(Box::new_in(
            ReadKeyExpr {
                receiver: Box::new_in(clone_output_expression(allocator, &rk.receiver), allocator),
                index: Box::new_in(clone_output_expression(allocator, &rk.index), allocator),
                optional: false,
                source_span: rk.source_span,
            },
            allocator,
        )),
        OutputExpression::Literal(lit) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: clone_literal_value(&lit.value), source_span: lit.source_span },
            allocator,
        )),
        // For all other types, use the built-in clone_in
        other => other.clone_in(allocator),
    }
}

/// Clones a literal value.
fn clone_literal_value<'a>(value: &LiteralValue<'a>) -> LiteralValue<'a> {
    match value {
        LiteralValue::Null => LiteralValue::Null,
        LiteralValue::Undefined => LiteralValue::Undefined,
        LiteralValue::Boolean(b) => LiteralValue::Boolean(*b),
        LiteralValue::Number(n) => LiteralValue::Number(*n),
        LiteralValue::String(s) => LiteralValue::String(s.clone()),
    }
}

/// Converts an IR binary operator to an output binary operator.
fn convert_ir_binary_operator(op: crate::ir::expression::IrBinaryOperator) -> BinaryOperator {
    use crate::ir::expression::IrBinaryOperator;

    match op {
        IrBinaryOperator::Plus => BinaryOperator::Plus,
        IrBinaryOperator::Minus => BinaryOperator::Minus,
        IrBinaryOperator::Multiply => BinaryOperator::Multiply,
        IrBinaryOperator::Divide => BinaryOperator::Divide,
        IrBinaryOperator::Modulo => BinaryOperator::Modulo,
        IrBinaryOperator::Exponentiation => BinaryOperator::Exponentiation,
        IrBinaryOperator::Equals => BinaryOperator::Equals,
        IrBinaryOperator::NotEquals => BinaryOperator::NotEquals,
        IrBinaryOperator::Identical => BinaryOperator::Identical,
        IrBinaryOperator::NotIdentical => BinaryOperator::NotIdentical,
        IrBinaryOperator::Lower => BinaryOperator::Lower,
        IrBinaryOperator::LowerEquals => BinaryOperator::LowerEquals,
        IrBinaryOperator::Bigger => BinaryOperator::Bigger,
        IrBinaryOperator::BiggerEquals => BinaryOperator::BiggerEquals,
        IrBinaryOperator::And => BinaryOperator::And,
        IrBinaryOperator::Or => BinaryOperator::Or,
        IrBinaryOperator::NullishCoalesce => BinaryOperator::NullishCoalesce,
        IrBinaryOperator::In => BinaryOperator::In,
        IrBinaryOperator::Instanceof => BinaryOperator::Instanceof,
        IrBinaryOperator::Assign => BinaryOperator::Assign,
        IrBinaryOperator::AdditionAssignment => BinaryOperator::AdditionAssignment,
        IrBinaryOperator::SubtractionAssignment => BinaryOperator::SubtractionAssignment,
        IrBinaryOperator::MultiplicationAssignment => BinaryOperator::MultiplicationAssignment,
        IrBinaryOperator::DivisionAssignment => BinaryOperator::DivisionAssignment,
        IrBinaryOperator::RemainderAssignment => BinaryOperator::RemainderAssignment,
        IrBinaryOperator::ExponentiationAssignment => BinaryOperator::ExponentiationAssignment,
        IrBinaryOperator::AndAssignment => BinaryOperator::AndAssignment,
        IrBinaryOperator::OrAssignment => BinaryOperator::OrAssignment,
        IrBinaryOperator::NullishCoalesceAssignment => BinaryOperator::NullishCoalesceAssignment,
    }
}

//! Pure function extraction phase.
//!
//! Extracts pure function expressions for memoization via `ɵɵpureFunction`.
//!
//! This phase processes `PureFunctionExpr` nodes that were created by earlier phases
//! (such as pipe binding creation) and extracts their bodies to the constant pool
//! for deduplication and efficient code generation.
//!
//! The constant pool stores the function bodies and returns references that are
//! set on the expression's `fn_ref` field.
//!
//! IMPORTANT: Pure functions are pooled immediately as they're discovered during
//! expression traversal, matching TypeScript's single-pass visitor behavior.
//! This ensures consistent const index ordering with the reference implementation.
//!
//! Ported from Angular's `template/pipeline/src/phases/pure_function_extraction.ts`.

use std::cell::RefCell;

use oxc_allocator::Box;

use crate::ir::expression::{
    IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::output::ast::{OutputExpression, ReadVarExpr};
use crate::pipeline::compilation::{
    ComponentCompilationJob, HostBindingCompilationJob, ViewCompilationUnit,
};
use crate::pipeline::constant_pool::ConstantPool;

/// Extracts pure function bodies to the constant pool.
///
/// This phase:
/// 1. Visits all expressions in all ops (create and update)
/// 2. Finds `PureFunctionExpr` nodes with a body
/// 3. Generates a unique key for the body expression
/// 4. Pools the function in the constant pool immediately (matching TypeScript behavior)
/// 5. Sets `fn_ref` to reference the pooled function
/// 6. Clears the body (set to None)
pub fn extract_pure_functions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Use RefCell to allow interior mutability of the pool during visitor traversal.
    // This is needed because we need to pool functions immediately as they're found,
    // matching TypeScript's single-pass behavior for consistent const ordering.
    let pool_cell = RefCell::new(&mut job.pool);

    let view_xrefs: Vec<_> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        // Get mutable reference to the view - root is direct, others are boxed
        let view_opt: Option<&mut ViewCompilationUnit<'_>> = if view_xref.0 == 0 {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|b| b.as_mut())
        };

        if let Some(view) = view_opt {
            // Process create ops
            for op in view.create.iter_mut() {
                transform_expressions_in_create_op(
                    op,
                    &|expr, _flags| {
                        extract_pure_function(allocator, expr, &pool_cell);
                    },
                    VisitorContextFlag::NONE,
                );
            }

            // Process update ops
            for op in view.update.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, _flags| {
                        extract_pure_function(allocator, expr, &pool_cell);
                    },
                    VisitorContextFlag::NONE,
                );
            }
        }
    }
}

/// Extract a single pure function expression immediately, pooling it and updating the node.
///
/// This matches TypeScript's behavior where each PureFunctionExpr is processed
/// immediately when encountered during the visitor traversal.
fn extract_pure_function<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut IrExpression<'a>,
    pool_cell: &RefCell<&mut ConstantPool<'a>>,
) {
    if let IrExpression::PureFunction(pf) = expr {
        if let Some(body_box) = pf.body.take() {
            // Unbox the body expression
            let body = oxc_allocator::Box::unbox(body_box);

            // Generate a key from the body and arg count
            let body_key = generate_expression_key(&body);

            // Pool immediately (matching TypeScript's single-pass behavior)
            let fn_name = {
                let mut pool = pool_cell.borrow_mut();
                let (_, name) = pool.pool_pure_function(pf.args.len() as u32, &body_key, body);
                name
            };

            // Set fn_ref to a direct variable reference (not ctx.property).
            // Pure function references are module-level variables, not component properties.
            // Using OutputExpr with ReadVarExpr ensures this is emitted as just `_c0`
            // instead of `ctx._c0`.
            pf.fn_ref = Some(Box::new_in(
                IrExpression::OutputExpr(Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: fn_name, source_span: None },
                        allocator,
                    )),
                    allocator,
                )),
                allocator,
            ));
        }
    }
}

/// Generate a unique key for an expression (for deduplication).
///
/// This follows the key generation format from Angular's `GenericKeyFn.keyOf` in `constant_pool.ts`
/// and `PureFunctionConstant.keyOf` in `pure_function_extraction.ts`.
///
/// Key format by expression type:
/// - `PureFunctionParameterExpr`: `param(${index})`
/// - `LiteralExpr` (string): `"${value}"`
/// - `LiteralExpr` (number/boolean/null/undefined): `${value}`
/// - `RegularExpressionLiteralExpr`: `/${body}/${flags}`
/// - `LiteralArrayExpr`: `[${entries.join(',')}]`
/// - `LiteralMapExpr`: `{${entries.join(',')}}` with `key:value` or `"key":value` for quoted keys
/// - `ReadVarExpr` (LexicalRead): `read(${name})`
/// - `TypeofExpr`: `typeof(${keyOf(expr)})`
fn generate_expression_key(expr: &IrExpression<'_>) -> String {
    match expr {
        // PureFunctionParameterExpr -> `param(${expr.index})`
        IrExpression::PureFunctionParameter(p) => format!("param({})", p.index),

        // ReadVarExpr -> `read(${expr.name})`
        IrExpression::LexicalRead(r) => format!("read({})", r.name),

        // Handle AST expressions - these are the main expression types that need key generation
        IrExpression::Ast(ast) => generate_angular_expression_key(ast),

        // DerivedLiteralArray -> `[${entries.join(',')}]` with `...` prefix on spread entries.
        // The spread flag must participate in the key so `[a]` and `[...a]` don't collide in
        // the pure-function pool and silently swap runtime semantics.
        IrExpression::DerivedLiteralArray(arr) => {
            let entries: Vec<_> = arr
                .entries
                .iter()
                .zip(arr.spreads.iter())
                .map(|(entry, is_spread)| {
                    let key = generate_expression_key(entry);
                    if *is_spread { format!("...{}", key) } else { key }
                })
                .collect();
            format!("[{}]", entries.join(","))
        }

        // DerivedLiteralMap -> `{${entries.join(',')}}` with `...value` for spread entries.
        IrExpression::DerivedLiteralMap(map) => {
            let entries: Vec<_> = map
                .keys
                .iter()
                .zip(map.values.iter())
                .zip(map.quoted.iter())
                .zip(map.spreads.iter())
                .map(|(((key, value), quoted), is_spread)| {
                    let value_key = generate_expression_key(value);
                    if *is_spread {
                        format!("...{}", value_key)
                    } else {
                        let key_str =
                            if *quoted { format!("\"{}\"", key) } else { key.to_string() };
                        format!("{}:{}", key_str, value_key)
                    }
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }

        // Other IR expressions - generate keys for nested structures
        IrExpression::Reference(r) => format!("ref({})", r.target.0),
        IrExpression::Context(c) => format!("ctx({})", c.view.0),
        IrExpression::Empty(_) => "empty".to_string(),
        IrExpression::PureFunction(pf) => {
            let body_key = pf.body.as_ref().map(|b| generate_expression_key(b)).unwrap_or_default();
            let args_keys: Vec<_> = pf.args.iter().map(generate_expression_key).collect();
            format!("pf({},{})", body_key, args_keys.join(","))
        }
        IrExpression::Interpolation(interp) => {
            let expr_keys: Vec<_> =
                interp.expressions.iter().map(generate_expression_key).collect();
            format!("interp({:?},{})", interp.strings, expr_keys.join(","))
        }
        IrExpression::Binary(b) => {
            let lhs = generate_expression_key(&b.lhs);
            let rhs = generate_expression_key(&b.rhs);
            format!("binary({:?},{},{})", b.operator, lhs, rhs)
        }
        IrExpression::Ternary(t) => {
            let cond = generate_expression_key(&t.condition);
            let true_expr = generate_expression_key(&t.true_expr);
            let false_expr = generate_expression_key(&t.false_expr);
            format!("ternary({},{},{})", cond, true_expr, false_expr)
        }
        IrExpression::ReadVariable(rv) => format!("readvar({})", rv.xref.0),
        IrExpression::ResolvedPropertyRead(rpr) => {
            let receiver = generate_expression_key(&rpr.receiver);
            format!("prop({},{})", receiver, rpr.name)
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            let receiver = generate_expression_key(&rkr.receiver);
            let key = generate_expression_key(&rkr.key);
            format!("keyed({},{})", receiver, key)
        }
        IrExpression::ResolvedCall(rc) => {
            let receiver = generate_expression_key(&rc.receiver);
            let args: Vec<_> = rc.args.iter().map(generate_expression_key).collect();
            format!("call({},{})", receiver, args.join(","))
        }
        IrExpression::SafePropertyRead(spr) => {
            let receiver = generate_expression_key(&spr.receiver);
            format!("safeprop({},{})", receiver, spr.name)
        }
        IrExpression::SafeKeyedRead(skr) => {
            let receiver = generate_expression_key(&skr.receiver);
            let index = generate_expression_key(&skr.index);
            format!("safekeyed({},{})", receiver, index)
        }
        _ => format!("{:?}", std::mem::discriminant(expr)),
    }
}

/// Generate a key for an Angular AST expression.
///
/// This matches the key format from Angular's `GenericKeyFn.keyOf` in `constant_pool.ts`.
fn generate_angular_expression_key(expr: &crate::ast::expression::AngularExpression<'_>) -> String {
    use crate::ast::expression::{AngularExpression, LiteralValue};

    match expr {
        // LiteralExpr with string value -> `"${value}"`
        AngularExpression::LiteralPrimitive(lit) => match &lit.value {
            LiteralValue::String(s) => format!("\"{}\"", s),
            LiteralValue::Number(n) => {
                // Match JavaScript's String() behavior for numbers
                if n.is_nan() {
                    "NaN".to_string()
                } else if n.is_infinite() {
                    if *n > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() }
                } else if *n == 0.0 {
                    "0".to_string()
                } else {
                    // Format to match JavaScript's number-to-string conversion
                    let formatted = format!("{}", n);
                    // Remove trailing ".0" for whole numbers
                    if formatted.ends_with(".0") {
                        formatted[..formatted.len() - 2].to_string()
                    } else {
                        formatted
                    }
                }
            }
            LiteralValue::Boolean(b) => b.to_string(),
            LiteralValue::Null => "null".to_string(),
            LiteralValue::Undefined => "undefined".to_string(),
        },

        // RegularExpressionLiteralExpr -> `/${body}/${flags}`
        AngularExpression::RegularExpressionLiteral(regex) => {
            let flags = regex.flags.as_ref().map(|f| f.as_str()).unwrap_or("");
            format!("/{}/{}", regex.body, flags)
        }

        // LiteralArrayExpr -> `[${entries.join(',')}]`
        AngularExpression::LiteralArray(arr) => {
            let entries: Vec<_> =
                arr.expressions.iter().map(generate_angular_expression_key).collect();
            format!("[{}]", entries.join(","))
        }

        // LiteralMapExpr -> `{${entries.join(',')}}`
        AngularExpression::LiteralMap(map) => {
            use crate::ast::expression::LiteralMapKey;
            let entries: Vec<_> = map
                .keys
                .iter()
                .zip(map.values.iter())
                .filter_map(|(key, value)| {
                    if let LiteralMapKey::Property(prop) = key {
                        let key_str = if prop.quoted {
                            format!("\"{}\"", prop.key)
                        } else {
                            prop.key.to_string()
                        };
                        Some(format!("{}:{}", key_str, generate_angular_expression_key(value)))
                    } else {
                        // Skip spread keys for now
                        None
                    }
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }

        // PropertyRead with ImplicitReceiver -> `read(${name})`
        // This is equivalent to ReadVarExpr in Angular's output AST
        AngularExpression::PropertyRead(pr) => {
            if matches!(
                pr.receiver,
                AngularExpression::ImplicitReceiver(_) | AngularExpression::ThisReceiver(_)
            ) {
                format!("read({})", pr.name)
            } else {
                // Nested property read
                let receiver = generate_angular_expression_key(&pr.receiver);
                format!("prop({},{})", receiver, pr.name)
            }
        }

        // TypeofExpr -> `typeof(${keyOf(expr)})`
        AngularExpression::TypeofExpression(te) => {
            format!("typeof({})", generate_angular_expression_key(&te.expression))
        }

        // Handle other expression types that might appear in pure function bodies
        AngularExpression::Binary(b) => {
            let left = generate_angular_expression_key(&b.left);
            let right = generate_angular_expression_key(&b.right);
            format!("binary({:?},{},{})", b.operation, left, right)
        }

        AngularExpression::Conditional(c) => {
            let cond = generate_angular_expression_key(&c.condition);
            let true_expr = generate_angular_expression_key(&c.true_exp);
            let false_expr = generate_angular_expression_key(&c.false_exp);
            format!("cond({},{},{})", cond, true_expr, false_expr)
        }

        AngularExpression::Call(call) => {
            let receiver = generate_angular_expression_key(&call.receiver);
            let args: Vec<_> = call.args.iter().map(generate_angular_expression_key).collect();
            format!("call({},{})", receiver, args.join(","))
        }

        AngularExpression::SafeCall(call) => {
            let receiver = generate_angular_expression_key(&call.receiver);
            let args: Vec<_> = call.args.iter().map(generate_angular_expression_key).collect();
            format!("safecall({},{})", receiver, args.join(","))
        }

        AngularExpression::KeyedRead(kr) => {
            let receiver = generate_angular_expression_key(&kr.receiver);
            let key = generate_angular_expression_key(&kr.key);
            format!("keyed({},{})", receiver, key)
        }

        AngularExpression::SafeKeyedRead(skr) => {
            let receiver = generate_angular_expression_key(&skr.receiver);
            let key = generate_angular_expression_key(&skr.key);
            format!("safekeyed({},{})", receiver, key)
        }

        AngularExpression::SafePropertyRead(spr) => {
            let receiver = generate_angular_expression_key(&spr.receiver);
            format!("safeprop({},{})", receiver, spr.name)
        }

        AngularExpression::PrefixNot(pn) => {
            format!("not({})", generate_angular_expression_key(&pn.expression))
        }

        AngularExpression::Unary(u) => {
            format!("unary({:?},{})", u.operator, generate_angular_expression_key(&u.expr))
        }

        AngularExpression::NonNullAssert(nna) => {
            format!("nonnull({})", generate_angular_expression_key(&nna.expression))
        }

        AngularExpression::VoidExpression(ve) => {
            format!("void({})", generate_angular_expression_key(&ve.expression))
        }

        AngularExpression::ParenthesizedExpression(pe) => {
            // Parentheses don't change the key - the expression inside determines uniqueness
            generate_angular_expression_key(&pe.expression)
        }

        AngularExpression::BindingPipe(bp) => {
            let exp = generate_angular_expression_key(&bp.exp);
            let args: Vec<_> = bp.args.iter().map(generate_angular_expression_key).collect();
            format!("pipe({},{},{})", bp.name, exp, args.join(","))
        }

        AngularExpression::Interpolation(interp) => {
            let strings: Vec<_> = interp.strings.iter().map(|s| format!("\"{}\"", s)).collect();
            let exprs: Vec<_> =
                interp.expressions.iter().map(generate_angular_expression_key).collect();
            format!("interp([{}],[{}])", strings.join(","), exprs.join(","))
        }

        AngularExpression::Chain(chain) => {
            let exprs: Vec<_> =
                chain.expressions.iter().map(generate_angular_expression_key).collect();
            format!("chain({})", exprs.join(","))
        }

        AngularExpression::TemplateLiteral(tl) => {
            let elements: Vec<_> = tl.elements.iter().map(|e| format!("\"{}\"", e.text)).collect();
            let exprs: Vec<_> =
                tl.expressions.iter().map(generate_angular_expression_key).collect();
            format!("template([{}],[{}])", elements.join(","), exprs.join(","))
        }

        AngularExpression::TaggedTemplateLiteral(ttl) => {
            let tag = generate_angular_expression_key(&ttl.tag);
            let elements: Vec<_> =
                ttl.template.elements.iter().map(|e| format!("\"{}\"", e.text)).collect();
            let exprs: Vec<_> =
                ttl.template.expressions.iter().map(generate_angular_expression_key).collect();
            format!("tagged({},template([{}],[{}]))", tag, elements.join(","), exprs.join(","))
        }

        // Receivers don't typically appear in pure function bodies, but handle them for completeness
        AngularExpression::ImplicitReceiver(_) => "implicit".to_string(),
        AngularExpression::ThisReceiver(_) => "this".to_string(),
        AngularExpression::Empty(_) => "empty".to_string(),
        AngularExpression::SpreadElement(spread) => {
            format!("...{}", generate_angular_expression_key(&spread.expression))
        }
        AngularExpression::ArrowFunction(arrow) => {
            let params: Vec<_> =
                arrow.parameters.iter().map(|p| p.name.as_str().to_string()).collect();
            let body = generate_angular_expression_key(&arrow.body);
            format!("(({}) => {})", params.join(","), body)
        }
    }
}

/// Extracts pure functions for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn extract_pure_functions_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Use RefCell to allow interior mutability of the pool during visitor traversal.
    let pool_cell = RefCell::new(&mut job.pool);

    // Process create ops
    for op in job.root.create.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| {
                extract_pure_function(allocator, expr, &pool_cell);
            },
            VisitorContextFlag::NONE,
        );
    }

    // Process update ops
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                extract_pure_function(allocator, expr, &pool_cell);
            },
            VisitorContextFlag::NONE,
        );
    }
}

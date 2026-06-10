//! Resolve $event phase.
//!
//! Any variable inside a listener with the name `$event` will be transformed into an output lexical
//! read immediately, and does not participate in any of the normal logic for handling variables.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_dollar_event.ts`.

use std::cell::Cell;

use crate::ir::expression::{IrExpression, VisitorContextFlag, transform_expressions_in_update_op};
use crate::ir::ops::CreateOp;
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use crate::pipeline::expression_store::ExpressionStore;

/// The special variable name for event objects in listeners.
const DOLLAR_EVENT: &str = "$event";

/// Resolves $event to the event parameter in handlers.
///
/// This phase:
/// 1. Finds all `LexicalReadExpr('$event')` in event handlers
/// 2. Marks the listener as consuming $event
///
/// Note: Unlike the TypeScript implementation, we don't need to transform
/// the expression since LexicalRead will be converted to proper variable
/// access in the reify phase.
pub fn resolve_dollar_event(job: &mut ComponentCompilationJob<'_>) {
    // We need to borrow the expression store separately from the views
    // to avoid mutable borrow conflicts. We use a raw pointer to the store
    // since transform_dollar_event_create only needs read access to expressions.
    let expressions_ptr = &job.expressions as *const ExpressionStore<'_>;

    // Process each view's create ops (listeners are CreateOps)
    for view in job.all_views_mut() {
        // SAFETY: We only read from expressions, never modify it, and job.expressions
        // outlives this function call.
        let expressions = unsafe { &*expressions_ptr };
        transform_dollar_event_create(&mut view.create, expressions);
    }
}

/// Transform $event in create operations.
fn transform_dollar_event_create<'a>(
    ops: &mut crate::ir::list::CreateOpList<'a>,
    expressions: &ExpressionStore<'a>,
) {
    for op in ops.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                // Check handler expression for $event usage
                if let Some(handler) = &listener.handler_expression {
                    if expression_contains_dollar_event(handler, expressions) {
                        listener.consumes_dollar_event = true;
                    }
                }

                // Check handler ops for $event usage
                for handler_op in listener.handler_ops.iter_mut() {
                    let found = Cell::new(false);
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            if expression_contains_dollar_event(expr, expressions) {
                                found.set(true);
                            }
                        },
                        VisitorContextFlag::NONE,
                    );
                    if found.get() {
                        listener.consumes_dollar_event = true;
                        break;
                    }
                }
            }
            CreateOp::TwoWayListener(listener) => {
                // Check handler ops for $event usage
                for handler_op in listener.handler_ops.iter_mut() {
                    let found = Cell::new(false);
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            if expression_contains_dollar_event(expr, expressions) {
                                found.set(true);
                            }
                        },
                        VisitorContextFlag::NONE,
                    );
                    if found.get() {
                        // TwoWayListener doesn't have consumes_dollar_event field
                        // since it always implicitly consumes $event
                        break;
                    }
                }
            }
            CreateOp::AnimationListener(listener) => {
                // Check handler ops for $event usage
                for handler_op in listener.handler_ops.iter_mut() {
                    let found = Cell::new(false);
                    transform_expressions_in_update_op(
                        handler_op,
                        &|expr, _flags| {
                            if expression_contains_dollar_event(expr, expressions) {
                                found.set(true);
                            }
                        },
                        VisitorContextFlag::NONE,
                    );
                    if found.get() {
                        listener.consumes_dollar_event = true;
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Check if an expression contains a $event reference.
fn expression_contains_dollar_event<'a>(
    expr: &IrExpression<'a>,
    expressions: &ExpressionStore<'a>,
) -> bool {
    match expr {
        IrExpression::LexicalRead(lexical) => lexical.name.as_str() == DOLLAR_EVENT,
        IrExpression::SafeNavigationMigration(m) => {
            expression_contains_dollar_event(&m.expr, expressions)
        }
        IrExpression::SafeTernary(st) => {
            expression_contains_dollar_event(&st.guard, expressions)
                || expression_contains_dollar_event(&st.expr, expressions)
        }
        IrExpression::Ternary(t) => {
            expression_contains_dollar_event(&t.condition, expressions)
                || expression_contains_dollar_event(&t.true_expr, expressions)
                || expression_contains_dollar_event(&t.false_expr, expressions)
        }
        IrExpression::SafePropertyRead(sp) => {
            expression_contains_dollar_event(&sp.receiver, expressions)
        }
        IrExpression::SafeKeyedRead(sk) => {
            expression_contains_dollar_event(&sk.receiver, expressions)
                || expression_contains_dollar_event(&sk.index, expressions)
        }
        IrExpression::SafeInvokeFunction(sf) => {
            expression_contains_dollar_event(&sf.receiver, expressions)
                || sf.args.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        IrExpression::AssignTemporary(at) => {
            expression_contains_dollar_event(&at.expr, expressions)
        }
        IrExpression::PipeBinding(pb) => {
            pb.args.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            expression_contains_dollar_event(&pbv.args, expressions)
        }
        IrExpression::PureFunction(pf) => {
            pf.args.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        IrExpression::Interpolation(i) => {
            i.expressions.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        IrExpression::RestoreView(rv) => {
            use crate::ir::expression::RestoreViewTarget;
            if let RestoreViewTarget::Dynamic(e) = &rv.view {
                expression_contains_dollar_event(e, expressions)
            } else {
                false
            }
        }
        IrExpression::ResetView(rv) => expression_contains_dollar_event(&rv.expr, expressions),
        IrExpression::ConditionalCase(cc) => {
            cc.expr.as_ref().is_some_and(|e| expression_contains_dollar_event(e, expressions))
        }
        IrExpression::TwoWayBindingSet(tbs) => {
            expression_contains_dollar_event(&tbs.target, expressions)
                || expression_contains_dollar_event(&tbs.value, expressions)
        }
        IrExpression::StoreLet(sl) => expression_contains_dollar_event(&sl.value, expressions),
        IrExpression::ConstCollected(cc) => expression_contains_dollar_event(&cc.expr, expressions),
        IrExpression::Binary(binary) => {
            expression_contains_dollar_event(&binary.lhs, expressions)
                || expression_contains_dollar_event(&binary.rhs, expressions)
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            expression_contains_dollar_event(&rpr.receiver, expressions)
        }
        IrExpression::ResolvedBinary(rb) => {
            expression_contains_dollar_event(&rb.left, expressions)
                || expression_contains_dollar_event(&rb.right, expressions)
        }
        IrExpression::ResolvedCall(rc) => {
            expression_contains_dollar_event(&rc.receiver, expressions)
                || rc.args.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            expression_contains_dollar_event(&rkr.receiver, expressions)
                || expression_contains_dollar_event(&rkr.key, expressions)
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            expression_contains_dollar_event(&rspr.receiver, expressions)
        }
        // Check AST expressions for $event
        IrExpression::Ast(ast_expr) => ast_expression_contains_dollar_event(ast_expr),
        // DerivedLiteralArray: check entries
        IrExpression::DerivedLiteralArray(arr) => {
            arr.entries.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        // DerivedLiteralMap: check values
        IrExpression::DerivedLiteralMap(map) => {
            map.values.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }
        // ExpressionRef: dereference to stored AngularExpression and check it
        IrExpression::ExpressionRef(id) => {
            let stored_expr = expressions.get(*id);
            ast_expression_contains_dollar_event(stored_expr)
        }
        // Leaf expressions without nested content
        IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::ReadVariable(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::OutputExpr(_) => false, // OutputExpr is already converted, no $event

        // LiteralArray: check all elements
        IrExpression::LiteralArray(arr) => {
            arr.elements.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }

        // LiteralMap: check all values
        IrExpression::LiteralMap(map) => {
            map.values.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }

        // Not expression: check inner expression
        IrExpression::Not(not) => expression_contains_dollar_event(&not.expr, expressions),

        // Unary expression: check inner expression
        IrExpression::Unary(unary) => expression_contains_dollar_event(&unary.expr, expressions),

        // Typeof expression: check inner expression
        IrExpression::Typeof(typeof_expr) => {
            expression_contains_dollar_event(&typeof_expr.expr, expressions)
        }

        // Void expression: check inner expression
        IrExpression::Void(void_expr) => {
            expression_contains_dollar_event(&void_expr.expr, expressions)
        }

        // ResolvedTemplateLiteral: check all embedded expressions
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            rtl.expressions.iter().any(|e| expression_contains_dollar_event(e, expressions))
        }

        // ArrowFunction: check the body expression
        IrExpression::ArrowFunction(arrow_fn) => {
            expression_contains_dollar_event(&arrow_fn.body, expressions)
        }

        // Parenthesized: check the inner expression
        IrExpression::Parenthesized(paren) => {
            expression_contains_dollar_event(&paren.expr, expressions)
        }
    }
}

/// Check if an AST expression contains a $event reference.
fn ast_expression_contains_dollar_event(
    expr: &crate::ast::expression::AngularExpression<'_>,
) -> bool {
    use crate::ast::expression::AngularExpression;

    match expr {
        AngularExpression::PropertyRead(pr) => {
            if pr.name.as_str() == DOLLAR_EVENT {
                // Check if receiver is implicit (just "$event" by itself)
                if matches!(
                    pr.receiver,
                    AngularExpression::ImplicitReceiver(_) | AngularExpression::ThisReceiver(_)
                ) {
                    return true;
                }
            }
            ast_expression_contains_dollar_event(&pr.receiver)
        }
        AngularExpression::SafePropertyRead(spr) => {
            if spr.name.as_str() == DOLLAR_EVENT {
                if matches!(
                    spr.receiver,
                    AngularExpression::ImplicitReceiver(_) | AngularExpression::ThisReceiver(_)
                ) {
                    return true;
                }
            }
            ast_expression_contains_dollar_event(&spr.receiver)
        }
        AngularExpression::Call(call) => {
            ast_expression_contains_dollar_event(&call.receiver)
                || call.args.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::SafeCall(call) => {
            ast_expression_contains_dollar_event(&call.receiver)
                || call.args.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::KeyedRead(kr) => {
            ast_expression_contains_dollar_event(&kr.receiver)
                || ast_expression_contains_dollar_event(&kr.key)
        }
        AngularExpression::SafeKeyedRead(skr) => {
            ast_expression_contains_dollar_event(&skr.receiver)
                || ast_expression_contains_dollar_event(&skr.key)
        }
        AngularExpression::Binary(binary) => {
            ast_expression_contains_dollar_event(&binary.left)
                || ast_expression_contains_dollar_event(&binary.right)
        }
        AngularExpression::Unary(unary) => ast_expression_contains_dollar_event(&unary.expr),
        AngularExpression::PrefixNot(prefix) => {
            ast_expression_contains_dollar_event(&prefix.expression)
        }
        AngularExpression::Conditional(cond) => {
            ast_expression_contains_dollar_event(&cond.condition)
                || ast_expression_contains_dollar_event(&cond.true_exp)
                || ast_expression_contains_dollar_event(&cond.false_exp)
        }
        AngularExpression::ParenthesizedExpression(paren) => {
            ast_expression_contains_dollar_event(&paren.expression)
        }
        AngularExpression::BindingPipe(pipe) => {
            ast_expression_contains_dollar_event(&pipe.exp)
                || pipe.args.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::NonNullAssert(nna) => {
            ast_expression_contains_dollar_event(&nna.expression)
        }
        AngularExpression::LiteralArray(arr) => {
            arr.expressions.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::LiteralMap(map) => {
            map.values.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::Chain(chain) => {
            chain.expressions.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::Interpolation(interp) => {
            interp.expressions.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::TemplateLiteral(tl) => {
            tl.expressions.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::TaggedTemplateLiteral(ttl) => {
            ast_expression_contains_dollar_event(&ttl.tag)
                || ttl.template.expressions.iter().any(ast_expression_contains_dollar_event)
        }
        AngularExpression::TypeofExpression(te) => {
            ast_expression_contains_dollar_event(&te.expression)
        }
        AngularExpression::VoidExpression(ve) => {
            ast_expression_contains_dollar_event(&ve.expression)
        }
        AngularExpression::SpreadElement(spread) => {
            ast_expression_contains_dollar_event(&spread.expression)
        }
        AngularExpression::ArrowFunction(arrow) => {
            ast_expression_contains_dollar_event(&arrow.body)
        }
        // Leaf expressions
        AngularExpression::Empty(_)
        | AngularExpression::ImplicitReceiver(_)
        | AngularExpression::ThisReceiver(_)
        | AngularExpression::LiteralPrimitive(_)
        | AngularExpression::RegularExpressionLiteral(_) => false,
    }
}

/// Resolves $event references for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn resolve_dollar_event_for_host(job: &mut HostBindingCompilationJob<'_>) {
    // We need to borrow the expression store separately from the root create ops
    // to avoid mutable borrow conflicts. We use a raw pointer to the store
    // since transform_dollar_event_create only needs read access to expressions.
    let expressions_ptr = &job.expressions as *const ExpressionStore<'_>;

    // SAFETY: We only read from expressions, never modify it, and job.expressions
    // outlives this function call.
    let expressions = unsafe { &*expressions_ptr };

    // Process create ops for listeners
    transform_dollar_event_create(&mut job.root.create, expressions);
}

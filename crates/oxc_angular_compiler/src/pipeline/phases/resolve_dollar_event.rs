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
/// 1. Rewrites `LexicalReadExpr('$event')` in event handlers to an output
///    `ReadVarExpr('$event')` so it bypasses name resolution, matching the
///    TypeScript implementation.
/// 2. Marks the listener as consuming $event
///
/// Expressions stored as raw AST (Ast/ExpressionRef) cannot be rewritten here;
/// they are still emitted correctly by the reify phase's `$event` special-case.
pub fn resolve_dollar_event(job: &mut ComponentCompilationJob<'_>) {
    // We need to borrow the expression store separately from the views
    // to avoid mutable borrow conflicts. We use a raw pointer to the store
    // since transform_dollar_event_create only needs read access to expressions.
    let expressions_ptr = &job.expressions as *const ExpressionStore<'_>;
    let allocator = job.allocator;

    // Process each view's create ops (listeners are CreateOps)
    for view in job.all_views_mut() {
        // SAFETY: We only read from expressions, never modify it, and job.expressions
        // outlives this function call.
        let expressions = unsafe { &*expressions_ptr };
        transform_dollar_event_create(&mut view.create, expressions, allocator);
    }
}

/// Transform $event in create operations.
///
/// Listener handler ops are transformed as child operations, matching
/// `transformExpressionsInOp`. Each listener is walked once: every
/// `LexicalReadExpr('$event')` found is rewritten to an output `ReadVarExpr`,
/// like Angular's resolveDollarEvent, and marks the listener as consuming
/// `$event`, as does a raw AST expression that reads it.
fn transform_dollar_event_create<'a>(
    ops: &mut crate::ir::list::CreateOpList<'a>,
    expressions: &ExpressionStore<'a>,
    allocator: &'a oxc_allocator::Allocator,
) {
    for op in ops.iter_mut() {
        let found = Cell::new(false);
        let resolve = |expr: &mut IrExpression<'a>, _flags: VisitorContextFlag| {
            resolve_dollar_event_in_expr(expr, expressions, allocator, &found);
        };
        match op {
            CreateOp::Listener(listener) => {
                if let Some(handler) = &mut listener.handler_expression {
                    crate::ir::expression::transform_expressions_in_expression(
                        handler,
                        &resolve,
                        VisitorContextFlag::IN_CHILD_OPERATION,
                    );
                }
                for handler_op in listener.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &resolve,
                        VisitorContextFlag::IN_CHILD_OPERATION,
                    );
                }
                if found.get() {
                    listener.consumes_dollar_event = true;
                }
            }
            // Two-way listeners always consume `$event` so they omit this field.
            CreateOp::TwoWayListener(listener) => {
                for handler_op in listener.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &resolve,
                        VisitorContextFlag::IN_CHILD_OPERATION,
                    );
                }
            }
            CreateOp::AnimationListener(listener) => {
                for handler_op in listener.handler_ops.iter_mut() {
                    transform_expressions_in_update_op(
                        handler_op,
                        &resolve,
                        VisitorContextFlag::IN_CHILD_OPERATION,
                    );
                }
                if found.get() {
                    listener.consumes_dollar_event = true;
                }
            }
            _ => {}
        }
    }
}

/// Rewrites `LexicalReadExpr('$event')` to an output `ReadVarExpr` so the read
/// bypasses name resolution, like Angular's resolveDollarEvent, and records in
/// `found` that the listener reads `$event`.
///
/// The walk visits every IR node, so only this node itself is checked. Raw AST
/// (`Ast` / `ExpressionRef`) is a leaf to the walk and is searched whole.
fn resolve_dollar_event_in_expr<'a>(
    expr: &mut IrExpression<'a>,
    expressions: &ExpressionStore<'a>,
    allocator: &'a oxc_allocator::Allocator,
    found: &Cell<bool>,
) {
    match expr {
        IrExpression::LexicalRead(lexical) if lexical.name.as_str() == DOLLAR_EVENT => {
            found.set(true);
            *expr = IrExpression::OutputExpr(oxc_allocator::Box::new_in(
                crate::output::ast::OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                    crate::output::ast::ReadVarExpr {
                        name: lexical.name.clone(),
                        source_span: lexical.source_span,
                    },
                    &allocator,
                )),
                &allocator,
            ));
        }
        IrExpression::Ast(ast_expr) if ast_expression_contains_dollar_event(ast_expr) => {
            found.set(true);
        }
        IrExpression::ExpressionRef(id)
            if ast_expression_contains_dollar_event(expressions.get(*id)) =>
        {
            found.set(true);
        }
        _ => {}
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
    transform_dollar_event_create(&mut job.root.create, expressions, job.allocator);
}

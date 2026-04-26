//! Track function optimization phase.
//!
//! `track` functions in `for` repeaters can sometimes be "optimized,"
//! i.e. transformed into inline expressions, in lieu of an external function call.
//! For example, tracking by `$index` can be optimized into an inline `trackByIndex`
//! reference.
//!
//! This phase checks track expressions for optimizable cases.
//!
//! Ported from Angular's `template/pipeline/src/phases/track_fn_optimization.ts`.

use oxc_str::Ident;

use crate::ir::expression::{
    IrExpression, TrackContextExpr, VisitorContextFlag, transform_expressions_in_expression,
};
use crate::ir::ops::{CreateOp, StatementOp, UpdateOp, UpdateOpBase, XrefId};
use crate::output::ast::{OutputExpression, OutputStatement, ReturnStatement, WrappedIrExpr};
use crate::pipeline::compilation::ComponentCompilationJob;
use crate::r3::Identifiers;

/// Optimizes @for track functions.
///
/// This phase:
/// 1. Identifies track expressions that can be optimized
/// 2. Replaces `$index` tracking with built-in `repeaterTrackByIndex`
/// 3. Replaces `$item` tracking with built-in `repeaterTrackByIdentity`
/// 4. For other expressions, transforms ContextExpr to TrackContextExpr
pub fn optimize_track_fns(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;

    // Get a pointer to the expression store for looking up ExpressionRef values
    let expression_store_ptr =
        &job.expressions as *const crate::pipeline::expression_store::ExpressionStore<'_>;

    // Collect view xrefs
    let view_xrefs: std::vec::Vec<_> = job.all_views().map(|v| v.xref).collect();

    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            let view_xref = view.xref;
            for op in view.create.iter_mut() {
                if let CreateOp::RepeaterCreate(rep) = op {
                    // SAFETY: We're only reading from expression_store, not modifying it
                    let expressions = unsafe { &*expression_store_ptr };
                    optimize_track_expression(allocator, rep, root_xref, view_xref, expressions);
                }
            }
        }
    }
}

/// Optimize a repeater's track expression.
fn optimize_track_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    rep: &mut crate::ir::ops::RepeaterCreateOp<'a>,
    root_xref: XrefId,
    view_xref: XrefId,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'a>,
) {
    // Check for simple $index or $item tracking
    if let Some(opt_fn) = check_simple_track_variable(&rep.track, expressions) {
        // Set the optimized track function name
        rep.track_fn_name = Some(Ident::from(opt_fn));
        return;
    }

    // Check for method call pattern: this.fn($index) or this.fn($index, $item)
    // These can be passed directly to the repeater runtime.
    if let Some((method_name, is_root_context)) =
        check_track_by_function_call(&rep.track, root_xref)
    {
        rep.uses_component_instance = true;
        if is_root_context && view_xref == root_xref {
            // Method is on the component context in the root view
            // Emit as ctx.methodName
            let fn_name = format!("ctx.{}", method_name);
            let name_str = allocator.alloc_str(&fn_name);
            rep.track_fn_name = Some(Ident::from(name_str));
        } else {
            // Need to use componentInstance() to access the method
            // Create the full function reference string
            let fn_name = format!("{}().{}", Identifiers::COMPONENT_INSTANCE, method_name);
            let name_str = allocator.alloc_str(&fn_name);
            rep.track_fn_name = Some(Ident::from(name_str));
        }
        return;
    }

    // Note: Angular does NOT optimize bare property reads like `track trackByFn` into
    // direct references (e.g., `ctx.trackByFn`). Only `$index`, `$item`, and
    // `fn($index[, $item])` call patterns are optimized. Bare property reads fall through
    // to the non-optimizable case below, which creates a wrapper function like:
    //   `function _forTrack($index,$item) { return this.trackByFn; }`

    // The track function could not be optimized.
    // Replace context reads with TrackContextExpr, since context reads in a track
    // function are emitted specially (as `this` instead of `ctx`).
    //
    // Following Angular's implementation (track_fn_optimization.ts:54-70), we set
    // usesComponentInstance inside the transform callback when a ContextExpr is found.
    // This is the authoritative detection — transformExpressionsInExpression traverses
    // all expression variants, so no ContextExpr can be missed regardless of nesting.
    //
    // By phase 34 (this phase), resolve_names (phase 31) has already converted all
    // ImplicitReceiver AST nodes into Context IR expressions, so checking for Context
    // during the transform is sufficient.
    let found_context = std::cell::Cell::new(false);
    transform_expressions_in_expression(
        &mut rep.track,
        &|expr, _flags| {
            if let IrExpression::Context(ctx) = expr {
                found_context.set(true);
                *expr = IrExpression::TrackContext(oxc_allocator::Box::new_in(
                    TrackContextExpr { view: ctx.view, source_span: None },
                    allocator,
                ));
            }
        },
        VisitorContextFlag::NONE,
    );
    if found_context.get() {
        rep.uses_component_instance = true;
    }

    // Also create an op list for the tracking expression since it may need
    // additional ops when generating the final code (e.g. temporary variables).
    // TypeScript: const trackOpList = new ir.OpList<ir.UpdateOp>();
    // trackOpList.push(ir.createStatementOp(new o.ReturnStatement(op.track, op.track.sourceSpan)));
    // op.trackByOps = trackOpList;

    // Convert the track IR expression to output expression for the return statement
    // The track expression needs to be wrapped in a return statement
    // We wrap the IR expression in WrappedIrNode so it will be resolved during reify
    let track_output = OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
        WrappedIrExpr {
            node: oxc_allocator::Box::new_in(rep.track.clone_in(allocator), allocator),
            source_span: None,
        },
        allocator,
    ));

    let return_stmt = OutputStatement::Return(oxc_allocator::Box::new_in(
        ReturnStatement { value: track_output, source_span: None },
        allocator,
    ));

    let statement_op =
        UpdateOp::Statement(StatementOp { base: UpdateOpBase::default(), statement: return_stmt });

    let mut track_by_ops = oxc_allocator::Vec::new_in(allocator);
    track_by_ops.push(statement_op);
    rep.track_by_ops = Some(track_by_ops);
}

/// Check if the track expression is a simple variable read of $index or $item.
fn check_simple_track_variable(
    track: &IrExpression<'_>,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'_>,
) -> Option<&'static str> {
    // Check for LexicalRead of $index or $item
    if let IrExpression::LexicalRead(lr) = track {
        match lr.name.as_str() {
            "$index" => return Some(Identifiers::REPEATER_TRACK_BY_INDEX),
            "$item" => return Some(Identifiers::REPEATER_TRACK_BY_IDENTITY),
            _ => {}
        }
    }

    // Check for OutputExpr(ReadVar) - this is the form after track_variables phase
    // transforms the loop variable to $item or $index
    if let IrExpression::OutputExpr(output) = track {
        if let crate::output::ast::OutputExpression::ReadVar(rv) = output.as_ref() {
            match rv.name.as_str() {
                "$index" => return Some(Identifiers::REPEATER_TRACK_BY_INDEX),
                "$item" => return Some(Identifiers::REPEATER_TRACK_BY_IDENTITY),
                _ => {}
            }
        }
    }

    // Check for ExpressionRef pointing to a simple variable
    if let IrExpression::ExpressionRef(id) = track {
        let stored_expr = expressions.get(*id);
        return check_ast_for_simple_track_variable(stored_expr);
    }

    // Check for AST PropertyRead of $index or $item on implicit receiver
    if let IrExpression::Ast(ast) = track {
        return check_ast_for_simple_track_variable(ast.as_ref());
    }

    None
}

/// Check if an AST expression is a simple $index or $item variable read.
fn check_ast_for_simple_track_variable(
    ast: &crate::ast::expression::AngularExpression<'_>,
) -> Option<&'static str> {
    if let crate::ast::expression::AngularExpression::PropertyRead(pr) = ast {
        if matches!(&pr.receiver, crate::ast::expression::AngularExpression::ImplicitReceiver(_)) {
            match pr.name.as_str() {
                "$index" => return Some(Identifiers::REPEATER_TRACK_BY_INDEX),
                "$item" => return Some(Identifiers::REPEATER_TRACK_BY_IDENTITY),
                _ => {}
            }
        }
    }
    None
}

/// Check if the track expression is a function call pattern: `this.fn($index)` or `this.fn($index, $item)`.
///
/// These patterns can be optimized by passing the method reference directly to the repeater.
/// Returns the method name (as String) and whether the context is the root view.
fn check_track_by_function_call(
    track: &IrExpression<'_>,
    root_xref: XrefId,
) -> Option<(String, bool)> {
    // Handle ResolvedCall expressions (created by resolveNames phase)
    if let IrExpression::ResolvedCall(rc) = track {
        // Must have 1 or 2 arguments
        if rc.args.is_empty() || rc.args.len() > 2 {
            return None;
        }

        // Check arguments: Angular only optimizes these specific patterns to pass the method reference directly:
        // 1. trackByFn($index) - single index argument (runtime passes index as first param)
        // 2. trackByFn($index, $item) - both arguments in that order (matches runtime signature)
        //
        // Other patterns like trackByFn($item) need a wrapper function because the runtime
        // signature is (index, item) but the method only takes (item).
        let args_valid = if rc.args.len() == 1 {
            // Single argument: must be $index only
            is_index_variable(&rc.args[0])
        } else {
            // Two arguments: must be ($index, $item) in that order
            is_index_variable(&rc.args[0]) && is_item_variable(&rc.args[1])
        };

        if !args_valid {
            return None;
        }

        // Check receiver: must be a ResolvedPropertyRead on Context whose view is the root view.
        // Angular's isTrackByFunctionCall (track_fn_optimization.ts:96-100) requires
        // receiver.receiver.view === rootView, rejecting non-root-view contexts.
        if let IrExpression::ResolvedPropertyRead(rp) = rc.receiver.as_ref() {
            if let IrExpression::Context(ctx) = rp.receiver.as_ref() {
                if ctx.view == root_xref {
                    return Some((rp.name.to_string(), true));
                }
            }
        }

        return None;
    }

    // AST fallback path: After resolve_names (phase 31), track expressions should already
    // be ResolvedCall/ResolvedPropertyRead with Context receivers. If we reach here with
    // raw AST expressions, ImplicitReceiver lacks a view field so we cannot verify the
    // root-view guard that Angular's isTrackByFunctionCall requires. Reject optimization
    // to avoid mis-optimizing nested-view track calls.
    None
}

/// Check if an expression is the $index variable.
fn is_index_variable(expr: &IrExpression<'_>) -> bool {
    match expr {
        IrExpression::LexicalRead(lr) => lr.name.as_str() == "$index",
        IrExpression::OutputExpr(output) => {
            if let crate::output::ast::OutputExpression::ReadVar(rv) = output.as_ref() {
                rv.name.as_str() == "$index"
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Check if an expression is the $item variable.
fn is_item_variable(expr: &IrExpression<'_>) -> bool {
    match expr {
        IrExpression::LexicalRead(lr) => lr.name.as_str() == "$item",
        IrExpression::OutputExpr(output) => {
            if let crate::output::ast::OutputExpression::ReadVar(rv) = output.as_ref() {
                rv.name.as_str() == "$item"
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::expression::{ContextExpr, ResolvedCallExpr, ResolvedPropertyReadExpr};
    use crate::ir::ops::XrefId;
    use crate::output::ast::ReadVarExpr;
    use oxc_allocator::Allocator;

    /// Build a ResolvedCall IR expression representing `ctx.methodName($index)`,
    /// where the Context has the given `context_view`.
    fn make_track_method_call<'a>(
        alloc: &'a Allocator,
        method_name: &'a str,
        context_view: XrefId,
    ) -> IrExpression<'a> {
        let ctx = IrExpression::Context(oxc_allocator::Box::new_in(
            ContextExpr { view: context_view, source_span: None },
            alloc,
        ));
        let prop_read = IrExpression::ResolvedPropertyRead(oxc_allocator::Box::new_in(
            ResolvedPropertyReadExpr {
                receiver: oxc_allocator::Box::new_in(ctx, alloc),
                name: Ident::from(method_name),
                source_span: None,
            },
            alloc,
        ));
        let index_arg = IrExpression::OutputExpr(oxc_allocator::Box::new_in(
            OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                ReadVarExpr { name: Ident::from("$index"), source_span: None },
                alloc,
            )),
            alloc,
        ));
        let mut args = oxc_allocator::Vec::new_in(alloc);
        args.push(index_arg);
        IrExpression::ResolvedCall(oxc_allocator::Box::new_in(
            ResolvedCallExpr {
                receiver: oxc_allocator::Box::new_in(prop_read, alloc),
                args,
                source_span: None,
            },
            alloc,
        ))
    }

    #[test]
    fn test_root_view_context_is_accepted() {
        let alloc = Allocator::default();
        let root_xref = XrefId(0);
        let track = make_track_method_call(&alloc, "trackByFn", root_xref);

        let result = check_track_by_function_call(&track, root_xref);
        assert_eq!(result, Some(("trackByFn".to_string(), true)));
    }

    #[test]
    fn test_non_root_view_context_is_rejected() {
        let alloc = Allocator::default();
        let root_xref = XrefId(0);
        let non_root_xref = XrefId(5);
        let track = make_track_method_call(&alloc, "trackByFn", non_root_xref);

        // Before the fix, this would return Some — incorrectly accepting a
        // non-root context. After the fix, it returns None.
        let result = check_track_by_function_call(&track, root_xref);
        assert_eq!(result, None, "non-root context should NOT be optimized");
    }
}

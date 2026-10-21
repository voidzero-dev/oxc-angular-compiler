//! Any cast deletion phase.
//!
//! Find any function calls to `$any`, excluding `this.$any`, and delete them,
//! since they have no runtime effects.
//!
//! Ported from Angular's `template/pipeline/src/phases/any_cast.ts`.

use crate::ir::expression::{
    IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Removes $any() casts from template expressions.
///
/// The `$any()` function is a template type-cast that has no runtime effect.
/// This phase removes these calls, replacing them with their single argument.
///
/// Example:
/// - `$any(value)` → `value`
/// - `$any(obj.prop)` → `obj.prop`
pub fn delete_any_casts(job: &mut ComponentCompilationJob<'_>) {
    for view in job.all_views_mut() {
        // Process create operations
        for op in view.create.iter_mut() {
            transform_expressions_in_create_op(
                op,
                &|expr, _flags| remove_any_cast(expr),
                VisitorContextFlag::NONE,
            );
        }

        // Process update operations
        for op in view.update.iter_mut() {
            transform_expressions_in_update_op(
                op,
                &|expr, _flags| remove_any_cast(expr),
                VisitorContextFlag::NONE,
            );
        }
    }
}

/// Remove $any() calls by replacing them with their argument.
///
/// Matches calls of the form `$any(arg)` where:
/// - The receiver is a LexicalRead with name "$any" (implicit receiver, not `this.$any`)
/// - There is exactly one argument
///
/// This matches TypeScript's any_cast.ts which checks:
/// ```typescript
/// e instanceof o.InvokeFunctionExpr &&
/// e.fn instanceof ir.LexicalReadExpr &&
/// e.fn.name === '$any'
/// ```
fn remove_any_cast(expr: &mut IrExpression<'_>) {
    // Check if this is a ResolvedCall with LexicalRead("$any") as the receiver
    // This is how $any(arg) is represented after convert_ast_to_ir:
    // - $any on implicit receiver becomes LexicalRead("$any")
    // - The call becomes ResolvedCall(receiver: LexicalRead("$any"), args: [...])
    let IrExpression::ResolvedCall(call) = expr else { return };
    let is_any_cast = matches!(
        call.receiver.as_ref(),
        IrExpression::LexicalRead(lexical_read) if lexical_read.name.as_str() == "$any"
    );
    // If args.len() != 1, leave as-is (will be caught by type checker)
    if !is_any_cast || call.args.len() != 1 {
        return;
    }
    // This is a $any(arg) call - move the argument out and put it in the call's place.
    let Some(arg) = call.args.pop() else { return };
    *expr = arg;
    // Note: We only handle ResolvedCall with LexicalRead("$any") as receiver.
    // Standalone LexicalRead("$any") should not be transformed - it's the receiver
    // that the traversal visits before visiting the call. Only the call itself
    // (where we can see both receiver and args) should be replaced.
}

/// Deletes any casts for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn delete_any_casts_for_host(job: &mut HostBindingCompilationJob<'_>) {
    // Process create operations
    for op in job.root.create.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| remove_any_cast(expr),
            VisitorContextFlag::NONE,
        );
    }

    // Process update operations
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| remove_any_cast(expr),
            VisitorContextFlag::NONE,
        );
    }
}

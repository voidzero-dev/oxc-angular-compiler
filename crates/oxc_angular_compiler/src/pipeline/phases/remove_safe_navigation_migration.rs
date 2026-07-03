//! Remove `$safeNavigationMigration(...)` markers phase.
//!
//! Finds calls to the `$safeNavigationMigration` builtin and replaces them with a
//! [`SafeNavigationMigrationExpr`] wrapper around the single argument, so that the
//! argument's safe reads later expand under legacy (`== null ? null`) semantics
//! even when the compilation targets native optional chaining. The wrapper is
//! unwrapped by the `expandSafeReads` phase.
//!
//! Like Angular, detection keys on the *unqualified* helper: the call receiver must
//! be a bare `LexicalRead("$safeNavigationMigration")`, never a property on some
//! object (`svc.$safeNavigationMigration(...)`, which is a legitimate user call).
//! Because this phase runs before `resolveNames` — the same point Angular runs it,
//! right after `deleteAnyCasts` — the helper is still an unresolved `LexicalRead`,
//! which only exists for unqualified references.
//!
//! Ported from Angular's `template/pipeline/src/phases/safe_navigation_migration.ts`.

use oxc_allocator::Box as ArenaBox;

use crate::ir::expression::{
    EmptyExpr, IrExpression, SafeNavigationMigrationExpr, VisitorContextFlag,
    transform_expressions_in_create_op, transform_expressions_in_update_op,
};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Replaces `$safeNavigationMigration(arg)` calls with a `SafeNavigationMigration`
/// wrapper around `arg`.
pub fn remove_safe_navigation_migration(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();
    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            for op in view.create.iter_mut() {
                transform_expressions_in_create_op(
                    op,
                    &|expr, _flags| convert_marker(expr, &allocator),
                    VisitorContextFlag::NONE,
                );
            }
            for op in view.update.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, _flags| convert_marker(expr, &allocator),
                    VisitorContextFlag::NONE,
                );
            }
        }
    }
}

/// Convert a single `$safeNavigationMigration(arg)` call into a wrapper node.
///
/// Matches calls where:
/// - the receiver is a `LexicalRead` named `$safeNavigationMigration` (the
///   unqualified builtin on the implicit receiver, not `obj.$safeNavigationMigration`)
/// - there is exactly one argument
///
/// Mirrors Angular's `convertSafeNavigationMigrationCall`, which checks
/// `e.fn instanceof ir.LexicalReadExpr && e.fn.name === '$safeNavigationMigration'`.
/// A call with the wrong argument count is left untouched (surfaced later by the
/// type checker) rather than rewritten.
fn convert_marker<'a>(expr: &mut IrExpression<'a>, allocator: &'a oxc_allocator::Allocator) {
    let is_marker = match expr {
        IrExpression::ResolvedCall(call) => {
            call.args.len() == 1
                && matches!(
                    call.receiver.as_ref(),
                    IrExpression::LexicalRead(lr)
                        if lr.name.as_str() == "$safeNavigationMigration"
                )
        }
        _ => false,
    };
    if !is_marker {
        return;
    }

    let IrExpression::ResolvedCall(call) = expr else { return };
    let source_span = call.source_span;
    // Take the single argument out; the surrounding `ResolvedCall` is dropped when we
    // overwrite `*expr` below, so the placeholder left behind is never observed.
    let arg = std::mem::replace(
        &mut call.args[0],
        IrExpression::Empty(ArenaBox::new_in(EmptyExpr { source_span: None }, &allocator)),
    );
    *expr = IrExpression::SafeNavigationMigration(ArenaBox::new_in(
        SafeNavigationMigrationExpr { expr: ArenaBox::new_in(arg, &allocator), source_span },
        &allocator,
    ));
}

/// Host-binding variant — only processes the root unit (no embedded views).
pub fn remove_safe_navigation_migration_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    for op in job.root.create.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| convert_marker(expr, &allocator),
            VisitorContextFlag::NONE,
        );
    }
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| convert_marker(expr, &allocator),
            VisitorContextFlag::NONE,
        );
    }
}

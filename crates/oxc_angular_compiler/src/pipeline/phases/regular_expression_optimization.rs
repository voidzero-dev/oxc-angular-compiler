//! Regular expression optimization phase.
//!
//! Optimizes regular expression literals by hoisting non-global regex expressions
//! to shared constants. Global regex expressions (with 'g' flag) cannot be optimized
//! because they maintain internal state.
//!
//! Ported from Angular's `template/pipeline/src/phases/regular_expression_optimization.ts`.

use oxc_allocator::Box;
use oxc_str::Ident;

use crate::ast::expression::AngularExpression;
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::output::ast::{OutputExpression, ReadVarExpr};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Optimizes regular expression literals for runtime.
///
/// This phase identifies regex literals that don't have the global ('g') flag,
/// pools them as shared constants, and replaces the original expressions with
/// references to the pooled constants.
///
/// Global regexes (with 'g' flag) cannot be optimized because they maintain
/// internal state (lastIndex property).
///
/// Ported from Angular's `optimizeRegularExpressions` phase.
pub fn optimize_regular_expressions(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();

    // First pass: Pool regexes and collect their names for replacement
    // We need to collect regex info and pool them first, then do the replacement
    let mut regex_replacements: Vec<(String, Option<String>, Ident<'_>)> = Vec::new();

    // Collect and pool all non-global regexes from root view using split borrow
    {
        let ComponentCompilationJob { pool, root, .. } = job;
        for op in root.create.iter() {
            collect_and_pool_regexes_from_create_op(op, pool, &mut regex_replacements);
        }
        for op in root.update.iter() {
            collect_and_pool_regexes_from_update_op(op, pool, &mut regex_replacements);
        }
    }

    // Collect and pool from embedded views using split borrow
    for xref in &view_xrefs {
        let ComponentCompilationJob { pool, views, .. } = job;
        if let Some(view) = views.get(xref) {
            for op in view.create.iter() {
                collect_and_pool_regexes_from_create_op(op, pool, &mut regex_replacements);
            }
            for op in view.update.iter() {
                collect_and_pool_regexes_from_update_op(op, pool, &mut regex_replacements);
            }
        }
    }

    // Second pass: Replace regex expressions with references to pooled constants
    // Process root view
    {
        for op in job.root.create.iter_mut() {
            transform_regexes_in_create_op(op, allocator, &regex_replacements);
        }
        for op in job.root.update.iter_mut() {
            transform_regexes_in_update_op(op, allocator, &regex_replacements);
        }
    }

    // Process embedded views
    for xref in view_xrefs {
        if xref == root_xref {
            continue;
        }
        if let Some(view) = job.views.get_mut(&xref) {
            for op in view.create.iter_mut() {
                transform_regexes_in_create_op(op, allocator, &regex_replacements);
            }
            for op in view.update.iter_mut() {
                transform_regexes_in_update_op(op, allocator, &regex_replacements);
            }
        }
    }
}

/// Check if regex flags contain the global flag.
fn is_global_regex(flags: Option<&Ident<'_>>) -> bool {
    flags.map_or(false, |f| f.contains('g'))
}

/// Make a regex key for lookup/deduplication.
fn make_regex_key(body: &str, flags: Option<&str>) -> String {
    format!("/{}/{}", body, flags.unwrap_or(""))
}

/// Collect and pool regexes from a create operation.
fn collect_and_pool_regexes_from_create_op<'a>(
    op: &CreateOp<'a>,
    pool: &mut crate::pipeline::constant_pool::ConstantPool<'a>,
    replacements: &mut Vec<(String, Option<String>, Ident<'a>)>,
) {
    match op {
        CreateOp::Variable(var) => {
            collect_and_pool_from_expr(&var.initializer, pool, replacements);
        }
        CreateOp::Listener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                collect_and_pool_regexes_from_update_op(handler_op, pool, replacements);
            }
        }
        CreateOp::TwoWayListener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                collect_and_pool_regexes_from_update_op(handler_op, pool, replacements);
            }
        }
        CreateOp::AnimationListener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                collect_and_pool_regexes_from_update_op(handler_op, pool, replacements);
            }
        }
        CreateOp::Animation(animation) => {
            for handler_op in animation.handler_ops.iter() {
                collect_and_pool_regexes_from_update_op(handler_op, pool, replacements);
            }
        }
        _ => {}
    }
}

/// Collect and pool regexes from an update operation.
fn collect_and_pool_regexes_from_update_op<'a>(
    op: &UpdateOp<'a>,
    pool: &mut crate::pipeline::constant_pool::ConstantPool<'a>,
    replacements: &mut Vec<(String, Option<String>, Ident<'a>)>,
) {
    match op {
        UpdateOp::Property(prop) => {
            collect_and_pool_from_expr(&prop.expression, pool, replacements);
        }
        UpdateOp::StyleProp(style) => {
            collect_and_pool_from_expr(&style.expression, pool, replacements);
        }
        UpdateOp::ClassProp(class) => {
            collect_and_pool_from_expr(&class.expression, pool, replacements);
        }
        UpdateOp::Binding(bind) => {
            collect_and_pool_from_expr(&bind.expression, pool, replacements);
        }
        UpdateOp::Variable(var) => {
            collect_and_pool_from_expr(&var.initializer, pool, replacements);
        }
        UpdateOp::InterpolateText(interp) => {
            collect_and_pool_from_expr(&interp.interpolation, pool, replacements);
        }
        _ => {}
    }
}

/// Collect and pool regexes from an expression.
fn collect_and_pool_from_expr<'a>(
    expr: &IrExpression<'a>,
    pool: &mut crate::pipeline::constant_pool::ConstantPool<'a>,
    replacements: &mut Vec<(String, Option<String>, Ident<'a>)>,
) {
    if let IrExpression::Ast(ast_expr) = expr {
        if let AngularExpression::RegularExpressionLiteral(regex) = ast_expr.as_ref() {
            if !is_global_regex(regex.flags.as_ref()) {
                let body = regex.body.to_string();
                let flags = regex.flags.as_ref().map(|f| f.to_string());

                // Pool the regex and get the constant index
                let index = pool.pool_regex(&body, flags.as_deref());

                // Get the constant name from the pool
                if let Some(constant) = pool.get(index) {
                    let key = make_regex_key(&body, flags.as_deref());
                    // Only add if not already in replacements
                    if !replacements.iter().any(|(b, f, _)| make_regex_key(b, f.as_deref()) == key)
                    {
                        replacements.push((body, flags, constant.name.clone()));
                    }
                }
            }
        }
    }
}

/// Transform regexes in a create operation, replacing with pooled references.
fn transform_regexes_in_create_op<'a>(
    op: &mut CreateOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    replacements: &[(String, Option<String>, Ident<'a>)],
) {
    match op {
        CreateOp::Variable(var) => {
            transform_expr(&mut var.initializer, allocator, replacements);
        }
        CreateOp::Listener(listener) => {
            for handler_op in listener.handler_ops.iter_mut() {
                transform_regexes_in_update_op(handler_op, allocator, replacements);
            }
        }
        CreateOp::TwoWayListener(listener) => {
            for handler_op in listener.handler_ops.iter_mut() {
                transform_regexes_in_update_op(handler_op, allocator, replacements);
            }
        }
        CreateOp::AnimationListener(listener) => {
            for handler_op in listener.handler_ops.iter_mut() {
                transform_regexes_in_update_op(handler_op, allocator, replacements);
            }
        }
        CreateOp::Animation(animation) => {
            for handler_op in animation.handler_ops.iter_mut() {
                transform_regexes_in_update_op(handler_op, allocator, replacements);
            }
        }
        _ => {}
    }
}

/// Transform regexes in an update operation, replacing with pooled references.
fn transform_regexes_in_update_op<'a>(
    op: &mut UpdateOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    replacements: &[(String, Option<String>, Ident<'a>)],
) {
    match op {
        UpdateOp::Property(prop) => {
            transform_expr(&mut prop.expression, allocator, replacements);
        }
        UpdateOp::StyleProp(style) => {
            transform_expr(&mut style.expression, allocator, replacements);
        }
        UpdateOp::ClassProp(class) => {
            transform_expr(&mut class.expression, allocator, replacements);
        }
        UpdateOp::Binding(bind) => {
            transform_expr(&mut bind.expression, allocator, replacements);
        }
        UpdateOp::Variable(var) => {
            transform_expr(&mut var.initializer, allocator, replacements);
        }
        UpdateOp::InterpolateText(interp) => {
            transform_expr(&mut interp.interpolation, allocator, replacements);
        }
        _ => {}
    }
}

/// Transform an expression, replacing regex literals with references to pooled constants.
fn transform_expr<'a>(
    expr: &mut IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    replacements: &[(String, Option<String>, Ident<'a>)],
) {
    if let IrExpression::Ast(ast_expr) = expr {
        if let AngularExpression::RegularExpressionLiteral(regex) = ast_expr.as_ref() {
            if !is_global_regex(regex.flags.as_ref()) {
                let key =
                    make_regex_key(regex.body.as_str(), regex.flags.as_ref().map(|f| f.as_str()));

                // Find the replacement constant name
                if let Some((_, _, name)) =
                    replacements.iter().find(|(b, f, _)| make_regex_key(b, f.as_deref()) == key)
                {
                    // Replace with an OutputExpr that references the constant
                    *expr = IrExpression::OutputExpr(Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: name.clone(), source_span: None },
                            allocator,
                        )),
                        allocator,
                    ));
                }
            }
        }
    }
}

/// Optimizes regular expression literals for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn optimize_regular_expressions_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    let mut regex_replacements: Vec<(String, Option<String>, Ident<'_>)> = Vec::new();

    // First pass: Pool regexes from root unit
    {
        let HostBindingCompilationJob { pool, root, .. } = job;
        for op in root.create.iter() {
            collect_and_pool_regexes_from_create_op(op, pool, &mut regex_replacements);
        }
        for op in root.update.iter() {
            collect_and_pool_regexes_from_update_op(op, pool, &mut regex_replacements);
        }
    }

    // Second pass: Replace regex expressions with references to pooled constants
    for op in job.root.create.iter_mut() {
        transform_regexes_in_create_op(op, allocator, &regex_replacements);
    }
    for op in job.root.update.iter_mut() {
        transform_regexes_in_update_op(op, allocator, &regex_replacements);
    }
}

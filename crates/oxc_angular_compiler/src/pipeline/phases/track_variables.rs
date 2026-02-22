//! Track variables phase.
//!
//! Inside the `track` expression on a `for` repeater, the `$index` and `$item` variables are
//! ambiently available. In this phase, we find those variable usages, and replace them with the
//! appropriate output read.
//!
//! Ported from Angular's `template/pipeline/src/phases/track_variables.ts`.

use oxc_allocator::Box;
use oxc_span::Atom;

use crate::ast::expression::AngularExpression;
use crate::ir::expression::{
    IrExpression, ResolvedPropertyReadExpr, VisitorContextFlag, transform_expressions_in_expression,
};
use crate::ir::ops::CreateOp;
use crate::output::ast::{OutputExpression, ReadVarExpr};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Transforms track expressions in @for loops to resolve $index and $item references.
///
/// This phase:
/// 1. Finds all RepeaterCreate ops
/// 2. Transforms their track expressions
/// 3. Replaces LexicalRead($index) with o.variable('$index')
/// 4. Replaces LexicalRead(item_name) with o.variable('$item')
pub fn generate_track_variables(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // We need to borrow expression_store immutably while modifying views
    let expression_store_ptr =
        &job.expressions as *const crate::pipeline::expression_store::ExpressionStore<'_>;

    // Process each view's create operations
    for view in job.all_views_mut() {
        for op in view.create.iter_mut() {
            if let CreateOp::RepeaterCreate(rep) = op {
                // Get $index names and $implicit name for this repeater
                let index_names: Vec<Atom<'_>> = {
                    let mut names: Vec<Atom<'_>> = rep.var_names.index.iter().cloned().collect();
                    // Also include '$index' itself
                    names.push(Atom::from("$index"));
                    names
                };
                let implicit_name = rep.var_names.item.clone();

                // Transform the track expression
                // SAFETY: We're only reading from expression_store, not modifying it
                let expressions = unsafe { &*expression_store_ptr };
                transform_track_expression(
                    allocator,
                    &mut rep.track,
                    &index_names,
                    implicit_name.as_ref(),
                    expressions,
                );
            }
        }
    }
}

/// Transform the track expression to resolve $index and $implicit references.
fn transform_track_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &mut Box<'a, IrExpression<'a>>,
    index_names: &[Atom<'a>],
    implicit_name: Option<&Atom<'a>>,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'a>,
) {
    let index_names_clone: Vec<Atom<'a>> = index_names.to_vec();
    let implicit_name_clone: Option<Atom<'a>> = implicit_name.cloned();

    transform_expressions_in_expression(
        expr,
        &|inner_expr, _flags| {
            // Handle LexicalRead expressions
            if let IrExpression::LexicalRead(lexical) = inner_expr {
                // Check if this is an $index reference
                if index_names_clone.iter().any(|n| *n == lexical.name) {
                    // Replace with o.variable('$index')
                    *inner_expr = IrExpression::OutputExpr(Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from("$index"), source_span: None },
                            allocator,
                        )),
                        allocator,
                    ));
                    return;
                }

                // Check if this is the $implicit (item) reference
                if let Some(ref item_name) = implicit_name_clone {
                    if lexical.name == *item_name {
                        // Replace with o.variable('$item')
                        *inner_expr = IrExpression::OutputExpr(Box::new_in(
                            OutputExpression::ReadVar(Box::new_in(
                                ReadVarExpr { name: Atom::from("$item"), source_span: None },
                                allocator,
                            )),
                            allocator,
                        ));
                        return;
                    }
                }

                // Also check for $implicit directly
                if lexical.name.as_str() == "$implicit" {
                    *inner_expr = IrExpression::OutputExpr(Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from("$item"), source_span: None },
                            allocator,
                        )),
                        allocator,
                    ));
                    return;
                }
            }

            // Handle Ast expressions that contain PropertyRead
            if let IrExpression::Ast(ast_expr) = inner_expr {
                // Try to transform the entire AST expression, handling nested property reads
                if let Some(transformed) = transform_angular_expression_for_track(
                    allocator,
                    ast_expr.as_ref(),
                    &index_names_clone,
                    implicit_name_clone.as_ref(),
                ) {
                    *inner_expr = transformed;
                    return;
                }
            }

            // Handle ExpressionRef by looking up the stored expression
            if let IrExpression::ExpressionRef(id) = inner_expr {
                let stored_expr = expressions.get(*id);
                // Try to transform the stored expression, handling nested property reads
                if let Some(transformed) = transform_angular_expression_for_track(
                    allocator,
                    stored_expr,
                    &index_names_clone,
                    implicit_name_clone.as_ref(),
                ) {
                    *inner_expr = transformed;
                    return;
                }
            }
        },
        VisitorContextFlag::NONE,
    );
}

/// Transform an Angular expression for track, handling nested property reads.
///
/// This handles cases like `item.title` where `item` is the loop variable:
/// - `PropertyRead(ImplicitReceiver, "item")` -> `$item`
/// - `PropertyRead(PropertyRead(ImplicitReceiver, "item"), "title")` -> `$item.title`
fn transform_angular_expression_for_track<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &AngularExpression<'a>,
    index_names: &[Atom<'a>],
    implicit_name: Option<&Atom<'a>>,
) -> Option<IrExpression<'a>> {
    match expr {
        // Direct read of loop variable: `item` or `$index`
        AngularExpression::PropertyRead(prop_read)
            if matches!(prop_read.receiver, AngularExpression::ImplicitReceiver(_)) =>
        {
            let name = &prop_read.name;

            // Check if this is an $index reference
            if index_names.iter().any(|n| *n == *name) {
                return Some(IrExpression::OutputExpr(Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Atom::from("$index"), source_span: None },
                        allocator,
                    )),
                    allocator,
                )));
            }

            // Check if this is the loop variable (item) reference
            if let Some(item_name) = implicit_name {
                if *name == *item_name {
                    return Some(IrExpression::OutputExpr(Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from("$item"), source_span: None },
                            allocator,
                        )),
                        allocator,
                    )));
                }
            }

            // Check for $implicit directly
            if name.as_str() == "$implicit" {
                return Some(IrExpression::OutputExpr(Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Atom::from("$item"), source_span: None },
                        allocator,
                    )),
                    allocator,
                )));
            }

            None
        }

        // Nested property read: `item.title` (PropertyRead on PropertyRead)
        AngularExpression::PropertyRead(prop_read) => {
            // Try to transform the receiver
            if let Some(transformed_receiver) = transform_angular_expression_for_track(
                allocator,
                &prop_read.receiver,
                index_names,
                implicit_name,
            ) {
                // Build a resolved property read with the transformed receiver
                return Some(IrExpression::ResolvedPropertyRead(Box::new_in(
                    ResolvedPropertyReadExpr {
                        receiver: Box::new_in(transformed_receiver, allocator),
                        name: prop_read.name.clone(),
                        source_span: None,
                    },
                    allocator,
                )));
            }
            None
        }

        // Other expressions are not transformed at this level
        _ => None,
    }
}

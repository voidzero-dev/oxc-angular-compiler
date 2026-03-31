//! Pure literal structures phase.
//!
//! Transforms literal arrays and objects with non-constant entries into
//! `PureFunctionExpr`. This optimization ensures that the structure of arrays/objects
//! is stable (same identity) across change detection runs, which is important for
//! avoiding unnecessary re-renders.
//!
//! For example:
//! ```text
//! [1, x, 'hello']
//! ↓
//! PureFunctionExpr { body: DerivedLiteralArray([Ast(1), PureFunctionParam(0), Ast('hello')]), args: [Ast(x)] }
//! ```
//!
//! At runtime, Angular's `pureFunction` will return the same array identity if `x`
//! hasn't changed, avoiding unnecessary downstream updates.
//!
//! Ported from Angular's `template/pipeline/src/phases/pure_literal_structures.ts`.

use crate::ast::expression::{AngularExpression, LiteralMap};
use crate::ir::expression::{
    DerivedLiteralArrayExpr, DerivedLiteralMapExpr, IrExpression, PureFunctionExpr,
    PureFunctionParameterExpr, VisitorContextFlag, clone_angular_expression,
    transform_expressions_in_update_op,
};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use crate::pipeline::expression_store::ExpressionStore;
use oxc_allocator::{Box as AllocBox, Vec as AllocVec};

/// Generates pure literal structure wrappers for arrays and objects.
///
/// This phase transforms literal arrays/maps that contain non-constant entries
/// into `PureFunctionExpr`. Constant entries remain as-is while non-constant
/// entries are replaced with `PureFunctionParameterExpr` references.
///
/// The optimization is only applied to update operations (not create), as the
/// purpose is to maintain identity stability during change detection.
pub fn generate_pure_literal_structures(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // We need to access the expression store while modifying views.
    // Use a raw pointer to avoid borrow checker issues (same pattern as resolve_names).
    let expression_store_ptr = &job.expressions as *const ExpressionStore<'_>;

    // Process all views
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            // SAFETY: We're only reading from expression_store, not modifying it
            let expressions = unsafe { &*expression_store_ptr };

            // Only process update operations - this optimization is for change detection
            for op in view.update.iter_mut() {
                transform_expressions_in_update_op(
                    op,
                    &|expr, flags| {
                        // Skip if we're in a child operation (already inside a PureFunction)
                        if flags.contains(VisitorContextFlag::IN_CHILD_OPERATION) {
                            return;
                        }

                        transform_literal_structure(expr, allocator, expressions);
                    },
                    VisitorContextFlag::NONE,
                );
            }
        }
    }
}

/// Transform a literal array or map into a PureFunction if it has non-constant entries.
fn transform_literal_structure<'a>(
    expr: &mut IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) {
    // First, determine if we need to transform and create the pure function if needed.
    // We do this in a separate step to avoid borrowing issues.
    let pure_fn_opt: Option<PureFunctionExpr<'a>> = match expr {
        IrExpression::Ast(ast_expr) => {
            try_create_pure_function_for_angular_expr(ast_expr.as_ref(), allocator)
        }
        IrExpression::ExpressionRef(id) => {
            let stored_expr = expressions.get(*id);
            try_create_pure_function_for_angular_expr(stored_expr, allocator)
        }
        // Handle IrExpression::LiteralArray - elements are already IR expressions
        // TypeScript always creates a PureFunction for literal arrays, even if all constant
        IrExpression::LiteralArray(arr) => {
            create_pure_function_for_ir_array(&arr.elements, allocator, expressions)
        }
        // Handle IrExpression::LiteralMap - values are already IR expressions
        // TypeScript always creates a PureFunction for literal maps, even if all constant
        IrExpression::LiteralMap(map) => create_pure_function_for_ir_map(
            &map.keys,
            &map.values,
            &map.quoted,
            allocator,
            expressions,
        ),
        // Handle IrExpression::DerivedLiteralArray - created by pipe_variadic phase
        // for variadic pipe arguments
        IrExpression::DerivedLiteralArray(arr) => {
            create_pure_function_for_derived_array(&arr.entries, allocator, expressions)
        }
        // Handle IrExpression::DerivedLiteralMap - created for variadic maps
        IrExpression::DerivedLiteralMap(map) => create_pure_function_for_ir_map(
            &map.keys,
            &map.values,
            &map.quoted,
            allocator,
            expressions,
        ),
        _ => None,
    };

    // Now apply the transformation if we created a pure function
    if let Some(pure_fn) = pure_fn_opt {
        *expr = IrExpression::PureFunction(AllocBox::new_in(pure_fn, allocator));
    }
}

/// Try to create a PureFunctionExpr for an AngularExpression if needed.
/// Returns Some(PureFunctionExpr) if the expression is a literal array/map with non-constant entries,
/// None otherwise.
fn try_create_pure_function_for_angular_expr<'a>(
    angular_expr: &AngularExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> Option<PureFunctionExpr<'a>> {
    match angular_expr {
        // TypeScript always creates a PureFunction for literal arrays, even if all constant
        AngularExpression::LiteralArray(arr) => {
            create_pure_function_for_array(&arr.expressions, allocator)
        }
        // TypeScript always creates a PureFunction for literal maps, even if all constant
        AngularExpression::LiteralMap(map) => create_pure_function_for_map(map, allocator),
        _ => None,
    }
}

/// Check if an expression is constant (can be evaluated at compile time).
fn is_constant_expression(expr: &AngularExpression<'_>) -> bool {
    match expr {
        // Literals are constant
        AngularExpression::LiteralPrimitive(_) => true,
        // Empty expression is constant
        AngularExpression::Empty(_) => true,
        // Nested arrays are constant if all elements are constant
        AngularExpression::LiteralArray(arr) => arr.expressions.iter().all(is_constant_expression),
        // Nested maps are constant if all values are constant
        AngularExpression::LiteralMap(map) => map.values.iter().all(is_constant_expression),
        // Everything else is non-constant (variables, function calls, etc.)
        _ => false,
    }
}

/// Check if an IR expression is constant (can be evaluated at compile time).
fn is_constant_ir_expression(expr: &IrExpression<'_>, expressions: &ExpressionStore<'_>) -> bool {
    match expr {
        // Ast wrapping a constant angular expression
        IrExpression::Ast(ast) => is_constant_expression(ast),
        // ExpressionRef - look up the referenced expression and check if it's constant
        IrExpression::ExpressionRef(id) => {
            let stored_expr = expressions.get(*id);
            is_constant_expression(stored_expr)
        }
        // Nested arrays are constant if all elements are constant
        IrExpression::LiteralArray(arr) => {
            arr.elements.iter().all(|e| is_constant_ir_expression(e, expressions))
        }
        // Nested maps are constant if all values are constant
        IrExpression::LiteralMap(map) => {
            map.values.iter().all(|v| is_constant_ir_expression(v, expressions))
        }
        // Derived literals contain IR expressions
        IrExpression::DerivedLiteralArray(arr) => {
            arr.entries.iter().all(|e| is_constant_ir_expression(e, expressions))
        }
        IrExpression::DerivedLiteralMap(map) => {
            map.values.iter().all(|v| is_constant_ir_expression(v, expressions))
        }
        // Everything else is non-constant (pipes, property reads, etc.)
        _ => false,
    }
}

/// Resolve ExpressionRef to actual expression for pure function body.
/// If the expression is an ExpressionRef, look up the referenced Angular expression
/// and wrap it in IrExpression::Ast for proper emission.
fn resolve_expression_for_body<'a>(
    expr: &IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) -> IrExpression<'a> {
    match expr {
        IrExpression::ExpressionRef(id) => {
            // Resolve the reference to the actual Angular expression and wrap in Ast
            let angular_expr = expressions.get(*id);
            let cloned = clone_angular_expression(angular_expr, allocator);
            IrExpression::Ast(AllocBox::new_in(cloned, allocator))
        }
        // For other expressions, just clone
        _ => expr.clone_in(allocator),
    }
}

/// Create a PureFunctionExpr for a derived literal array with IR expression entries.
/// This is used for variadic pipe arguments created by the pipe_variadic phase.
fn create_pure_function_for_derived_array<'a>(
    entries: &oxc_allocator::Vec<'a, IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) -> Option<PureFunctionExpr<'a>> {
    let mut args: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_entries: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut param_index: u32 = 0;

    for expr in entries.iter() {
        if is_constant_ir_expression(expr, expressions) {
            // Constant entry: resolve and clone the expression
            body_entries.push(resolve_expression_for_body(expr, allocator, expressions));
        } else {
            // Non-constant entry: add to args and replace with PureFunctionParameterExpr
            args.push(expr.clone_in(allocator));

            body_entries.push(IrExpression::PureFunctionParameter(AllocBox::new_in(
                PureFunctionParameterExpr { index: param_index, source_span: None },
                allocator,
            )));
            param_index += 1;
        }
    }

    // Create the derived array body
    // TypeScript always creates a PureFunction, even with 0 args (all constant)
    let body = IrExpression::DerivedLiteralArray(AllocBox::new_in(
        DerivedLiteralArrayExpr { entries: body_entries, source_span: None },
        allocator,
    ));

    Some(PureFunctionExpr {
        body: Some(AllocBox::new_in(body, allocator)),
        args,
        fn_ref: None,     // Set by pure_function_extraction phase
        var_offset: None, // Set by var_counting phase
        source_span: None,
    })
}

/// Create a PureFunctionExpr for a literal array with IR expression elements.
fn create_pure_function_for_ir_array<'a>(
    elements: &oxc_allocator::Vec<'a, IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) -> Option<PureFunctionExpr<'a>> {
    let mut args: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_entries: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut param_index: u32 = 0;

    for expr in elements.iter() {
        if is_constant_ir_expression(expr, expressions) {
            // Constant entry: resolve and clone the expression
            body_entries.push(resolve_expression_for_body(expr, allocator, expressions));
        } else {
            // Non-constant entry: add to args and replace with PureFunctionParameterExpr
            args.push(expr.clone_in(allocator));

            body_entries.push(IrExpression::PureFunctionParameter(AllocBox::new_in(
                PureFunctionParameterExpr { index: param_index, source_span: None },
                allocator,
            )));
            param_index += 1;
        }
    }

    // Create the derived array body
    // TypeScript always creates a PureFunction, even with 0 args (all constant)
    let body = IrExpression::DerivedLiteralArray(AllocBox::new_in(
        DerivedLiteralArrayExpr { entries: body_entries, source_span: None },
        allocator,
    ));

    Some(PureFunctionExpr {
        body: Some(AllocBox::new_in(body, allocator)),
        args,
        fn_ref: None,     // Set by pure_function_extraction phase
        var_offset: None, // Set by var_counting phase
        source_span: None,
    })
}

/// Create a PureFunctionExpr for a literal map with IR expression values.
fn create_pure_function_for_ir_map<'a>(
    keys: &oxc_allocator::Vec<'a, oxc_span::Ident<'a>>,
    values: &oxc_allocator::Vec<'a, IrExpression<'a>>,
    quoted: &oxc_allocator::Vec<'a, bool>,
    allocator: &'a oxc_allocator::Allocator,
    expressions: &ExpressionStore<'a>,
) -> Option<PureFunctionExpr<'a>> {
    let mut args: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_keys: AllocVec<'a, oxc_span::Ident<'a>> = AllocVec::new_in(allocator);
    let mut body_values: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_quoted: AllocVec<'a, bool> = AllocVec::new_in(allocator);
    let mut param_index: u32 = 0;

    for (i, value) in values.iter().enumerate() {
        // Get key and quoted from the arrays
        let key = keys.get(i).cloned().unwrap_or_else(|| oxc_span::Ident::from(""));
        let is_quoted = quoted.get(i).copied().unwrap_or(false);
        body_keys.push(key);
        body_quoted.push(is_quoted);

        if is_constant_ir_expression(value, expressions) {
            // Constant value: resolve and clone
            body_values.push(resolve_expression_for_body(value, allocator, expressions));
        } else {
            // Non-constant value: add to args and replace with PureFunctionParameterExpr
            args.push(value.clone_in(allocator));

            body_values.push(IrExpression::PureFunctionParameter(AllocBox::new_in(
                PureFunctionParameterExpr { index: param_index, source_span: None },
                allocator,
            )));
            param_index += 1;
        }
    }

    // Create the derived map body
    // TypeScript always creates a PureFunction, even with 0 args (all constant)
    let body = IrExpression::DerivedLiteralMap(AllocBox::new_in(
        DerivedLiteralMapExpr {
            keys: body_keys,
            values: body_values,
            quoted: body_quoted,
            source_span: None,
        },
        allocator,
    ));

    Some(PureFunctionExpr {
        body: Some(AllocBox::new_in(body, allocator)),
        args,
        fn_ref: None,     // Set by pure_function_extraction phase
        var_offset: None, // Set by var_counting phase
        source_span: None,
    })
}

/// Create a PureFunctionExpr for a literal array.
///
/// Creates a derived array body with PureFunctionParameterExpr for non-constant entries,
/// and collects non-constant entries as args.
fn create_pure_function_for_array<'a>(
    expressions: &AllocVec<'a, AngularExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) -> Option<PureFunctionExpr<'a>> {
    let mut args: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_entries: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut param_index: u32 = 0;

    for expr in expressions.iter() {
        if is_constant_expression(expr) {
            // Constant entry: clone the AST expression and wrap in IrExpression::Ast
            let cloned = clone_angular_expression(expr, allocator);
            body_entries.push(IrExpression::Ast(AllocBox::new_in(cloned, allocator)));
        } else {
            // Non-constant entry: add to args and replace with PureFunctionParameterExpr
            let cloned = clone_angular_expression(expr, allocator);
            args.push(IrExpression::Ast(AllocBox::new_in(cloned, allocator)));

            body_entries.push(IrExpression::PureFunctionParameter(AllocBox::new_in(
                PureFunctionParameterExpr { index: param_index, source_span: None },
                allocator,
            )));
            param_index += 1;
        }
    }

    // Create the derived array body
    // TypeScript always creates a PureFunction, even with 0 args (all constant)
    let body = IrExpression::DerivedLiteralArray(AllocBox::new_in(
        DerivedLiteralArrayExpr { entries: body_entries, source_span: None },
        allocator,
    ));

    Some(PureFunctionExpr {
        body: Some(AllocBox::new_in(body, allocator)),
        args,
        fn_ref: None,     // Set by pure_function_extraction phase
        var_offset: None, // Set by var_counting phase
        source_span: None,
    })
}

/// Create a PureFunctionExpr for a literal map.
fn create_pure_function_for_map<'a>(
    map: &LiteralMap<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> Option<PureFunctionExpr<'a>> {
    let mut args: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_keys: AllocVec<'a, oxc_span::Ident<'a>> = AllocVec::new_in(allocator);
    let mut body_values: AllocVec<'a, IrExpression<'a>> = AllocVec::new_in(allocator);
    let mut body_quoted: AllocVec<'a, bool> = AllocVec::new_in(allocator);
    let mut param_index: u32 = 0;

    for (i, value) in map.values.iter().enumerate() {
        use crate::ast::expression::LiteralMapKey;
        // Extract key and quoted from LiteralMapKey
        let (key, quoted) = map
            .keys
            .get(i)
            .and_then(|k| {
                if let LiteralMapKey::Property(prop) = k {
                    Some((prop.key.clone(), prop.quoted))
                } else {
                    None // Skip spread keys
                }
            })
            .unwrap_or_else(|| (oxc_span::Ident::from(""), false));
        body_keys.push(key);
        body_quoted.push(quoted);

        if is_constant_expression(value) {
            // Constant value: clone and wrap in IrExpression::Ast
            let cloned = clone_angular_expression(value, allocator);
            body_values.push(IrExpression::Ast(AllocBox::new_in(cloned, allocator)));
        } else {
            // Non-constant value: add to args and replace with PureFunctionParameterExpr
            let cloned = clone_angular_expression(value, allocator);
            args.push(IrExpression::Ast(AllocBox::new_in(cloned, allocator)));

            body_values.push(IrExpression::PureFunctionParameter(AllocBox::new_in(
                PureFunctionParameterExpr { index: param_index, source_span: None },
                allocator,
            )));
            param_index += 1;
        }
    }

    // Create the derived map body
    // TypeScript always creates a PureFunction, even with 0 args (all constant)
    let body = IrExpression::DerivedLiteralMap(AllocBox::new_in(
        DerivedLiteralMapExpr {
            keys: body_keys,
            values: body_values,
            quoted: body_quoted,
            source_span: None,
        },
        allocator,
    ));

    Some(PureFunctionExpr {
        body: Some(AllocBox::new_in(body, allocator)),
        args,
        fn_ref: None,     // Set by pure_function_extraction phase
        var_offset: None, // Set by var_counting phase
        source_span: None,
    })
}

/// Generates pure literal structures for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn generate_pure_literal_structures_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // We need to access the expression store while modifying views.
    // Use a raw pointer to avoid borrow checker issues (same pattern as resolve_names).
    let expression_store_ptr = &job.expressions as *const ExpressionStore<'_>;

    // SAFETY: We're only reading from expression_store, not modifying it
    let expressions = unsafe { &*expression_store_ptr };

    // Only process update operations - this optimization is for change detection
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, flags| {
                // Skip if we're in a child operation (already inside a PureFunction)
                if flags.contains(VisitorContextFlag::IN_CHILD_OPERATION) {
                    return;
                }

                transform_literal_structure(expr, allocator, expressions);
            },
            VisitorContextFlag::NONE,
        );
    }
}

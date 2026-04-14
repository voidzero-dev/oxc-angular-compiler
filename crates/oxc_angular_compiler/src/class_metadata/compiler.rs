//! Class metadata compilation.
//!
//! Ported from Angular's `render3/r3_class_metadata_compiler.ts`.
//!
//! Generates:
//! - `setClassMetadata(type, decorators, ctorParams, propDecorators)`
//! - `setClassMetadataAsync(type, resolver, callback)` for deferred dependencies

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::metadata::{R3ClassMetadata, R3DeferPerComponentDependency};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, DynamicImportExpr,
    DynamicImportUrl, ExpressionStatement, FnParam, InvokeFunctionExpr, LiteralArrayExpr,
    LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
    TypeofExpr,
};
use crate::r3::Identifiers;

/// Compiles class metadata wrapped in a dev-only IIFE.
///
/// Generates:
/// ```javascript
/// (() => {
///   (typeof ngDevMode === "undefined" || ngDevMode) &&
///     i0.ɵɵsetClassMetadata(type, decorators, ctorParams, propDecorators);
/// })();
/// ```
///
/// Ported from Angular's `compileClassMetadata` function.
pub fn compile_class_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3ClassMetadata<'a>,
) -> OutputExpression<'a> {
    let fn_call = internal_compile_class_metadata(allocator, metadata);
    let guarded = dev_only_guarded_expression(allocator, fn_call);
    let stmt = expr_stmt(allocator, guarded);
    let arrow = create_arrow_iife(allocator, stmt);
    arrow
}

/// Compiles class metadata for components, handling deferred dependencies.
///
/// If there are no deferred dependencies, generates regular `setClassMetadata`.
/// Otherwise, generates `setClassMetadataAsync` with dynamic imports.
///
/// Ported from Angular's `compileComponentClassMetadata` function.
pub fn compile_component_class_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3ClassMetadata<'a>,
    dependencies: Option<&[R3DeferPerComponentDependency<'a>]>,
) -> OutputExpression<'a> {
    match dependencies {
        None | Some([]) => {
            // No deferred dependencies - use regular setClassMetadata
            compile_class_metadata(allocator, metadata)
        }
        Some(deps) => {
            // Has deferred dependencies - use setClassMetadataAsync
            let mut params = Vec::new_in(allocator);
            for dep in deps {
                params.push(FnParam { name: dep.symbol_name.clone() });
            }
            let resolver = compile_component_metadata_async_resolver(allocator, deps);
            internal_compile_set_class_metadata_async(allocator, metadata, params, resolver)
        }
    }
}

/// Compiles class metadata with an opaque async resolver.
///
/// Used when we have a reference to the compiled dependency resolver function.
///
/// Ported from Angular's `compileOpaqueAsyncClassMetadata` function.
pub fn compile_opaque_async_class_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3ClassMetadata<'a>,
    defer_resolver: OutputExpression<'a>,
    deferred_dependency_names: &[Ident<'a>],
) -> OutputExpression<'a> {
    let mut params = Vec::new_in(allocator);
    for name in deferred_dependency_names {
        params.push(FnParam { name: name.clone() });
    }
    internal_compile_set_class_metadata_async(allocator, metadata, params, defer_resolver)
}

/// Compiles the dependency resolver function for `setClassMetadataAsync`.
///
/// Generates:
/// ```javascript
/// () => [
///   import('./cmp-a').then(m => m.CmpA),
///   import('./cmp-b').then(m => m.CmpB),
/// ]
/// ```
///
/// Ported from Angular's `compileComponentMetadataAsyncResolver` function.
pub fn compile_component_metadata_async_resolver<'a>(
    allocator: &'a Allocator,
    dependencies: &[R3DeferPerComponentDependency<'a>],
) -> OutputExpression<'a> {
    let mut dynamic_imports = Vec::new_in(allocator);

    for dep in dependencies {
        // Create: (m) => m.CmpA  (or m.default for default imports)
        let mut inner_params = Vec::new_in(allocator);
        inner_params.push(FnParam { name: Ident::from("m") });

        let prop_name =
            if dep.is_default_import { Ident::from("default") } else { dep.symbol_name.clone() };

        let inner_body = OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Ident::from("m"), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: prop_name,
                optional: false,
                source_span: None,
            },
            allocator,
        ));

        let inner_fn = OutputExpression::ArrowFunction(Box::new_in(
            ArrowFunctionExpr {
                params: inner_params,
                body: ArrowFunctionBody::Expression(Box::new_in(inner_body, allocator)),
                source_span: None,
            },
            allocator,
        ));

        // Create: import('./cmp-a').then((m) => m.CmpA)
        let dynamic_import = OutputExpression::DynamicImport(Box::new_in(
            DynamicImportExpr {
                url: DynamicImportUrl::String(dep.import_path.clone()),
                url_comment: None,
                source_span: None,
            },
            allocator,
        ));

        let then_prop = OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(dynamic_import, allocator),
                name: Ident::from("then"),
                optional: false,
                source_span: None,
            },
            allocator,
        ));

        let mut then_args = Vec::new_in(allocator);
        then_args.push(inner_fn);

        let then_call = OutputExpression::InvokeFunction(Box::new_in(
            InvokeFunctionExpr {
                fn_expr: Box::new_in(then_prop, allocator),
                args: then_args,
                pure: false,
                optional: false,
                source_span: None,
            },
            allocator,
        ));

        dynamic_imports.push(then_call);
    }

    // Create: () => [...]
    let array = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: dynamic_imports, source_span: None },
        allocator,
    ));

    let empty_params = Vec::new_in(allocator);
    OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: empty_params,
            body: ArrowFunctionBody::Expression(Box::new_in(array, allocator)),
            source_span: None,
        },
        allocator,
    ))
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Compiles the internal `setClassMetadata` call without wrappers.
fn internal_compile_class_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3ClassMetadata<'a>,
) -> OutputExpression<'a> {
    let import = import_expr(allocator, Identifiers::SET_CLASS_METADATA);

    let mut args = Vec::new_in(allocator);
    args.push(metadata.r#type.clone_in(allocator));
    args.push(metadata.decorators.clone_in(allocator));
    args.push(
        metadata
            .ctor_parameters
            .as_ref()
            .map(|e| e.clone_in(allocator))
            .unwrap_or_else(|| literal_null(allocator)),
    );
    args.push(
        metadata
            .prop_decorators
            .as_ref()
            .map(|e| e.clone_in(allocator))
            .unwrap_or_else(|| literal_null(allocator)),
    );

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(import, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Compiles `setClassMetadataAsync` with wrapper params and resolver.
fn internal_compile_set_class_metadata_async<'a>(
    allocator: &'a Allocator,
    metadata: &R3ClassMetadata<'a>,
    wrapper_params: Vec<'a, FnParam<'a>>,
    dependency_resolver_fn: OutputExpression<'a>,
) -> OutputExpression<'a> {
    // Create the inner setClassMetadata call
    let set_class_metadata_call = internal_compile_class_metadata(allocator, metadata);

    // Create wrapper: (deps...) => { setClassMetadata(...); }
    let mut wrapper_stmts = Vec::new_in(allocator);
    wrapper_stmts.push(expr_stmt(allocator, set_class_metadata_call));

    let set_class_meta_wrapper = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: wrapper_params,
            body: ArrowFunctionBody::Statements(wrapper_stmts),
            source_span: None,
        },
        allocator,
    ));

    // Create: setClassMetadataAsync(type, resolver, wrapper)
    let import = import_expr(allocator, Identifiers::SET_CLASS_METADATA_ASYNC);

    let mut args = Vec::new_in(allocator);
    args.push(metadata.r#type.clone_in(allocator));
    args.push(dependency_resolver_fn);
    args.push(set_class_meta_wrapper);

    let set_class_meta_async = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(import, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Wrap in dev-only guard and IIFE
    let guarded = dev_only_guarded_expression(allocator, set_class_meta_async);
    let stmt = expr_stmt(allocator, guarded);
    create_arrow_iife(allocator, stmt)
}

/// Creates: (typeof ngDevMode === "undefined" || ngDevMode) && expr
///
/// Ported from Angular's `devOnlyGuardedExpression` in `util.ts`.
fn dev_only_guarded_expression<'a>(
    allocator: &'a Allocator,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    guarded_expression(allocator, "ngDevMode", expr)
}

/// Creates: (typeof guard === "undefined" || guard) && expr
fn guarded_expression<'a>(
    allocator: &'a Allocator,
    guard: &'static str,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let guard_var = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(guard), source_span: None },
        allocator,
    ));

    // typeof guard
    let typeof_guard = OutputExpression::Typeof(Box::new_in(
        TypeofExpr {
            expr: Box::new_in(guard_var.clone_in(allocator), allocator),
            source_span: None,
        },
        allocator,
    ));

    // typeof guard === "undefined"
    let guard_not_defined = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Identical,
            lhs: Box::new_in(typeof_guard, allocator),
            rhs: Box::new_in(literal_string(allocator, "undefined"), allocator),
            source_span: None,
        },
        allocator,
    ));

    // typeof guard === "undefined" || guard
    let guard_undefined_or_true = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(guard_not_defined, allocator),
            rhs: Box::new_in(guard_var, allocator),
            source_span: None,
        },
        allocator,
    ));

    // (typeof guard === "undefined" || guard) && expr
    OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::And,
            lhs: Box::new_in(guard_undefined_or_true, allocator),
            rhs: Box::new_in(expr, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Creates an import expression: i0.identifier
fn import_expr<'a>(allocator: &'a Allocator, identifier: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(identifier),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates a null literal.
fn literal_null<'a>(allocator: &'a Allocator) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Null, source_span: None },
        allocator,
    ))
}

/// Creates a string literal.
fn literal_string<'a>(allocator: &'a Allocator, value: &'static str) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(value)), source_span: None },
        allocator,
    ))
}

/// Creates an expression statement.
fn expr_stmt<'a>(allocator: &'a Allocator, expr: OutputExpression<'a>) -> OutputStatement<'a> {
    OutputStatement::Expression(Box::new_in(
        ExpressionStatement { expr, source_span: None },
        allocator,
    ))
}

/// Creates an immediately invoked arrow function: (() => { stmt })()
fn create_arrow_iife<'a>(
    allocator: &'a Allocator,
    stmt: OutputStatement<'a>,
) -> OutputExpression<'a> {
    let empty_params = Vec::new_in(allocator);
    let mut stmts = Vec::new_in(allocator);
    stmts.push(stmt);

    let arrow = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: empty_params,
            body: ArrowFunctionBody::Statements(stmts),
            source_span: None,
        },
        allocator,
    ));

    // Call the arrow function: arrow()
    let empty_args = Vec::new_in(allocator);
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(arrow, allocator),
            args: empty_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

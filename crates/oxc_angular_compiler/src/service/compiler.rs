//! Service compilation implementation.
//!
//! Ported from Angular's `service_compiler.ts:compileService`.
//!
//! Generates the `ɵprov` initializer for `@Service`-decorated classes:
//! ```javascript
//! ɵprov = /*@__PURE__*/ ɵɵdefineService({
//!   token: MyService,
//!   factory: MyService.ɵfac
//! });
//! ```
//!
//! When the user passes `autoProvided: false`, the field is emitted. When the
//! user supplies a custom `factory` expression, it is wrapped in an arrow
//! function: `() => factory()` (per upstream `service_compiler.ts:35-42`).

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::metadata::R3ServiceMetadata;
use crate::output::ast::{
    FunctionExpr, InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue,
    OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr, ReturnStatement,
};
use crate::r3::Identifiers;

/// Result of compiling a service.
#[derive(Debug)]
pub struct ServiceCompileResult<'a> {
    /// The compiled expression: `ɵɵdefineService({...})`.
    pub expression: OutputExpression<'a>,
}

/// Compiles a service from its metadata into an `ɵɵdefineService(...)` call.
pub fn compile_service<'a>(
    allocator: &'a Allocator,
    metadata: &R3ServiceMetadata<'a>,
) -> ServiceCompileResult<'a> {
    let factory_expr = build_factory_expression(allocator, metadata);
    let definition_map = build_definition_map(allocator, metadata, factory_expr);
    let expression = create_define_service_call(allocator, definition_map);

    ServiceCompileResult { expression }
}

/// Build the `factory` entry of the definition map.
///
/// Two cases, mirroring `service_compiler.ts:33-42`:
/// - `meta.factory === undefined` → `delegateToFactory(...)` → `MyService.ɵfac`.
/// - `meta.factory` supplied → arrow wrapper `() => factory()`.
fn build_factory_expression<'a>(
    allocator: &'a Allocator,
    metadata: &R3ServiceMetadata<'a>,
) -> OutputExpression<'a> {
    match &metadata.factory {
        None => create_factory_delegation(allocator, &metadata.r#type),
        Some(user_factory) => create_factory_arrow(allocator, user_factory),
    }
}

/// `MyService.ɵfac`
fn create_factory_delegation<'a>(
    allocator: &'a Allocator,
    type_expr: &OutputExpression<'a>,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(type_expr.clone_in(allocator), allocator),
            name: Ident::from("ɵfac"),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// `() => factory()` — wraps a user-supplied factory in an arrow that calls it.
fn create_factory_arrow<'a>(
    allocator: &'a Allocator,
    factory: &OutputExpression<'a>,
) -> OutputExpression<'a> {
    let params = Vec::new_in(allocator);

    let factory_call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(factory.clone_in(allocator), allocator),
            args: Vec::new_in(allocator),
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut body = Vec::new_in(allocator);
    body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: factory_call, source_span: None },
        allocator,
    )));

    OutputExpression::Function(Box::new_in(
        FunctionExpr { name: None, params, statements: body, source_span: None },
        allocator,
    ))
}

fn build_definition_map<'a>(
    allocator: &'a Allocator,
    metadata: &R3ServiceMetadata<'a>,
    factory_expr: OutputExpression<'a>,
) -> Vec<'a, LiteralMapEntry<'a>> {
    let mut entries = Vec::new_in(allocator);

    entries.push(LiteralMapEntry::new(
        Ident::from("token"),
        metadata.r#type.clone_in(allocator),
        false,
    ));

    entries.push(LiteralMapEntry::new(Ident::from("factory"), factory_expr, false));

    // Only emit autoProvided when explicitly disabled — matches upstream's
    // `if (meta.autoProvided === false)` gate in service_compiler.ts:45-47.
    if matches!(metadata.auto_provided, Some(false)) {
        entries.push(LiteralMapEntry::new(
            Ident::from("autoProvided"),
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(false), source_span: None },
                allocator,
            )),
            false,
        ));
    }

    entries
}

/// `/*@__PURE__*/ i0.ɵɵdefineService({...})`
fn create_define_service_call<'a>(
    allocator: &'a Allocator,
    definition_map: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    let define_service_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::DEFINE_SERVICE),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: definition_map, source_span: None },
        allocator,
    ));

    let mut args = Vec::new_in(allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(define_service_fn, allocator),
            args,
            pure: true,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::emitter::JsEmitter;

    fn make_type_expr<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from(name), source_span: None },
            allocator,
        ))
    }

    #[test]
    fn compiles_basic_service() {
        let allocator = Allocator::default();
        let metadata = R3ServiceMetadata {
            name: Ident::from("MyService"),
            r#type: make_type_expr(&allocator, "MyService"),
            type_argument_count: 0,
            auto_provided: None,
            factory: None,
        };

        let result = compile_service(&allocator, &metadata);
        let js = JsEmitter::new().emit_expression(&result.expression);

        assert!(js.contains("ɵɵdefineService"), "should emit defineService. Got: {js}");
        assert!(js.contains("token:MyService"), "should reference class as token. Got: {js}");
        assert!(js.contains("factory:MyService.ɵfac"), "should delegate to ɵfac. Got: {js}");
        assert!(
            !js.contains("autoProvided"),
            "autoProvided should be omitted when None. Got: {js}"
        );
    }

    #[test]
    fn emits_auto_provided_false() {
        let allocator = Allocator::default();
        let metadata = R3ServiceMetadata {
            name: Ident::from("MyService"),
            r#type: make_type_expr(&allocator, "MyService"),
            type_argument_count: 0,
            auto_provided: Some(false),
            factory: None,
        };

        let result = compile_service(&allocator, &metadata);
        let js = JsEmitter::new().emit_expression(&result.expression);

        assert!(js.contains("autoProvided:false"), "should emit autoProvided: false. Got: {js}");
    }

    #[test]
    fn does_not_emit_auto_provided_true() {
        let allocator = Allocator::default();
        let metadata = R3ServiceMetadata {
            name: Ident::from("MyService"),
            r#type: make_type_expr(&allocator, "MyService"),
            type_argument_count: 0,
            auto_provided: Some(true),
            factory: None,
        };

        let result = compile_service(&allocator, &metadata);
        let js = JsEmitter::new().emit_expression(&result.expression);

        assert!(
            !js.contains("autoProvided"),
            "autoProvided: true should match default and be omitted. Got: {js}"
        );
    }

    #[test]
    fn wraps_custom_factory_in_arrow() {
        let allocator = Allocator::default();
        let factory_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("makeService"), source_span: None },
            &allocator,
        ));
        let metadata = R3ServiceMetadata {
            name: Ident::from("MyService"),
            r#type: make_type_expr(&allocator, "MyService"),
            type_argument_count: 0,
            auto_provided: None,
            factory: Some(factory_expr),
        };

        let result = compile_service(&allocator, &metadata);
        let js = JsEmitter::new().emit_expression(&result.expression);

        assert!(
            js.contains("makeService()"),
            "should call user factory inside arrow. Got: {js}"
        );
        assert!(
            !js.contains("MyService.ɵfac"),
            "should not delegate to ɵfac when custom factory supplied. Got: {js}"
        );
    }
}

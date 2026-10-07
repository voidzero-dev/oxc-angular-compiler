//! Service definition generation (ɵfac + ɵprov).
//!
//! Generates both static fields that Angular's `@Service` runtime expects:
//! - `ɵfac`: the standard factory function. For `@Service`, ctor deps are
//!   intentionally empty — the v22 service runtime resolves dependencies via
//!   `inject()` calls in the constructor body, not the ɵfac. See upstream
//!   `compiler-cli/src/ngtsc/annotations/src/service.ts:178`, which calls
//!   `toFactoryMetadata({...meta, deps: []}, FactoryTarget.Service)`.
//! - `ɵprov`: the `ɵɵdefineService({...})` initializer.

use oxc_allocator::{Allocator, Vec as OxcVec};

use super::compiler::compile_service;
use super::decorator::ServiceMetadata;
use super::metadata::R3ServiceMetadata;
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3FactoryDeps, R3FactoryMetadata,
    compile_factory_function,
};
use crate::output::ast::OutputExpression;

/// Both static field initializers for a `@Service` class.
pub struct ServiceDefinition<'a> {
    /// The `ɵprov` initializer: `ɵɵdefineService({...})`.
    pub prov_definition: OutputExpression<'a>,
    /// The `ɵfac` initializer: a constructor factory with empty deps.
    pub fac_definition: OutputExpression<'a>,
}

/// Generate `ɵfac` and `ɵprov` definitions from R3 metadata.
pub fn generate_service_definition<'a>(
    allocator: &'a Allocator,
    core_namespace: &'a str,
    metadata: &R3ServiceMetadata<'a>,
) -> ServiceDefinition<'a> {
    // Generate ɵfac BEFORE ɵprov so namespace-index assignment order matches
    // upstream's [fac, prov, ...] ordering.
    let fac_definition = generate_fac_definition(allocator, core_namespace, metadata);
    let prov_result = compile_service(allocator, core_namespace, metadata);

    ServiceDefinition { prov_definition: prov_result.expression, fac_definition }
}

/// Convenience: extract `R3ServiceMetadata` from `ServiceMetadata` and emit.
pub fn generate_service_definition_from_decorator<'a>(
    allocator: &'a Allocator,
    core_namespace: &'a str,
    metadata: &ServiceMetadata<'a>,
    type_argument_count: u32,
) -> ServiceDefinition<'a> {
    let r3_metadata = metadata.to_r3_metadata(allocator, type_argument_count);
    generate_service_definition(allocator, core_namespace, &r3_metadata)
}

/// Emit the ɵfac factory function. `@Service` factories never inject ctor
/// params — upstream passes `deps: []` deliberately, since the v22 service
/// runtime expects `inject()` calls inside the constructor body.
fn generate_fac_definition<'a>(
    allocator: &'a Allocator,
    core_namespace: &'a str,
    metadata: &R3ServiceMetadata<'a>,
) -> OutputExpression<'a> {
    let factory_name = allocator.alloc_str(&format!("{}_Factory", metadata.name));

    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: metadata.name,
        type_expr: metadata.r#type.clone_in(allocator),
        type_decl: metadata.r#type.clone_in(allocator),
        type_argument_count: metadata.type_argument_count,
        deps: R3FactoryDeps::Valid(OxcVec::new_in(&allocator)),
        target: FactoryTarget::Service,
    });

    let result = compile_factory_function(allocator, core_namespace, &factory_meta, factory_name);
    result.expression
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::ReadVarExpr;
    use crate::output::emitter::JsEmitter;
    use oxc_allocator::Box;
    use oxc_str::Ident;

    #[test]
    fn fac_has_no_inject_calls() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyService"), source_span: None },
            &&allocator,
        ));
        let metadata = R3ServiceMetadata {
            name: Ident::from("MyService"),
            r#type: type_expr,
            type_argument_count: 0,
            auto_provided: None,
            factory: None,
        };

        let def = generate_service_definition(&allocator, "i0", &metadata);
        let emitter = JsEmitter::new();
        let fac_js = emitter.emit_expression(&def.fac_definition);
        let prov_js = emitter.emit_expression(&def.prov_definition);

        assert!(
            !fac_js.contains("ɵɵinject") && !fac_js.contains("inject("),
            "ɵfac should not call inject — services resolve deps in the ctor body. Got: {fac_js}"
        );
        assert!(
            prov_js.contains("ɵɵdefineService"),
            "ɵprov should use defineService. Got: {prov_js}"
        );
        assert!(
            prov_js.contains("MyService.ɵfac"),
            "ɵprov should delegate to ɵfac. Got: {prov_js}"
        );
    }
}

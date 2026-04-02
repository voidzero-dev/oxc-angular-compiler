//! Injectable definition generation (ɵprov and ɵfac).
//!
//! This module generates the Angular runtime definitions that are added
//! as static properties on injectable classes:
//!
//! - `ɵprov`: Injectable provider definition created by `ɵɵdefineInjectable()`
//! - `ɵfac`: Factory function for instantiating the injectable
//!
//! ## Generated Output
//!
//! ```javascript
//! // ɵprov definition:
//! MyService.ɵprov = /*@__PURE__*/ i0.ɵɵdefineInjectable({
//!   token: MyService,
//!   factory: MyService.ɵfac,  // For simple services
//!   providedIn: 'root'
//! });
//!
//! // ɵfac definition:
//! MyService.ɵfac = function MyService_Factory(__ngFactoryType__) {
//!   return new (__ngFactoryType__ || MyService)();
//! };
//! ```
//!
//! Or with a custom factory (useClass, useFactory, useValue, useExisting):
//!
//! ```javascript
//! MyService.ɵprov = /*@__PURE__*/ i0.ɵɵdefineInjectable({
//!   token: MyService,
//!   factory: () => new OtherService(),  // or inject call, etc.
//!   providedIn: 'root'
//! });
//! // Note: ɵfac is still generated for potential subclass instantiation
//! ```

use oxc_allocator::{Allocator, Vec as OxcVec};

use super::compiler::compile_injectable;
use super::decorator::InjectableMetadata;
use super::metadata::R3InjectableMetadata;
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata, compile_factory_function,
};
use crate::output::ast::OutputExpression;

/// Result of generating injectable definitions.
///
/// Injectables need both `ɵprov` and `ɵfac`:
/// - `ɵprov`: Provider metadata for Angular's DI system
/// - `ɵfac`: Factory function to instantiate the class
pub struct InjectableDefinition<'a> {
    /// The ɵprov definition (injectable provider metadata for Angular runtime).
    /// This is the result of `ɵɵdefineInjectable({...})`.
    pub prov_definition: OutputExpression<'a>,

    /// The ɵfac factory function.
    /// This is needed for Angular to instantiate the class.
    pub fac_definition: OutputExpression<'a>,
}

/// Generate ɵprov and ɵfac definitions for an injectable.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Injectable R3 metadata (already converted from decorator metadata)
///
/// # Returns
///
/// The ɵprov and ɵfac definitions as output expressions.
///
/// # Example Output
///
/// ```javascript
/// // Simple service:
/// MyService.ɵprov = /*@__PURE__*/ i0.ɵɵdefineInjectable({
///   token: MyService,
///   factory: MyService.ɵfac,
///   providedIn: 'root'
/// });
/// MyService.ɵfac = function MyService_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || MyService)();
/// };
///
/// // With useClass:
/// MyService.ɵprov = /*@__PURE__*/ i0.ɵɵdefineInjectable({
///   token: MyService,
///   factory: __ngFactoryType__ => OtherService.ɵfac(__ngFactoryType__),
///   providedIn: 'root'
/// });
/// ```
pub fn generate_injectable_definition<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectableMetadata<'a>,
) -> InjectableDefinition<'a> {
    // IMPORTANT: Generate ɵfac BEFORE ɵprov to match Angular's namespace index assignment order.
    // Angular processes results in order [fac, prov, ...] during the transform phase
    // (see packages/compiler-cli/src/ngtsc/annotations/src/injectable.ts:218-253),
    // so factory dependencies get registered first, followed by prov definition dependencies.
    // This ensures namespace indices (i0, i1, i2, ...) are assigned in the same order.
    let fac_definition = generate_fac_definition(allocator, metadata);
    let prov_result = compile_injectable(allocator, metadata);

    InjectableDefinition { prov_definition: prov_result.expression, fac_definition }
}

/// Generate the ɵfac factory function for an injectable.
///
/// Creates an expression like:
/// ```javascript
/// function MyService_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || MyService)(
///     i0.ɵɵinject(Dep1),
///     i0.ɵɵinject(Dep2)
///   );
/// }
/// ```
///
/// For injectables with useClass/useFactory/useValue/useExisting, the ɵfac
/// is still generated but the ɵprov's factory field will contain a custom
/// factory function instead of referencing ɵfac.
///
/// If no constructor is defined (deps is None), generates an inherited factory:
/// ```javascript
/// /*@__PURE__*/ (() => {
///   let ɵMyService_BaseFactory;
///   return function MyService_Factory(__ngFactoryType__) {
///     return (ɵMyService_BaseFactory || (ɵMyService_BaseFactory = ɵɵgetInheritedFactory(MyService)))(__ngFactoryType__ || MyService);
///   };
/// })()
/// ```
fn generate_fac_definition<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectableMetadata<'a>,
) -> OutputExpression<'a> {
    // Factory function name: ServiceName_Factory
    let factory_name = allocator.alloc_str(&format!("{}_Factory", metadata.name));

    // Convert deps to R3FactoryDeps
    let factory_deps = match &metadata.deps {
        Some(deps) => {
            // Clone deps to new allocator-owned vec
            let mut factory_deps: OxcVec<'a, R3DependencyMetadata<'a>> =
                OxcVec::with_capacity_in(deps.len(), allocator);
            for dep in deps {
                factory_deps.push(R3DependencyMetadata {
                    token: dep.token.as_ref().map(|t| t.clone_in(allocator)),
                    attribute_name_type: dep
                        .attribute_name_type
                        .as_ref()
                        .map(|a| a.clone_in(allocator)),
                    host: dep.host,
                    optional: dep.optional,
                    self_: dep.self_,
                    skip_self: dep.skip_self,
                });
            }
            R3FactoryDeps::Valid(factory_deps)
        }
        None => R3FactoryDeps::None,
    };

    // Create factory metadata
    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: metadata.name,
        type_expr: metadata.r#type.clone_in(allocator),
        type_decl: metadata.r#type.clone_in(allocator),
        type_argument_count: 0, // Injectables typically don't have type arguments
        deps: factory_deps,
        target: FactoryTarget::Injectable,
    });

    // Compile the factory function
    let result = compile_factory_function(allocator, &factory_meta, factory_name);
    result.expression
}

/// Generate ɵprov definition directly from injectable decorator metadata.
///
/// This is a convenience function that converts decorator metadata to R3 metadata
/// and generates the definition in one step.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Injectable metadata extracted from `@Injectable` decorator
///
/// # Returns
///
/// `Some(InjectableDefinition)` if conversion and compilation succeeded,
/// `None` if the metadata couldn't be converted to R3 format.
pub fn generate_injectable_definition_from_decorator<'a>(
    allocator: &'a Allocator,
    metadata: &InjectableMetadata<'a>,
) -> Option<InjectableDefinition<'a>> {
    let r3_metadata = metadata.to_r3_metadata(allocator)?;
    Some(generate_injectable_definition(allocator, &r3_metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injectable::metadata::R3InjectableMetadataBuilder;
    use crate::output::ast::ReadVarExpr;
    use crate::output::emitter::JsEmitter;
    use oxc_allocator::Box;
    use oxc_span::Ident;

    #[test]
    fn test_generate_simple_injectable_definition() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyService"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("MyService"))
            .r#type(type_expr)
            .provided_in_root()
            .build()
            .unwrap();

        let definition = generate_injectable_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.prov_definition);

        // Should have defineInjectable call
        assert!(js.contains("ɵɵdefineInjectable"), "Should contain ɵɵdefineInjectable");
        // Should have token
        assert!(js.contains("token"), "Should contain token property");
        assert!(js.contains("MyService"), "Should reference MyService");
        // Should have factory
        assert!(js.contains("factory"), "Should contain factory property");
        // Should have providedIn
        assert!(js.contains("providedIn"), "Should contain providedIn property");
        assert!(js.contains("root"), "Should have providedIn: 'root'");
    }

    #[test]
    fn test_generate_injectable_definition_no_provided_in() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("LocalService"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("LocalService"))
            .r#type(type_expr)
            .build()
            .unwrap();

        let definition = generate_injectable_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.prov_definition);

        // Should have defineInjectable call
        assert!(js.contains("ɵɵdefineInjectable"), "Should contain ɵɵdefineInjectable");
        // Should NOT have providedIn (when it's None)
        assert!(!js.contains("providedIn"), "Should NOT contain providedIn property");
    }

    #[test]
    fn test_generate_injectable_definition_with_platform() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("PlatformService"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("PlatformService"))
            .r#type(type_expr)
            .provided_in_platform()
            .build()
            .unwrap();

        let definition = generate_injectable_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.prov_definition);

        assert!(js.contains("platform"), "Should have providedIn: 'platform'");
    }

    #[test]
    fn test_generate_injectable_definition_with_any() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("AnyService"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("AnyService"))
            .r#type(type_expr)
            .provided_in_any()
            .build()
            .unwrap();

        let definition = generate_injectable_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.prov_definition);

        assert!(js.contains("any"), "Should have providedIn: 'any'");
    }

    #[test]
    fn test_generate_injectable_definition_is_pure() {
        // Test that the generated expression has pure=true set on the function call.
        // Note: The @__PURE__ annotation emission depends on the emitter implementation.
        // This test verifies the definition is correctly structured for tree-shaking.
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TreeShakableService"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("TreeShakableService"))
            .r#type(type_expr)
            .provided_in_root()
            .build()
            .unwrap();

        let definition = generate_injectable_definition(&allocator, &metadata);

        // The prov_definition should be an InvokeFunction with pure=true
        match &definition.prov_definition {
            OutputExpression::InvokeFunction(invoke) => {
                assert!(invoke.pure, "Injectable definition should be marked as pure");
            }
            _ => panic!("Expected InvokeFunction expression"),
        }
    }

    #[test]
    fn test_injectable_with_constructor_deps_from_decorator() {
        use crate::injectable::decorator::extract_injectable_metadata;
        use oxc_ast::ast::Statement;
        use oxc_parser::Parser;
        use oxc_span::SourceType;

        let allocator = Allocator::default();
        let code = r#"
            @Injectable()
            class InitService {
                constructor(
                    @Inject(WINDOW) private win: Window,
                    private sdkLoadService: SdkLoadService,
                    @Inject(DOCUMENT) private document: Document,
                ) {}
            }
        "#;

        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        // Find the class
        let class = parser_ret.program.body.iter().find_map(|stmt| match stmt {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            _ => None,
        });

        let class = class.expect("Should find class declaration");
        let metadata = extract_injectable_metadata(&allocator, class, Some(code));
        let metadata = metadata.expect("Should extract Injectable metadata");

        // Verify deps are extracted
        assert!(metadata.deps.is_some(), "Should have constructor deps");
        let deps = metadata.deps.as_ref().unwrap();
        assert_eq!(deps.len(), 3, "Should have 3 dependencies");

        // Generate the full definition and check the output
        let definition = generate_injectable_definition_from_decorator(&allocator, &metadata);
        let definition = definition.expect("Should generate definition");

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.fac_definition);

        // The factory should NOT use getInheritedFactory
        assert!(
            !js.contains("getInheritedFactory"),
            "Factory should NOT use getInheritedFactory when deps exist"
        );

        // The factory should call ɵɵinject
        assert!(js.contains("inject"), "Factory should contain inject calls, got: {}", js);

        // Should inject WINDOW (from @Inject)
        assert!(js.contains("WINDOW"), "Factory should inject WINDOW token, got: {}", js);

        // Should inject SdkLoadService (from type annotation)
        assert!(js.contains("SdkLoadService"), "Factory should inject SdkLoadService, got: {}", js);

        // Should inject DOCUMENT (from @Inject)
        assert!(js.contains("DOCUMENT"), "Factory should inject DOCUMENT token, got: {}", js);
    }

    #[test]
    fn test_injectable_without_constructor_uses_inherited_factory() {
        use crate::injectable::decorator::extract_injectable_metadata;
        use oxc_ast::ast::Statement;
        use oxc_parser::Parser;
        use oxc_span::SourceType;

        let allocator = Allocator::default();
        let code = r#"
            @Injectable()
            class MyService {}
        "#;

        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        let class = parser_ret.program.body.iter().find_map(|stmt| match stmt {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            _ => None,
        });

        let class = class.expect("Should find class declaration");
        let metadata = extract_injectable_metadata(&allocator, class, Some(code));
        let metadata = metadata.expect("Should extract Injectable metadata");

        // Verify no deps (no constructor)
        assert!(metadata.deps.is_none(), "Should not have constructor deps");

        // Generate definition
        let definition = generate_injectable_definition_from_decorator(&allocator, &metadata);
        let definition = definition.expect("Should generate definition");

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.fac_definition);

        // Should use inherited factory pattern
        assert!(
            js.contains("getInheritedFactory") || js.contains("ɵɵgetInheritedFactory"),
            "Factory should use getInheritedFactory when no constructor, got: {}",
            js
        );
    }

    #[test]
    fn test_injectable_with_optional_skip_self_deps() {
        use crate::injectable::decorator::extract_injectable_metadata;
        use oxc_ast::ast::Statement;
        use oxc_parser::Parser;
        use oxc_span::SourceType;

        let allocator = Allocator::default();
        let code = r#"
            @Injectable()
            class CoreModule {
                constructor(@Optional() @SkipSelf() parentModule?: CoreModule) {}
            }
        "#;

        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        let class = parser_ret.program.body.iter().find_map(|stmt| match stmt {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            _ => None,
        });

        let class = class.expect("Should find class declaration");
        let metadata = extract_injectable_metadata(&allocator, class, Some(code));
        let metadata = metadata.expect("Should extract Injectable metadata");

        // Generate definition
        let definition = generate_injectable_definition_from_decorator(&allocator, &metadata);
        let definition = definition.expect("Should generate definition");

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.fac_definition);

        // Should NOT use getInheritedFactory
        assert!(!js.contains("getInheritedFactory"), "Factory should NOT use getInheritedFactory");

        // Should call inject with CoreModule token
        assert!(js.contains("inject(CoreModule") || js.contains("inject( CoreModule"));

        // Check flags are correct: Optional(8) | SkipSelf(4) = 12
        assert!(
            js.contains(",12)") || js.contains(", 12)"),
            "Factory should pass flags = 12 (Optional | SkipSelf), got: {}",
            js
        );
    }
}

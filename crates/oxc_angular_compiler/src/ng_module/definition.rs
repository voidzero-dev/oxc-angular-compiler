//! NgModule definition generation (ɵmod, ɵfac, ɵinj).
//!
//! This module generates the Angular runtime definitions that are added
//! as static properties on NgModule classes:
//!
//! - `ɵmod`: NgModule definition created by `ɵɵdefineNgModule()`
//! - `ɵfac`: Factory function for instantiation
//! - `ɵinj`: Injector definition created by `ɵɵdefineInjector()`
//!
//! ## Generated Output
//!
//! ```javascript
//! // Module definition:
//! AppModule.ɵmod = /*@__PURE__*/ i0.ɵɵdefineNgModule({
//!   type: AppModule,
//!   bootstrap: [AppComponent],
//!   declarations: [MyComponent],
//!   imports: [CommonModule],
//!   exports: [MyComponent]
//! });
//!
//! // Factory function:
//! AppModule.ɵfac = function AppModule_Factory(__ngFactoryType__) {
//!   return new (__ngFactoryType__ || AppModule)();
//! };
//!
//! // Injector definition:
//! AppModule.ɵinj = /*@__PURE__*/ i0.ɵɵdefineInjector({
//!   providers: [...],
//!   imports: [CommonModule]
//! });
//! ```

use oxc_allocator::{Allocator, Vec as OxcVec};

use super::compiler::compile_ng_module;
use super::decorator::NgModuleMetadata;
use super::metadata::R3NgModuleMetadata;
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata, compile_factory_function,
};
use crate::injector::{R3InjectorMetadataBuilder, compile_injector};
use crate::output::ast::{OutputExpression, OutputStatement, ReadVarExpr};
use crate::output::emitter::JsEmitter;

/// Result of generating NgModule definition.
///
/// NgModules have a main definition (`ɵmod`) and potentially side-effect
/// statements for scope registration.
pub struct NgModuleDefinition<'a> {
    /// The ɵmod definition (NgModule metadata for Angular runtime).
    /// This is the result of `ɵɵdefineNgModule({...})`.
    pub mod_definition: OutputExpression<'a>,

    /// Additional side-effect statements (scope registration, module ID registration).
    /// These are IIFEs that should be executed after the module definition.
    pub statements: oxc_allocator::Vec<'a, OutputStatement<'a>>,
}

/// Full NgModule definition including ɵmod, ɵfac, and ɵinj.
pub struct FullNgModuleDefinition<'a> {
    /// The ɵmod definition (NgModule metadata for Angular runtime).
    pub mod_definition: OutputExpression<'a>,

    /// The ɵfac factory function for instantiation.
    pub fac_definition: OutputExpression<'a>,

    /// The ɵinj injector definition.
    pub inj_definition: OutputExpression<'a>,

    /// Additional side-effect statements (scope registration, module ID registration).
    pub statements: oxc_allocator::Vec<'a, OutputStatement<'a>>,
}

/// Generate ɵmod definition for an NgModule.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - NgModule R3 metadata (already converted from decorator metadata)
///
/// # Returns
///
/// The ɵmod definition and any side-effect statements.
///
/// # Example Output
///
/// ```javascript
/// // Module with inline scope:
/// AppModule.ɵmod = /*@__PURE__*/ i0.ɵɵdefineNgModule({
///   type: AppModule,
///   bootstrap: [AppComponent],
///   declarations: [MyComponent, MyDirective],
///   imports: [CommonModule],
///   exports: [MyComponent]
/// });
///
/// // Module with side-effect scope (for tree-shaking):
/// AppModule.ɵmod = /*@__PURE__*/ i0.ɵɵdefineNgModule({ type: AppModule });
/// (function() {
///   (typeof ngJitMode === "undefined" || ngJitMode) &&
///     i0.ɵɵsetNgModuleScope(AppModule, { declarations: [...], imports: [...] });
/// })();
/// ```
pub fn generate_ng_module_definition<'a>(
    allocator: &'a Allocator,
    metadata: &R3NgModuleMetadata<'a>,
) -> NgModuleDefinition<'a> {
    let result = compile_ng_module(allocator, metadata);
    NgModuleDefinition { mod_definition: result.expression, statements: result.statements }
}

/// Generate ɵmod definition directly from NgModule decorator metadata.
///
/// This is a convenience function that converts decorator metadata to R3 metadata
/// and generates the definition in one step.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - NgModule metadata extracted from `@NgModule` decorator
///
/// # Returns
///
/// `Some(NgModuleDefinition)` if conversion and compilation succeeded,
/// `None` if the metadata couldn't be converted to R3 format.
pub fn generate_ng_module_definition_from_decorator<'a>(
    allocator: &'a Allocator,
    metadata: &NgModuleMetadata<'a>,
) -> Option<NgModuleDefinition<'a>> {
    let r3_metadata = metadata.to_r3_metadata(allocator)?;
    Some(generate_ng_module_definition(allocator, &r3_metadata))
}

/// Generate the full JavaScript output for an NgModule definition.
///
/// This function takes the NgModule definition and generates the complete
/// JavaScript code including:
/// 1. The main ɵmod definition assignment
/// 2. Any side-effect statements (scope registration)
///
/// # Arguments
///
/// * `class_name` - The name of the NgModule class
/// * `definition` - The generated NgModule definition
///
/// # Returns
///
/// A string containing the full JavaScript code for the NgModule definition.
pub fn emit_ng_module_definition(class_name: &str, definition: &NgModuleDefinition<'_>) -> String {
    let emitter = JsEmitter::new();

    // Start with the main ɵmod definition
    let mut output =
        format!("{}.ɵmod = {};", class_name, emitter.emit_expression(&definition.mod_definition));

    // Append any side-effect statements
    for stmt in &definition.statements {
        output.push('\n');
        output.push_str(&emitter.emit_statement(stmt));
    }

    output
}

/// Generate full NgModule definitions (ɵmod, ɵfac, ɵinj) from decorator metadata.
///
/// This generates all three required definitions for an NgModule:
/// - ɵmod: Module definition
/// - ɵfac: Factory function for instantiation
/// - ɵinj: Injector definition with providers and imports
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - NgModule metadata extracted from `@NgModule` decorator
///
/// # Returns
///
/// `Some(FullNgModuleDefinition)` if generation succeeded,
/// `None` if the metadata couldn't be converted to R3 format.
pub fn generate_full_ng_module_definition<'a>(
    allocator: &'a Allocator,
    metadata: &NgModuleMetadata<'a>,
) -> Option<FullNgModuleDefinition<'a>> {
    let r3_metadata = metadata.to_r3_metadata(allocator)?;

    // IMPORTANT: Generate ɵfac BEFORE ɵmod and ɵinj to match Angular's namespace index assignment order.
    // Angular processes results in order [fac, mod, inj, ...] during the transform phase
    // (see packages/compiler-cli/src/ngtsc/annotations/ng_module/src/handler.ts:1056-1076),
    // so factory dependencies get registered first, followed by module and injector dependencies.
    // This ensures namespace indices (i0, i1, i2, ...) are assigned in the same order.

    // Generate ɵfac first
    let fac_definition = generate_ng_module_fac(allocator, metadata);

    // Generate ɵmod second
    let mod_result = compile_ng_module(allocator, &r3_metadata);

    // Generate ɵinj third
    let inj_definition = generate_ng_module_inj(allocator, metadata);

    Some(FullNgModuleDefinition {
        mod_definition: mod_result.expression,
        fac_definition,
        inj_definition,
        statements: mod_result.statements,
    })
}

/// Generate ɵfac factory function for an NgModule.
fn generate_ng_module_fac<'a>(
    allocator: &'a Allocator,
    metadata: &NgModuleMetadata<'a>,
) -> OutputExpression<'a> {
    let factory_name = allocator.alloc_str(&format!("{}_Factory", metadata.class_name));

    let type_expr = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
        ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
        allocator,
    ));

    // Convert deps to R3FactoryDeps
    let factory_deps = match &metadata.deps {
        Some(deps) => {
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

    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: metadata.class_name.clone(),
        type_expr: type_expr.clone_in(allocator),
        type_decl: type_expr,
        type_argument_count: 0,
        deps: factory_deps,
        target: FactoryTarget::NgModule,
    });

    let result = compile_factory_function(allocator, &factory_meta, factory_name);
    result.expression
}

/// Generate ɵinj injector definition for an NgModule.
fn generate_ng_module_inj<'a>(
    allocator: &'a Allocator,
    metadata: &NgModuleMetadata<'a>,
) -> OutputExpression<'a> {
    let type_expr = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
        ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
        allocator,
    ));

    let mut builder = R3InjectorMetadataBuilder::new(allocator)
        .name(metadata.class_name.clone())
        .r#type(type_expr);

    // Add providers if present
    if let Some(providers) = &metadata.providers {
        builder = builder.providers(providers.clone_in(allocator));
    }

    // Add imports for the injector.
    // Prefer raw_imports_expr which preserves call expressions like StoreModule.forRoot(...)
    // and spread elements, needed for ModuleWithProviders provider resolution.
    if let Some(raw_imports) = &metadata.raw_imports_expr {
        builder = builder.raw_imports(raw_imports.clone_in(allocator));
    } else {
        for import in &metadata.imports {
            let import_expr = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                ReadVarExpr { name: import.clone(), source_span: None },
                allocator,
            ));
            builder = builder.add_import(import_expr);
        }
    }

    let inj_metadata = builder.build().expect("Failed to build injector metadata");
    let result = compile_injector(allocator, &inj_metadata);
    result.expression
}

/// Emit the full NgModule definition as JavaScript code.
///
/// Generates code like:
/// ```javascript
/// MyModule.ɵmod = i0.ɵɵdefineNgModule({...});
/// MyModule.ɵfac = function MyModule_Factory(__ngFactoryType__) {...};
/// MyModule.ɵinj = i0.ɵɵdefineInjector({...});
/// ```
pub fn emit_full_ng_module_definition(
    class_name: &str,
    definition: &FullNgModuleDefinition<'_>,
) -> String {
    let emitter = JsEmitter::new();

    let mut output = format!(
        "{}.ɵmod = {};\n{}.ɵfac = {};\n{}.ɵinj = {};",
        class_name,
        emitter.emit_expression(&definition.mod_definition),
        class_name,
        emitter.emit_expression(&definition.fac_definition),
        class_name,
        emitter.emit_expression(&definition.inj_definition)
    );

    // Append any side-effect statements
    for stmt in &definition.statements {
        output.push('\n');
        output.push_str(&emitter.emit_statement(stmt));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ng_module::metadata::{R3NgModuleMetadataBuilder, R3Reference, R3SelectorScopeMode};
    use crate::output::ast::ReadVarExpr;
    use oxc_allocator::Box;
    use oxc_span::Atom;

    #[test]
    fn test_generate_simple_ng_module_definition() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("AppModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.mod_definition);

        // Should have defineNgModule call
        assert!(js.contains("ɵɵdefineNgModule"), "Should contain ɵɵdefineNgModule");
        // Should have type
        assert!(js.contains("type"), "Should contain type property");
        assert!(js.contains("AppModule"), "Should reference AppModule type");
        // Should have no side-effect statements for basic module
        assert!(definition.statements.is_empty(), "Should have no side-effect statements");
    }

    #[test]
    fn test_generate_ng_module_with_declarations() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("MyModule"), source_span: None },
            &allocator,
        ));
        let component_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("MyComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_declaration(R3Reference::value_only(component_expr))
            .selector_scope_mode(R3SelectorScopeMode::Inline)
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.mod_definition);

        // Should have declarations array
        assert!(js.contains("declarations"), "Should contain declarations");
        assert!(js.contains("MyComponent"), "Should reference MyComponent");
    }

    #[test]
    fn test_generate_ng_module_with_imports_exports() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("SharedModule"), source_span: None },
            &allocator,
        ));
        let import_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("CommonModule"), source_span: None },
            &allocator,
        ));
        let export_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("SharedComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_import(R3Reference::value_only(import_expr))
            .add_export(R3Reference::value_only(export_expr))
            .selector_scope_mode(R3SelectorScopeMode::Inline)
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.mod_definition);

        // Should have imports and exports
        assert!(js.contains("imports"), "Should contain imports");
        assert!(js.contains("CommonModule"), "Should reference CommonModule");
        assert!(js.contains("exports"), "Should contain exports");
        assert!(js.contains("SharedComponent"), "Should reference SharedComponent");
    }

    #[test]
    fn test_generate_ng_module_with_bootstrap() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("RootModule"), source_span: None },
            &allocator,
        ));
        let bootstrap_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("AppComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_bootstrap(R3Reference::value_only(bootstrap_expr))
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.mod_definition);

        // Should have bootstrap array
        assert!(js.contains("bootstrap"), "Should contain bootstrap");
        assert!(js.contains("AppComponent"), "Should reference AppComponent");
    }

    #[test]
    fn test_generate_ng_module_with_side_effect_scope() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("JitModule"), source_span: None },
            &allocator,
        ));
        let decl_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("JitComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_declaration(R3Reference::value_only(decl_expr))
            .selector_scope_mode(R3SelectorScopeMode::SideEffect)
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);

        // Should have side-effect statements
        assert!(!definition.statements.is_empty(), "Should have side-effect statements");
        assert_eq!(definition.statements.len(), 1, "Should have exactly one side-effect statement");
    }

    #[test]
    fn test_emit_ng_module_definition() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("TestModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);
        let js = emit_ng_module_definition("TestModule", &definition);

        // Should have proper assignment format
        assert!(
            js.contains("TestModule.ɵmod ="),
            "Should have correct assignment format, got: {}",
            js
        );
        assert!(js.contains("ɵɵdefineNgModule"), "Should contain defineNgModule");
    }

    #[test]
    fn test_generate_from_decorator_metadata() {
        use crate::ng_module::decorator::extract_ng_module_metadata;
        use oxc_ast::ast::{Declaration, Statement};
        use oxc_parser::Parser;
        use oxc_span::SourceType;

        let allocator = Allocator::default();
        let code = r#"
            @NgModule({
                declarations: [AppComponent],
                imports: [CommonModule],
                bootstrap: [AppComponent]
            })
            export class AppModule {}
        "#;

        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        // Find the class
        let class = parser_ret.program.body.iter().find_map(|stmt| match stmt {
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
                _ => None,
            },
            _ => None,
        });

        let class = class.expect("Should find class declaration");
        let metadata = extract_ng_module_metadata(&allocator, class);
        let metadata = metadata.expect("Should extract NgModule metadata");

        let definition = generate_ng_module_definition_from_decorator(&allocator, &metadata);
        let definition = definition.expect("Should generate definition");

        let js = emit_ng_module_definition("AppModule", &definition);

        assert!(js.contains("AppModule.ɵmod"), "Should have ɵmod assignment");
        assert!(js.contains("declarations"), "Should have declarations");
        assert!(js.contains("imports"), "Should have imports");
        assert!(js.contains("bootstrap"), "Should have bootstrap");
    }

    #[test]
    fn test_ng_module_with_constructor_deps() {
        use crate::ng_module::decorator::extract_ng_module_metadata;
        use oxc_ast::ast::Statement;
        use oxc_parser::Parser;
        use oxc_span::SourceType;

        let allocator = Allocator::default();
        // Test the CoreModule pattern: optional param with @Optional @SkipSelf decorators
        let code = r#"
            @NgModule({})
            class CoreModule {
                constructor(@Optional() @SkipSelf() parentModule?: CoreModule) {}
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
        let metadata = extract_ng_module_metadata(&allocator, class);
        let metadata = metadata.expect("Should extract NgModule metadata");

        // Verify deps are extracted
        assert!(metadata.deps.is_some(), "Should have constructor deps");
        let deps = metadata.deps.as_ref().unwrap();
        assert_eq!(deps.len(), 1, "Should have 1 dependency");
        assert!(deps[0].token.is_some(), "Dependency token should be Some (not None)");
        assert!(deps[0].optional, "Should be optional");
        assert!(deps[0].skip_self, "Should have skip_self");

        // Generate the full definition and check the output
        let definition = generate_full_ng_module_definition(&allocator, &metadata);
        let definition = definition.expect("Should generate definition");

        let js = emit_full_ng_module_definition("CoreModule", &definition);
        println!("Generated JS:\n{}", js);

        // The factory should call ɵɵinject, NOT ɵɵinvalidFactoryDep
        assert!(
            !js.contains("invalidFactoryDep"),
            "Factory should NOT contain invalidFactoryDep - token extraction failed"
        );
        assert!(
            js.contains("ɵɵinject(CoreModule") || js.contains("inject(CoreModule"),
            "Factory should contain inject call with CoreModule token"
        );
        // Check flags are correct: Optional(8) | SkipSelf(4) = 12
        assert!(
            js.contains(",12)") || js.contains(", 12)"),
            "Factory should pass flags = 12 (Optional | SkipSelf)"
        );
    }

    #[test]
    fn test_ng_module_definition_is_pure() {
        // Test that the generated expression has pure=true set on the function call.
        // This enables tree-shaking via the @__PURE__ annotation.
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("TreeShakableModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .build()
            .expect("Failed to build metadata");

        let definition = generate_ng_module_definition(&allocator, &metadata);

        // The mod_definition should be an InvokeFunction with pure=true
        match &definition.mod_definition {
            OutputExpression::InvokeFunction(invoke) => {
                assert!(invoke.pure, "NgModule definition should be marked as pure");
            }
            _ => panic!("Expected InvokeFunction expression"),
        }
    }
}

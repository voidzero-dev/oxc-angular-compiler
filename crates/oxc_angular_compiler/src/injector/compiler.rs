//! Injector compilation implementation.
//!
//! Ported from Angular's `render3/r3_injector_compiler.ts`.
//!
//! Generates injector definitions like:
//! ```javascript
//! ɵinj = ɵɵdefineInjector({
//!   providers: [...],
//!   imports: [Module1, Module2]
//! })
//! ```

use oxc_allocator::{Allocator, Box, Vec};
use oxc_span::Ident;

use super::metadata::R3InjectorMetadata;
use crate::output::ast::{
    InvokeFunctionExpr, LiteralArrayExpr, LiteralMapEntry, LiteralMapExpr, OutputExpression,
    OutputStatement, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Result of compiling an injector.
#[derive(Debug)]
pub struct InjectorCompileResult<'a> {
    /// The compiled expression: `ɵɵdefineInjector({...})`
    pub expression: OutputExpression<'a>,

    /// Additional statements (always empty for injectors).
    pub statements: Vec<'a, OutputStatement<'a>>,
}

/// Compiles an injector from its metadata.
///
/// This is the main entry point for injector compilation.
pub fn compile_injector<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectorMetadata<'a>,
) -> InjectorCompileResult<'a> {
    compile_injector_from_metadata(allocator, metadata)
}

/// Internal implementation of injector compilation.
pub fn compile_injector_from_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectorMetadata<'a>,
) -> InjectorCompileResult<'a> {
    // Build the definition map
    let definition_map = build_definition_map(allocator, metadata);

    // Create the expression: ɵɵdefineInjector(definitionMap)
    let expression = create_define_injector_call(allocator, definition_map);

    InjectorCompileResult { expression, statements: Vec::new_in(allocator) }
}

/// Builds the definition map for the injector.
fn build_definition_map<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectorMetadata<'a>,
) -> Vec<'a, LiteralMapEntry<'a>> {
    let mut entries = Vec::new_in(allocator);

    // providers: [...] (only if present)
    if let Some(providers) = &metadata.providers {
        entries.push(LiteralMapEntry {
            key: Ident::from("providers"),
            value: providers.clone_in(allocator),
            quoted: false,
        });
    }

    // imports: [...] (only if non-empty)
    if metadata.has_imports() {
        // Prefer raw_imports (preserves call expressions like StoreModule.forRoot(...))
        let imports_value = if let Some(raw_imports) = &metadata.raw_imports {
            raw_imports.clone_in(allocator)
        } else {
            let mut imports_items = Vec::new_in(allocator);
            for import in &metadata.imports {
                imports_items.push(import.clone_in(allocator));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: imports_items, source_span: None },
                allocator,
            ))
        };

        entries.push(LiteralMapEntry {
            key: Ident::from("imports"),
            value: imports_value,
            quoted: false,
        });
    }

    entries
}

/// Creates the `ɵɵdefineInjector({...})` call expression.
fn create_define_injector_call<'a>(
    allocator: &'a Allocator,
    definition_map: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    // Create i0.ɵɵdefineInjector
    let define_injector_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::DEFINE_INJECTOR),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create the literal map expression
    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: definition_map, source_span: None },
        allocator,
    ));

    // Create the function call
    let mut args = Vec::new_in(allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(define_injector_fn, allocator),
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
    use crate::injector::metadata::R3InjectorMetadataBuilder;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_compile_simple_injector() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectorMetadataBuilder::new(&allocator)
            .name(Ident::from("MyModule"))
            .r#type(type_expr)
            .build()
            .unwrap();

        let result = compile_injector(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjector"));
    }

    #[test]
    fn test_compile_injector_with_providers() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ProviderModule"), source_span: None },
            &allocator,
        ));

        let providers_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("PROVIDERS"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectorMetadataBuilder::new(&allocator)
            .name(Ident::from("ProviderModule"))
            .r#type(type_expr)
            .providers(providers_expr)
            .build()
            .unwrap();

        let result = compile_injector(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjector"));
        assert!(output.contains("providers"));
        assert!(output.contains("PROVIDERS"));
    }

    #[test]
    fn test_compile_injector_with_imports() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ImportModule"), source_span: None },
            &allocator,
        ));

        let import1 = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("CommonModule"), source_span: None },
            &allocator,
        ));
        let import2 = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("FormsModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectorMetadataBuilder::new(&allocator)
            .name(Ident::from("ImportModule"))
            .r#type(type_expr)
            .add_import(import1)
            .add_import(import2)
            .build()
            .unwrap();

        let result = compile_injector(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjector"));
        assert!(output.contains("imports"));
        assert!(output.contains("CommonModule"));
        assert!(output.contains("FormsModule"));
    }

    #[test]
    fn test_compile_injector_empty_statements() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("EmptyModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectorMetadataBuilder::new(&allocator)
            .name(Ident::from("EmptyModule"))
            .r#type(type_expr)
            .build()
            .unwrap();

        let result = compile_injector(&allocator, &metadata);

        // Statements should always be empty for injectors
        assert!(result.statements.is_empty());
    }
}

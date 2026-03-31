//! NgModule compilation implementation.
//!
//! Ported from Angular's `render3/r3_module_compiler.ts`.
//!
//! Generates NgModule definitions like:
//! ```javascript
//! ɵmod = ɵɵdefineNgModule({
//!   type: AppModule,
//!   bootstrap: [AppComponent],
//!   declarations: [MyComponent, MyDirective],
//!   imports: [CommonModule],
//!   exports: [MyComponent]
//! })
//! ```

use oxc_allocator::{Allocator, Box, Vec};
use oxc_span::Ident;

use super::metadata::{R3NgModuleMetadata, R3Reference};
use crate::output::ast::{
    BinaryOperator, BinaryOperatorExpr, FunctionExpr, InvokeFunctionExpr, LiteralArrayExpr,
    LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue, OutputExpression, OutputStatement,
    ReadPropExpr, ReadVarExpr, ReturnStatement,
};
use crate::r3::Identifiers;

/// Result of compiling an NgModule.
#[derive(Debug)]
pub struct NgModuleCompileResult<'a> {
    /// The compiled expression: `ɵɵdefineNgModule({...})`
    pub expression: OutputExpression<'a>,

    /// Additional statements (scope registration, module ID registration).
    pub statements: Vec<'a, OutputStatement<'a>>,
}

/// Compiles an NgModule from its metadata.
///
/// This is the main entry point for NgModule compilation.
pub fn compile_ng_module<'a>(
    allocator: &'a Allocator,
    metadata: &R3NgModuleMetadata<'a>,
) -> NgModuleCompileResult<'a> {
    compile_ng_module_from_metadata(allocator, metadata)
}

/// Internal implementation of NgModule compilation.
pub fn compile_ng_module_from_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3NgModuleMetadata<'a>,
) -> NgModuleCompileResult<'a> {
    let mut statements = Vec::new_in(allocator);

    // Build the definition map
    let definition_map = build_definition_map(allocator, metadata);

    // Create the expression: ɵɵdefineNgModule(definitionMap)
    let expression = create_define_ng_module_call(allocator, definition_map);

    // Add scope side effect if needed
    if metadata.should_set_scope_side_effect() {
        if let Some(scope_stmt) = create_set_scope_side_effect(allocator, metadata) {
            statements.push(scope_stmt);
        }
    }

    // Add module ID registration if needed
    if let Some(id) = &metadata.id {
        let register_stmt = create_register_ng_module_type(allocator, &metadata.r#type.value, id);
        statements.push(register_stmt);
    }

    NgModuleCompileResult { expression, statements }
}

/// Builds the definition map for the NgModule.
fn build_definition_map<'a>(
    allocator: &'a Allocator,
    metadata: &R3NgModuleMetadata<'a>,
) -> Vec<'a, LiteralMapEntry<'a>> {
    let mut entries = Vec::new_in(allocator);

    // type: ModuleClass
    entries.push(LiteralMapEntry {
        key: Ident::from("type"),
        value: metadata.r#type.value.clone_in(allocator),
        quoted: false,
    });

    // bootstrap: [ComponentClass, ...]
    if metadata.has_bootstrap() {
        let bootstrap_array =
            create_reference_array(allocator, &metadata.bootstrap, metadata.contains_forward_decls);
        entries.push(LiteralMapEntry {
            key: Ident::from("bootstrap"),
            value: bootstrap_array,
            quoted: false,
        });
    }

    // Inline scope if mode is Inline
    if metadata.should_inline_scope() {
        // declarations: [DirectiveClass, PipeClass, ...]
        if metadata.has_declarations() {
            let declarations_array = create_reference_array(
                allocator,
                &metadata.declarations,
                metadata.contains_forward_decls,
            );
            entries.push(LiteralMapEntry {
                key: Ident::from("declarations"),
                value: declarations_array,
                quoted: false,
            });
        }

        // imports: [ImportedModule, ...]
        if metadata.has_imports() {
            let imports_array = create_reference_array(
                allocator,
                &metadata.imports,
                metadata.contains_forward_decls,
            );
            entries.push(LiteralMapEntry {
                key: Ident::from("imports"),
                value: imports_array,
                quoted: false,
            });
        }

        // exports: [ExportedClass, ...]
        if metadata.has_exports() {
            let exports_array = create_reference_array(
                allocator,
                &metadata.exports,
                metadata.contains_forward_decls,
            );
            entries.push(LiteralMapEntry {
                key: Ident::from("exports"),
                value: exports_array,
                quoted: false,
            });
        }
    }

    // schemas: [CUSTOM_ELEMENTS_SCHEMA, ...]
    if metadata.has_schemas() {
        let schemas_array =
            create_reference_array(allocator, &metadata.schemas, metadata.contains_forward_decls);
        entries.push(LiteralMapEntry {
            key: Ident::from("schemas"),
            value: schemas_array,
            quoted: false,
        });
    }

    // id: 'unique-module-id'
    if let Some(id) = &metadata.id {
        entries.push(LiteralMapEntry {
            key: Ident::from("id"),
            value: id.clone_in(allocator),
            quoted: false,
        });
    }

    entries
}

/// Creates an array expression from references.
/// If `wrap_in_function` is true, wraps in an arrow function for forward decls.
fn create_reference_array<'a>(
    allocator: &'a Allocator,
    refs: &[R3Reference<'a>],
    wrap_in_function: bool,
) -> OutputExpression<'a> {
    let mut items = Vec::new_in(allocator);
    for r in refs {
        items.push(r.value.clone_in(allocator));
    }

    let array = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: items, source_span: None },
        allocator,
    ));

    if wrap_in_function {
        // () => [Class1, Class2, ...]
        let params = Vec::new_in(allocator);
        let mut body = Vec::new_in(allocator);
        body.push(OutputStatement::Return(Box::new_in(
            ReturnStatement { value: array, source_span: None },
            allocator,
        )));

        OutputExpression::Function(Box::new_in(
            FunctionExpr { name: None, params, statements: body, source_span: None },
            allocator,
        ))
    } else {
        array
    }
}

/// Creates the `ɵɵdefineNgModule({...})` call expression.
fn create_define_ng_module_call<'a>(
    allocator: &'a Allocator,
    definition_map: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    // Create i0.ɵɵdefineNgModule
    let define_ng_module_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::DEFINE_NG_MODULE),
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
            fn_expr: Box::new_in(define_ng_module_fn, allocator),
            args,
            pure: true,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates the ɵɵsetNgModuleScope side effect call.
///
/// Generates:
/// ```javascript
/// (function() {
///   (typeof ngJitMode === "undefined" || ngJitMode) &&
///     ɵɵsetNgModuleScope(ModuleClass, {
///       declarations: [...],
///       imports: [...],
///       exports: [...]
///     });
/// })()
/// ```
fn create_set_scope_side_effect<'a>(
    allocator: &'a Allocator,
    metadata: &R3NgModuleMetadata<'a>,
) -> Option<OutputStatement<'a>> {
    // Only create if there's something to set
    if !metadata.has_declarations() && !metadata.has_imports() && !metadata.has_exports() {
        return None;
    }

    // Build the scope map
    let mut scope_entries = Vec::new_in(allocator);

    if metadata.has_declarations() {
        let decls = create_reference_array(
            allocator,
            &metadata.declarations,
            metadata.contains_forward_decls,
        );
        scope_entries.push(LiteralMapEntry {
            key: Ident::from("declarations"),
            value: decls,
            quoted: false,
        });
    }

    if metadata.has_imports() {
        let imports =
            create_reference_array(allocator, &metadata.imports, metadata.contains_forward_decls);
        scope_entries.push(LiteralMapEntry {
            key: Ident::from("imports"),
            value: imports,
            quoted: false,
        });
    }

    if metadata.has_exports() {
        let exports =
            create_reference_array(allocator, &metadata.exports, metadata.contains_forward_decls);
        scope_entries.push(LiteralMapEntry {
            key: Ident::from("exports"),
            value: exports,
            quoted: false,
        });
    }

    let scope_map = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: scope_entries, source_span: None },
        allocator,
    ));

    // Create: ɵɵsetNgModuleScope(ModuleClass, scopeMap)
    let set_scope_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::SET_NG_MODULE_SCOPE),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut set_scope_args = Vec::new_in(allocator);
    set_scope_args.push(metadata.r#type.value.clone_in(allocator));
    set_scope_args.push(scope_map);

    let set_scope_call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(set_scope_fn, allocator),
            args: set_scope_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create: (typeof ngJitMode === "undefined" || ngJitMode)
    let typeof_expr = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from("typeof ngJitMode"), source_span: None },
        allocator,
    ));

    let undefined_check = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Equals,
            lhs: Box::new_in(typeof_expr, allocator),
            rhs: Box::new_in(
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from("undefined")),
                        source_span: None,
                    },
                    allocator,
                )),
                allocator,
            ),
            source_span: None,
        },
        allocator,
    ));

    let ng_jit_mode = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from("ngJitMode"), source_span: None },
        allocator,
    ));

    let jit_mode_check = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(undefined_check, allocator),
            rhs: Box::new_in(ng_jit_mode, allocator),
            source_span: None,
        },
        allocator,
    ));

    // Create: (jitModeCheck) && setNgModuleScope(...)
    let guarded_call = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::And,
            lhs: Box::new_in(jit_mode_check, allocator),
            rhs: Box::new_in(set_scope_call, allocator),
            source_span: None,
        },
        allocator,
    ));

    // Wrap in IIFE: (function() { ... })()
    let params = Vec::new_in(allocator);
    let mut body = Vec::new_in(allocator);
    body.push(OutputStatement::Expression(Box::new_in(
        crate::output::ast::ExpressionStatement { expr: guarded_call, source_span: None },
        allocator,
    )));

    let iife_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr { name: None, params, statements: body, source_span: None },
        allocator,
    ));

    let iife_call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(iife_fn, allocator),
            args: Vec::new_in(allocator),
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    Some(OutputStatement::Expression(Box::new_in(
        crate::output::ast::ExpressionStatement { expr: iife_call, source_span: None },
        allocator,
    )))
}

/// Creates the ɵɵregisterNgModuleType call for module ID registration.
fn create_register_ng_module_type<'a>(
    allocator: &'a Allocator,
    module_type: &OutputExpression<'a>,
    id: &OutputExpression<'a>,
) -> OutputStatement<'a> {
    let register_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::REGISTER_NG_MODULE_TYPE),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut args = Vec::new_in(allocator);
    args.push(module_type.clone_in(allocator));
    args.push(id.clone_in(allocator));

    let call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(register_fn, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    OutputStatement::Expression(Box::new_in(
        crate::output::ast::ExpressionStatement { expr: call, source_span: None },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ng_module::metadata::{R3NgModuleMetadataBuilder, R3SelectorScopeMode};
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_compile_simple_ng_module() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("AppModule"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .build()
            .unwrap();

        let result = compile_ng_module(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineNgModule"));
        assert!(output.contains("AppModule"));
    }

    #[test]
    fn test_compile_ng_module_with_declarations() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyModule"), source_span: None },
            &allocator,
        ));
        let component_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_declaration(R3Reference::value_only(component_expr))
            .selector_scope_mode(R3SelectorScopeMode::Inline)
            .build()
            .unwrap();

        let result = compile_ng_module(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineNgModule"));
        assert!(output.contains("declarations"));
        assert!(output.contains("MyComponent"));
    }

    #[test]
    fn test_compile_ng_module_with_imports_exports() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("SharedModule"), source_span: None },
            &allocator,
        ));
        let import_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("CommonModule"), source_span: None },
            &allocator,
        ));
        let export_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("SharedComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_import(R3Reference::value_only(import_expr))
            .add_export(R3Reference::value_only(export_expr))
            .selector_scope_mode(R3SelectorScopeMode::Inline)
            .build()
            .unwrap();

        let result = compile_ng_module(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("imports"));
        assert!(output.contains("CommonModule"));
        assert!(output.contains("exports"));
        assert!(output.contains("SharedComponent"));
    }

    #[test]
    fn test_compile_ng_module_with_side_effect_scope() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("JitModule"), source_span: None },
            &allocator,
        ));
        let decl_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("JitComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_declaration(R3Reference::value_only(decl_expr))
            .selector_scope_mode(R3SelectorScopeMode::SideEffect)
            .build()
            .unwrap();

        let result = compile_ng_module(&allocator, &metadata);

        // Should have side effect statement
        assert_eq!(result.statements.len(), 1);
    }

    #[test]
    fn test_compile_ng_module_with_forward_decls() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ForwardModule"), source_span: None },
            &allocator,
        ));
        let decl_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ForwardComponent"), source_span: None },
            &allocator,
        ));

        let metadata = R3NgModuleMetadataBuilder::new(&allocator)
            .r#type(R3Reference::value_only(type_expr))
            .add_declaration(R3Reference::value_only(decl_expr))
            .selector_scope_mode(R3SelectorScopeMode::Inline)
            .contains_forward_decls(true)
            .build()
            .unwrap();

        let result = compile_ng_module(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // Should contain arrow function wrapper
        assert!(output.contains("function"));
        assert!(output.contains("return"));
    }
}

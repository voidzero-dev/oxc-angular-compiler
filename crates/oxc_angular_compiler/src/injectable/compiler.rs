//! Injectable compilation implementation.
//!
//! Ported from Angular's `injectable_compiler_2.ts`.
//!
//! Generates injectable definitions like:
//! ```javascript
//! ɵprov = /*@__PURE__*/ ɵɵdefineInjectable({
//!   token: MyService,
//!   factory: MyService.ɵfac,
//!   providedIn: 'root'
//! })
//! ```

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::metadata::{InjectableProvider, ProvidedIn, R3InjectableMetadata};
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DelegatedFnOrClassMetadata,
    R3DependencyMetadata, R3ExpressionFactoryMetadata, R3FactoryDelegateType, R3FactoryDeps,
    R3FactoryMetadata, compile_factory_function,
};
use crate::output::ast::{
    FnParam, FunctionExpr, InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr, ReturnStatement,
};
use crate::r3::Identifiers;

/// Result of compiling an injectable.
#[derive(Debug)]
pub struct InjectableCompileResult<'a> {
    /// The compiled expression: `ɵɵdefineInjectable({...})`
    pub expression: OutputExpression<'a>,

    /// Additional statements (usually empty).
    pub statements: Vec<'a, OutputStatement<'a>>,
}

/// Compiles an injectable from its metadata.
///
/// This is the main entry point for injectable compilation.
pub fn compile_injectable<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectableMetadata<'a>,
) -> InjectableCompileResult<'a> {
    compile_injectable_from_metadata(allocator, metadata)
}

/// Internal implementation of injectable compilation.
pub fn compile_injectable_from_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectableMetadata<'a>,
) -> InjectableCompileResult<'a> {
    // Build the factory expression based on provider type
    let factory_expr = build_factory_expression(allocator, metadata);

    // Build the definition map
    let definition_map = build_definition_map(allocator, metadata, factory_expr);

    // Create the expression: ɵɵdefineInjectable(definitionMap)
    let expression = create_define_injectable_call(allocator, definition_map);

    InjectableCompileResult { expression, statements: Vec::new_in(allocator) }
}

/// Builds the factory expression based on the provider type.
///
/// This follows Angular's injectable_compiler_2.ts:53-121 pattern:
/// - For useClass with deps: use delegated factory with Class type
/// - For useFactory with deps: use delegated factory with Function type
/// - For useValue/useExisting: use expression factory
/// - For simple cases: use direct delegation or arrow wrappers
///
/// See: `packages/compiler/src/injectable_compiler_2.ts:53-121`
fn build_factory_expression<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectableMetadata<'a>,
) -> OutputExpression<'a> {
    // Create base factory metadata (used for delegated/expression cases)
    let base_meta = R3ConstructorFactoryMetadata {
        name: metadata.name.clone(),
        type_expr: metadata.r#type.clone_in(allocator),
        type_decl: metadata.r#type.clone_in(allocator),
        type_argument_count: metadata.type_argument_count,
        deps: R3FactoryDeps::Valid(Vec::new_in(allocator)), // Empty deps for base
        target: FactoryTarget::Injectable,
    };

    match &metadata.provider {
        InjectableProvider::Default => {
            // Default: delegate to the class's own factory
            // factory: MyClass.ɵfac
            create_factory_delegation(allocator, &metadata.r#type)
        }

        InjectableProvider::UseClass { class_expr, is_forward_ref, deps } => {
            match deps {
                Some(deps) if !deps.is_empty() => {
                    // useClass with deps: use delegated factory with conditional pattern
                    // This generates the proper __ngConditionalFactory__ pattern.
                    // See: injectable_compiler_2.ts:69-75
                    let factory_meta = R3FactoryMetadata::Delegated(R3DelegatedFnOrClassMetadata {
                        base: base_meta,
                        delegate: class_expr.clone_in(allocator),
                        delegate_type: R3FactoryDelegateType::Class,
                        delegate_deps: clone_deps_vec(allocator, deps),
                    });
                    let factory_name = format!("{}_Factory", metadata.name);
                    let factory_name = allocator.alloc_str(&factory_name);
                    let result = compile_factory_function(allocator, &factory_meta, factory_name);
                    result.expression
                }
                _ if *is_forward_ref => {
                    // Forward reference: wrap in arrow function
                    create_forward_ref_factory(allocator, class_expr)
                }
                _ => {
                    // useClass without deps: delegate to the alternative class's factory
                    create_factory_delegation(allocator, class_expr)
                }
            }
        }

        InjectableProvider::UseFactory { factory, deps } => {
            match deps {
                Some(deps) if !deps.is_empty() => {
                    // useFactory with deps: use delegated factory with conditional pattern
                    // This generates the proper __ngConditionalFactory__ pattern.
                    // See: injectable_compiler_2.ts:88-95
                    let factory_meta = R3FactoryMetadata::Delegated(R3DelegatedFnOrClassMetadata {
                        base: base_meta,
                        delegate: factory.clone_in(allocator),
                        delegate_type: R3FactoryDelegateType::Function,
                        delegate_deps: clone_deps_vec(allocator, deps),
                    });
                    let factory_name = format!("{}_Factory", metadata.name);
                    let factory_name = allocator.alloc_str(&factory_name);
                    let result = compile_factory_function(allocator, &factory_meta, factory_name);
                    result.expression
                }
                _ => {
                    // useFactory without deps: wrap factory in arrow function
                    create_use_factory_wrapper(allocator, factory)
                }
            }
        }

        InjectableProvider::UseValue { value } => {
            // useValue: use expression factory with conditional pattern
            // See: injectable_compiler_2.ts:102-106
            let factory_meta = R3FactoryMetadata::Expression(R3ExpressionFactoryMetadata {
                base: base_meta,
                expression: value.clone_in(allocator),
            });
            let factory_name = format!("{}_Factory", metadata.name);
            let factory_name = allocator.alloc_str(&factory_name);
            let result = compile_factory_function(allocator, &factory_meta, factory_name);
            result.expression
        }

        InjectableProvider::UseExisting { existing, is_forward_ref } => {
            // useExisting: use expression factory with inject() call
            // See: injectable_compiler_2.ts:107-112
            let inject_expr = create_inject_call(allocator, existing, *is_forward_ref);
            let factory_meta = R3FactoryMetadata::Expression(R3ExpressionFactoryMetadata {
                base: base_meta,
                expression: inject_expr,
            });
            let factory_name = format!("{}_Factory", metadata.name);
            let factory_name = allocator.alloc_str(&factory_name);
            let result = compile_factory_function(allocator, &factory_meta, factory_name);
            result.expression
        }
    }
}

/// Clone a dependencies vector into a new allocator.
fn clone_deps_vec<'a>(
    allocator: &'a Allocator,
    deps: &[R3DependencyMetadata<'a>],
) -> Vec<'a, R3DependencyMetadata<'a>> {
    let mut result = Vec::with_capacity_in(deps.len(), allocator);
    for dep in deps {
        result.push(R3DependencyMetadata {
            token: dep.token.as_ref().map(|t| t.clone_in(allocator)),
            attribute_name_type: dep.attribute_name_type.as_ref().map(|t| t.clone_in(allocator)),
            host: dep.host,
            optional: dep.optional,
            self_: dep.self_,
            skip_self: dep.skip_self,
        });
    }
    result
}

/// Creates an inject() call expression for useExisting.
fn create_inject_call<'a>(
    allocator: &'a Allocator,
    existing: &OutputExpression<'a>,
    is_forward_ref: bool,
) -> OutputExpression<'a> {
    // Resolve forward ref if needed
    let token_expr = if is_forward_ref {
        let resolve_fn = OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Ident::from("i0"), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: Ident::from(Identifiers::RESOLVE_FORWARD_REF),
                optional: false,
                source_span: None,
            },
            allocator,
        ));

        let mut resolve_args = Vec::new_in(allocator);
        resolve_args.push(existing.clone_in(allocator));

        OutputExpression::InvokeFunction(Box::new_in(
            InvokeFunctionExpr {
                fn_expr: Box::new_in(resolve_fn, allocator),
                args: resolve_args,
                pure: false,
                optional: false,
                source_span: None,
            },
            allocator,
        ))
    } else {
        existing.clone_in(allocator)
    };

    // inject(token)
    let inject_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::INJECT),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut inject_args = Vec::new_in(allocator);
    inject_args.push(token_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(inject_fn, allocator),
            args: inject_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates a factory delegation: `MyClass.ɵfac`
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

/// Creates a forward reference factory: `(t) => resolveForwardRef(Type).ɵfac(t)`
fn create_forward_ref_factory<'a>(
    allocator: &'a Allocator,
    type_expr: &OutputExpression<'a>,
) -> OutputExpression<'a> {
    let param_name = Ident::from("t");
    let mut params = Vec::new_in(allocator);
    params.push(FnParam { name: param_name.clone() });

    // resolveForwardRef(Type)
    let resolve_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::RESOLVE_FORWARD_REF),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut resolve_args = Vec::new_in(allocator);
    resolve_args.push(type_expr.clone_in(allocator));

    let resolved_type = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(resolve_fn, allocator),
            args: resolve_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // resolveForwardRef(Type).ɵfac
    let fac_access = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(resolved_type, allocator),
            name: Ident::from("ɵfac"),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // resolveForwardRef(Type).ɵfac(t)
    let mut call_args = Vec::new_in(allocator);
    call_args.push(OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: param_name, source_span: None },
        allocator,
    )));

    let fac_call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fac_access, allocator),
            args: call_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create the arrow function
    let mut body = Vec::new_in(allocator);
    body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: fac_call, source_span: None },
        allocator,
    )));

    OutputExpression::Function(Box::new_in(
        FunctionExpr { name: None, params, statements: body, source_span: None },
        allocator,
    ))
}

/// Creates a useFactory wrapper: `() => factory()`
fn create_use_factory_wrapper<'a>(
    allocator: &'a Allocator,
    factory: &OutputExpression<'a>,
) -> OutputExpression<'a> {
    let params = Vec::new_in(allocator);

    // factory()
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

/// Builds the definition map for the injectable.
fn build_definition_map<'a>(
    allocator: &'a Allocator,
    metadata: &R3InjectableMetadata<'a>,
    factory_expr: OutputExpression<'a>,
) -> Vec<'a, LiteralMapEntry<'a>> {
    let mut entries = Vec::new_in(allocator);

    // token: MyService
    entries.push(LiteralMapEntry {
        key: Ident::from("token"),
        value: metadata.r#type.clone_in(allocator),
        quoted: false,
    });

    // factory: <factory_function>
    entries.push(LiteralMapEntry {
        key: Ident::from("factory"),
        value: factory_expr,
        quoted: false,
    });

    // providedIn: 'root' (only if not None)
    match &metadata.provided_in {
        ProvidedIn::Root => {
            entries.push(LiteralMapEntry {
                key: Ident::from("providedIn"),
                value: OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from("root")),
                        source_span: None,
                    },
                    allocator,
                )),
                quoted: false,
            });
        }
        ProvidedIn::Platform => {
            entries.push(LiteralMapEntry {
                key: Ident::from("providedIn"),
                value: OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from("platform")),
                        source_span: None,
                    },
                    allocator,
                )),
                quoted: false,
            });
        }
        ProvidedIn::Any => {
            entries.push(LiteralMapEntry {
                key: Ident::from("providedIn"),
                value: OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from("any")),
                        source_span: None,
                    },
                    allocator,
                )),
                quoted: false,
            });
        }
        ProvidedIn::Module(module_expr) => {
            entries.push(LiteralMapEntry {
                key: Ident::from("providedIn"),
                value: module_expr.clone_in(allocator),
                quoted: false,
            });
        }
        ProvidedIn::None => {
            // Don't add providedIn field
        }
    }

    entries
}

/// Creates the `ɵɵdefineInjectable({...})` call expression.
fn create_define_injectable_call<'a>(
    allocator: &'a Allocator,
    definition_map: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    // Create i0.ɵɵdefineInjectable
    let define_injectable_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::DEFINE_INJECTABLE),
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
            fn_expr: Box::new_in(define_injectable_fn, allocator),
            args,
            pure: true, // Pure function for tree-shaking
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injectable::metadata::R3InjectableMetadataBuilder;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_compile_simple_injectable() {
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

        let result = compile_injectable(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjectable"));
        assert!(output.contains("MyService"));
        assert!(output.contains("root"));
    }

    #[test]
    fn test_compile_injectable_with_use_value() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("CONFIG_TOKEN"), source_span: None },
            &allocator,
        ));
        let value_expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::String(Ident::from("config_value")),
                source_span: None,
            },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("CONFIG_TOKEN"))
            .r#type(type_expr)
            .use_value(value_expr)
            .provided_in_root()
            .build()
            .unwrap();

        let result = compile_injectable(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjectable"));
        assert!(output.contains("factory"));
        assert!(output.contains("config_value"));
    }

    #[test]
    fn test_compile_injectable_with_use_existing() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("AliasService"), source_span: None },
            &allocator,
        ));
        let existing_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("RealService"), source_span: None },
            &allocator,
        ));

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("AliasService"))
            .r#type(type_expr)
            .use_existing(existing_expr, false)
            .provided_in_root()
            .build()
            .unwrap();

        let result = compile_injectable(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjectable"));
        assert!(output.contains("inject")); // inject(RealService)
    }

    #[test]
    fn test_compile_injectable_no_provided_in() {
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

        let result = compile_injectable(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineInjectable"));
        assert!(!output.contains("providedIn")); // Should not have providedIn
    }

    #[test]
    fn test_compile_injectable_with_use_factory_and_deps() {
        // This test verifies that useFactory with deps generates the conditional factory pattern
        // to fix NG0200 circular dependency errors.
        //
        // Expected output pattern:
        // function CipherService_Factory(__ngFactoryType__) {
        //   let __ngConditionalFactory__ = null;
        //   if (__ngFactoryType__) {
        //     __ngConditionalFactory__ = new __ngFactoryType__();
        //   } else {
        //     __ngConditionalFactory__ = cipherServiceFactory(ɵɵinject(Dep1), ...);
        //   }
        //   return __ngConditionalFactory__;
        // }

        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("CipherService"), source_span: None },
            &allocator,
        ));
        let factory_fn = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("cipherServiceFactory"), source_span: None },
            &allocator,
        ));

        // Create dependency metadata
        let mut deps = Vec::new_in(&allocator);
        deps.push(R3DependencyMetadata {
            token: Some(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from("LogService"), source_span: None },
                &allocator,
            ))),
            attribute_name_type: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        });

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("CipherService"))
            .r#type(type_expr)
            .use_factory(factory_fn, Some(deps))
            .provided_in_root()
            .build()
            .unwrap();

        let result = compile_injectable(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // Verify the conditional factory pattern is present
        assert!(output.contains("__ngFactoryType__"), "Should have factory type parameter");
        assert!(
            output.contains("__ngConditionalFactory__"),
            "Should have conditional factory variable"
        );
        assert!(
            output.contains("if (__ngFactoryType__)"),
            "Should have if statement for factory type"
        );
        assert!(
            output.contains("cipherServiceFactory"),
            "Should call the delegated factory function"
        );
        assert!(output.contains("inject"), "Should inject dependencies");
    }

    #[test]
    fn test_compile_injectable_with_use_class_and_deps() {
        // This test verifies that useClass with deps generates the conditional factory pattern.

        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("BaseService"), source_span: None },
            &allocator,
        ));
        let class_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ConcreteService"), source_span: None },
            &allocator,
        ));

        // Create dependency metadata
        let mut deps = Vec::new_in(&allocator);
        deps.push(R3DependencyMetadata {
            token: Some(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from("DepService"), source_span: None },
                &allocator,
            ))),
            attribute_name_type: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        });

        let metadata = R3InjectableMetadataBuilder::new()
            .name(Ident::from("BaseService"))
            .r#type(type_expr)
            .use_class(class_expr, false, Some(deps))
            .provided_in_root()
            .build()
            .unwrap();

        let result = compile_injectable(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // Verify the conditional factory pattern is present
        assert!(output.contains("__ngFactoryType__"), "Should have factory type parameter");
        assert!(
            output.contains("__ngConditionalFactory__"),
            "Should have conditional factory variable"
        );
        assert!(
            output.contains("if (__ngFactoryType__)"),
            "Should have if statement for factory type"
        );
        assert!(output.contains("new ConcreteService"), "Should instantiate the delegate class");
        assert!(output.contains("inject"), "Should inject dependencies");
    }
}

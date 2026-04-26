//! Builder functions for class metadata expressions.
//!
//! These functions build the decorator, constructor parameter, and property
//! decorator metadata arrays needed for `setClassMetadata()` calls.

use oxc_allocator::{Allocator, Box, Vec as AllocVec};
use oxc_ast::ast::{
    Class, ClassElement, Decorator, Expression, FormalParameter, MethodDefinitionKind, PropertyKey,
    TSType, TSTypeName,
};
use oxc_str::Ident;

use crate::component::{ImportMap, NamespaceRegistry, R3DependencyMetadata};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry,
    LiteralMapExpr, LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::output::oxc_converter::convert_oxc_expression;

/// Build the decorators metadata array expression.
///
/// Creates: `[{ type: Component, args: [{ selector: '...', ... }] }]`
pub fn build_decorator_metadata_array<'a>(
    allocator: &'a Allocator,
    decorators: &[&Decorator<'a>],
    source_text: Option<&'a str>,
) -> OutputExpression<'a> {
    let mut decorator_entries = AllocVec::new_in(allocator);

    for decorator in decorators {
        let mut map_entries = AllocVec::new_in(allocator);

        // Get decorator type name
        let type_expr = match &decorator.expression {
            Expression::CallExpression(call) => match &call.callee {
                Expression::Identifier(id) => Some(OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: id.name.into(), source_span: None },
                    allocator,
                ))),
                Expression::StaticMemberExpression(member) => {
                    // Handle namespaced decorators like ng.Component
                    convert_oxc_expression(allocator, &member.object, source_text).map(|receiver| {
                        OutputExpression::ReadProp(Box::new_in(
                            ReadPropExpr {
                                receiver: Box::new_in(receiver, allocator),
                                name: member.property.name.into(),
                                optional: false,
                                source_span: None,
                            },
                            allocator,
                        ))
                    })
                }
                _ => None,
            },
            Expression::Identifier(id) => Some(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: id.name.into(), source_span: None },
                allocator,
            ))),
            _ => None,
        };

        let Some(type_expr) = type_expr else {
            continue;
        };

        // Add "type" entry
        map_entries.push(LiteralMapEntry {
            key: Ident::from("type"),
            value: type_expr,
            quoted: false,
        });

        // Add "args" entry if the decorator has arguments
        if let Expression::CallExpression(call) = &decorator.expression
            && !call.arguments.is_empty()
        {
            let mut args = AllocVec::new_in(allocator);
            for arg in &call.arguments {
                let expr = arg.to_expression();
                if let Some(converted) = convert_oxc_expression(allocator, expr, source_text) {
                    args.push(converted);
                }
            }

            if !args.is_empty() {
                map_entries.push(LiteralMapEntry {
                    key: Ident::from("args"),
                    value: OutputExpression::LiteralArray(Box::new_in(
                        LiteralArrayExpr { entries: args, source_span: None },
                        allocator,
                    )),
                    quoted: false,
                });
            }
        }

        // Create the decorator object: { type: ..., args: [...] }
        decorator_entries.push(OutputExpression::LiteralMap(Box::new_in(
            LiteralMapExpr { entries: map_entries, source_span: None },
            allocator,
        )));
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: decorator_entries, source_span: None },
        allocator,
    ))
}

/// Build constructor parameters metadata.
///
/// Creates: `() => [{ type: SomeService, decorators: [...] }, ...]`
/// Returns `None` if the class has no constructor.
///
/// For imported types, generates namespace-prefixed references (e.g., `i1.SomeService`)
/// using the constructor dependency metadata and namespace registry. This matches
/// Angular's behavior where type-only imports need namespace imports because
/// TypeScript types are erased at runtime.
pub fn build_ctor_params_metadata<'a>(
    allocator: &'a Allocator,
    class: &Class<'a>,
    constructor_deps: Option<&[R3DependencyMetadata<'a>]>,
    namespace_registry: &mut NamespaceRegistry<'a>,
    import_map: &ImportMap<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    // Find constructor
    let constructor = class.body.body.iter().find_map(|element| {
        if let ClassElement::MethodDefinition(method) = element
            && method.kind == MethodDefinitionKind::Constructor
        {
            return method.value.params.items.as_slice().into();
        }
        None
    })?;

    let mut param_entries = AllocVec::new_in(allocator);

    for (i, param) in constructor.iter().enumerate() {
        let mut map_entries = AllocVec::new_in(allocator);

        // Extract type from TypeScript type annotation, using namespace-prefixed
        // references for imported types when constructor dependency info is available.
        let type_expr = build_param_type_expression(
            allocator,
            param,
            constructor_deps.and_then(|deps| deps.get(i)),
            namespace_registry,
            import_map,
        )
        .unwrap_or_else(|| {
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                allocator,
            ))
        });

        map_entries.push(LiteralMapEntry {
            key: Ident::from("type"),
            value: type_expr,
            quoted: false,
        });

        // Extract decorators from the parameter
        let param_decorators = extract_angular_decorators_from_param(param);
        if !param_decorators.is_empty() {
            let decorators_array =
                build_decorator_metadata_array(allocator, &param_decorators, source_text);
            map_entries.push(LiteralMapEntry {
                key: Ident::from("decorators"),
                value: decorators_array,
                quoted: false,
            });
        }

        param_entries.push(OutputExpression::LiteralMap(Box::new_in(
            LiteralMapExpr { entries: map_entries, source_span: None },
            allocator,
        )));
    }

    // Return null if no parameters
    if param_entries.is_empty() {
        return None;
    }

    // Wrap in arrow function: () => [...]
    let array_expr = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: param_entries, source_span: None },
        allocator,
    ));

    Some(OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: AllocVec::new_in(allocator),
            body: ArrowFunctionBody::Expression(Box::new_in(array_expr, allocator)),
            source_span: None,
        },
        allocator,
    )))
}

/// Build property decorators metadata.
///
/// Creates: `{ propName: [{ type: Input, args: [...] }], ... }`
/// Returns `None` if no properties have Angular decorators.
pub fn build_prop_decorators_metadata<'a>(
    allocator: &'a Allocator,
    class: &Class<'a>,
    source_text: Option<&'a str>,
) -> Option<OutputExpression<'a>> {
    const ANGULAR_PROP_DECORATORS: &[&str] = &[
        "Input",
        "Output",
        "HostBinding",
        "HostListener",
        "ViewChild",
        "ViewChildren",
        "ContentChild",
        "ContentChildren",
    ];

    let mut prop_entries = AllocVec::new_in(allocator);

    for element in &class.body.body {
        let (decorators, property_name) = match element {
            ClassElement::PropertyDefinition(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key))
            }
            ClassElement::MethodDefinition(method) => {
                (&method.decorators, get_property_key_name(&method.key))
            }
            ClassElement::AccessorProperty(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key))
            }
            _ => continue,
        };

        let Some(prop_name) = property_name else {
            continue;
        };

        // Filter to Angular property decorators
        let angular_decorators: std::vec::Vec<_> = decorators
            .iter()
            .filter(|d| {
                let name = get_decorator_name(d);
                name.is_some_and(|n| ANGULAR_PROP_DECORATORS.contains(&n))
            })
            .collect();

        if angular_decorators.is_empty() {
            continue;
        }

        // Build decorators array for this property
        let decorators_array =
            build_decorator_metadata_array(allocator, &angular_decorators, source_text);

        prop_entries.push(LiteralMapEntry {
            key: prop_name,
            value: decorators_array,
            quoted: false,
        });
    }

    if prop_entries.is_empty() {
        return None;
    }

    Some(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: prop_entries, source_span: None },
        allocator,
    )))
}

// ============================================================================
// Internal helper functions
// ============================================================================

/// Build the type expression for a constructor parameter, using namespace-prefixed
/// references for imported types.
///
/// TypeScript type annotations are erased at runtime, so imported types need namespace
/// imports (e.g., `i1.SomeService`) to be available as runtime values.
///
/// When the type annotation name matches the dep token name, the dep's `token_source_module`
/// is used directly. When they differ (e.g., `@Inject(DARK_THEME) theme$: Observable<boolean>`),
/// we look up the type annotation name in the `import_map` to find its source module
/// independently. This matches Angular's behavior where type references in `setClassMetadata`
/// always use namespace-prefixed imports regardless of whether `@Inject` is used.
fn build_param_type_expression<'a>(
    allocator: &'a Allocator,
    param: &FormalParameter<'a>,
    dep: Option<&R3DependencyMetadata<'a>>,
    namespace_registry: &mut NamespaceRegistry<'a>,
    import_map: &ImportMap<'a>,
) -> Option<OutputExpression<'a>> {
    // Extract the type name from the type annotation
    let type_name = extract_param_type_name(param);

    // Use namespace prefix when the type annotation matches the dep token name
    // and the dep has a source module (imported type).
    if let Some(dep) = dep {
        if let Some(ref source_module) = dep.token_source_module {
            if let Some(ref token) = dep.token {
                let type_matches_token =
                    type_name.as_ref().is_some_and(|tn| tn.as_str() == token.as_str());

                if type_matches_token {
                    let name = type_name.unwrap_or_else(|| token.clone());
                    let namespace = namespace_registry.get_or_assign(source_module);
                    return Some(OutputExpression::ReadProp(Box::new_in(
                        ReadPropExpr {
                            receiver: Box::new_in(
                                OutputExpression::ReadVar(Box::new_in(
                                    ReadVarExpr { name: namespace, source_span: None },
                                    allocator,
                                )),
                                allocator,
                            ),
                            name,
                            optional: false,
                            source_span: None,
                        },
                        allocator,
                    )));
                }
            }
        }
    }

    // When the type annotation differs from the dep token (e.g., @Inject(TOKEN) param: SomeType),
    // look up the type annotation name in the import_map to find its source module independently.
    // Only generate namespace-prefixed references for non-type-only imports, since type-only
    // imports (`import type { X }` / `import { type X }`) are erased at runtime and don't
    // resolve to values. Angular's compiler uses typeToValue() which skips interfaces and
    // type aliases; checking is_type_only is the closest heuristic without a full type checker.
    if let Some(ref tn) = type_name {
        if let Some(import_info) = import_map.get(tn) {
            if import_info.is_type_only {
                // Type-only imports are erased at runtime — emit undefined.
                return None;
            }
            let namespace = namespace_registry.get_or_assign(&import_info.source_module);
            return Some(OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: namespace, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    name: tn.clone(),
                    optional: false,
                    source_span: None,
                },
                allocator,
            )));
        }
    }

    // Fall back to extracting the bare type name from the type annotation
    // (for local/global types not in the import_map)
    extract_param_type_expression(allocator, param)
}

/// Extract the type name (as an Atom) from a constructor parameter's type annotation.
///
/// Returns the simple type name from the annotation, if present.
/// Used to get the type name for namespace-prefixed references in metadata.
fn extract_param_type_name<'a>(param: &FormalParameter<'a>) -> Option<Ident<'a>> {
    let type_annotation = param.type_annotation.as_ref()?;
    match &type_annotation.type_annotation {
        TSType::TSTypeReference(type_ref) => match &type_ref.type_name {
            TSTypeName::IdentifierReference(id) => Some(id.name.into()),
            TSTypeName::QualifiedName(qualified) => Some(qualified.right.name.into()),
            TSTypeName::ThisExpression(_) => None,
        },
        _ => None,
    }
}

/// Extract the type expression from a constructor parameter's type annotation.
///
/// This is the fallback path for local types that don't need namespace prefixes.
fn extract_param_type_expression<'a>(
    allocator: &'a Allocator,
    param: &FormalParameter<'a>,
) -> Option<OutputExpression<'a>> {
    // Get the type annotation from the formal parameter
    let type_annotation = param.type_annotation.as_ref()?;

    // Extract the type name from the annotation
    match &type_annotation.type_annotation {
        TSType::TSTypeReference(type_ref) => {
            // Handle simple type references like SomeService
            match &type_ref.type_name {
                TSTypeName::IdentifierReference(id) => Some(OutputExpression::ReadVar(
                    Box::new_in(ReadVarExpr { name: id.name.into(), source_span: None }, allocator),
                )),
                TSTypeName::QualifiedName(qualified) => {
                    // Handle qualified names like ns.SomeType
                    Some(OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: qualified.right.name.into(), source_span: None },
                        allocator,
                    )))
                }
                TSTypeName::ThisExpression(_) => {
                    // this type annotation is not useful for metadata
                    None
                }
            }
        }
        _ => None,
    }
}

/// Extract Angular decorators from a constructor parameter.
fn extract_angular_decorators_from_param<'a, 'b>(
    param: &'b FormalParameter<'a>,
) -> std::vec::Vec<&'b Decorator<'a>> {
    const ANGULAR_PARAM_DECORATORS: &[&str] =
        &["Inject", "Optional", "Self", "SkipSelf", "Host", "Attribute"];

    param
        .decorators
        .iter()
        .filter(|d| {
            let name = get_decorator_name(d);
            name.is_some_and(|n| ANGULAR_PARAM_DECORATORS.contains(&n))
        })
        .collect()
}

/// Get the name of a decorator.
fn get_decorator_name<'a>(decorator: &Decorator<'a>) -> Option<&'a str> {
    match &decorator.expression {
        Expression::CallExpression(call) => match &call.callee {
            Expression::Identifier(id) => Some(id.name.as_str()),
            Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
            _ => None,
        },
        Expression::Identifier(id) => Some(id.name.as_str()),
        _ => None,
    }
}

/// Get property key name as an Atom.
fn get_property_key_name<'a>(key: &PropertyKey<'a>) -> Option<Ident<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.into()),
        PropertyKey::StringLiteral(lit) => Some(lit.value.into()),
        _ => None,
    }
}

//! Builder functions for class metadata expressions.
//!
//! These functions build the decorator, constructor parameter, and property
//! decorator metadata arrays needed for `setClassMetadata()` calls.

use oxc_allocator::{Allocator, Box, Vec as AllocVec};
use oxc_ast::ast::{
    Class, ClassElement, Decorator, Expression, FormalParameter, MethodDefinitionKind,
    ObjectPropertyKind, PropertyKey, TSType, TSTypeName,
};
use oxc_str::Ident;

use crate::component::{ImportMap, NamespaceRegistry, R3DependencyMetadata};
use crate::directive::{
    R3InputMetadata, StringConsts, resolve_template_literal, try_parse_signal_input,
    try_parse_signal_model, try_parse_signal_output, unwrap_initializer_api_expr,
};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry,
    LiteralMapExpr, LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::output::oxc_converter::convert_oxc_expression;

/// Build the decorators metadata array expression.
///
/// Creates: `[{ type: Component, args: [{ selector: '...', ... }] }]`
///
/// When `inlined_template` and/or `inlined_styles` are provided (typically for
/// `@Component` decorators with `templateUrl`/`styleUrls`/`styleUrl` resolved
/// via `ResolvedResources`), the first argument of the first decorator (the
/// component config object literal) is rewritten so that `templateUrl` becomes
/// `template` (with content inlined) and `styleUrls`/`styleUrl` are folded into
/// the `styles` array. This matches Angular's `transformDecoratorResources` (see
/// `inline_component_resources` below for the source-cited semantics) and is
/// required for TestBed JIT recompilation, since Angular's
/// `componentNeedsResolution(metadata)` check throws when `templateUrl` is set
/// without a sibling `template` field, or when `styleUrls?.length > 0`, even
/// though the AOT-compiled `ɵcmp` already has the template baked in.
pub fn build_decorator_metadata_array<'a>(
    allocator: &'a Allocator,
    decorators: &[&Decorator<'a>],
    source_text: Option<&'a str>,
    inlined_template: Option<&'a str>,
    inlined_styles: Option<&[Ident<'a>]>,
    consts: Option<&StringConsts<'a>>,
) -> OutputExpression<'a> {
    let mut decorator_entries = AllocVec::new_in(allocator);

    for (decorator_idx, decorator) in decorators.iter().enumerate() {
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
        map_entries.push(LiteralMapEntry::new(Ident::from("type"), type_expr, false));

        // Add "args" entry if the decorator has arguments
        if let Expression::CallExpression(call) = &decorator.expression
            && !call.arguments.is_empty()
        {
            // Gate resource inlining on the decorator's name, matching Angular's
            // `if (dec.name !== 'Component') return dec;` at the top of
            // `transformDecoratorResources`. Without this, other decorators that
            // happen to use resource-shaped keys (e.g. `@Inject({ templateUrl: … })`,
            // legal TS even if nonsensical) get their literals stripped.
            let is_component_decorator =
                get_decorator_name(decorator).is_some_and(|n| n == "Component");

            let mut args = AllocVec::new_in(allocator);
            for (arg_idx, arg) in call.arguments.iter().enumerate() {
                let expr = arg.to_expression();
                if let Some(mut converted) = convert_oxc_expression(allocator, expr, source_text) {
                    // Inline resolved templates/styles into the first arg of the
                    // first @Component decorator. Other decorators / other args
                    // are left alone.
                    if is_component_decorator && decorator_idx == 0 && arg_idx == 0 {
                        inline_component_resources(
                            allocator,
                            &mut converted,
                            inlined_template,
                            inlined_styles,
                        );
                        // Drop config fields whose value is a template literal with an
                        // unresolvable `${…}` interpolation, matching the AOT `ɵcmp` path
                        // (which drops e.g. an unresolved `selector`). Otherwise the raw
                        // template literal would leak verbatim into `setClassMetadata`.
                        if let Some(consts) = consts {
                            drop_unresolvable_template_literal_fields(
                                allocator,
                                &mut converted,
                                expr,
                                consts,
                            );
                        }
                    }
                    args.push(converted);
                }
            }

            if !args.is_empty() {
                map_entries.push(LiteralMapEntry::new(
                    Ident::from("args"),
                    OutputExpression::LiteralArray(Box::new_in(
                        LiteralArrayExpr { entries: args, source_span: None },
                        allocator,
                    )),
                    false,
                ));
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

/// Rewrite the `@Component` config map so external resource references are
/// inlined into the `setClassMetadata` args.
///
/// Mirrors Angular's `transformDecoratorResources` (in
/// `compiler-cli/src/ngtsc/annotations/component/src/resources.ts`), which
/// operates on a `Map<string, ts.Expression>` and uses `Map.delete` /
/// `Map.set` semantics:
///
/// - **Fast path**: bail out unchanged when the source has none of `templateUrl`,
///   `styleUrls`, `styleUrl`, or `styles` — preserves the original AST for
///   best source-map fidelity.
/// - **`templateUrl` → `template`**: when present, `templateUrl` is deleted and
///   `template` is set to the inlined content. If the source already had a
///   `template` key (illegal but possible), the existing entry is overwritten
///   *in place* with the inlined value (matches `Map.set` on an existing key).
///   Otherwise the new `template` is appended at the end (matches `Map.set` on
///   a fresh key).
/// - **`styleUrls` / `styleUrl` / existing `styles`**: all deleted; the
///   consolidated `styles` array (whitespace-only entries filtered) is appended
///   at the end. `inlined_styles` is the FINAL canonical list — the caller is
///   responsible for merging inline + resolved content (which `resolve_styles`
///   already does into `ComponentMetadata::styles`).
fn inline_component_resources<'a>(
    allocator: &'a Allocator,
    expr: &mut OutputExpression<'a>,
    inlined_template: Option<&'a str>,
    inlined_styles: Option<&[Ident<'a>]>,
) {
    let OutputExpression::LiteralMap(map_box) = expr else {
        return;
    };

    // Fast-path: no resource fields → preserve original AST.
    let has_template_url = map_box.entries.iter().any(|e| e.key.as_str() == "templateUrl");
    let has_style_field = map_box
        .entries
        .iter()
        .any(|e| matches!(e.key.as_str(), "styleUrls" | "styleUrl" | "styles"));
    if !has_template_url && !has_style_field {
        return;
    }

    let original_entries = std::mem::replace(&mut map_box.entries, AllocVec::new_in(allocator));

    // First pass: drop the deleted keys; if both `templateUrl` and `template`
    // existed in source, overwrite the existing `template` in place (Map.set
    // semantics).
    let mut template_emitted = false;
    for entry in original_entries {
        match entry.key.as_str() {
            "templateUrl" | "styleUrls" | "styleUrl" | "styles" => {
                // Dropped — replacements (if any) are emitted below.
            }
            "template" if has_template_url && inlined_template.is_some() => {
                // Overwrite-in-place: emit the inlined value at the source
                // `template` key's original position.
                map_box.entries.push(build_template_entry(allocator, inlined_template.unwrap()));
                template_emitted = true;
            }
            _ => map_box.entries.push(entry),
        }
    }

    // If `templateUrl` was in source but no source `template` slot received
    // the in-place overwrite, append the resolved template at the end —
    // matching `Map.set('template', …)` on a key that didn't previously exist.
    if has_template_url
        && !template_emitted
        && let Some(tpl) = inlined_template
    {
        map_box.entries.push(build_template_entry(allocator, tpl));
    }

    // Styles are *always* appended at the end (we always delete the pre-existing
    // `styles`/`styleUrl(s)`, mirroring Angular's unconditional `metadata.delete`
    // for all three keys before `metadata.set('styles', …)`).
    if let Some(styles) = inlined_styles {
        let mut style_entries = AllocVec::new_in(allocator);
        for style in styles {
            // Match Angular's `style.trim().length > 0` filter.
            if style.as_str().trim().is_empty() {
                continue;
            }
            style_entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(*style), source_span: None },
                allocator,
            )));
        }
        if !style_entries.is_empty() {
            map_box.entries.push(LiteralMapEntry::new(
                Ident::from("styles"),
                OutputExpression::LiteralArray(Box::new_in(
                    LiteralArrayExpr { entries: style_entries, source_span: None },
                    allocator,
                )),
                false,
            ));
        }
    }
}

/// Remove config fields from a converted `@Component` args map when the source
/// value is a template literal whose `${…}` interpolation can't be statically
/// resolved against `consts` (e.g. `selector: \`${UNRESOLVED}-tag\``).
///
/// Angular's partial evaluator (and OXC's AOT `ɵcmp` extraction) drops such
/// fields rather than emitting a half-evaluated literal. Resolvable template
/// literals are left untouched (converted as-is); only the unresolvable ones are
/// dropped, so the raw `${…}` text never leaks into `setClassMetadata`.
fn drop_unresolvable_template_literal_fields<'a>(
    allocator: &'a Allocator,
    converted: &mut OutputExpression<'a>,
    source: &Expression<'a>,
    consts: &StringConsts<'a>,
) {
    let Expression::ObjectExpression(obj) = source else {
        return;
    };
    let OutputExpression::LiteralMap(map) = converted else {
        return;
    };

    for property in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(prop) = property else {
            continue;
        };
        let Expression::TemplateLiteral(tpl) = &prop.value else {
            continue;
        };
        // An empty-interpolation template literal is a plain string — keep it.
        if tpl.expressions.is_empty() {
            continue;
        }
        if resolve_template_literal(allocator, tpl, consts).is_some() {
            continue; // Resolvable — leave the converted value as-is.
        }
        if let Some(key) = get_property_key_name(&prop.key) {
            map.entries.retain(|entry| entry.is_spread || entry.key != key);
        }
    }
}

/// Build a `template: "…"` map entry from the inlined content.
fn build_template_entry<'a>(allocator: &'a Allocator, content: &'a str) -> LiteralMapEntry<'a> {
    LiteralMapEntry::new(
        Ident::from("template"),
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(Ident::from(content)), source_span: None },
            allocator,
        )),
        false,
    )
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

        map_entries.push(LiteralMapEntry::new(Ident::from("type"), type_expr, false));

        // Extract decorators from the parameter
        let param_decorators = extract_angular_decorators_from_param(param);
        if !param_decorators.is_empty() {
            let decorators_array = build_decorator_metadata_array(
                allocator,
                &param_decorators,
                source_text,
                None,
                None,
                None,
            );
            map_entries.push(LiteralMapEntry::new(
                Ident::from("decorators"),
                decorators_array,
                false,
            ));
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
    namespace_registry: &mut NamespaceRegistry<'a>,
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
        let (decorators, property_name, value) = match element {
            ClassElement::PropertyDefinition(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key), prop.value.as_ref())
            }
            ClassElement::MethodDefinition(method) => {
                (&method.decorators, get_property_key_name(&method.key), None)
            }
            ClassElement::AccessorProperty(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key), prop.value.as_ref())
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

        if !angular_decorators.is_empty() {
            // Build decorators array from the real decorators present in source.
            let decorators_array = build_decorator_metadata_array(
                allocator,
                &angular_decorators,
                source_text,
                None,
                None,
                None,
            );
            prop_entries.push(LiteralMapEntry::new(prop_name, decorators_array, false));
            continue;
        }

        // No real Angular prop decorator. Synthesize one for initializer-API members
        // (`input()`/`output()`/`model()`/`viewChild()`/…) so JIT recompilation
        // (`TestBed.overrideComponent`) can reflect them — signal members live only in
        // the AOT `ɵcmp`, which the JIT recompile discards. Mirrors Angular's
        // compiler-cli `initializer_api_transforms` (applied by the Angular CLI in test
        // builds); without it, `setInput`/router-binding fail with NG0315/NG0303/NG0950.
        if let Some(value) = value
            && let Some(decorators_array) = build_initializer_api_prop_decorators(
                allocator,
                value,
                &prop_name,
                source_text,
                namespace_registry,
            )
        {
            prop_entries.push(LiteralMapEntry::new(prop_name, decorators_array, false));
        }
    }

    if prop_entries.is_empty() {
        return None;
    }

    Some(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: prop_entries, source_span: None },
        allocator,
    )))
}

/// Build the synthetic prop-decorator array for a field initialized with an
/// Angular initializer API (`input()`, `output()`, `model()`, or a signal query).
/// Returns `None` when the initializer is not a recognized initializer API.
fn build_initializer_api_prop_decorators<'a>(
    allocator: &'a Allocator,
    value: &Expression<'a>,
    property_name: &Ident<'a>,
    source_text: Option<&'a str>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> Option<OutputExpression<'a>> {
    let mut decorators = AllocVec::new_in(allocator);

    if let Some(input) = try_parse_signal_input(allocator, value, property_name.clone()) {
        // input() / input.required() → `Input({ isSignal, alias, required })`
        decorators.push(build_signal_input_decorator(allocator, namespace_registry, &input));
    } else if let Some(model) = try_parse_signal_model(allocator, value, property_name.clone()) {
        // model() → `Input({ isSignal, alias, required })` + `Output("<name>Change")`
        decorators.push(build_signal_input_decorator(allocator, namespace_registry, &model.input));
        decorators.push(build_core_decorator_with_string_arg(
            allocator,
            namespace_registry,
            "Output",
            model.output.1.clone(),
        ));
    } else if let Some((_, binding)) = try_parse_signal_output(value, property_name.clone()) {
        // output() / outputFromObservable() → `Output("<binding>")`
        decorators.push(build_core_decorator_with_string_arg(
            allocator,
            namespace_registry,
            "Output",
            binding,
        ));
    } else if let Some(query) =
        build_signal_query_decorator(allocator, value, source_text, namespace_registry)
    {
        decorators.push(query);
    }

    if decorators.is_empty() {
        return None;
    }

    Some(OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: decorators, source_span: None },
        allocator,
    )))
}

/// Build `{ type: i0.Input, args: [{ isSignal: true, alias, required }] }`.
///
/// Matches the `setClassMetadata` shape emitted by `@angular/compiler-cli` (verified against
/// ngc's output) for both `input()`/`input.required()` and `model()`'s input: a three-field
/// config with no `transform` key (signal inputs handle transforms via the input signal at
/// runtime, so the decorator carries no transform).
fn build_signal_input_decorator<'a>(
    allocator: &'a Allocator,
    namespace_registry: &mut NamespaceRegistry<'a>,
    input: &R3InputMetadata<'a>,
) -> OutputExpression<'a> {
    let mut config = AllocVec::new_in(allocator);
    config.push(LiteralMapEntry::new(Ident::from("isSignal"), bool_literal(allocator, true), false));
    config.push(LiteralMapEntry::new(
        Ident::from("alias"),
        string_literal(allocator, input.binding_property_name.clone()),
        false,
    ));
    config.push(LiteralMapEntry::new(
        Ident::from("required"),
        bool_literal(allocator, input.required),
        false,
    ));

    let mut args = AllocVec::new_in(allocator);
    args.push(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: config, source_span: None },
        allocator,
    )));
    build_core_decorator(allocator, namespace_registry, "Input", args)
}

/// Build a query decorator from a signal-query initializer
/// (`viewChild`/`viewChildren`/`contentChild`/`contentChildren`), reusing the source
/// positional arguments: `Decorator(<predicate>, { ...<sourceOptions>, isSignal: true })`.
/// Mirrors Angular's `queryFunctionsTransforms`.
fn build_signal_query_decorator<'a>(
    allocator: &'a Allocator,
    value: &Expression<'a>,
    source_text: Option<&'a str>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> Option<OutputExpression<'a>> {
    let Expression::CallExpression(call) = unwrap_initializer_api_expr(value) else {
        return None;
    };
    let decorator_name = signal_query_decorator_name(&call.callee)?;

    // Predicate: the first positional argument (required), reused as-is. A query with
    // no locator is invalid (ngc errors); skip synthesis rather than emit a malformed
    // decorator.
    let predicate =
        convert_oxc_expression(allocator, call.arguments.first()?.to_expression(), source_text)?;
    let mut args = AllocVec::new_in(allocator);
    args.push(predicate);

    // Options: `{ ...<sourceOptions>, isSignal: true }`. Spread the second positional
    // argument verbatim (matching Angular's `factory.createSpreadAssignment(callArgs[1])`),
    // which preserves any options expression, object literal or not.
    let mut options = AllocVec::new_in(allocator);
    if let Some(second) = call.arguments.get(1)
        && let Some(source_options) =
            convert_oxc_expression(allocator, second.to_expression(), source_text)
    {
        options.push(LiteralMapEntry::spread(source_options));
    }
    options.push(LiteralMapEntry::new(Ident::from("isSignal"), bool_literal(allocator, true), false));
    args.push(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: options, source_span: None },
        allocator,
    )));

    Some(build_core_decorator(allocator, namespace_registry, decorator_name, args))
}

/// Map a signal-query initializer callee to its decorator name, handling the direct
/// (`viewChild()`), required (`viewChild.required()`), and namespaced (`core.viewChild()`)
/// forms.
fn signal_query_decorator_name(callee: &Expression<'_>) -> Option<&'static str> {
    fn name_of(function: &str) -> Option<&'static str> {
        match function {
            "viewChild" => Some("ViewChild"),
            "viewChildren" => Some("ViewChildren"),
            "contentChild" => Some("ContentChild"),
            "contentChildren" => Some("ContentChildren"),
            _ => None,
        }
    }

    match callee {
        Expression::Identifier(id) => name_of(id.name.as_str()),
        Expression::StaticMemberExpression(member) => {
            if member.property.name == "required" {
                match &member.object {
                    Expression::Identifier(id) => name_of(id.name.as_str()),
                    Expression::StaticMemberExpression(inner) => name_of(inner.property.name.as_str()),
                    _ => None,
                }
            } else {
                // Namespaced call: `core.viewChild(...)`.
                name_of(member.property.name.as_str())
            }
        }
        _ => None,
    }
}

/// Build `{ type: i0.<name>, args: ["<arg>"] }` for a decorator taking a single string.
fn build_core_decorator_with_string_arg<'a>(
    allocator: &'a Allocator,
    namespace_registry: &mut NamespaceRegistry<'a>,
    decorator_name: &'static str,
    arg: Ident<'a>,
) -> OutputExpression<'a> {
    let mut args = AllocVec::new_in(allocator);
    args.push(string_literal(allocator, arg));
    build_core_decorator(allocator, namespace_registry, decorator_name, args)
}

/// Build a synthetic Angular core decorator metadata object: `{ type: i0.<name>, args: [...] }`.
/// The decorator type is referenced through the `@angular/core` namespace import (`i0`), since a
/// component using signal APIs imports `input`/`output`/… rather than the `Input`/`Output`/query
/// decorators themselves.
fn build_core_decorator<'a>(
    allocator: &'a Allocator,
    namespace_registry: &mut NamespaceRegistry<'a>,
    decorator_name: &'static str,
    args: AllocVec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    let core_namespace = namespace_registry.get_or_assign(&Ident::from("@angular/core"));
    let type_expr = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: core_namespace, source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(decorator_name),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut entries = AllocVec::new_in(allocator);
    entries.push(LiteralMapEntry::new(Ident::from("type"), type_expr, false));
    if !args.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("args"),
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: args, source_span: None },
                allocator,
            )),
            false,
        ));
    }

    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ))
}

/// Build a boolean literal output expression.
fn bool_literal<'a>(allocator: &'a Allocator, value: bool) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Boolean(value), source_span: None },
        allocator,
    ))
}

/// Build a string literal output expression.
fn string_literal<'a>(allocator: &'a Allocator, value: Ident<'a>) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(value), source_span: None },
        allocator,
    ))
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
    // Narrow `T | null` unions to `T` so optional-DI patterns expose the type.
    let ts_type = crate::util::resolve_di_token_type(&type_annotation.type_annotation)?;
    match ts_type {
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

    // Narrow `T | null` unions to `T` so optional-DI patterns expose the type.
    let ts_type = crate::util::resolve_di_token_type(&type_annotation.type_annotation)?;

    // Extract the type name from the annotation
    match ts_type {
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

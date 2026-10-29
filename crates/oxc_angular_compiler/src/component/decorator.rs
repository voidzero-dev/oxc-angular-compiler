//! Angular `@Component` decorator parser.
//!
//! This module extracts metadata from `@Component({...})` decorators
//! on TypeScript class declarations.

use oxc_allocator::{Allocator, Vec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, Class, ClassElement, Declaration, Decorator,
    Expression, MethodDefinitionKind, ObjectExpression, ObjectPropertyKind, PropertyKey, Statement,
};
use oxc_span::Span;
use oxc_str::Ident;

use super::dependency::R3DependencyMetadata;
use super::metadata::{
    ChangeDetectionStrategy, ComponentMetadata, HostDirectiveMetadata, HostMetadata, ImportRef,
    TemplateDependency, ViewEncapsulation,
};
use super::transform::ImportMap;
use crate::directive::{
    HostKey, HostValue, MemberHostBindings, StringConsts, evaluate_host_directives,
    extract_input_metadata_in, extract_output_metadata_in, merge_by_class_property,
    merge_host_bindings, parse_decorator_io, try_unwrap_forward_ref,
};
use crate::output::oxc_converter::convert_oxc_expression;
use crate::util::is_metadata_property;

/// Extract component metadata from a class with decorators.
///
/// Searches for a `@Component({...})` decorator and parses its properties.
/// Returns `None` if no `@Component` decorator is found.
///
/// The `implicit_standalone` parameter determines the default value for `standalone`
/// when not explicitly set in the decorator. This should be:
/// - `true` for Angular v19+
/// - `false` for Angular v18 and earlier
/// - `true` when the Angular version is unknown (assume latest)
///
/// The `import_map` parameter is used to resolve the source module for constructor
/// dependency tokens. This enables tracking where imports come from (e.g., `"@angular/core"`).
///
/// # Example
///
/// ```typescript
/// @Component({
///   selector: 'app-root',
///   template: '<h1>Hello</h1>',
///   standalone: true,
/// })
/// export class AppComponent {}
/// ```
pub fn extract_component_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    implicit_standalone: bool,
    import_map: &ImportMap<'a>,
    source_text: Option<&'a str>,
    consts: &StringConsts<'a>,
) -> Option<ComponentMetadata<'a>> {
    // Get the class name
    let class_name: Ident<'a> = class.id.as_ref()?.name.clone().into();
    let class_span = class.span;

    let config_obj = component_config(class, consts)?;

    // Create metadata with defaults (standalone uses implicit_standalone when not explicitly set)
    let mut metadata =
        ComponentMetadata::new(allocator, class_name, class_span, implicit_standalone);
    if let Some(source) = source_text {
        metadata.class_name_pos = crate::class_debug_info::class_name_pos(source, class);
    }

    // Whether `imports` and `deferredImports` are provably empty; a spread or
    // accessor this loop skips could add them.
    let mut imports_provably_empty = true;

    // Parse each property in the config object
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop
            && is_metadata_property(prop)
        {
            // A string key with a lone surrogate (`'\uD800'`) has no `&str`
            // form and names no option, so it is ignored like any unknown key.
            if matches!(&prop.key, PropertyKey::StringLiteral(lit) if lit.value.has_lone_surrogate())
            {
                continue;
            }
            let key_name = get_property_key_name(&prop.key, consts)?;

            match key_name.as_str() {
                "selector" => {
                    metadata.selector =
                        crate::directive::evaluate_string_value(allocator, &prop.value, consts)
                            // ngtsc maps `selector: ''` to the default selector
                            // ('ng-component' for components), same as a missing
                            // selector. See annotations/directive/src/shared.ts
                            // (`resolved === '' ? defaultSelector : resolved`).
                            .filter(|s| !s.as_str().is_empty());
                }
                "template" => {
                    // A value that isn't statically resolvable (e.g. a
                    // template literal referencing an unknown identifier)
                    // leaves `metadata.template` as `None`: the caller then
                    // emits the component with no ɵcmp and no diagnostic.
                    // Upstream errors instead ("Unresolved identifier found
                    // for @Component.template field" in local mode,
                    // "template must be a string" otherwise) — pre-existing.
                    metadata.template =
                        crate::directive::evaluate_string_value(allocator, &prop.value, consts);
                    // The text between the quotes or backticks.
                    metadata.template_span = match &prop.value {
                        Expression::StringLiteral(lit) => Some(lit.span),
                        Expression::TemplateLiteral(lit) if lit.expressions.is_empty() => {
                            Some(lit.span)
                        }
                        _ => None,
                    }
                    .map(|span| Span::new(span.start + 1, span.end - 1));
                }
                "templateUrl" => {
                    // Evaluated like ngtsc's `parseTemplateDeclaration`, so a
                    // URL imported from another file resolves when the consts
                    // carry the cross-file resolver. The resource extractor
                    // (`extract_component_resource_urls`) reads it the same way.
                    metadata.template_url =
                        crate::directive::evaluate_string_value(allocator, &prop.value, consts);
                }
                "styles" => {
                    // ngtsc's `reflectObjectLiteral` drops computed keys and
                    // folds duplicate `styles` entries with `Map.set` — the
                    // last one is the only one `parseDirectiveStyles` sees.
                    if prop.computed {
                        continue;
                    }
                    // Anything that isn't styles is an error `styles_error` reports.
                    match crate::directive::evaluate_styles(&prop.value, consts) {
                        Ok(styles) => {
                            metadata.styles = Vec::from_iter_in(
                                styles.iter().map(|style| Ident::from(allocator.alloc_str(style))),
                                &allocator,
                            );
                            // A later `styles:` overwrites an earlier failed
                            // one: `Map.set` makes the last entry the only
                            // one upstream reads.
                            metadata.styles_resolved = true;
                        }
                        Err(_) => {
                            // Failed evaluation is reported as a diagnostic;
                            // here it marks the value unknown so readers do
                            // not mistake the empty array for "no styles".
                            metadata.styles.clear();
                            metadata.styles_resolved = false;
                        }
                    }
                }
                "styleUrls" | "styleUrl" => {
                    read_style_urls(allocator, &prop.value, consts, &mut metadata.style_urls);
                }
                "standalone" => {
                    // Only override the implicit value if an explicit boolean is provided
                    if let Some(value) = extract_boolean_value(&prop.value) {
                        metadata.standalone = value;
                    }
                }
                "encapsulation" => {
                    metadata.encapsulation = extract_encapsulation(&prop.value);
                }
                "changeDetection" => {
                    metadata.change_detection = Some(extract_change_detection(&prop.value));
                }
                "host" => {
                    // ngtsc's `reflectObjectLiteral` drops computed keys, and
                    // `host_metadata_error` doesn't see this property either —
                    // an invalid one must not be applied here where it would
                    // be dropped silently.
                    if !prop.computed {
                        metadata.host =
                            extract_host_metadata(allocator, &prop.value, source_text, consts);
                    }
                }
                "imports" => {
                    imports_provably_empty &= is_empty_array_literal(&prop.value);
                    // For standalone components, we need:
                    // 1. The identifier list for local analysis
                    metadata.imports = extract_identifier_array(allocator, &prop.value);
                    metadata.import_entry_classes =
                        extract_import_entry_classes(allocator, &prop.value, consts);
                    metadata.import_refs = extract_import_refs(allocator, &prop.value, consts);
                    // 2. The raw expression to pass to ɵɵgetComponentDepsFactory in RuntimeResolved mode
                    metadata.raw_imports =
                        convert_oxc_expression(allocator, &prop.value, source_text);
                }
                "deferredImports" => {
                    imports_provably_empty &= is_empty_array_literal(&prop.value);
                    // Symbols explicitly opted into lazy loading via `@defer`.
                    // Mirrors Angular's `@Component.deferredImports` field; in
                    // local compilation this is the only source of deferrable
                    // dependencies (full compilation additionally derives them
                    // from template usage of `imports: [...]`, which requires
                    // cross-file selector info OXC doesn't have).
                    metadata.deferred_imports = extract_identifier_array(allocator, &prop.value);
                }
                "exportAs" => {
                    // exportAs can be comma-separated: "foo, bar"
                    if let Some(export_as) =
                        crate::directive::evaluate_string_value(allocator, &prop.value, consts)
                    {
                        for part in export_as.as_str().split(',') {
                            let trimmed = part.trim();
                            if !trimmed.is_empty() {
                                metadata.export_as.push(Ident::from(allocator.alloc_str(trimmed)));
                            }
                        }
                    }
                }
                "preserveWhitespaces" => {
                    metadata.preserve_whitespaces =
                        Some(extract_boolean_value(&prop.value).unwrap_or(false));
                }
                "animations" => {
                    // Extract animations expression as full OutputExpression
                    // Handles both identifier references and complex array expressions
                    metadata.animations =
                        convert_oxc_expression(allocator, &prop.value, source_text);
                }
                "schemas" => {
                    // Extract schemas identifiers (e.g., [CUSTOM_ELEMENTS_SCHEMA, NO_ERRORS_SCHEMA])
                    metadata.schemas = extract_identifier_array(allocator, &prop.value);
                }
                "providers" => {
                    // Extract providers as full OutputExpression
                    // Handles complex expressions like [{provide: TOKEN, useFactory: Factory}]
                    metadata.providers =
                        convert_oxc_expression(allocator, &prop.value, source_text);
                }
                "viewProviders" => {
                    // Extract view providers as full OutputExpression
                    metadata.view_providers =
                        convert_oxc_expression(allocator, &prop.value, source_text);
                }
                "hostDirectives" => {
                    // Extract host directives array
                    // Handles both simple identifiers and complex objects with inputs/outputs
                    metadata.host_directives =
                        extract_host_directives(allocator, &prop.value, import_map, consts);
                }
                "signals" => {
                    // Extract signals flag (true if component uses signal-based inputs)
                    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:382-390
                    if let Some(value) = extract_boolean_value(&prop.value) {
                        metadata.is_signal = value;
                    }
                }
                _ => {
                    // Unknown property - ignore
                }
            }
        } else {
            imports_provably_empty = false;
        }
    }
    metadata.imports_provably_empty = imports_provably_empty;

    // Merge @HostBinding/@HostListener members into the `host:` bindings the
    // way ngtsc's `extractHostBindings` does (shared with @Directive).
    let members = MemberHostBindings::of(allocator, class, Some(consts));
    if !members.is_empty() || metadata.host.is_some() {
        let host = metadata.host.get_or_insert_with(|| HostMetadata::new(allocator));
        merge_host_bindings(
            &mut host.properties,
            &mut host.listeners,
            &mut host.attributes,
            members,
        );
    }

    // Detect if the component extends another class
    // Similar to Angular's: const usesInheritance = reflector.hasBaseClass(clazz);
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:393
    // NOTE: This must be set BEFORE extract_constructor_deps because that function
    // uses this information to determine whether to use inherited factory pattern.
    let has_superclass = class.heritage.is_some();
    metadata.uses_inheritance = has_superclass;

    // Extract constructor dependencies for factory generation
    // This enables proper DI for component constructors
    metadata.constructor_deps =
        extract_constructor_deps(allocator, class, import_map, has_superclass, consts);

    // Inputs/outputs from the `inputs:`/`outputs:` metadata, overridden by
    // @Input/@Output/signal members ({...fromMeta, ...fromFields} in ngtsc).
    let io = parse_decorator_io(allocator, config_obj, class, source_text, consts);
    metadata.inputs = merge_by_class_property(
        io.inputs,
        extract_input_metadata_in(allocator, class, source_text, Some(consts)),
        |i| i.class_property_name.as_str(),
    );
    metadata.outputs = merge_by_class_property(
        io.outputs,
        extract_output_metadata_in(allocator, class, Some(consts)),
        |o| o.0.as_str(),
    );

    // Detect if ngOnChanges lifecycle hook is implemented
    // Similar to Angular's: const usesOnChanges = members.some(member => ...)
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:315-319
    metadata.lifecycle.uses_on_changes = has_ng_on_changes_method(class);

    // Set declaration_list_emit_mode based on standalone flag and raw imports.
    // This matches Angular's local compilation mode logic:
    // - Non-standalone components use RuntimeResolved (dependencies resolved at runtime)
    // - Standalone components with raw imports also use RuntimeResolved
    // - Standalone components without raw imports use Direct
    // See: packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts:1249-1252
    if !metadata.standalone || metadata.raw_imports.is_some() {
        metadata.declaration_list_emit_mode =
            super::metadata::DeclarationListEmitMode::RuntimeResolved;
    }

    // Populate declarations from imports with source module information.
    // This converts the identifier names from the imports array into TemplateDependency
    // entries with their source modules resolved from the import_map.
    // This is only done for Direct mode (standalone without raw_imports) because
    // RuntimeResolved mode passes imports directly to ɵɵgetComponentDepsFactory.
    if metadata.declaration_list_emit_mode == super::metadata::DeclarationListEmitMode::Direct {
        populate_declarations_from_imports(allocator, &mut metadata, import_map);
    }

    Some(metadata)
}

/// Populate template declarations from the imports array.
///
/// For each import identifier, creates a `TemplateDependency` with the source module
/// resolved from the import_map. This enables generating namespaced references like
/// `i1.DirectiveClass` instead of bare `DirectiveClass`.
///
/// Note: This treats all imports as directive dependencies for now. In a full implementation,
/// the template compiler would distinguish between directives, components, and pipes based
/// on the actual template usage.
fn populate_declarations_from_imports<'a>(
    allocator: &'a Allocator,
    metadata: &mut ComponentMetadata<'a>,
    import_map: &ImportMap<'a>,
) {
    for import_name in &metadata.imports {
        // Create a template dependency for this import
        // We treat all imports as directive dependencies for simplicity
        // A more sophisticated implementation would analyze the template to determine
        // the actual dependency type (directive, component, pipe, or NgModule)
        let mut dep = TemplateDependency::directive(
            &allocator,
            import_name.clone(),
            // Use a placeholder selector - the actual selector isn't used for dependencies array
            Ident::from("*"),
            false, // is_component - unknown at this point
        );

        // Resolve the source module from the import map
        if let Some(import_info) = import_map.get(import_name) {
            dep = dep.with_source_module(import_info.source_module.clone());
        }

        metadata.declarations.push(dep);
    }
}

/// Find Angular's @Component decorator (imported from `@angular/core`, in the
/// file `consts` was collected from) in a list of decorators: see
/// [`crate::directive::find_angular_class_decorator`].
pub fn find_component_decorator<'a>(
    decorators: &'a [Decorator<'a>],
    consts: &StringConsts<'_>,
) -> Option<&'a Decorator<'a>> {
    crate::directive::find_angular_class_decorator(decorators, "Component", consts)
}

/// Get the name of a property key as a string.
///
/// Resolves same-file `const` identifiers in computed keys (`[FOO]: bar`) so the
/// emitted component metadata matches the official Angular compiler's output.
fn get_property_key_name<'a>(
    key: &PropertyKey<'a>,
    consts: &StringConsts<'a>,
) -> Option<Ident<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(lit) => lit.value.as_str().map(Ident::from),
        // Computed identifier reference: `[FOO]: bar` — resolve against same-file consts.
        PropertyKey::Identifier(id) => consts.get(id.name.as_str()).cloned(),
        _ => None,
    }
}

/// Whether `expr` is the array literal `[]`.
fn is_empty_array_literal(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::ArrayExpression(array) if array.elements.is_empty())
}

/// Extract a boolean value from an expression.
fn extract_boolean_value(expr: &Expression<'_>) -> Option<bool> {
    match expr {
        Expression::BooleanLiteral(lit) => Some(lit.value.into()),
        _ => None,
    }
}
/// The `{...}` config object of `class`'s `@Component(...)`, or `None` when
/// the class has no `@Component` call with an object literal argument.
fn component_config<'a>(
    class: &'a Class<'a>,
    consts: &StringConsts<'_>,
) -> Option<&'a ObjectExpression<'a>> {
    let component_decorator = find_component_decorator(&class.decorators, consts)?;
    let Expression::CallExpression(call_expr) = &component_decorator.expression else {
        return None;
    };
    match call_expr.arguments.first()? {
        Argument::ObjectExpression(obj) => Some(obj),
        _ => None,
    }
}

/// One `styleUrl` / `styleUrls` property: an array literal replaces the URLs
/// read so far; any other value adds to them.
fn read_style_urls<'a>(
    allocator: &'a Allocator,
    value: &'a Expression<'a>,
    consts: &StringConsts<'a>,
    urls: &mut Vec<'a, Ident<'a>>,
) {
    if matches!(value, Expression::ArrayExpression(_)) {
        urls.clear();
    }
    crate::directive::evaluate_style_urls(allocator, value, consts, urls);
}

/// The `templateUrl` and `styleUrl(s)` of `class`'s `@Component`, read exactly
/// as [`extract_component_metadata`] reads them, and `None` exactly when it
/// gives `None`. Only those properties are evaluated, so the import value
/// resolver in `consts` (if any) reads another file only when one of them
/// names an imported binding — never for `host`, `styles`, inputs or deps.
pub fn extract_component_resource_urls_of<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    consts: &StringConsts<'a>,
) -> Option<(Option<Ident<'a>>, Vec<'a, Ident<'a>>)> {
    class.id.as_ref()?;
    let config_obj = component_config(class, consts)?;
    let mut template_url = None;
    let mut style_urls = Vec::new_in(&allocator);
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop
            && is_metadata_property(prop)
        {
            if matches!(&prop.key, PropertyKey::StringLiteral(lit) if lit.value.has_lone_surrogate())
            {
                continue;
            }
            match get_property_key_name(&prop.key, consts)?.as_str() {
                "templateUrl" => {
                    template_url =
                        crate::directive::evaluate_string_value(allocator, &prop.value, consts);
                }
                "styleUrls" | "styleUrl" => {
                    read_style_urls(allocator, &prop.value, consts, &mut style_urls);
                }
                _ => {}
            }
        }
    }
    Some((template_url, style_urls))
}

/// Extract an array of identifiers (for imports).
fn extract_identifier_array<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> Vec<'a, Ident<'a>> {
    let mut result = Vec::new_in(&allocator);

    let Expression::ArrayExpression(arr) = expr else {
        return result;
    };

    for element in &arr.elements {
        match element {
            ArrayExpressionElement::Identifier(id) => {
                result.push(id.name.clone().into());
            }
            // Handle spread elements, etc. - for now just collect identifiers
            _ => {}
        }
    }

    result
}

/// The class each element of an array literal names, by position: `X`, or
/// `forwardRef(() => X)` with `@angular/core`'s `forwardRef`, with the span of
/// the identifier.
fn extract_import_entry_classes<'a>(
    allocator: &'a Allocator,
    expr: &'a Expression<'a>,
    consts: &StringConsts<'_>,
) -> Vec<'a, Option<(Ident<'a>, Span)>> {
    let mut result = Vec::new_in(&allocator);
    if let Expression::ArrayExpression(arr) = expr {
        for element in &arr.elements {
            let class = element.as_expression().and_then(|element| {
                match try_unwrap_forward_ref(element, Some(consts)).unwrap_or(element) {
                    Expression::Identifier(id) => Some((id.name, id.span)),
                    _ => None,
                }
            });
            result.push(class);
        }
    }
    result
}

/// The classes an `imports` value names: the entries of an array literal
/// that [`extract_import_entry_classes`] reads, or of the array a same-file
/// `const` holds when `imports` names one (`imports: IMPORTS`). The spans are
/// those of the identifiers in the `const`'s array, which every component
/// naming it shares.
fn extract_import_refs<'a>(
    allocator: &'a Allocator,
    expr: &'a Expression<'a>,
    consts: &StringConsts<'a>,
) -> Vec<'a, ImportRef<'a>> {
    let expr = match expr {
        Expression::Identifier(id) => match top_level_const_init(consts, &id.name) {
            Some(init @ Expression::ArrayExpression(_)) => init,
            _ => return Vec::new_in(&allocator),
        },
        _ => expr,
    };
    let mut refs = Vec::new_in(&allocator);
    for (name, span) in extract_import_entry_classes(allocator, expr, consts).into_iter().flatten()
    {
        refs.push(ImportRef { name, span });
    }
    refs
}

/// The initializer of the top-level variable (`const`, `let` or `var`,
/// exported or not) that binds `name` in the file `consts` were collected
/// from. Like ngtsc's partial evaluator, it reads through `as const` and
/// through a variable that names another one (`const IMPORTS = BASE`).
fn top_level_const_init<'a>(consts: &StringConsts<'a>, name: &str) -> Option<&'a Expression<'a>> {
    let mut name = name;
    // `const A = B; const B = A;` never ends.
    for _ in 0..16 {
        let init = consts.program()?.body.iter().find_map(|stmt| {
            let decl = match stmt {
                Statement::VariableDeclaration(decl) => decl,
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::VariableDeclaration(decl) => decl,
                    _ => return None,
                },
                _ => return None,
            };
            decl.declarations.iter().find_map(|declarator| match &declarator.id {
                BindingPattern::BindingIdentifier(id) if id.name == name => {
                    declarator.init.as_ref()
                }
                _ => None,
            })
        })?;
        match init.get_inner_expression() {
            Expression::Identifier(id) => name = id.name.as_str(),
            init => return Some(init),
        }
    }
    None
}

/// Extract ViewEncapsulation from an expression.
fn extract_encapsulation(expr: &Expression<'_>) -> ViewEncapsulation {
    // Look for patterns like:
    // - ViewEncapsulation.None
    // - ViewEncapsulation.Emulated
    // - ViewEncapsulation.ShadowDom
    // - 0, 2, 3 (numeric values)
    match expr {
        Expression::StaticMemberExpression(member) => match member.property.name.as_str() {
            "None" => ViewEncapsulation::None,
            "ShadowDom" => ViewEncapsulation::ShadowDom,
            "Emulated" => ViewEncapsulation::Emulated,
            _ => ViewEncapsulation::default(),
        },
        Expression::NumericLiteral(num) => {
            // Angular's numeric values: Emulated = 0, None = 2, ShadowDom = 3
            match num.value as i32 {
                0 => ViewEncapsulation::Emulated,
                2 => ViewEncapsulation::None,
                3 => ViewEncapsulation::ShadowDom,
                _ => ViewEncapsulation::default(),
            }
        }
        _ => ViewEncapsulation::default(),
    }
}

/// Extract ChangeDetectionStrategy from an expression.
fn extract_change_detection(expr: &Expression<'_>) -> ChangeDetectionStrategy {
    match expr {
        Expression::StaticMemberExpression(member) => match member.property.name.as_str() {
            "OnPush" => ChangeDetectionStrategy::OnPush,
            "Eager" => ChangeDetectionStrategy::Eager,
            // `Default` (value 1) is the pre-v22 spelling of `Eager`. Keep it
            // distinct so partial emit can preserve the author's exact member
            // for older Angular targets that lack `Eager`.
            "Default" => ChangeDetectionStrategy::Default,
            _ => ChangeDetectionStrategy::default(),
        },
        Expression::NumericLiteral(num) => {
            // Angular v22 numeric values: OnPush = 0, Eager = 1 (Default = 1).
            match num.value as i32 {
                0 => ChangeDetectionStrategy::OnPush,
                1 => ChangeDetectionStrategy::Eager,
                _ => ChangeDetectionStrategy::default(),
            }
        }
        _ => ChangeDetectionStrategy::default(),
    }
}

/// Extract host metadata from a host expression.
///
/// Reference: packages/compiler/src/render3/view/compiler.ts:560-604
fn extract_host_metadata<'a>(
    allocator: &'a Allocator,
    expr: &'a Expression<'a>,
    source_text: Option<&'a str>,
    consts: &StringConsts<'a>,
) -> Option<HostMetadata<'a>> {
    // An unusable host is reported by `decorator_io_errors`.
    let entries =
        crate::directive::evaluate_host_metadata(allocator, expr, "Component", consts).ok()?;

    let mut host = HostMetadata::new(allocator);
    for (key, value) in entries {
        let kind = HostKey::of(key.as_str());
        if matches!(kind, HostKey::Attribute) {
            host.attributes.push((key, value.to_output(allocator, source_text)?));
            continue;
        }
        // Only a plain attribute has an expression value.
        let HostValue::String(value) = value else { continue };
        match kind {
            HostKey::Property => host.properties.push((key, value)),
            HostKey::Listener => host.listeners.push((key, value)),
            HostKey::Class => host.class_attr = Some(value),
            HostKey::Style => host.style_attr = Some(value),
            HostKey::Attribute => {}
        }
    }
    Some(host)
}

/// Extract host directives from a `hostDirectives` expression, read through
/// the evaluator like ngtsc's `extractHostDirectives` (see
/// [`crate::directive::evaluate_host_directives`]).
fn extract_host_directives<'a>(
    allocator: &'a Allocator,
    expr: &'a Expression<'a>,
    import_map: &ImportMap<'a>,
    consts: &StringConsts<'a>,
) -> Vec<'a, HostDirectiveMetadata<'a>> {
    let entries = evaluate_host_directives(allocator, expr, consts);
    let mut result = Vec::with_capacity_in(entries.len(), &allocator);
    for entry in entries {
        let has_mapping_keys = entry.has_mapping_keys();
        let mut meta = HostDirectiveMetadata::new(allocator, entry.name);
        // Look up the source module from the import map
        if let Some(import_info) = import_map.get(&entry.name) {
            meta.source_module = Some(import_info.source_module);
        }
        meta.is_forward_reference = entry.is_forward_reference;
        meta.has_mapping_keys = has_mapping_keys;
        if let Some(inputs) = entry.inputs {
            meta.inputs = inputs;
        }
        if let Some(outputs) = entry.outputs {
            meta.outputs = outputs;
        }
        result.push(meta);
    }
    result
}

/// Check if a class has an `ngOnChanges` method.
///
/// This is used to determine if the component implements the `OnChanges` interface
/// and needs the `NgOnChangesFeature` added to its features array.
///
/// Per Angular's shared.ts (lines 315-319):
/// ```typescript
/// const usesOnChanges = members.some(
///   (member) =>
///     !member.isStatic && member.kind === ClassMemberKind.Method && member.name === 'ngOnChanges',
/// );
/// ```
fn has_ng_on_changes_method(class: &Class<'_>) -> bool {
    class.body.body.iter().any(|element| {
        if let ClassElement::MethodDefinition(method) = element {
            // Check: not static, is a method, named "ngOnChanges"
            // MethodDefinitionKind::Method covers regular methods (not constructor, getter, setter)
            !method.r#static
                && method.kind == MethodDefinitionKind::Method
                && method.key.static_name().is_some_and(|name| name == "ngOnChanges")
        } else {
            false
        }
    })
}

/// Extract constructor dependencies from a class.
///
/// Returns:
/// - `Some(deps)`: Constructor found OR no constructor but no superclass (use simple factory)
/// - `None`: No constructor AND has superclass (use inherited factory pattern)
///
/// This distinction is important because Angular handles these cases differently:
/// - `Some([])` (empty vec) = No deps needed → Generate `new Class()`
/// - `None` = Must inherit from parent → Generate `ɵɵgetInheritedFactory` IIFE pattern
///
/// The logic matches Angular's `getConstructorDependencies` in `di.ts`:
/// ```typescript
/// if (ctorParams === null) {
///   if (reflector.hasBaseClass(clazz)) {
///     return null;  // use inherited factory
///   } else {
///     ctorParams = [];  // use simple factory
///   }
/// }
/// ```
///
/// Handles parameter decorators like `@Inject()`, `@Optional()`, `@SkipSelf()`,
/// `@Self()`, `@Host()`, and `@Attribute()`.
///
/// Example:
/// ```typescript
/// @Component({ selector: 'app-root', template: '' })
/// class AppComponent {
///   constructor(
///     private broadcasterService: BroadcasterService,
///     @Inject(WINDOW) private win: Window,
///     @Optional() private optionalService?: OptionalService,
///   ) {}
/// }
/// ```
/// Returns: `Some(Vec)` containing R3DependencyMetadata for each parameter.
fn extract_constructor_deps<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    import_map: &ImportMap<'a>,
    has_superclass: bool,
    consts: &StringConsts<'a>,
) -> Option<Vec<'a, R3DependencyMetadata<'a>>> {
    // Find the constructor method
    let constructor = crate::util::find_constructor(class);

    match constructor {
        Some(ctor) => {
            // Constructor found - extract parameters (may be empty)
            let mut deps = Vec::new_in(&allocator);
            let params = &ctor.value.params;

            for param in &params.items {
                let dep = extract_param_dependency(allocator, param, import_map, consts);
                deps.push(dep);
            }

            Some(deps)
        }
        None => {
            // No constructor found
            // If class has a superclass, use inherited factory pattern (return None)
            // If class has no superclass, use simple factory with empty deps (return Some([]))
            // See: packages/compiler-cli/src/ngtsc/annotations/common/src/di.ts:47-52
            if has_superclass { None } else { Some(Vec::new_in(&allocator)) }
        }
    }
}

/// Extract dependency metadata from a single constructor parameter.
///
/// The `import_map` is used to look up the source module of the token,
/// enabling proper tracking of import origins for constructor dependencies.
/// Only Angular's parameter decorators count, imported from `@angular/core`
/// (see [`crate::directive::angular_param_decorator`]).
fn extract_param_dependency<'a>(
    allocator: &'a Allocator,
    param: &'a oxc_ast::ast::FormalParameter<'a>,
    import_map: &ImportMap<'a>,
    consts: &StringConsts<'a>,
) -> R3DependencyMetadata<'a> {
    // Extract flags and @Inject token from decorators
    let mut optional = false;
    let mut skip_self = false;
    let mut self_ = false;
    let mut host = false;
    let mut inject_token: Option<(Ident<'a>, &'a [Ident<'a>])> = None;
    let mut inject_token_is_string = false;
    let mut attribute_name: Option<Ident<'a>> = None;

    for decorator in &param.decorators {
        if let Some(name) = crate::directive::angular_param_decorator(decorator, Some(consts)) {
            match name {
                "Inject" => {
                    // @Inject(TOKEN) - extract the token
                    if let Expression::CallExpression(call) = &decorator.expression {
                        if let Some(arg) = call.arguments.first() {
                            // `@Inject('TOKEN')`: a string token, as ngtsc compiles it.
                            // The last `@Inject` wins, so the flag is set on every one.
                            let string_token =
                                arg.as_expression().and_then(extract_string_inject_token);
                            inject_token_is_string = string_token.is_some();
                            inject_token = string_token
                                .map(|token| (token, &[][..]))
                                .or_else(|| extract_inject_token(allocator, arg));
                        }
                    }
                }
                "Optional" => optional = true,
                "SkipSelf" => skip_self = true,
                "Self" => self_ = true,
                "Host" => host = true,
                "Attribute" => {
                    // @Attribute('attrName') - extract the attribute name
                    if let Expression::CallExpression(call) = &decorator.expression {
                        if let Some(Argument::StringLiteral(s)) = call.arguments.first()
                            && let Some(name) = s.value.as_str()
                        {
                            attribute_name = Some(name.into());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Determine the token:
    // 1. If @Inject(TOKEN) is present, use TOKEN
    // 2. Otherwise, use the type annotation
    let from_inject = inject_token.is_some();
    let (token, member_path) = inject_token
        .or_else(|| extract_param_token(allocator, param))
        .map_or((None, &[][..]), |(t, m)| (Some(t), m));

    // Handle @Attribute decorator
    if let Some(attr_name) = attribute_name {
        return R3DependencyMetadata::attribute(attr_name);
    }

    if inject_token_is_string && let Some(value) = token {
        let mut dep = R3DependencyMetadata::new(value);
        dep.token_is_string = true;
        dep.optional = optional;
        dep.skip_self = skip_self;
        dep.self_ = self_;
        dep.host = host;
        return dep;
    }

    // Build the dependency metadata
    let mut dep = match &token {
        Some(token_name) => {
            // Look up the token in the import map to find its source module and import type
            if let Some(import_info) = import_map.get(token_name) {
                if import_info.is_type_only {
                    // Type-only imports (`import type { X }` / `import { type X }`)
                    // are erased at runtime and cannot be used as DI tokens. The whole
                    // factory must collapse to `ɵɵinvalidFactory()` — matching Angular's
                    // `ValueUnavailableKind.TYPE_ONLY_IMPORT`. See issue #288.
                    R3DependencyMetadata::type_only_invalid()
                } else {
                    let mut d = R3DependencyMetadata::new(token_name.clone());
                    d.token_source_module = Some(import_info.source_module.clone());
                    d.token_member_path = member_path;
                    d.token_is_namespace_import =
                        !member_path.is_empty() && !import_info.is_named_import;
                    // Carry the exported name so namespaced refs (`i1.X`) use the export
                    // name, not the local alias, for aliased imports.
                    d.token_imported_name = import_info.imported_name.clone();
                    // Type-annotation tokens use namespace imports (`i1.X`). Like ngtsc,
                    // an `@Inject(TOKEN)` token keeps the expression as written and a
                    // default import keeps its local name; import elision keeps both.
                    d.has_named_import = from_inject || import_info.is_default_import;
                    d
                }
            } else {
                let mut d = R3DependencyMetadata::new(*token_name);
                d.token_member_path = member_path;
                d
            }
        }
        None => R3DependencyMetadata::invalid(),
    };

    dep.optional = optional;
    dep.skip_self = skip_self;
    dep.self_ = self_;
    dep.host = host;

    dep
}

/// The value of a string `@Inject` token: a string literal, or a template literal with no
/// substitutions, also inside parentheses or a TypeScript cast.
fn extract_string_inject_token<'a>(expr: &'a Expression<'a>) -> Option<Ident<'a>> {
    match expr.get_inner_expression() {
        Expression::StringLiteral(lit) => lit.value.as_str().map(Ident::from),
        Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
            tpl.quasis.first()?.value.cooked.as_ref()?.as_str().map(Ident::from)
        }
        _ => None,
    }
}

/// Extract the injection token from an @Inject decorator argument: its leftmost
/// name and the members read off it (`TOKEN.a.b` → `TOKEN`, `[a, b]`).
fn extract_inject_token<'a>(
    allocator: &'a Allocator,
    arg: &'a Argument<'a>,
) -> Option<(Ident<'a>, &'a [Ident<'a>])> {
    let mut members = std::vec::Vec::new();
    let mut expr = arg.as_expression()?;
    while let Expression::StaticMemberExpression(member) = expr {
        members.push(member.property.name);
        expr = &member.object;
    }
    let Expression::Identifier(id) = expr else { return None };
    members.reverse();
    Some((id.name, allocator.alloc_slice_copy(&members)))
}

/// Extract the injection token from a parameter's type annotation: its
/// leftmost name and the members read off it (`ns.one.One` → `ns`, `[one, One]`).
fn extract_param_token<'a>(
    allocator: &'a Allocator,
    param: &'a oxc_ast::ast::FormalParameter<'a>,
) -> Option<(Ident<'a>, &'a [Ident<'a>])> {
    // Get the type annotation (directly on FormalParameter)
    let type_annotation = param.type_annotation.as_ref()?;
    // Narrow `T | null` unions to `T` to match the reference compiler.
    let ts_type = crate::util::resolve_di_token_type(&type_annotation.type_annotation)?;

    // Handle TSTypeReference: `SomeClass`, or a qualified `ns.SomeClass`.
    // For primitive types, `this` or other patterns, return None (invalid dependency).
    let oxc_ast::ast::TSType::TSTypeReference(type_ref) = ts_type else { return None };
    let path = crate::class_metadata::entity_path(&type_ref.type_name)?;
    let (root, members) = path.split_first()?;
    Some((*root, allocator.alloc_slice_copy(members)))
}

// =============================================================================
// Decorator Span Collection for Removal
// =============================================================================

/// Collect the spans of Angular's constructor parameter decorators.
///
/// Parameter decorators like `@Optional()`, `@Inject()`, `@Host()`, `@Self()`,
/// `@SkipSelf()`, and `@Attribute()` need to be removed from the output since
/// their metadata is extracted into the factory function's inject flags.
/// Like ngtsc, only Angular's own count (imported from `@angular/core`, under
/// any alias or through a namespace import, see
/// [`crate::directive::angular_param_decorator`]); another module's is left
/// in place.
///
/// These spans are used by `transform.rs` to remove the decorators from the
/// source text during transformation.
pub fn collect_constructor_decorator_spans(
    class: &Class<'_>,
    consts: &StringConsts<'_>,
    spans: &mut std::vec::Vec<Span>,
) {
    for_each_constructor_param_decorator(class, |decorator| {
        if crate::directive::angular_param_decorator(decorator, Some(consts)).is_some() {
            spans.push(decorator.span);
        }
    });
}

/// Collect the spans of every constructor parameter decorator, Angular's or
/// not. JIT output removes them all: Angular's go into `ctorParameters`, the
/// rest into the class's `__decorate` call as `__param(...)`.
pub fn collect_all_constructor_decorator_spans(class: &Class<'_>, spans: &mut std::vec::Vec<Span>) {
    for_each_constructor_param_decorator(class, |decorator| spans.push(decorator.span));
}

/// Call `f` with each decorator on the constructor's parameters, in order.
fn for_each_constructor_param_decorator<'a>(
    class: &'a Class<'a>,
    mut f: impl FnMut(&'a Decorator<'a>),
) {
    let constructor = crate::util::find_constructor(class);
    for param in constructor.into_iter().flat_map(|ctor| &ctor.value.params.items) {
        param.decorators.iter().for_each(&mut f);
    }
}

/// Collect all Angular decorator spans from class members (properties, methods, accessors).
///
/// Member decorators like `@Input()`, `@Output()`, `@HostBinding()`, `@HostListener()`,
/// `@ViewChild()`, `@ViewChildren()`, `@ContentChild()`, and `@ContentChildren()` need
/// to be removed from the output since their metadata is compiled into the definition.
/// Like ngtsc, only Angular's own count (imported from `@angular/core`, under any
/// alias or through a namespace import, see
/// [`crate::directive::angular_member_decorator`]); a same-named decorator from
/// another module is left in place.
///
/// These spans are used by `transform.rs` to remove the decorators from the
/// source text during transformation.
pub fn collect_member_decorator_spans(
    class: &Class<'_>,
    consts: &StringConsts<'_>,
    spans: &mut std::vec::Vec<Span>,
) {
    for element in &class.body.body {
        let decorators = match element {
            ClassElement::PropertyDefinition(prop) => &prop.decorators,
            ClassElement::MethodDefinition(method) => {
                // Skip constructor - it's handled separately
                if method.kind == MethodDefinitionKind::Constructor {
                    continue;
                }
                &method.decorators
            }
            ClassElement::AccessorProperty(accessor) => &accessor.decorators,
            _ => continue,
        };

        for decorator in decorators {
            if crate::directive::angular_member_decorator(decorator, Some(consts)).is_some() {
                spans.push(decorator.span);
            }
        }
    }
}

/// Collect ALL decorator spans from class members (properties, methods, accessors),
/// regardless of whether they are Angular-specific or not.
///
/// This is used when lowering a class that has Angular decorators: since the class
/// declaration is converted to a class expression, ALL member decorators must be
/// removed (decorators are not valid on class expressions in TypeScript).
pub fn collect_all_member_decorator_spans(class: &Class<'_>, spans: &mut std::vec::Vec<Span>) {
    for element in &class.body.body {
        let decorators = match element {
            ClassElement::PropertyDefinition(prop) => &prop.decorators,
            ClassElement::MethodDefinition(method) => {
                // Skip constructor - it's handled separately
                if method.kind == MethodDefinitionKind::Constructor {
                    continue;
                }
                &method.decorators
            }
            ClassElement::AccessorProperty(accessor) => &accessor.decorators,
            _ => continue,
        };

        for decorator in decorators {
            spans.push(decorator.span);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::transform::build_import_map;
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    /// Helper function to parse TypeScript code and extract component metadata
    /// from the first @Component decorated class found.
    ///
    /// The callback receives the extracted metadata and can perform assertions.
    fn with_extracted_metadata<F>(code: &str, implicit_standalone: bool, callback: F)
    where
        F: FnOnce(Option<&ComponentMetadata<'_>>),
    {
        let allocator = Allocator::default();
        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        // Build import map from the program body
        let import_map = build_import_map(&allocator, &parser_ret.program.body, None);
        let consts = crate::directive::collect_string_consts(&allocator, &parser_ret.program);

        // Find the first class declaration (handles plain, export default, and export named)
        let mut found_metadata = None;
        for stmt in &parser_ret.program.body {
            let class = match stmt {
                Statement::ClassDeclaration(class) => Some(class.as_ref()),
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                _ => None,
            };

            if let Some(class) = class {
                if let Some(metadata) = extract_component_metadata(
                    &allocator,
                    class,
                    implicit_standalone,
                    &import_map,
                    Some(code),
                    &consts,
                ) {
                    found_metadata = Some(metadata);
                    break;
                }
            }
        }

        callback(found_metadata.as_ref());
    }

    /// Shorthand for tests that expect metadata to be found with implicit_standalone=true.
    fn assert_metadata<F>(code: &str, callback: F)
    where
        F: FnOnce(&ComponentMetadata<'_>),
    {
        with_extracted_metadata(code, true, |meta| {
            let meta = meta.expect("Expected to find @Component metadata");
            callback(meta);
        });
    }

    /// Shorthand for tests that expect no metadata to be found.
    fn assert_no_metadata(code: &str) {
        with_extracted_metadata(code, true, |meta| {
            assert!(meta.is_none(), "Expected no @Component metadata to be found");
        });
    }

    // =========================================================================
    // Basic extraction tests
    // =========================================================================

    #[test]
    fn test_extract_selector() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({ selector: 'app-test' })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "app-test");
        });
    }

    #[test]
    fn test_extract_selector_with_attribute() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({ selector: '[appDirective]' })
            class TestDirective {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appDirective]");
        });
    }

    #[test]
    fn test_extract_empty_selector_falls_back_to_default() {
        // ngtsc maps `selector: ''` to the default selector ('ng-component'),
        // same as a missing selector. Storing None lets every emit path
        // (ɵcmp selectors, partial ɵɵngDeclareComponent, .d.ts) apply that
        // default. See https://github.com/voidzero-dev/oxc-angular-compiler/issues/514
        let code = r#"
            import {Component} from '@angular/core';
            @Component({ selector: '', template: '' })
            class EmptySelectorComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.selector.is_none(), "Empty selector should normalize to None");
        });
    }

    #[test]
    fn test_extract_class_name() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({ selector: 'my-component' })
            class MyAwesomeComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "MyAwesomeComponent");
        });
    }

    // =========================================================================
    // Template extraction tests
    // =========================================================================

    #[test]
    fn test_extract_inline_template() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '<h1>Hello World</h1>'
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.template.as_ref().unwrap().as_str(), "<h1>Hello World</h1>");
            assert!(meta.template_url.is_none());
        });
    }

    #[test]
    fn test_extract_template_url() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                templateUrl: './test.component.html'
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.template.is_none());
            assert_eq!(meta.template_url.as_ref().unwrap().as_str(), "./test.component.html");
        });
    }

    #[test]
    fn test_extract_template_with_backticks() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: `
                    <div>
                        <span>Multi-line template</span>
                    </div>
                `
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.template.is_some());
            let template = meta.template.as_ref().unwrap();
            assert!(template.contains("Multi-line template"));
        });
    }

    // =========================================================================
    // Standalone tests
    // =========================================================================

    #[test]
    fn test_extract_standalone_true() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                standalone: true
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.standalone);
        });
    }

    #[test]
    fn test_extract_standalone_false() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                standalone: false
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.standalone);
        });
    }

    #[test]
    fn test_standalone_defaults_to_implicit_value_true() {
        // When standalone is not specified, it should use the implicit value
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        // Test with implicit_standalone=true
        with_extracted_metadata(code, true, |meta| {
            let meta = meta.unwrap();
            assert!(meta.standalone);
        });
    }

    #[test]
    fn test_standalone_defaults_to_implicit_value_false() {
        // When standalone is not specified, it should use the implicit value
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        // Test with implicit_standalone=false
        with_extracted_metadata(code, false, |meta| {
            let meta = meta.unwrap();
            assert!(!meta.standalone);
        });
    }

    // =========================================================================
    // Encapsulation tests
    // =========================================================================

    #[test]
    fn test_extract_encapsulation_none() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                encapsulation: ViewEncapsulation.None
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.encapsulation, ViewEncapsulation::None);
        });
    }

    #[test]
    fn test_extract_encapsulation_shadow_dom() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                encapsulation: ViewEncapsulation.ShadowDom
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.encapsulation, ViewEncapsulation::ShadowDom);
        });
    }

    #[test]
    fn test_extract_encapsulation_emulated() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                encapsulation: ViewEncapsulation.Emulated
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.encapsulation, ViewEncapsulation::Emulated);
        });
    }

    #[test]
    fn test_extract_encapsulation_numeric_none() {
        // Angular uses numeric values: Emulated=0, None=2, ShadowDom=3
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                encapsulation: 2
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.encapsulation, ViewEncapsulation::None);
        });
    }

    #[test]
    fn test_extract_encapsulation_numeric_shadow_dom() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                encapsulation: 3
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.encapsulation, ViewEncapsulation::ShadowDom);
        });
    }

    #[test]
    fn test_encapsulation_defaults_to_emulated() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.encapsulation, ViewEncapsulation::Emulated);
        });
    }

    // =========================================================================
    // Change detection tests
    // =========================================================================

    #[test]
    fn test_extract_change_detection_on_push() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                changeDetection: ChangeDetectionStrategy.OnPush
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, Some(ChangeDetectionStrategy::OnPush));
        });
    }

    #[test]
    fn test_extract_change_detection_default_is_distinct() {
        // `Default` (value 1) is the pre-v22 spelling of `Eager`. It is kept as
        // a distinct variant so partial emit can preserve the author's member.
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                changeDetection: ChangeDetectionStrategy.Default
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, Some(ChangeDetectionStrategy::Default));
        });
    }

    #[test]
    fn test_extract_change_detection_numeric_eager() {
        // Angular v22 numeric values: OnPush = 0, Eager = 1.
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                changeDetection: 1
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, Some(ChangeDetectionStrategy::Eager));
        });
    }

    #[test]
    fn test_change_detection_unspecified_is_none() {
        // When the decorator omits `changeDetection`, the metadata leaves it
        // `None` so the emitter applies the target version's default (OnPush in
        // v22+, Default/Eager before).
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, None);
        });
    }

    // =========================================================================
    // Styles tests
    // =========================================================================

    #[test]
    fn test_extract_styles_array() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                styles: ['.host { display: block; }', ':host { color: red; }']
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.styles.len(), 2);
            assert_eq!(meta.styles[0].as_str(), ".host { display: block; }");
            assert_eq!(meta.styles[1].as_str(), ":host { color: red; }");
        });
    }

    #[test]
    fn test_extract_styles_single_string() {
        // Legacy support: styles can be a single string
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                styles: '.host { display: block; }'
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.styles.len(), 1);
            assert_eq!(meta.styles[0].as_str(), ".host { display: block; }");
        });
    }

    #[test]
    fn test_extract_style_urls() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                styleUrls: ['./test.component.css', './shared.css']
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.style_urls.len(), 2);
            assert_eq!(meta.style_urls[0].as_str(), "./test.component.css");
            assert_eq!(meta.style_urls[1].as_str(), "./shared.css");
        });
    }

    #[test]
    fn test_extract_style_url_single() {
        // styleUrl (singular) support
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                styleUrl: './test.component.css'
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.style_urls.len(), 1);
            assert_eq!(meta.style_urls[0].as_str(), "./test.component.css");
        });
    }

    // =========================================================================
    // Host metadata tests
    // =========================================================================

    #[test]
    fn test_extract_host_properties() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                host: {
                    '[class.active]': 'isActive',
                    '[attr.aria-label]': 'label'
                }
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.properties.len(), 2);
        });
    }

    #[test]
    fn test_extract_host_listeners() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                host: {
                    '(click)': 'onClick()',
                    '(keydown.enter)': 'onEnter($event)'
                }
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.listeners.len(), 2);
        });
    }

    #[test]
    fn test_extract_host_static_attributes() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                host: {
                    'role': 'button',
                    'tabindex': '0'
                }
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.attributes.len(), 2);
        });
    }

    #[test]
    fn test_extract_host_class_attr() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                host: {
                    'class': 'btn btn-primary'
                }
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.class_attr.as_ref().unwrap().as_str(), "btn btn-primary");
        });
    }

    #[test]
    fn test_extract_host_style_attr() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                host: {
                    'style': 'display: block; color: red;'
                }
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.style_attr.as_ref().unwrap().as_str(), "display: block; color: red;");
        });
    }

    // =========================================================================
    // Imports tests
    // =========================================================================

    #[test]
    fn test_extract_imports() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                imports: [CommonModule, RouterModule, SharedModule]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.imports.len(), 3);
            assert_eq!(meta.imports[0].as_str(), "CommonModule");
            assert_eq!(meta.imports[1].as_str(), "RouterModule");
            assert_eq!(meta.imports[2].as_str(), "SharedModule");
        });
    }

    // =========================================================================
    // Other metadata tests
    // =========================================================================

    #[test]
    fn test_extract_export_as() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                exportAs: 'myComponent'
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.export_as.len(), 1);
            assert_eq!(meta.export_as[0].as_str(), "myComponent");
        });
    }

    #[test]
    fn test_extract_export_as_multiple() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                exportAs: 'foo, bar, baz'
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.export_as.len(), 3);
            assert_eq!(meta.export_as[0].as_str(), "foo");
            assert_eq!(meta.export_as[1].as_str(), "bar");
            assert_eq!(meta.export_as[2].as_str(), "baz");
        });
    }

    #[test]
    fn test_extract_preserve_whitespaces_true() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                preserveWhitespaces: true
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.preserve_whitespaces, Some(true));
        });
    }

    #[test]
    fn test_extract_preserve_whitespaces_false() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                preserveWhitespaces: false
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.preserve_whitespaces, Some(false));
        });
    }

    #[test]
    fn test_preserve_whitespaces_unset() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.preserve_whitespaces, None);
        });
    }

    #[test]
    fn test_extract_schemas() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                schemas: [CUSTOM_ELEMENTS_SCHEMA, NO_ERRORS_SCHEMA]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.schemas.len(), 2);
            assert_eq!(meta.schemas[0].as_str(), "CUSTOM_ELEMENTS_SCHEMA");
            assert_eq!(meta.schemas[1].as_str(), "NO_ERRORS_SCHEMA");
        });
    }

    // =========================================================================
    // Edge cases and special scenarios
    // =========================================================================

    #[test]
    fn test_no_component_decorator_returns_none() {
        let code = r#"
            class PlainClass {}
        "#;
        assert_no_metadata(code);
    }

    #[test]
    fn test_component_decorator_without_call_returns_none() {
        // @Component without () should not match
        let code = r#"
            @Component
            class TestComponent {}
        "#;
        assert_no_metadata(code);
    }

    #[test]
    fn test_empty_component_decorator() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({})
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.selector.is_none());
            assert!(meta.template.is_none());
            assert_eq!(meta.class_name.as_str(), "TestComponent");
        });
    }

    #[test]
    fn test_exported_class() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({ selector: 'app-test', template: '' })
            export class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "app-test");
        });
    }

    #[test]
    fn test_export_default_class() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({ selector: 'app-test', template: '' })
            export default class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "app-test");
        });
    }

    #[test]
    fn test_namespaced_component_decorator() {
        // Handle ng.Component or core.Component
        let code = r#"
            import * as ng from '@angular/core';
            @ng.Component({ selector: 'app-test', template: '' })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "app-test");
        });
    }

    #[test]
    fn test_full_component_decorator() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-complete',
                template: '<div>{{title}}</div>',
                styles: [':host { display: block; }'],
                standalone: true,
                encapsulation: ViewEncapsulation.None,
                changeDetection: ChangeDetectionStrategy.OnPush,
                host: {
                    'class': 'app-complete',
                    '[class.active]': 'isActive',
                    '(click)': 'onClick()'
                },
                imports: [CommonModule],
                exportAs: 'complete',
                preserveWhitespaces: true,
                schemas: [CUSTOM_ELEMENTS_SCHEMA]
            })
            class CompleteComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "app-complete");
            assert_eq!(meta.template.as_ref().unwrap().as_str(), "<div>{{title}}</div>");
            assert_eq!(meta.styles.len(), 1);
            assert!(meta.standalone);
            assert_eq!(meta.encapsulation, ViewEncapsulation::None);
            assert_eq!(meta.change_detection, Some(ChangeDetectionStrategy::OnPush));
            assert!(meta.host.is_some());
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.class_attr.as_ref().unwrap().as_str(), "app-complete");
            assert_eq!(host.properties.len(), 1);
            assert_eq!(host.listeners.len(), 1);
            assert_eq!(meta.imports.len(), 1);
            assert_eq!(meta.export_as.len(), 1);
            assert_eq!(meta.export_as[0].as_str(), "complete");
            assert_eq!(meta.preserve_whitespaces, Some(true));
            assert_eq!(meta.schemas.len(), 1);
        });
    }

    // =========================================================================
    // Host directives tests
    // =========================================================================

    #[test]
    fn test_extract_host_directives_simple() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [TooltipDirective]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "TooltipDirective");
            assert!(!meta.host_directives[0].is_forward_reference);
            assert!(meta.host_directives[0].inputs.is_empty());
            assert!(meta.host_directives[0].outputs.is_empty());
        });
    }

    #[test]
    fn test_extract_host_directives_multiple() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [TooltipDirective, HighlightDirective, DragDropDirective]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 3);
            assert_eq!(meta.host_directives[0].directive.as_str(), "TooltipDirective");
            assert_eq!(meta.host_directives[1].directive.as_str(), "HighlightDirective");
            assert_eq!(meta.host_directives[2].directive.as_str(), "DragDropDirective");
        });
    }

    #[test]
    fn test_extract_host_directives_object_form() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    { directive: ColorDirective }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "ColorDirective");
        });
    }

    #[test]
    fn test_extract_host_directives_with_input_mappings() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    {
                        directive: ColorDirective,
                        inputs: ['color: bgColor']
                    }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "ColorDirective");
            assert_eq!(meta.host_directives[0].inputs.len(), 1);
            // ("bgColor", "color") - (public, internal)
            assert_eq!(meta.host_directives[0].inputs[0].0.as_str(), "bgColor");
            assert_eq!(meta.host_directives[0].inputs[0].1.as_str(), "color");
        });
    }

    #[test]
    fn test_extract_host_directives_with_output_mappings() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    {
                        directive: ClickDirective,
                        outputs: ['clicked: trackClick']
                    }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].outputs.len(), 1);
            assert_eq!(meta.host_directives[0].outputs[0].0.as_str(), "trackClick");
            assert_eq!(meta.host_directives[0].outputs[0].1.as_str(), "clicked");
        });
    }

    #[test]
    fn test_extract_host_directives_with_input_output_mappings() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    {
                        directive: ResizableDirective,
                        inputs: ['minWidth: resizeMinWidth', 'maxWidth: resizeMaxWidth'],
                        outputs: ['resized: onResized']
                    }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].inputs.len(), 2);
            assert_eq!(meta.host_directives[0].outputs.len(), 1);
        });
    }

    #[test]
    fn test_extract_host_directives_same_name_mapping() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    {
                        directive: OpacityDirective,
                        inputs: ['opacity']
                    }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].inputs.len(), 1);
            // Same name for both public and internal
            assert_eq!(meta.host_directives[0].inputs[0].0.as_str(), "opacity");
            assert_eq!(meta.host_directives[0].inputs[0].1.as_str(), "opacity");
        });
    }

    #[test]
    fn test_extract_host_directives_tuple_array_mapping() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    {
                        directive: ColorDirective,
                        inputs: [['color', 'bgColor']]
                    }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].inputs.len(), 1);
            // Tuple array: ["internal", "public"]
            assert_eq!(meta.host_directives[0].inputs[0].0.as_str(), "bgColor");
            assert_eq!(meta.host_directives[0].inputs[0].1.as_str(), "color");
        });
    }

    #[test]
    fn test_extract_host_directives_forward_ref() {
        let code = r#"
            import {Component, forwardRef} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [forwardRef(() => MyDirective)]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "MyDirective");
            assert!(meta.host_directives[0].is_forward_reference);
        });
    }

    #[test]
    fn test_extract_host_directives_forward_ref_in_object() {
        let code = r#"
            import {Component, forwardRef} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    {
                        directive: forwardRef(() => MyDirective),
                        inputs: ['value']
                    }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "MyDirective");
            assert!(meta.host_directives[0].is_forward_reference);
            assert_eq!(meta.host_directives[0].inputs.len(), 1);
        });
    }

    #[test]
    fn test_extract_host_directives_empty() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: []
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.host_directives.is_empty());
        });
    }

    #[test]
    fn test_extract_host_directives_mixed() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    AccessibilityDirective,
                    {
                        directive: AnimationDirective,
                        inputs: ['animation: animationType']
                    },
                    FocusDirective
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 3);
            assert_eq!(meta.host_directives[0].directive.as_str(), "AccessibilityDirective");
            assert!(meta.host_directives[0].inputs.is_empty());

            assert_eq!(meta.host_directives[1].directive.as_str(), "AnimationDirective");
            assert_eq!(meta.host_directives[1].inputs.len(), 1);

            assert_eq!(meta.host_directives[2].directive.as_str(), "FocusDirective");
            assert!(meta.host_directives[2].inputs.is_empty());
        });
    }

    #[test]
    fn test_extract_host_directives_source_module_from_import() {
        // Test that host directive source_module is populated from imports
        let code = r#"
            import {Component} from '@angular/core';
            import { AriaDisableDirective } from "../a11y/aria-disable.directive";
            import { FocusDirective, HighlightDirective } from "@angular/cdk/a11y";

            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [AriaDisableDirective, FocusDirective]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 2);

            // AriaDisableDirective from local path
            assert_eq!(meta.host_directives[0].directive.as_str(), "AriaDisableDirective");
            assert_eq!(
                meta.host_directives[0].source_module.as_ref().unwrap().as_str(),
                "../a11y/aria-disable.directive"
            );

            // FocusDirective from @angular/cdk/a11y
            assert_eq!(meta.host_directives[1].directive.as_str(), "FocusDirective");
            assert_eq!(
                meta.host_directives[1].source_module.as_ref().unwrap().as_str(),
                "@angular/cdk/a11y"
            );
        });
    }

    #[test]
    fn test_extract_host_directives_source_module_object_form() {
        // Test that source_module is populated in object form
        let code = r#"
            import {Component} from '@angular/core';
            import { ColorDirective } from "./directives/color";

            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [
                    { directive: ColorDirective, inputs: ['color'] }
                ]
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "ColorDirective");
            assert_eq!(
                meta.host_directives[0].source_module.as_ref().unwrap().as_str(),
                "./directives/color"
            );
        });
    }

    #[test]
    fn test_extract_host_directives_no_source_module_for_local() {
        // Test that local directives (not imported) have no source_module
        let code = r#"
            import {Component, Directive} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [LocalDirective]
            })
            class TestComponent {}

            @Directive({ selector: '[local]' })
            class LocalDirective {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].directive.as_str(), "LocalDirective");
            // Local directive should not have a source_module
            assert!(meta.host_directives[0].source_module.is_none());
        });
    }

    #[test]
    fn test_extract_host_directives_mixed_imported_and_local() {
        // Test mixed imported and local directives
        let code = r#"
            import {Component} from '@angular/core';
            import { ImportedDirective } from "@angular/library";

            @Component({
                selector: 'app-test',
                template: '',
                hostDirectives: [ImportedDirective, LocalDirective]
            })
            class TestComponent {}

            class LocalDirective {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 2);

            // Imported directive should have source_module
            assert_eq!(meta.host_directives[0].directive.as_str(), "ImportedDirective");
            assert_eq!(
                meta.host_directives[0].source_module.as_ref().unwrap().as_str(),
                "@angular/library"
            );

            // Local directive should not have source_module
            assert_eq!(meta.host_directives[1].directive.as_str(), "LocalDirective");
            assert!(meta.host_directives[1].source_module.is_none());
        });
    }

    // =========================================================================
    // @HostBinding/@HostListener decorator extraction tests
    // =========================================================================

    #[test]
    fn test_extract_host_binding_decorator() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostBinding('class.active') isActive = false;
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.properties.len(), 1);
            // Keys are wrapped with brackets: "class.active" -> "[class.active]"
            assert_eq!(host.properties[0].0.as_str(), "[class.active]");
            assert_eq!(host.properties[0].1.as_str(), "isActive");
        });
    }

    #[test]
    fn test_extract_host_binding_without_name() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostBinding() title = 'Hello';
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.properties.len(), 1);
            // When no name is provided, uses the property name, wrapped in brackets
            assert_eq!(host.properties[0].0.as_str(), "[title]");
            assert_eq!(host.properties[0].1.as_str(), "title");
        });
    }

    #[test]
    fn test_extract_host_listener_decorator() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostListener('click') onClick() {}
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.listeners.len(), 1);
            // Keys are wrapped with parentheses: "click" -> "(click)"
            assert_eq!(host.listeners[0].0.as_str(), "(click)");
            // Method expression includes empty parens when no args
            assert_eq!(host.listeners[0].1.as_str(), "onClick()");
        });
    }

    #[test]
    fn test_extract_host_listener_with_args() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostListener('keydown', ['$event']) onKeyDown(event: KeyboardEvent) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.listeners.len(), 1);
            // Keys are wrapped with parentheses
            assert_eq!(host.listeners[0].0.as_str(), "(keydown)");
            // Method expression includes args: "onKeyDown($event)"
            assert_eq!(host.listeners[0].1.as_str(), "onKeyDown($event)");
        });
    }

    #[test]
    fn test_extract_host_listener_with_multiple_args() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostListener('click', ['$event', '$event.target']) onClick(event: MouseEvent, target: Element) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.listeners.len(), 1);
            // Keys are wrapped with parentheses
            assert_eq!(host.listeners[0].0.as_str(), "(click)");
            // Method expression includes all args (comma-separated, no spaces - matching Angular)
            assert_eq!(host.listeners[0].1.as_str(), "onClick($event,$event.target)");
        });
    }

    #[test]
    fn test_extract_multiple_host_decorators() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostBinding('class.active') isActive = false;
                @HostBinding('attr.aria-label') ariaLabel = 'test';
                @HostListener('click') onClick() {}
                @HostListener('mouseenter') onMouseEnter() {}
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.properties.len(), 2);
            assert_eq!(host.listeners.len(), 2);
        });
    }

    #[test]
    fn test_merge_host_decorators_with_host_property() {
        // Test that @HostBinding/@HostListener are merged with @Component({ host: {} })
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                host: {
                    '[class.highlighted]': 'isHighlighted',
                    '(focus)': 'onFocus()'
                }
            })
            class TestComponent {
                @HostBinding('class.active') isActive = false;
                @HostListener('click') onClick() {}
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            // Should have 2 properties: from host:{} and from @HostBinding
            assert_eq!(host.properties.len(), 2);
            // Should have 2 listeners: from host:{} and from @HostListener
            assert_eq!(host.listeners.len(), 2);
        });
    }

    #[test]
    fn test_host_binding_on_getter() {
        let code = r#"
            import {Component, HostBinding, HostListener} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @HostBinding('style.color') get textColor() { return 'red'; }
            }
        "#;
        assert_metadata(code, |meta| {
            let host = meta.host.as_ref().expect("Expected host metadata");
            assert_eq!(host.properties.len(), 1);
            // Keys are wrapped with brackets
            assert_eq!(host.properties[0].0.as_str(), "[style.color]");
            assert_eq!(host.properties[0].1.as_str(), "textColor");
        });
    }

    #[test]
    fn test_no_host_decorators() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                normalProperty = 'value';
                normalMethod() {}
            }
        "#;
        assert_metadata(code, |meta| {
            // No host metadata should be created when there are no host bindings/listeners
            assert!(meta.host.is_none());
        });
    }

    // =========================================================================
    // Constructor dependency extraction tests
    // =========================================================================

    #[test]
    fn test_component_without_constructor_no_superclass() {
        // Component without constructor AND without superclass
        // -> should use simple factory (Some with empty deps)
        // See: packages/compiler-cli/src/ngtsc/annotations/common/src/di.ts:47-52
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            // No constructor but no superclass = simple factory with empty deps
            let deps =
                meta.constructor_deps.as_ref().expect("Should have Some([]) for simple factory");
            assert!(deps.is_empty(), "Should have empty deps vec");
            assert!(!meta.uses_inheritance, "Should not have inheritance");
        });
    }

    #[test]
    fn test_component_without_constructor_with_superclass() {
        // Component without constructor AND with superclass
        // -> should use inherited factory (None)
        // See: packages/compiler-cli/src/ngtsc/annotations/common/src/di.ts:47-52
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent extends BaseComponent {}
        "#;
        assert_metadata(code, |meta| {
            // No constructor but has superclass = inherited factory pattern
            assert!(meta.constructor_deps.is_none(), "Should be None for inherited factory");
            assert!(meta.uses_inheritance, "Should have inheritance");
        });
    }

    #[test]
    fn test_component_with_simple_constructor_deps() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-root',
                template: '<div></div>'
            })
            class AppComponent {
                constructor(
                    private broadcasterService: BroadcasterService,
                    private router: Router,
                    private ngZone: NgZone,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 3);

            // Check each dependency
            assert_eq!(deps[0].token.as_ref().unwrap().as_str(), "BroadcasterService");
            assert_eq!(deps[1].token.as_ref().unwrap().as_str(), "Router");
            assert_eq!(deps[2].token.as_ref().unwrap().as_str(), "NgZone");

            // All should have default flags
            for dep in deps {
                assert!(!dep.optional);
                assert!(!dep.skip_self);
                assert!(!dep.self_);
                assert!(!dep.host);
            }
        });
    }

    #[test]
    fn test_component_with_inject_decorator() {
        let code = r#"
            import {Component, Inject} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Inject(WINDOW) private win: Window,
                    private normalService: NormalService,
                    @Inject(DOCUMENT) private document: Document,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 3);

            // First dep: @Inject(WINDOW) - token should be WINDOW, not Window
            assert_eq!(deps[0].token.as_ref().unwrap().as_str(), "WINDOW");

            // Second dep: no @Inject - token should be NormalService
            assert_eq!(deps[1].token.as_ref().unwrap().as_str(), "NormalService");

            // Third dep: @Inject(DOCUMENT) - token should be DOCUMENT, not Document
            assert_eq!(deps[2].token.as_ref().unwrap().as_str(), "DOCUMENT");
        });
    }

    #[test]
    fn test_component_with_optional_decorator() {
        let code = r#"
            import {Component, Optional} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() private optionalService?: OptionalService,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert!(dep.optional, "Should have optional flag");
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "OptionalService");
        });
    }

    #[test]
    fn test_component_optional_with_nullable_type() {
        // Regression test for issue #285:
        // `@Optional() svc: MyService | null` is the canonical optional-DI pattern,
        // but the union with `null` previously caused token extraction to fail and
        // emit `ɵɵinvalidFactoryDep`, which throws at runtime.
        //
        // Angular's reference compiler filters out `null` literal type nodes from
        // the union; when exactly one type remains it becomes the token.
        let code = r#"
            import {Component, Optional} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() private svc: MyService | null,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert!(dep.optional, "Should have optional flag");
            assert_eq!(
                dep.token.as_ref().expect("token should resolve to MyService").as_str(),
                "MyService",
                "null should be filtered from the union, leaving MyService as the token",
            );
        });
    }

    #[test]
    fn test_component_nullable_type_without_decorator() {
        // Bare `svc: MyService | null` (no `@Optional()`) should also have its
        // token resolved to `MyService`. Token resolution is independent of
        // optionality — `@Optional()` only controls the inject flag.
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    private svc: MyService | null,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert!(!dep.optional, "Should not have optional flag");
            assert_eq!(
                dep.token.as_ref().expect("token should resolve to MyService").as_str(),
                "MyService",
            );
        });
    }

    #[test]
    fn test_component_nullable_type_null_first() {
        // `null` can appear in either position of the union — both must be filtered.
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() private svc: null | MyService,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            let dep = &deps[0];
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "MyService");
        });
    }

    #[test]
    fn test_component_parenthesized_nullable_type() {
        // Parenthesized nullable union: `(MyService | null)`.
        // oxc preserves `TSParenthesizedType` in the AST, so the resolver must
        // unwrap it before checking for unions, or the dependency falls through
        // to `ɵɵinvalidFactoryDep` (issue #285 follow-up).
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() private svc: (MyService | null),
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            let dep = &deps[0];
            assert_eq!(
                dep.token.as_ref().expect("token should resolve to MyService").as_str(),
                "MyService",
            );
        });
    }

    #[test]
    fn test_component_paren_around_inner_type_in_union() {
        // Parens around just the non-null side: `(MyService) | null`.
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() private svc: (MyService) | null,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            let dep = &deps[0];
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "MyService");
        });
    }

    #[test]
    fn test_component_with_skip_self_decorator() {
        let code = r#"
            import {Component, Optional, SkipSelf} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() @SkipSelf() private parentModule?: ParentModule,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert!(dep.optional, "Should have optional flag");
            assert!(dep.skip_self, "Should have skip_self flag");
            assert!(!dep.self_, "Should not have self_ flag");
            assert!(!dep.host, "Should not have host flag");
        });
    }

    #[test]
    fn test_component_with_self_and_host_decorators() {
        let code = r#"
            import {Component, Self, Host} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Self() private selfService: SelfService,
                    @Host() private hostService: HostService,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 2);

            // First dep: @Self()
            assert!(deps[0].self_, "First dep should have self_ flag");
            assert!(!deps[0].host, "First dep should not have host flag");

            // Second dep: @Host()
            assert!(!deps[1].self_, "Second dep should not have self_ flag");
            assert!(deps[1].host, "Second dep should have host flag");
        });
    }

    #[test]
    fn test_component_with_combined_decorators() {
        let code = r#"
            import {Component, Optional, Inject} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Optional() @Inject(TOKEN) private service: SomeService,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert!(dep.optional, "Should have optional flag");
            // Token should come from @Inject, not type annotation
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "TOKEN");
        });
    }

    #[test]
    fn test_component_with_attribute_decorator() {
        let code = r#"
            import {Component, Attribute} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    @Attribute('title') private title: string,
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            // For @Attribute, the attribute_name should be set
            assert_eq!(dep.attribute_name.as_ref().unwrap().as_str(), "title");
        });
    }

    // =========================================================================
    // Declaration list emit mode tests
    // =========================================================================

    #[test]
    fn test_standalone_component_uses_direct_mode() {
        use super::super::metadata::DeclarationListEmitMode;

        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                standalone: true
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.standalone);
            // Standalone components without raw imports use Direct mode
            assert_eq!(meta.declaration_list_emit_mode, DeclarationListEmitMode::Direct);
        });
    }

    #[test]
    fn test_non_standalone_component_uses_runtime_resolved_mode() {
        use super::super::metadata::DeclarationListEmitMode;

        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: '',
                standalone: false
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.standalone);
            // Non-standalone components use RuntimeResolved mode
            assert_eq!(meta.declaration_list_emit_mode, DeclarationListEmitMode::RuntimeResolved);
        });
    }

    #[test]
    fn test_implicit_non_standalone_uses_runtime_resolved_mode() {
        use super::super::metadata::DeclarationListEmitMode;

        // When implicit_standalone=false (Angular v18 and earlier behavior)
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        with_extracted_metadata(code, false, |meta| {
            let meta = meta.unwrap();
            assert!(!meta.standalone);
            // Implicitly non-standalone components use RuntimeResolved mode
            assert_eq!(meta.declaration_list_emit_mode, DeclarationListEmitMode::RuntimeResolved);
        });
    }

    #[test]
    fn test_implicit_standalone_uses_direct_mode() {
        use super::super::metadata::DeclarationListEmitMode;

        // When implicit_standalone=true (Angular v19+ behavior)
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        with_extracted_metadata(code, true, |meta| {
            let meta = meta.unwrap();
            assert!(meta.standalone);
            // Implicitly standalone components use Direct mode
            assert_eq!(meta.declaration_list_emit_mode, DeclarationListEmitMode::Direct);
        });
    }

    #[test]
    fn test_standalone_with_imports_uses_runtime_resolved_mode() {
        use super::super::metadata::DeclarationListEmitMode;

        let code = r#"
            import {Component} from '@angular/core';
            import { AsyncPipe, DatePipe } from '@angular/common';

            @Component({
                selector: 'app-test',
                standalone: true,
                imports: [AsyncPipe, DatePipe],
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.standalone);
            // Standalone components WITH imports use RuntimeResolved mode
            assert_eq!(meta.declaration_list_emit_mode, DeclarationListEmitMode::RuntimeResolved);
            // And raw_imports should be populated with the array expression
            assert!(meta.raw_imports.is_some());
        });
    }

    #[test]
    fn test_standalone_with_variable_imports_uses_runtime_resolved_mode() {
        use super::super::metadata::DeclarationListEmitMode;

        let code = r#"
            import {Component} from '@angular/core';
            const MY_IMPORTS = [AsyncPipe, DatePipe];

            @Component({
                selector: 'app-test',
                standalone: true,
                imports: MY_IMPORTS,
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.standalone);
            // Standalone components with variable imports use RuntimeResolved mode
            assert_eq!(meta.declaration_list_emit_mode, DeclarationListEmitMode::RuntimeResolved);
            // raw_imports should be populated with the variable reference
            assert!(meta.raw_imports.is_some());
        });
    }

    // =========================================================================
    // Import map / token source module tests
    // =========================================================================

    #[test]
    fn test_constructor_dep_token_source_module_from_named_import() {
        // Test that token_source_module is populated from named imports
        let code = r#"
            import {Component} from '@angular/core';
            import { AuthService } from "@bitwarden/common/auth/abstractions/auth.service";

            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(private authService: AuthService) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "AuthService");
            assert_eq!(
                dep.token_source_module.as_ref().unwrap().as_str(),
                "@bitwarden/common/auth/abstractions/auth.service"
            );
        });
    }

    #[test]
    fn test_constructor_dep_token_source_module_multiple_imports() {
        // Test that multiple imports from the same module are tracked correctly
        let code = r#"
            import {Component} from '@angular/core';
            import { ServiceA, ServiceB } from "./services";
            import { Router } from "@angular/router";

            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    private serviceA: ServiceA,
                    private serviceB: ServiceB,
                    private router: Router
                ) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 3);

            // ServiceA from ./services
            assert_eq!(deps[0].token.as_ref().unwrap().as_str(), "ServiceA");
            assert_eq!(deps[0].token_source_module.as_ref().unwrap().as_str(), "./services");

            // ServiceB from ./services
            assert_eq!(deps[1].token.as_ref().unwrap().as_str(), "ServiceB");
            assert_eq!(deps[1].token_source_module.as_ref().unwrap().as_str(), "./services");

            // Router from @angular/router
            assert_eq!(deps[2].token.as_ref().unwrap().as_str(), "Router");
            assert_eq!(deps[2].token_source_module.as_ref().unwrap().as_str(), "@angular/router");
        });
    }

    #[test]
    fn test_constructor_dep_token_source_module_with_inject_decorator() {
        // Test that @Inject token source module is tracked from import
        let code = r#"
            import {Component, Inject} from '@angular/core';
            import { WINDOW } from "@bitwarden/common";

            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(@Inject(WINDOW) private win: Window) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            // Token is WINDOW, not Window (from @Inject)
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "WINDOW");
            // Source module should be from the WINDOW import
            assert_eq!(dep.token_source_module.as_ref().unwrap().as_str(), "@bitwarden/common");
        });
    }

    #[test]
    fn test_constructor_dep_no_source_module_for_local_class() {
        // Test that local classes (not imported) have no token_source_module
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(private localService: LocalService) {}
            }

            class LocalService {}
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "LocalService");
            // No source module for local classes
            assert!(dep.token_source_module.is_none());
        });
    }

    #[test]
    fn test_constructor_dep_token_source_module_with_alias() {
        // Test that aliased imports use the local name as key
        let code = r#"
            import {Component} from '@angular/core';
            import { AuthService as Auth } from "@bitwarden/common/auth";

            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(private auth: Auth) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            // Token is the local alias "Auth"
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "Auth");
            // Source module is from the aliased import
            assert_eq!(
                dep.token_source_module.as_ref().unwrap().as_str(),
                "@bitwarden/common/auth"
            );
        });
    }

    #[test]
    fn test_constructor_dep_token_source_module_default_import() {
        // Test that default imports are tracked
        let code = r#"
            import {Component} from '@angular/core';
            import DefaultService from "@bitwarden/common/default";

            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(private service: DefaultService) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.constructor_deps.as_ref().unwrap();
            assert_eq!(deps.len(), 1);
            let dep = &deps[0];
            assert_eq!(dep.token.as_ref().unwrap().as_str(), "DefaultService");
            assert_eq!(
                dep.token_source_module.as_ref().unwrap().as_str(),
                "@bitwarden/common/default"
            );
        });
    }

    // =========================================================================
    // Decorator Span Collection Tests
    // =========================================================================

    /// Helper to parse code and get the first class
    fn with_first_class<F>(code: &str, callback: F)
    where
        F: FnOnce(&Class<'_>),
    {
        let allocator = Allocator::default();
        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        for stmt in &parser_ret.program.body {
            let class = match stmt {
                Statement::ClassDeclaration(class) => Some(class.as_ref()),
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                _ => None,
            };

            if let Some(class) = class {
                callback(class);
                return;
            }
        }
        panic!("No class found in code");
    }

    /// [`collect_member_decorator_spans`] for `class`, declared in `code`.
    fn collect_member_decorators(code: &str, class: &Class<'_>, spans: &mut std::vec::Vec<Span>) {
        let allocator = Allocator::default();
        let parser_ret = Parser::new(&allocator, code, SourceType::tsx()).parse();
        let consts = crate::directive::collect_string_consts(&allocator, &parser_ret.program);
        collect_member_decorator_spans(class, &consts, spans);
    }

    /// [`collect_constructor_decorator_spans`] for `class`, declared in `code`.
    fn collect_constructor_decorators(
        code: &str,
        class: &Class<'_>,
        spans: &mut std::vec::Vec<Span>,
    ) {
        let allocator = Allocator::default();
        let parser_ret = Parser::new(&allocator, code, SourceType::tsx()).parse();
        let consts = crate::directive::collect_string_consts(&allocator, &parser_ret.program);
        collect_constructor_decorator_spans(class, &consts, spans);
    }

    #[test]
    fn test_collect_constructor_decorator_spans() {
        let code = r#"
            import { Component, Optional, Inject, Host, Self } from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(
                    private service: SomeService,
                    @Optional() private optionalService: OptionalService,
                    @Inject(TOKEN) private injected: InjectedService,
                    @Host() @Self() private hostSelf: HostSelfService
                ) {}
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_constructor_decorators(code, class, &mut spans);

            // Should collect 4 decorators: @Optional, @Inject, @Host, @Self
            assert_eq!(spans.len(), 4);
        });
    }

    /// Like ngtsc, only Angular's parameter decorators are removed: aliased or
    /// namespaced ones are, another module's (even named `Inject`) and a
    /// local one aren't.
    #[test]
    fn test_collect_constructor_decorator_spans_by_import() {
        let code = r#"
            import * as ng from '@angular/core';
            import { Component, Inject as Inj } from '@angular/core';
            import { Inject, Optional } from './other';
            function Self(): ParameterDecorator { return () => {}; }
            @Component({ selector: 'app-test', template: '' })
            class TestComponent {
                constructor(
                    @Inj(TOKEN) a: A,
                    @ng.Optional() b: B,
                    @Inject(TOKEN) c: C,
                    @Optional() d: D,
                    @Self() e: E,
                ) {}
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_constructor_decorators(code, class, &mut spans);
            let removed: std::vec::Vec<&str> =
                spans.iter().map(|span| &code[span.start as usize..span.end as usize]).collect();
            assert_eq!(removed, ["@Inj(TOKEN)", "@ng.Optional()"]);
        });
    }

    #[test]
    fn test_collect_constructor_decorator_spans_empty() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                constructor(private service: SomeService) {}
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_constructor_decorators(code, class, &mut spans);

            // No parameter decorators
            assert_eq!(spans.len(), 0);
        });
    }

    #[test]
    fn test_collect_constructor_decorator_spans_no_constructor() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_constructor_decorators(code, class, &mut spans);

            // No constructor
            assert_eq!(spans.len(), 0);
        });
    }

    #[test]
    fn test_collect_member_decorator_spans() {
        let code = r#"
            import {Component, Input, Output, HostBinding, HostListener, ViewChild, ViewChildren, ContentChild, ContentChildren} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @Input() name: string;
                @Input('aliased') aliasedProp: string;
                @Output() clicked = new EventEmitter();
                @HostBinding('class.active') isActive = false;
                @HostListener('click', ['$event']) onClick(event: any) {}
                @ViewChild('ref') viewRef: any;
                @ViewChildren('items') items: any;
                @ContentChild('content') content: any;
                @ContentChildren('contents') contents: any;
                // Regular property - no decorator
                private someField: string;
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_member_decorators(code, class, &mut spans);

            // Should collect 9 member decorators (all @Input, @Output, @Host*, @*Child)
            assert_eq!(spans.len(), 9);
        });
    }

    #[test]
    fn test_collect_member_decorator_spans_ignores_non_angular() {
        let code = r#"
            import {Component, Input} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @Input() name: string;
                @CustomDecorator() custom: string;
                @SomeOther() other: string;
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_member_decorators(code, class, &mut spans);

            // Should only collect @Input, ignoring custom decorators
            assert_eq!(spans.len(), 1);
        });
    }

    /// Like ngtsc, only `@angular/core`'s decorators are compiled and removed:
    /// under an alias or a namespace too, but not another module's, a local
    /// or an undeclared one with the same name.
    #[test]
    fn test_collect_member_decorator_spans_by_import() {
        let code = r#"
            import {Component, Input as In, Output} from '@angular/core';
            import * as core from '@angular/core';
            import {HostBinding} from './other';
            import * as other from './other';
            function ViewChild(s: string): any { return () => {}; }
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                @In() aliased: string;
                @core.Input() namespaced: string;
                @Output() out: any;
                @HostBinding('class.a') foreign = true;
                @other.Input() foreignNamespace: string;
                @ViewChild('ref') local: any;
                @ContentChild('ref') undeclared: any;
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_member_decorators(code, class, &mut spans);
            let texts: std::vec::Vec<&str> =
                spans.iter().map(|s| &code[s.start as usize..s.end as usize]).collect();
            assert_eq!(texts, ["@In()", "@core.Input()", "@Output()"]);
        });
    }

    #[test]
    fn test_collect_member_decorator_spans_empty() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                private name: string;
                private age: number;
            }
        "#;
        with_first_class(code, |class| {
            let mut spans = std::vec::Vec::new();
            collect_member_decorators(code, class, &mut spans);

            // No Angular member decorators
            assert_eq!(spans.len(), 0);
        });
    }

    // =========================================================================
    // Lifecycle detection tests (ngOnChanges)
    // =========================================================================

    #[test]
    fn test_lifecycle_ng_on_changes_detected() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent implements OnChanges {
                ngOnChanges(changes: SimpleChanges) {
                    console.log(changes);
                }
            }
        "#;
        assert_metadata(code, |meta| {
            assert!(
                meta.lifecycle.uses_on_changes,
                "Expected uses_on_changes to be true when ngOnChanges method exists"
            );
        });
    }

    #[test]
    fn test_lifecycle_ng_on_changes_not_detected_without_method() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                ngOnInit() {
                    console.log('init');
                }
            }
        "#;
        assert_metadata(code, |meta| {
            assert!(
                !meta.lifecycle.uses_on_changes,
                "Expected uses_on_changes to be false when ngOnChanges method is missing"
            );
        });
    }

    #[test]
    fn test_lifecycle_ng_on_changes_not_detected_for_static_method() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {
                static ngOnChanges(changes: SimpleChanges) {
                    console.log(changes);
                }
            }
        "#;
        assert_metadata(code, |meta| {
            assert!(
                !meta.lifecycle.uses_on_changes,
                "Expected uses_on_changes to be false when ngOnChanges is static"
            );
        });
    }

    #[test]
    fn test_lifecycle_ng_on_changes_async_method() {
        let code = r#"
            import {Component} from '@angular/core';
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent implements OnChanges {
                async ngOnChanges(changes: SimpleChanges) {
                    await this.processChanges(changes);
                }
            }
        "#;
        assert_metadata(code, |meta| {
            assert!(
                meta.lifecycle.uses_on_changes,
                "Expected uses_on_changes to be true for async ngOnChanges method"
            );
        });
    }
}

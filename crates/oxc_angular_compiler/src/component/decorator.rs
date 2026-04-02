//! Angular `@Component` decorator parser.
//!
//! This module extracts metadata from `@Component({...})` decorators
//! on TypeScript class declarations.

use oxc_allocator::{Allocator, Vec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, Class, ClassElement, Decorator, Expression,
    MethodDefinitionKind, ObjectPropertyKind, PropertyKey,
};
use oxc_span::{Ident, Span};

use super::dependency::R3DependencyMetadata;
use super::metadata::{
    ChangeDetectionStrategy, ComponentMetadata, HostDirectiveMetadata, HostMetadata,
    TemplateDependency, ViewEncapsulation,
};
use super::transform::ImportMap;
use crate::directive::{
    extract_host_bindings, extract_host_listeners, extract_input_metadata, extract_output_metadata,
};
use crate::output::oxc_converter::convert_oxc_expression;

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
) -> Option<ComponentMetadata<'a>> {
    // Get the class name
    let class_name: Ident<'a> = class.id.as_ref()?.name.clone().into();
    let class_span = class.span;

    // Find the @Component decorator
    let component_decorator = find_component_decorator(&class.decorators)?;

    // Get the decorator call arguments
    let call_expr = match &component_decorator.expression {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Verify it's calling 'Component'
    if !is_component_call(&call_expr.callee) {
        return None;
    }

    // Get the first argument (the config object)
    let config_arg = call_expr.arguments.first()?;
    let config_obj = match config_arg {
        Argument::ObjectExpression(obj) => obj,
        _ => return None,
    };

    // Create metadata with defaults (standalone uses implicit_standalone when not explicitly set)
    let mut metadata =
        ComponentMetadata::new(allocator, class_name, class_span, implicit_standalone);

    // Parse each property in the config object
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            let key_name = get_property_key_name(&prop.key)?;

            match key_name.as_str() {
                "selector" => {
                    metadata.selector = extract_string_value(&prop.value);
                }
                "template" => {
                    metadata.template = extract_string_value(&prop.value);
                }
                "templateUrl" => {
                    metadata.template_url = extract_string_value(&prop.value);
                }
                "styles" => {
                    if let Some(styles) = extract_string_array(allocator, &prop.value) {
                        metadata.styles = styles;
                    } else if let Some(style) = extract_string_value(&prop.value) {
                        // Single style string (legacy support)
                        metadata.styles.push(style);
                    }
                }
                "styleUrls" | "styleUrl" => {
                    if let Some(urls) = extract_string_array(allocator, &prop.value) {
                        metadata.style_urls = urls;
                    } else if let Some(url) = extract_string_value(&prop.value) {
                        metadata.style_urls.push(url);
                    }
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
                    metadata.change_detection = extract_change_detection(&prop.value);
                }
                "host" => {
                    metadata.host = extract_host_metadata(allocator, &prop.value);
                }
                "imports" => {
                    // For standalone components, we need:
                    // 1. The identifier list for local analysis
                    metadata.imports = extract_identifier_array(allocator, &prop.value);
                    // 2. The raw expression to pass to ɵɵgetComponentDepsFactory in RuntimeResolved mode
                    metadata.raw_imports =
                        convert_oxc_expression(allocator, &prop.value, source_text);
                }
                "exportAs" => {
                    // exportAs can be comma-separated: "foo, bar"
                    if let Some(export_as) = extract_string_value(&prop.value) {
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
                        extract_boolean_value(&prop.value).unwrap_or(false);
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
                        extract_host_directives(allocator, &prop.value, import_map);
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
        }
    }

    // Extract host bindings and listeners from @HostBinding/@HostListener decorators on class members
    // These are merged with any host metadata from the @Component({ host: {} }) property
    let host_bindings = extract_host_bindings(allocator, class);
    let host_listeners = extract_host_listeners(allocator, class);

    if !host_bindings.is_empty() || !host_listeners.is_empty() {
        let host = metadata.host.get_or_insert_with(|| HostMetadata::new(allocator));

        // Add @HostBinding properties
        // Wrap with brackets: "class.active" -> "[class.active]"
        for (host_prop, class_prop) in host_bindings {
            let wrapped_key =
                Ident::from(allocator.alloc_str(&format!("[{}]", host_prop.as_str())));
            host.properties.push((wrapped_key, class_prop));
        }

        // Add @HostListener events
        // Wrap event name with parentheses and build method expression with args
        // Reference: Angular's shared.ts:713 - `bindings.listeners[eventName] = \`${member.name}(${args.join(',')})\``
        for (event_name, method_name, args) in host_listeners {
            // Wrap event name: "click" -> "(click)"
            let wrapped_key =
                Ident::from(allocator.alloc_str(&format!("({})", event_name.as_str())));

            // Build method expression with args: "handleClick" + ["$event"] -> "handleClick($event)"
            let method_expr = if args.is_empty() {
                Ident::from(allocator.alloc_str(&format!("{}()", method_name.as_str())))
            } else {
                let args_str: String =
                    args.iter().map(|a| a.as_str()).collect::<std::vec::Vec<_>>().join(",");
                Ident::from(allocator.alloc_str(&format!("{}({})", method_name.as_str(), args_str)))
            };

            host.listeners.push((wrapped_key, method_expr));
        }
    }

    // Detect if the component extends another class
    // Similar to Angular's: const usesInheritance = reflector.hasBaseClass(clazz);
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:393
    // NOTE: This must be set BEFORE extract_constructor_deps because that function
    // uses this information to determine whether to use inherited factory pattern.
    let has_superclass = class.super_class.is_some();
    metadata.uses_inheritance = has_superclass;

    // Extract constructor dependencies for factory generation
    // This enables proper DI for component constructors
    metadata.constructor_deps =
        extract_constructor_deps(allocator, class, import_map, has_superclass);

    // Extract inputs from @Input decorators on class members
    metadata.inputs = extract_input_metadata(allocator, class);

    // Extract outputs from @Output decorators on class members
    metadata.outputs = extract_output_metadata(allocator, class);

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
            allocator,
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

/// Find the @Component decorator in a list of decorators.
pub fn find_component_decorator<'a>(decorators: &'a [Decorator<'a>]) -> Option<&'a Decorator<'a>> {
    decorators.iter().find(|d| match &d.expression {
        Expression::CallExpression(call) => is_component_call(&call.callee),
        Expression::Identifier(id) => id.name == "Component",
        _ => false,
    })
}

/// Find the span of the @Component decorator on a class.
///
/// Returns the span including any leading whitespace/newlines that should be removed
/// along with the decorator.
pub fn find_component_decorator_span(class: &Class<'_>) -> Option<Span> {
    find_component_decorator(&class.decorators).map(|d| d.span)
}

/// Check if a callee expression is a call to 'Component'.
fn is_component_call(callee: &Expression<'_>) -> bool {
    match callee {
        Expression::Identifier(id) => id.name == "Component",
        // Handle namespaced imports like ng.Component or core.Component
        Expression::StaticMemberExpression(member) => {
            matches!(&member.property.name.as_str(), &"Component")
        }
        _ => false,
    }
}

/// Get the name of a property key as a string.
fn get_property_key_name<'a>(key: &PropertyKey<'a>) -> Option<Ident<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(lit) => Some(lit.value.clone().into()),
        _ => None,
    }
}

/// Extract a string value from an expression.
fn extract_string_value<'a>(expr: &Expression<'a>) -> Option<Ident<'a>> {
    match expr {
        Expression::StringLiteral(lit) => Some(lit.value.clone().into()),
        Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
            // Simple template literal with no expressions: `template string`
            // Use cooked value to properly interpret escape sequences (\n -> newline)
            // Angular evaluates template literals, so we need cooked, not raw
            tpl.quasis.first().and_then(|q| q.value.cooked.clone().map(Into::into))
        }
        _ => None,
    }
}

/// Extract a boolean value from an expression.
fn extract_boolean_value(expr: &Expression<'_>) -> Option<bool> {
    match expr {
        Expression::BooleanLiteral(lit) => Some(lit.value.into()),
        _ => None,
    }
}
/// Extract an array of strings from an expression.
fn extract_string_array<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> Option<Vec<'a, Ident<'a>>> {
    let Expression::ArrayExpression(arr) = expr else {
        return None;
    };

    let mut result = Vec::new_in(allocator);
    for element in &arr.elements {
        if let ArrayExpressionElement::StringLiteral(lit) = element {
            result.push(lit.value.clone().into());
        } else if let ArrayExpressionElement::TemplateLiteral(tpl) = element {
            if tpl.expressions.is_empty() {
                // Use cooked value to properly interpret escape sequences
                if let Some(quasi) = tpl.quasis.first() {
                    if let Some(cooked) = &quasi.value.cooked {
                        result.push(cooked.clone().into());
                    }
                }
            }
        }
    }

    Some(result)
}

/// Extract an array of identifiers (for imports).
fn extract_identifier_array<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> Vec<'a, Ident<'a>> {
    let mut result = Vec::new_in(allocator);

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
            "Default" => ChangeDetectionStrategy::Default,
            _ => ChangeDetectionStrategy::default(),
        },
        Expression::NumericLiteral(num) => {
            // Angular's numeric values: Default = 0, OnPush = 1
            match num.value as i32 {
                1 => ChangeDetectionStrategy::OnPush,
                _ => ChangeDetectionStrategy::default(),
            }
        }
        _ => ChangeDetectionStrategy::default(),
    }
}

/// Extract host metadata from a host object expression.
///
/// Reference: packages/compiler/src/render3/view/compiler.ts:560-604
fn extract_host_metadata<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> Option<HostMetadata<'a>> {
    let Expression::ObjectExpression(obj) = expr else {
        return None;
    };

    let mut host = HostMetadata {
        properties: Vec::new_in(allocator),
        attributes: Vec::new_in(allocator),
        listeners: Vec::new_in(allocator),
        class_attr: None,
        style_attr: None,
    };

    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            let Some(key_name) = get_property_key_name(&prop.key) else {
                continue;
            };
            let Some(value) = extract_string_value(&prop.value) else {
                continue;
            };

            let key_str = key_name.as_str();

            if key_str.starts_with('[') && key_str.ends_with(']') {
                // Property binding: [class.active]
                host.properties.push((key_name, value));
            } else if key_str.starts_with('(') && key_str.ends_with(')') {
                // Event listener: (click)
                host.listeners.push((key_name, value));
            } else {
                // Check for special attributes (class and style)
                // Reference: compiler.ts:567-588
                match key_str {
                    "class" => {
                        host.class_attr = Some(value);
                    }
                    "style" => {
                        host.style_attr = Some(value);
                    }
                    _ => {
                        // Regular static attribute
                        host.attributes.push((key_name, value));
                    }
                }
            }
        }
    }

    Some(host)
}

/// Extract host directives from a hostDirectives array expression.
///
/// Handles the following patterns:
/// - Simple identifier: `hostDirectives: [TooltipDirective]`
/// - Object with directive: `hostDirectives: [{ directive: ColorDirective }]`
/// - Object with mappings: `hostDirectives: [{ directive: D, inputs: ['color', 'bgColor'], outputs: ['changed'] }]`
/// - Forward ref: `hostDirectives: [forwardRef(() => MyDirective)]`
///
/// Reference: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:1873-1985
fn extract_host_directives<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
    import_map: &ImportMap<'a>,
) -> Vec<'a, HostDirectiveMetadata<'a>> {
    let mut result = Vec::new_in(allocator);

    let Expression::ArrayExpression(arr) = expr else {
        return result;
    };

    for element in &arr.elements {
        if let Some(meta) = extract_single_host_directive(allocator, element, import_map) {
            result.push(meta);
        }
    }

    result
}

/// Extract a single host directive from an array element.
///
/// Handles:
/// - Identifier: `TooltipDirective`
/// - Object: `{ directive: ColorDirective, inputs: [...], outputs: [...] }`
/// - ForwardRef call: `forwardRef(() => MyDirective)`
fn extract_single_host_directive<'a>(
    allocator: &'a Allocator,
    element: &ArrayExpressionElement<'a>,
    import_map: &ImportMap<'a>,
) -> Option<HostDirectiveMetadata<'a>> {
    match element {
        // Simple identifier: TooltipDirective
        ArrayExpressionElement::Identifier(id) => {
            let name: Ident<'a> = id.name.clone().into();
            let mut meta = HostDirectiveMetadata::new(allocator, name.clone());
            // Look up the source module from the import map
            if let Some(import_info) = import_map.get(&name) {
                meta.source_module = Some(import_info.source_module.clone());
            }
            Some(meta)
        }

        // Object expression: { directive: ColorDirective, inputs: [...], outputs: [...] }
        ArrayExpressionElement::ObjectExpression(obj) => {
            let mut directive_name: Option<Ident<'a>> = None;
            let mut inputs = Vec::new_in(allocator);
            let mut outputs = Vec::new_in(allocator);
            let mut is_forward_reference = false;

            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    match key_name.as_str() {
                        "directive" => {
                            // Extract directive class name
                            // Can be identifier or forwardRef call
                            let (name, is_forward_ref) = extract_directive_reference(&prop.value);
                            directive_name = name;
                            is_forward_reference = is_forward_ref;
                        }
                        "inputs" => {
                            // Extract input mappings
                            inputs = extract_io_mappings(allocator, &prop.value);
                        }
                        "outputs" => {
                            // Extract output mappings
                            outputs = extract_io_mappings(allocator, &prop.value);
                        }
                        _ => {
                            // Unknown property - ignore
                        }
                    }
                }
            }

            directive_name.map(|name| {
                let mut meta = HostDirectiveMetadata::new(allocator, name.clone());
                // Look up the source module from the import map
                if let Some(import_info) = import_map.get(&name) {
                    meta.source_module = Some(import_info.source_module.clone());
                }
                meta.inputs = inputs;
                meta.outputs = outputs;
                meta.is_forward_reference = is_forward_reference;
                meta
            })
        }

        // ForwardRef call: forwardRef(() => MyDirective)
        ArrayExpressionElement::CallExpression(call) => {
            if is_forward_ref_call(&call.callee) {
                if let Some(name) = extract_forward_ref_directive_name(call.arguments.first()) {
                    let mut meta = HostDirectiveMetadata::new(allocator, name.clone());
                    // Look up the source module from the import map
                    if let Some(import_info) = import_map.get(&name) {
                        meta.source_module = Some(import_info.source_module.clone());
                    }
                    meta.is_forward_reference = true;
                    return Some(meta);
                }
            }
            None
        }

        _ => None,
    }
}

/// Extract a directive reference from an expression.
///
/// Returns the directive class name and whether it's a forward reference.
fn extract_directive_reference<'a>(expr: &Expression<'a>) -> (Option<Ident<'a>>, bool) {
    match expr {
        // Simple identifier: ColorDirective
        Expression::Identifier(id) => (Some(id.name.clone().into()), false),

        // ForwardRef call: forwardRef(() => ColorDirective)
        Expression::CallExpression(call) => {
            if is_forward_ref_call(&call.callee) {
                (extract_forward_ref_directive_name(call.arguments.first()), true)
            } else {
                (None, false)
            }
        }

        _ => (None, false),
    }
}

/// Check if a callee is a forwardRef call.
fn is_forward_ref_call(callee: &Expression<'_>) -> bool {
    match callee {
        Expression::Identifier(id) => id.name == "forwardRef",
        _ => false,
    }
}

/// Extract the directive name from a forwardRef argument.
///
/// Handles: `forwardRef(() => MyDirective)`
fn extract_forward_ref_directive_name<'a>(arg: Option<&Argument<'a>>) -> Option<Ident<'a>> {
    let arg = arg?;
    match arg {
        // forwardRef(() => MyDirective)
        Argument::ArrowFunctionExpression(arrow) => {
            // The body should be an identifier (the directive class)
            let body = &arrow.body;

            // Check if it's an expression body (single return expression)
            // Arrow functions with expression body have their expression
            // wrapped in the body. For `() => Directive`, the expression
            // is the directive identifier.
            if body.statements.is_empty() {
                return None;
            }
            // For expression arrow functions, the parser puts it in
            // an ExpressionStatement
            if let Some(oxc_ast::ast::Statement::ExpressionStatement(stmt)) =
                body.statements.first()
            {
                if let Expression::Identifier(id) = &stmt.expression {
                    return Some(id.name.clone().into());
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract input/output mappings from an array expression.
///
/// Handles two formats:
/// - Simple string: `["color"]` - same public and internal name
/// - Mapping string: `["color: bgColor"]` - public name mapped to internal name
///
/// Returns Vec of (publicName, internalName) pairs.
fn extract_io_mappings<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> Vec<'a, (Ident<'a>, Ident<'a>)> {
    let mut result = Vec::new_in(allocator);

    let Expression::ArrayExpression(arr) = expr else {
        return result;
    };

    for element in &arr.elements {
        if let Some((public_name, internal_name)) = parse_mapping_element(allocator, element) {
            result.push((public_name, internal_name));
        }
    }

    result
}

/// Parse a single mapping element from an input/output array.
///
/// Handles:
/// - Simple string: `"color"` -> ("color", "color")
/// - Mapping string: `"color: bgColor"` -> ("bgColor", "color")
/// - Tuple array: `["color", "bgColor"]` -> ("bgColor", "color")
fn parse_mapping_element<'a>(
    allocator: &'a Allocator,
    element: &ArrayExpressionElement<'a>,
) -> Option<(Ident<'a>, Ident<'a>)> {
    match element {
        // Simple string: "color" - same public and internal name
        ArrayExpressionElement::StringLiteral(lit) => {
            let value = lit.value.as_str();

            // Check for mapping format: "internalName: publicName"
            if let Some(colon_pos) = value.find(':') {
                let internal_name = value[..colon_pos].trim();
                let public_name = value[colon_pos + 1..].trim();
                Some((
                    Ident::from(allocator.alloc_str(public_name)),
                    Ident::from(allocator.alloc_str(internal_name)),
                ))
            } else {
                // Same name for both
                Some((lit.value.clone().into(), lit.value.clone().into()))
            }
        }

        // Tuple array: ["internalName", "publicName"]
        ArrayExpressionElement::ArrayExpression(arr) => {
            let elements = &arr.elements;
            if elements.len() >= 2 {
                let internal = match elements.first() {
                    Some(ArrayExpressionElement::StringLiteral(lit)) => {
                        Some(lit.value.clone().into())
                    }
                    _ => None,
                };
                let public = match elements.get(1) {
                    Some(ArrayExpressionElement::StringLiteral(lit)) => {
                        Some(lit.value.clone().into())
                    }
                    _ => None,
                };
                if let (Some(internal_name), Some(public_name)) = (internal, public) {
                    return Some((public_name, internal_name));
                }
            }
            None
        }

        _ => None,
    }
}

// =============================================================================
// Constructor Dependency Extraction
// =============================================================================

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
) -> Option<Vec<'a, R3DependencyMetadata<'a>>> {
    // Find the constructor method
    let constructor = class.body.body.iter().find_map(|element| {
        if let ClassElement::MethodDefinition(method) = element {
            if method.kind == MethodDefinitionKind::Constructor {
                return Some(method);
            }
        }
        None
    });

    match constructor {
        Some(ctor) => {
            // Constructor found - extract parameters (may be empty)
            let mut deps = Vec::new_in(allocator);
            let params = &ctor.value.params;

            for param in &params.items {
                let dep = extract_param_dependency(param, import_map);
                deps.push(dep);
            }

            Some(deps)
        }
        None => {
            // No constructor found
            // If class has a superclass, use inherited factory pattern (return None)
            // If class has no superclass, use simple factory with empty deps (return Some([]))
            // See: packages/compiler-cli/src/ngtsc/annotations/common/src/di.ts:47-52
            if has_superclass { None } else { Some(Vec::new_in(allocator)) }
        }
    }
}

/// Extract dependency metadata from a single constructor parameter.
///
/// The `import_map` is used to look up the source module of the token,
/// enabling proper tracking of import origins for constructor dependencies.
fn extract_param_dependency<'a>(
    param: &'a oxc_ast::ast::FormalParameter<'a>,
    import_map: &ImportMap<'a>,
) -> R3DependencyMetadata<'a> {
    // Extract flags and @Inject token from decorators
    let mut optional = false;
    let mut skip_self = false;
    let mut self_ = false;
    let mut host = false;
    let mut inject_token: Option<Ident<'a>> = None;
    let mut attribute_name: Option<Ident<'a>> = None;

    for decorator in &param.decorators {
        if let Some(name) = get_decorator_name(&decorator.expression) {
            match name.as_str() {
                "Inject" => {
                    // @Inject(TOKEN) - extract the token
                    if let Expression::CallExpression(call) = &decorator.expression {
                        if let Some(arg) = call.arguments.first() {
                            inject_token = extract_inject_token(arg);
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
                        if let Some(Argument::StringLiteral(s)) = call.arguments.first() {
                            attribute_name = Some(s.value.clone().into());
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
    let token = inject_token.or_else(|| extract_param_token(param));

    // Handle @Attribute decorator
    if let Some(attr_name) = attribute_name {
        return R3DependencyMetadata::attribute(attr_name);
    }

    // Build the dependency metadata
    let mut dep = match &token {
        Some(token_name) => {
            let mut d = R3DependencyMetadata::new(token_name.clone());
            // Look up the token in the import map to find its source module and import type
            if let Some(import_info) = import_map.get(token_name) {
                d.token_source_module = Some(import_info.source_module.clone());
                // Always use namespace imports for DI tokens (has_named_import = false).
                // Import elision removes @Inject(TOKEN) argument imports since they're
                // only used in decorator positions that get compiled away.
                // Using bare TOKEN would fail at runtime because the import is gone.
            }
            d
        }
        None => R3DependencyMetadata::invalid(),
    };

    dep.optional = optional;
    dep.skip_self = skip_self;
    dep.self_ = self_;
    dep.host = host;

    dep
}

/// Get the name of a decorator from its expression.
fn get_decorator_name<'a>(expr: &'a Expression<'a>) -> Option<Ident<'a>> {
    match expr {
        // @Optional
        Expression::Identifier(id) => Some(id.name.clone().into()),
        // @Optional()
        Expression::CallExpression(call) => {
            if let Expression::Identifier(id) = &call.callee {
                Some(id.name.clone().into())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extract the injection token from an @Inject decorator argument.
fn extract_inject_token<'a>(arg: &'a Argument<'a>) -> Option<Ident<'a>> {
    match arg {
        Argument::Identifier(id) => Some(id.name.clone().into()),
        _ => {
            // For other expressions, try to get the expression form
            let expr = arg.to_expression();
            match expr {
                Expression::Identifier(id) => Some(id.name.clone().into()),
                _ => None,
            }
        }
    }
}

/// Extract the injection token from a parameter's type annotation.
fn extract_param_token<'a>(param: &'a oxc_ast::ast::FormalParameter<'a>) -> Option<Ident<'a>> {
    // Get the type annotation (directly on FormalParameter)
    let type_annotation = param.type_annotation.as_ref()?;
    let ts_type = &type_annotation.type_annotation;

    // Handle TSTypeReference: SomeClass, SomeModule, etc.
    if let oxc_ast::ast::TSType::TSTypeReference(type_ref) = ts_type {
        // Get the type name
        let type_name = match &type_ref.type_name {
            oxc_ast::ast::TSTypeName::IdentifierReference(id) => Some(id.name.clone().into()),
            oxc_ast::ast::TSTypeName::QualifiedName(_)
            | oxc_ast::ast::TSTypeName::ThisExpression(_) => {
                // Qualified names like Namespace.Type or 'this' type - not valid injection tokens
                None
            }
        };
        return type_name;
    }

    // For primitive types or other patterns, return None (invalid dependency)
    None
}

// =============================================================================
// Decorator Span Collection for Removal
// =============================================================================

/// Collect all decorator spans from constructor parameters.
///
/// Parameter decorators like `@Optional()`, `@Inject()`, `@Host()`, `@Self()`,
/// `@SkipSelf()`, and `@Attribute()` need to be removed from the output since
/// their metadata is extracted into the factory function's inject flags.
///
/// These spans are used by `transform.rs` to remove the decorators from the
/// source text during transformation.
pub fn collect_constructor_decorator_spans(class: &Class<'_>, spans: &mut std::vec::Vec<Span>) {
    // Find the constructor method
    for element in &class.body.body {
        if let ClassElement::MethodDefinition(method) = element {
            if method.kind == MethodDefinitionKind::Constructor {
                // Iterate over constructor parameters
                for param in &method.value.params.items {
                    // Collect all decorator spans from this parameter
                    for decorator in &param.decorators {
                        spans.push(decorator.span);
                    }
                }
                // Only one constructor per class
                break;
            }
        }
    }
}

/// Collect all Angular decorator spans from class members (properties, methods, accessors).
///
/// Member decorators like `@Input()`, `@Output()`, `@HostBinding()`, `@HostListener()`,
/// `@ViewChild()`, `@ViewChildren()`, `@ContentChild()`, and `@ContentChildren()` need
/// to be removed from the output since their metadata is compiled into the definition.
///
/// These spans are used by `transform.rs` to remove the decorators from the
/// source text during transformation.
pub fn collect_member_decorator_spans(class: &Class<'_>, spans: &mut std::vec::Vec<Span>) {
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
            if let Some(name) = get_decorator_name(&decorator.expression) {
                // Only collect Angular-specific member decorators
                match name.as_str() {
                    "Input" | "Output" | "HostBinding" | "HostListener" | "ViewChild"
                    | "ViewChildren" | "ContentChild" | "ContentChildren" => {
                        spans.push(decorator.span);
                    }
                    _ => {}
                }
            }
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

        // Find the first class declaration (handles plain, export default, and export named)
        let mut found_metadata = None;
        for stmt in &parser_ret.program.body {
            let class = match stmt {
                Statement::ClassDeclaration(class) => Some(class.as_ref()),
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
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
            @Component({ selector: '[appDirective]' })
            class TestDirective {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appDirective]");
        });
    }

    #[test]
    fn test_extract_class_name() {
        let code = r#"
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
            @Component({
                selector: 'app-test',
                template: '',
                changeDetection: ChangeDetectionStrategy.OnPush
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, ChangeDetectionStrategy::OnPush);
        });
    }

    #[test]
    fn test_extract_change_detection_default() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: '',
                changeDetection: ChangeDetectionStrategy.Default
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, ChangeDetectionStrategy::Default);
        });
    }

    #[test]
    fn test_extract_change_detection_numeric_on_push() {
        // Angular uses: Default=0, OnPush=1
        let code = r#"
            @Component({
                selector: 'app-test',
                template: '',
                changeDetection: 1
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, ChangeDetectionStrategy::OnPush);
        });
    }

    #[test]
    fn test_change_detection_defaults_to_default() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.change_detection, ChangeDetectionStrategy::Default);
        });
    }

    // =========================================================================
    // Styles tests
    // =========================================================================

    #[test]
    fn test_extract_styles_array() {
        let code = r#"
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
            @Component({
                selector: 'app-test',
                template: '',
                preserveWhitespaces: true
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.preserve_whitespaces);
        });
    }

    #[test]
    fn test_extract_preserve_whitespaces_false() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: '',
                preserveWhitespaces: false
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.preserve_whitespaces);
        });
    }

    #[test]
    fn test_preserve_whitespaces_defaults_to_false() {
        let code = r#"
            @Component({
                selector: 'app-test',
                template: ''
            })
            class TestComponent {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.preserve_whitespaces);
        });
    }

    #[test]
    fn test_extract_schemas() {
        let code = r#"
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
            assert_eq!(meta.change_detection, ChangeDetectionStrategy::OnPush);
            assert!(meta.host.is_some());
            let host = meta.host.as_ref().unwrap();
            assert_eq!(host.class_attr.as_ref().unwrap().as_str(), "app-complete");
            assert_eq!(host.properties.len(), 1);
            assert_eq!(host.listeners.len(), 1);
            assert_eq!(meta.imports.len(), 1);
            assert_eq!(meta.export_as.len(), 1);
            assert_eq!(meta.export_as[0].as_str(), "complete");
            assert!(meta.preserve_whitespaces);
            assert_eq!(meta.schemas.len(), 1);
        });
    }

    // =========================================================================
    // Host directives tests
    // =========================================================================

    #[test]
    fn test_extract_host_directives_simple() {
        let code = r#"
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
    fn test_component_with_skip_self_decorator() {
        let code = r#"
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
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
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

    #[test]
    fn test_collect_constructor_decorator_spans() {
        let code = r#"
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
            collect_constructor_decorator_spans(class, &mut spans);

            // Should collect 4 decorators: @Optional, @Inject, @Host, @Self
            assert_eq!(spans.len(), 4);
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
            collect_constructor_decorator_spans(class, &mut spans);

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
            collect_constructor_decorator_spans(class, &mut spans);

            // No constructor
            assert_eq!(spans.len(), 0);
        });
    }

    #[test]
    fn test_collect_member_decorator_spans() {
        let code = r#"
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
            collect_member_decorator_spans(class, &mut spans);

            // Should collect 9 member decorators (all @Input, @Output, @Host*, @*Child)
            assert_eq!(spans.len(), 9);
        });
    }

    #[test]
    fn test_collect_member_decorator_spans_ignores_non_angular() {
        let code = r#"
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
            collect_member_decorator_spans(class, &mut spans);

            // Should only collect @Input, ignoring custom decorators
            assert_eq!(spans.len(), 1);
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
            collect_member_decorator_spans(class, &mut spans);

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

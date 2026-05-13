//! Angular `@Directive` decorator parser.
//!
//! This module provides utilities for finding and extracting metadata from
//! `@Directive({...})` decorators on TypeScript class declarations.

use std::collections::HashMap;

use oxc_allocator::{Allocator, Box, Vec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, Class, ClassElement, Declaration, Decorator,
    Expression, MethodDefinitionKind, ObjectPropertyKind, Program, PropertyKey, Statement,
    VariableDeclarationKind,
};
use oxc_span::Span;
use oxc_str::Ident;

use super::metadata::{
    R3DirectiveMetadata, R3DirectiveMetadataBuilder, R3HostDirectiveMetadata, R3HostMetadata,
};
use crate::factory::R3DependencyMetadata;
use crate::output::ast::{OutputAstBuilder, OutputExpression, ReadVarExpr};
use crate::output::oxc_converter::convert_oxc_expression;

/// Find the @Directive decorator in a list of decorators.
fn find_directive_decorator<'a>(decorators: &'a [Decorator<'a>]) -> Option<&'a Decorator<'a>> {
    decorators.iter().find(|d| match &d.expression {
        Expression::CallExpression(call) => is_directive_call(&call.callee),
        Expression::Identifier(id) => id.name == "Directive",
        _ => false,
    })
}

/// Find the span of the @Directive decorator on a class.
///
/// Returns the span of the decorator, which can be used to remove the decorator
/// from the source code during compilation.
///
/// This is necessary because Angular's JIT runtime will process any remaining
/// decorators and create conflicting property definitions (like `ɵfac` getters)
/// that interfere with the AOT-compiled assignments.
pub fn find_directive_decorator_span(class: &Class<'_>) -> Option<Span> {
    find_directive_decorator(&class.decorators).map(|d| d.span)
}

/// Check if a callee expression is a call to 'Directive'.
fn is_directive_call(callee: &Expression<'_>) -> bool {
    match callee {
        Expression::Identifier(id) => id.name == "Directive",
        // Handle namespaced imports like ng.Directive or core.Directive
        Expression::StaticMemberExpression(member) => {
            matches!(&member.property.name.as_str(), &"Directive")
        }
        _ => false,
    }
}

/// Extract directive metadata from a class with decorators.
///
/// Searches for a `@Directive({...})` decorator and parses its properties.
/// Returns `None` if no `@Directive` decorator is found.
///
/// The `implicit_standalone` parameter determines the default value for `standalone`
/// when not explicitly set in the decorator. This should be:
/// - `true` for Angular v19+
/// - `false` for Angular v18 and earlier
/// - `true` when the Angular version is unknown (assume latest)
///
/// # Example
///
/// ```typescript
/// @Directive({
///   selector: '[appHighlight]',
///   standalone: true,
///   host: {
///     '[class.active]': 'isActive'
///   }
/// })
/// export class HighlightDirective {}
/// ```
pub fn extract_directive_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    implicit_standalone: bool,
    source_text: Option<&'a str>,
    consts: &StringConsts<'a>,
) -> Option<R3DirectiveMetadata<'a>> {
    // Get the class name
    let class_name: Ident<'a> = class.id.as_ref()?.name.clone().into();

    // Find the @Directive decorator
    let directive_decorator = find_directive_decorator(&class.decorators)?;

    // Get the decorator call arguments
    let call_expr = match &directive_decorator.expression {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Verify it's calling 'Directive'
    if !is_directive_call(&call_expr.callee) {
        return None;
    }

    // Create builder with defaults
    let mut builder = R3DirectiveMetadataBuilder::new(allocator)
        .name(class_name.clone())
        .r#type(OutputAstBuilder::variable(allocator, class_name))
        .is_standalone(implicit_standalone);

    // Get the first argument (the config object) - may be absent for @Directive()
    let config_obj = match call_expr.arguments.first() {
        Some(Argument::ObjectExpression(obj)) => Some(obj),
        _ => None,
    };

    // Track host metadata from the decorator
    let mut host_from_decorator: Option<R3HostMetadata<'a>> = None;

    // Parse each property in the config object (if present)
    if let Some(config_obj) = config_obj {
        for prop in &config_obj.properties {
            if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                let Some(key_name) = get_property_key_name(&prop.key, consts) else {
                    continue;
                };

                match key_name.as_str() {
                    "selector" => {
                        if let Some(selector) = extract_string_value(&prop.value, consts) {
                            builder = builder.selector(selector);
                        }
                    }
                    "standalone" => {
                        if let Some(value) = extract_boolean_value(&prop.value) {
                            builder = builder.is_standalone(value);
                        }
                    }
                    "exportAs" => {
                        if let Some(export_as) = extract_string_value(&prop.value, consts) {
                            // exportAs can be comma-separated: "foo, bar"
                            for part in export_as.as_str().split(',') {
                                let trimmed = part.trim();
                                if !trimmed.is_empty() {
                                    builder = builder
                                        .add_export_as(Ident::from(allocator.alloc_str(trimmed)));
                                }
                            }
                        }
                    }
                    "providers" => {
                        if let Some(providers) =
                            convert_oxc_expression(allocator, &prop.value, source_text)
                        {
                            builder = builder.providers(providers);
                        }
                    }
                    "host" => {
                        host_from_decorator =
                            extract_host_metadata(allocator, &prop.value, consts);
                    }
                    "hostDirectives" => {
                        let host_directives =
                            extract_host_directives(allocator, &prop.value, consts);
                        for hd in host_directives {
                            builder = builder.add_host_directive(hd);
                        }
                    }
                    _ => {
                        // Unknown property - ignore
                    }
                }
            }
        }
    }

    // Extract @Input/@Output/@HostBinding/@HostListener from class members
    builder = builder.extract_from_class(allocator, class, source_text);

    // Detect if ngOnChanges lifecycle hook is implemented
    // Similar to Angular's: const usesOnChanges = members.some(member => ...)
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:315-319
    builder = builder.uses_on_changes(has_ng_on_changes_method(class));

    // Detect if the directive extends another class
    // Similar to Angular's: const usesInheritance = reflector.hasBaseClass(clazz);
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:393
    let has_superclass = class.super_class.is_some();
    builder = builder.uses_inheritance(has_superclass);

    // Extract constructor dependencies for factory generation
    // This enables proper DI for directive constructors
    // See: packages/compiler-cli/src/ngtsc/annotations/common/src/di.ts
    let constructor_deps = extract_constructor_deps(allocator, class, has_superclass, source_text);
    if let Some(deps) = constructor_deps {
        builder = builder.deps(deps);
    }

    // Now we need to merge host metadata from decorator with host metadata from class members
    // The builder already has host data from extract_from_class, we need to merge the decorator host
    let mut metadata = builder.build()?;

    // Merge host metadata from decorator into the existing host metadata
    if let Some(decorator_host) = host_from_decorator {
        // Merge properties
        for prop in decorator_host.properties {
            metadata.host.properties.push(prop);
        }
        // Merge listeners
        for listener in decorator_host.listeners {
            metadata.host.listeners.push(listener);
        }
        // Merge attributes
        for attr in decorator_host.attributes {
            metadata.host.attributes.push(attr);
        }
        // Set class and style attrs if not already set
        if decorator_host.class_attr.is_some() && metadata.host.class_attr.is_none() {
            metadata.host.class_attr = decorator_host.class_attr;
        }
        if decorator_host.style_attr.is_some() && metadata.host.style_attr.is_none() {
            metadata.host.style_attr = decorator_host.style_attr;
        }
    }

    Some(metadata)
}

// =============================================================================
// Constructor Dependency Extraction
// =============================================================================

/// Extract constructor dependencies from a directive class.
///
/// This function analyzes the constructor parameters to determine what
/// dependencies Angular's DI system needs to inject when creating the directive.
///
/// # Arguments
///
/// * `allocator` - Memory allocator for creating new nodes
/// * `class` - The class AST node to extract constructor deps from
/// * `has_superclass` - Whether the class extends another class
///
/// # Returns
///
/// - `Some(Vec)` if the class has a constructor (may be empty if no params)
/// - `Some(empty Vec)` if no constructor but no superclass (implicit no-arg constructor)
/// - `None` if no constructor and has superclass (use inherited factory pattern)
///
/// # Example
///
/// ```typescript
/// @Directive({ selector: '[bitRow]' })
/// export class BitRowDefDirective<T> extends CdkRowDef<T> {
///   constructor(template: TemplateRef<any>) {
///     super(template);
///   }
/// }
/// ```
///
/// Returns: Some([R3DependencyMetadata { token: TemplateRef, ... }])
fn extract_constructor_deps<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    has_superclass: bool,
    source_text: Option<&'a str>,
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
            let params = &ctor.value.params;
            let mut deps = Vec::with_capacity_in(params.items.len(), allocator);

            for param in &params.items {
                let dep = extract_param_dependency(allocator, param, source_text);
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
fn extract_param_dependency<'a>(
    allocator: &'a Allocator,
    param: &oxc_ast::ast::FormalParameter<'a>,
    source_text: Option<&'a str>,
) -> R3DependencyMetadata<'a> {
    // Extract flags and @Inject token from decorators
    let mut optional = false;
    let mut skip_self = false;
    let mut self_ = false;
    let mut host = false;
    let mut inject_token: Option<OutputExpression<'a>> = None;
    let mut attribute_name: Option<Ident<'a>> = None;

    for decorator in &param.decorators {
        if let Some(name) = get_decorator_name_from_expr(&decorator.expression) {
            match name.as_str() {
                "Inject" => {
                    // @Inject(TOKEN) - extract the token
                    if let Expression::CallExpression(call) = &decorator.expression {
                        if let Some(arg) = call.arguments.first() {
                            inject_token =
                                convert_oxc_expression(allocator, arg.to_expression(), source_text);
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
    let token = inject_token.or_else(|| extract_param_token(allocator, param));

    // Handle @Attribute decorator
    if let Some(attr_name) = attribute_name {
        return R3DependencyMetadata {
            token: Some(OutputExpression::Literal(Box::new_in(
                crate::output::ast::LiteralExpr {
                    value: crate::output::ast::LiteralValue::String(attr_name),
                    source_span: None,
                },
                allocator,
            ))),
            attribute_name_type: token, // The type annotation
            host,
            optional,
            self_,
            skip_self,
        };
    }

    R3DependencyMetadata { token, attribute_name_type: None, host, optional, self_, skip_self }
}

/// Get the name of a decorator from its expression.
fn get_decorator_name_from_expr<'a>(expr: &'a Expression<'a>) -> Option<Ident<'a>> {
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

/// Extract the injection token from a parameter's type annotation.
///
/// Returns a bare `ReadVar` expression with the type name. The caller
/// (`resolve_factory_dep_namespaces` in `transform.rs`) is responsible for
/// looking up the correct namespace based on the import map and converting
/// it to a namespace-prefixed `ReadProp` (e.g., `i1.Store`).
fn extract_param_token<'a>(
    allocator: &'a Allocator,
    param: &oxc_ast::ast::FormalParameter<'a>,
) -> Option<OutputExpression<'a>> {
    // Get the type annotation (directly on FormalParameter)
    let type_annotation = param.type_annotation.as_ref()?;
    let ts_type = &type_annotation.type_annotation;

    // Handle TSTypeReference: SomeClass, SomeModule, etc.
    if let oxc_ast::ast::TSType::TSTypeReference(type_ref) = ts_type {
        // Get the type name
        let type_name = match &type_ref.type_name {
            oxc_ast::ast::TSTypeName::IdentifierReference(id) => id.name.clone().into(),
            oxc_ast::ast::TSTypeName::QualifiedName(_)
            | oxc_ast::ast::TSTypeName::ThisExpression(_) => {
                // Qualified names like Namespace.Type or 'this' type - not valid injection tokens
                return None;
            }
        };

        return Some(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: type_name, source_span: None },
            allocator,
        )));
    }

    // For primitive types or other patterns, return None (invalid dependency)
    None
}

/// Check if a class has an `ngOnChanges` method (non-static).
///
/// This is used to determine if the directive implements the `OnChanges` interface
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

/// File-scope map of `const NAME = "value"` declarations.
///
/// Used to resolve identifier references inside decorator metadata — primarily
/// `host: { [ATTR_NAME]: '' }` or `host: { type: VALUE }` patterns — to match
/// the official Angular compiler's compile-time constant folding.
///
/// Only literal string values (string literals and single-quasi template literals)
/// are captured; computed initializers and cross-file imports are out of scope.
pub type StringConsts<'a> = HashMap<&'a str, Ident<'a>>;

/// Walk the top-level statements of a program and collect string-valued `const`
/// declarations.
///
/// Matches both bare `const X = '...'` and `export const X = '...'`. Reassignment
/// kinds (`let`/`var`) are skipped — only `const` is safe to fold.
pub fn collect_string_consts<'a>(program: &Program<'a>) -> StringConsts<'a> {
    let mut map = StringConsts::default();
    for stmt in &program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(d) => d.as_ref(),
            Statement::ExportNamedDeclaration(e) => match &e.declaration {
                Some(Declaration::VariableDeclaration(d)) => d.as_ref(),
                _ => continue,
            },
            _ => continue,
        };
        if !matches!(decl.kind, VariableDeclarationKind::Const) {
            continue;
        }
        for vd in &decl.declarations {
            let BindingPattern::BindingIdentifier(id) = &vd.id else {
                continue;
            };
            let Some(init) = &vd.init else { continue };
            if let Some(value) = literal_string_from_expression(init) {
                map.insert(id.name.as_str(), value);
            }
        }
    }
    map
}

/// Extract a literal string value (`'foo'` or `` `foo` ``) from an expression.
/// Returns `None` for anything that isn't a plain string at compile time.
fn literal_string_from_expression<'a>(expr: &Expression<'a>) -> Option<Ident<'a>> {
    match expr {
        Expression::StringLiteral(lit) => Some(lit.value.clone().into()),
        Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
            tpl.quasis.first().and_then(|q| q.value.cooked.clone().map(Into::into))
        }
        _ => None,
    }
}

/// Get the name of a property key as a string.
///
/// Resolves same-file `const` identifiers in computed keys (`[FOO]: bar`) so the
/// emitted directive metadata matches the official Angular compiler's output.
fn get_property_key_name<'a>(
    key: &PropertyKey<'a>,
    consts: &StringConsts<'a>,
) -> Option<Ident<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(lit) => Some(lit.value.clone().into()),
        // Computed identifier reference: `[FOO]: bar` — resolve against same-file consts.
        PropertyKey::Identifier(id) => consts.get(id.name.as_str()).cloned(),
        _ => None,
    }
}

/// Extract a string value from an expression.
///
/// Resolves same-file `const` identifier references in value position
/// (`host: { type: FOO }`) so the emitted metadata matches the official
/// Angular compiler's output.
fn extract_string_value<'a>(
    expr: &Expression<'a>,
    consts: &StringConsts<'a>,
) -> Option<Ident<'a>> {
    match expr {
        Expression::Identifier(id) => consts.get(id.name.as_str()).cloned(),
        _ => literal_string_from_expression(expr),
    }
}

/// Extract a boolean value from an expression.
fn extract_boolean_value(expr: &Expression<'_>) -> Option<bool> {
    match expr {
        Expression::BooleanLiteral(lit) => Some(lit.value.into()),
        _ => None,
    }
}

/// Extract host metadata from a host object expression.
///
/// Reference: packages/compiler/src/render3/view/compiler.ts:560-604
fn extract_host_metadata<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
    consts: &StringConsts<'a>,
) -> Option<R3HostMetadata<'a>> {
    let Expression::ObjectExpression(obj) = expr else {
        return None;
    };

    let mut host = R3HostMetadata::new(allocator);

    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            let Some(key_name) = get_property_key_name(&prop.key, consts) else {
                continue;
            };
            let Some(value) = extract_string_value(&prop.value, consts) else {
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
                match key_str {
                    "class" => {
                        host.class_attr = Some(value);
                    }
                    "style" => {
                        host.style_attr = Some(value);
                    }
                    _ => {
                        // Regular static attribute - convert to OutputExpression
                        let attr_expr = OutputAstBuilder::string(allocator, value);
                        host.attributes.push((key_name, attr_expr));
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
    consts: &StringConsts<'a>,
) -> Vec<'a, R3HostDirectiveMetadata<'a>> {
    let mut result = Vec::new_in(allocator);

    let Expression::ArrayExpression(arr) = expr else {
        return result;
    };

    for element in &arr.elements {
        if let Some(meta) = extract_single_host_directive(allocator, element, consts) {
            result.push(meta);
        }
    }

    result
}

/// Extract a single host directive from an array element.
fn extract_single_host_directive<'a>(
    allocator: &'a Allocator,
    element: &ArrayExpressionElement<'a>,
    consts: &StringConsts<'a>,
) -> Option<R3HostDirectiveMetadata<'a>> {
    match element {
        // Simple identifier: TooltipDirective
        ArrayExpressionElement::Identifier(id) => Some(R3HostDirectiveMetadata {
            directive: OutputAstBuilder::variable(allocator, id.name.clone().into()),
            is_forward_reference: false,
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
        }),

        // Object expression: { directive: ColorDirective, inputs: [...], outputs: [...] }
        ArrayExpressionElement::ObjectExpression(obj) => {
            let mut directive_expr = None;
            let mut inputs = Vec::new_in(allocator);
            let mut outputs = Vec::new_in(allocator);
            let mut is_forward_reference = false;

            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key, consts) else {
                        continue;
                    };

                    match key_name.as_str() {
                        "directive" => {
                            let (expr, is_forward_ref) =
                                extract_directive_reference(allocator, &prop.value);
                            directive_expr = expr;
                            is_forward_reference = is_forward_ref;
                        }
                        "inputs" => {
                            inputs = extract_io_mappings(allocator, &prop.value);
                        }
                        "outputs" => {
                            outputs = extract_io_mappings(allocator, &prop.value);
                        }
                        _ => {}
                    }
                }
            }

            directive_expr.map(|directive| R3HostDirectiveMetadata {
                directive,
                is_forward_reference,
                inputs,
                outputs,
            })
        }

        // ForwardRef call: forwardRef(() => MyDirective)
        ArrayExpressionElement::CallExpression(call) => {
            if is_forward_ref_call(&call.callee) {
                if let Some(name) = extract_forward_ref_directive_name(call.arguments.first()) {
                    return Some(R3HostDirectiveMetadata {
                        directive: OutputAstBuilder::variable(allocator, name),
                        is_forward_reference: true,
                        inputs: Vec::new_in(allocator),
                        outputs: Vec::new_in(allocator),
                    });
                }
            }
            None
        }

        _ => None,
    }
}

/// Extract a directive reference from an expression.
///
/// Returns the directive expression and whether it's a forward reference.
fn extract_directive_reference<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> (Option<crate::output::ast::OutputExpression<'a>>, bool) {
    match expr {
        // Simple identifier: ColorDirective
        Expression::Identifier(id) => {
            (Some(OutputAstBuilder::variable(allocator, id.name.clone().into())), false)
        }

        // ForwardRef call: forwardRef(() => ColorDirective)
        Expression::CallExpression(call) => {
            if is_forward_ref_call(&call.callee) {
                if let Some(name) = extract_forward_ref_directive_name(call.arguments.first()) {
                    return (Some(OutputAstBuilder::variable(allocator, name)), true);
                }
            }
            (None, false)
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
        Argument::ArrowFunctionExpression(arrow) => {
            let body = &arrow.body;
            if body.statements.is_empty() {
                return None;
            }
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
/// - Mapping string: `["color: bgColor"]` - internal name mapped to public name
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    /// Helper function to parse TypeScript code and find the directive decorator span
    /// from the first class found.
    fn find_span_in_code(code: &str) -> Option<Span> {
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
                return find_directive_decorator_span(class);
            }
        }

        None
    }

    #[test]
    fn test_find_directive_decorator_span() {
        let code = r#"
            @Directive({ selector: '[myDir]' })
            class MyDirective {}
        "#;
        let span = find_span_in_code(code);
        assert!(span.is_some());
    }

    #[test]
    fn test_find_directive_decorator_span_exported() {
        let code = r#"
            @Directive({ selector: '[myDir]' })
            export class MyDirective {}
        "#;
        let span = find_span_in_code(code);
        assert!(span.is_some());
    }

    #[test]
    fn test_find_directive_decorator_span_namespaced() {
        let code = r#"
            @ng.Directive({ selector: '[myDir]' })
            class MyDirective {}
        "#;
        let span = find_span_in_code(code);
        assert!(span.is_some());
    }

    #[test]
    fn test_no_directive_decorator_returns_none() {
        let code = r#"
            class PlainClass {}
        "#;
        let span = find_span_in_code(code);
        assert!(span.is_none());
    }

    #[test]
    fn test_component_decorator_returns_none() {
        // @Component should NOT be found by find_directive_decorator_span
        let code = r#"
            @Component({ selector: 'app-test', template: '' })
            class MyComponent {}
        "#;
        let span = find_span_in_code(code);
        assert!(span.is_none());
    }

    #[test]
    fn test_directive_without_call_still_matches() {
        // @Directive without () is technically invalid Angular, but we still
        // match it for defensive decorator removal (matches component behavior)
        let code = r#"
            @Directive
            class MyDirective {}
        "#;
        let span = find_span_in_code(code);
        // This matches the component decorator behavior for consistency
        assert!(span.is_some());
    }

    // =========================================================================
    // extract_directive_metadata tests
    // =========================================================================

    /// Helper function to parse TypeScript code and extract directive metadata
    /// from the first @Directive decorated class found.
    fn with_extracted_metadata<F>(code: &str, implicit_standalone: bool, callback: F)
    where
        F: FnOnce(Option<&R3DirectiveMetadata<'_>>),
    {
        let allocator = Allocator::default();
        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();
        let consts = collect_string_consts(&parser_ret.program);

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
                if let Some(metadata) = extract_directive_metadata(
                    &allocator,
                    class,
                    implicit_standalone,
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
    fn assert_directive_metadata<F>(code: &str, callback: F)
    where
        F: FnOnce(&R3DirectiveMetadata<'_>),
    {
        with_extracted_metadata(code, true, |meta| {
            let meta = meta.expect("Expected to find @Directive metadata");
            callback(meta);
        });
    }

    /// Shorthand for tests that expect no metadata to be found.
    fn assert_no_directive_metadata(code: &str) {
        with_extracted_metadata(code, true, |meta| {
            assert!(meta.is_none(), "Expected no @Directive metadata to be found");
        });
    }

    #[test]
    fn test_extract_directive_selector() {
        let code = r#"
            @Directive({ selector: '[appHighlight]' })
            class HighlightDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appHighlight]");
            assert_eq!(meta.name.as_str(), "HighlightDirective");
        });
    }

    #[test]
    fn test_extract_directive_standalone_true() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                standalone: true
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert!(meta.is_standalone);
        });
    }

    #[test]
    fn test_extract_directive_standalone_false() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                standalone: false
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert!(!meta.is_standalone);
        });
    }

    #[test]
    fn test_extract_directive_standalone_defaults_to_implicit() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            class TestDirective {}
        "#;
        // Test with implicit_standalone=true
        with_extracted_metadata(code, true, |meta| {
            assert!(meta.unwrap().is_standalone);
        });
        // Test with implicit_standalone=false
        with_extracted_metadata(code, false, |meta| {
            assert!(!meta.unwrap().is_standalone);
        });
    }

    #[test]
    fn test_extract_directive_export_as() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                exportAs: 'testDir'
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.export_as.len(), 1);
            assert_eq!(meta.export_as[0].as_str(), "testDir");
        });
    }

    #[test]
    fn test_extract_directive_export_as_multiple() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                exportAs: 'foo, bar, baz'
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.export_as.len(), 3);
            assert_eq!(meta.export_as[0].as_str(), "foo");
            assert_eq!(meta.export_as[1].as_str(), "bar");
            assert_eq!(meta.export_as[2].as_str(), "baz");
        });
    }

    #[test]
    fn test_extract_directive_host_property_bindings() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                host: {
                    '[class.active]': 'isActive',
                    '[attr.aria-label]': 'label'
                }
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.properties.len(), 2);
        });
    }

    #[test]
    fn test_extract_directive_host_listeners() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                host: {
                    '(click)': 'onClick()',
                    '(mouseenter)': 'onMouseEnter($event)'
                }
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.listeners.len(), 2);
        });
    }

    #[test]
    fn test_extract_directive_host_static_attributes() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                host: {
                    'role': 'button',
                    'tabindex': '0'
                }
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.attributes.len(), 2);
        });
    }

    // Identifier resolution in host: { } — match the official Angular compiler,
    // which folds same-file `const` references at compile time and emits hostAttrs.

    #[test]
    fn test_extract_directive_host_computed_key_identifier() {
        let code = r#"
            const ATTR = 'data-foo';
            @Directive({ selector: '[d]', host: { [ATTR]: '' } })
            class D {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.attributes.len(), 1);
            assert_eq!(meta.host.attributes[0].0.as_str(), "data-foo");
        });
    }

    #[test]
    fn test_extract_directive_host_value_identifier() {
        let code = r#"
            const VAL = 'submit';
            @Directive({ selector: '[d]', host: { type: VAL } })
            class D {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.attributes.len(), 1);
            assert_eq!(meta.host.attributes[0].0.as_str(), "type");
        });
    }

    #[test]
    fn test_extract_directive_host_template_literal_const() {
        let code = r#"
            const ATTR = `data-foo`;
            @Directive({ selector: '[d]', host: { [ATTR]: '' } })
            class D {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.attributes.len(), 1);
            assert_eq!(meta.host.attributes[0].0.as_str(), "data-foo");
        });
    }

    #[test]
    fn test_extract_directive_host_unknown_identifier_dropped() {
        // Unresolved identifier (no matching const) is still dropped — current behavior.
        let code = r#"
            @Directive({ selector: '[d]', host: { [UNKNOWN]: '' } })
            class D {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.attributes.len(), 0);
        });
    }

    #[test]
    fn test_extract_directive_host_exported_const_identifier() {
        // `export const` (not just `const`) in the same file must also be resolved.
        let code = r#"
            export const MARKER_ATTR = 'data-marker';
            @Directive({
                selector: '[marker]',
                host: { [MARKER_ATTR]: '' }
            })
            class MarkerDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.attributes.len(), 1);
            assert_eq!(meta.host.attributes[0].0.as_str(), "data-marker");
        });
    }

    #[test]
    fn test_extract_directive_host_class_attr() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                host: {
                    'class': 'highlight-directive'
                }
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.class_attr.as_ref().unwrap().as_str(), "highlight-directive");
        });
    }

    #[test]
    fn test_extract_directive_host_style_attr() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                host: {
                    'style': 'background-color: yellow;'
                }
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(
                meta.host.style_attr.as_ref().unwrap().as_str(),
                "background-color: yellow;"
            );
        });
    }

    #[test]
    fn test_extract_directive_host_directives_simple() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                hostDirectives: [TooltipDirective]
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert!(!meta.host_directives[0].is_forward_reference);
        });
    }

    #[test]
    fn test_extract_directive_host_directives_with_mappings() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                hostDirectives: [
                    {
                        directive: ColorDirective,
                        inputs: ['color: bgColor'],
                        outputs: ['colorChange']
                    }
                ]
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.host_directives[0].inputs.len(), 1);
            assert_eq!(meta.host_directives[0].outputs.len(), 1);
            // Check input mapping: "color: bgColor" -> (public="bgColor", internal="color")
            assert_eq!(meta.host_directives[0].inputs[0].0.as_str(), "bgColor");
            assert_eq!(meta.host_directives[0].inputs[0].1.as_str(), "color");
        });
    }

    #[test]
    fn test_extract_directive_host_directives_forward_ref() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                hostDirectives: [forwardRef(() => MyDirective)]
            })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host_directives.len(), 1);
            assert!(meta.host_directives[0].is_forward_reference);
        });
    }

    #[test]
    fn test_extract_directive_with_inputs_from_class() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            class TestDirective {
                @Input() name: string;
                @Input('aliasedValue') value: number;
            }
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.inputs.len(), 2);
            assert_eq!(meta.inputs[0].class_property_name.as_str(), "name");
            assert_eq!(meta.inputs[0].binding_property_name.as_str(), "name");
            assert_eq!(meta.inputs[1].class_property_name.as_str(), "value");
            assert_eq!(meta.inputs[1].binding_property_name.as_str(), "aliasedValue");
        });
    }

    #[test]
    fn test_extract_directive_with_outputs_from_class() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            class TestDirective {
                @Output() clicked = new EventEmitter<void>();
                @Output('valueChanged') onChange = new EventEmitter<string>();
            }
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.outputs.len(), 2);
            assert_eq!(meta.outputs[0].0.as_str(), "clicked");
            assert_eq!(meta.outputs[0].1.as_str(), "clicked");
            assert_eq!(meta.outputs[1].0.as_str(), "onChange");
            assert_eq!(meta.outputs[1].1.as_str(), "valueChanged");
        });
    }

    #[test]
    fn test_extract_directive_with_host_binding_decorator() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            class TestDirective {
                @HostBinding('class.active') isActive = false;
            }
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.properties.len(), 1);
            assert_eq!(meta.host.properties[0].0.as_str(), "[class.active]");
            assert_eq!(meta.host.properties[0].1.as_str(), "isActive");
        });
    }

    #[test]
    fn test_extract_directive_with_host_listener_decorator() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            class TestDirective {
                @HostListener('click') onClick() {}
            }
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.host.listeners.len(), 1);
            assert_eq!(meta.host.listeners[0].0.as_str(), "(click)");
            assert_eq!(meta.host.listeners[0].1.as_str(), "onClick()");
        });
    }

    #[test]
    fn test_extract_directive_merges_host_from_decorator_and_class() {
        let code = r#"
            @Directive({
                selector: '[appTest]',
                host: {
                    '[class.highlighted]': 'isHighlighted',
                    '(focus)': 'onFocus()'
                }
            })
            class TestDirective {
                @HostBinding('class.active') isActive = false;
                @HostListener('click') onClick() {}
            }
        "#;
        assert_directive_metadata(code, |meta| {
            // Should have 2 properties: from decorator and from @HostBinding
            assert_eq!(meta.host.properties.len(), 2);
            // Should have 2 listeners: from decorator and from @HostListener
            assert_eq!(meta.host.listeners.len(), 2);
        });
    }

    #[test]
    fn test_extract_directive_plain_class_returns_none() {
        let code = r#"
            class PlainClass {}
        "#;
        assert_no_directive_metadata(code);
    }

    #[test]
    fn test_extract_directive_component_decorator_does_not_match() {
        let code = r#"
            @Component({ selector: 'app-test', template: '' })
            class TestComponent {}
        "#;
        assert_no_directive_metadata(code);
    }

    #[test]
    fn test_directive_without_call_returns_none() {
        // @Directive without () returns None for metadata extraction
        // (this is technically invalid Angular, but we handle it gracefully)
        let code = r#"
            @Directive
            class MyDirective {}
        "#;
        assert_no_directive_metadata(code);
    }

    #[test]
    fn test_empty_directive_decorator() {
        // @Directive({}) - explicit empty config object
        let code = r#"
            @Directive({})
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert!(meta.selector.is_none());
            assert_eq!(meta.name.as_str(), "TestDirective");
        });
    }

    #[test]
    fn test_directive_with_empty_parens() {
        // @Directive() - no config argument at all
        // This is common for abstract base directive classes
        let code = r#"
            @Directive()
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert!(meta.selector.is_none());
            assert_eq!(meta.name.as_str(), "TestDirective");
        });
    }

    #[test]
    fn test_exported_directive() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            export class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appTest]");
        });
    }

    #[test]
    fn test_export_default_directive() {
        let code = r#"
            @Directive({ selector: '[appTest]' })
            export default class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appTest]");
        });
    }

    #[test]
    fn test_namespaced_directive_decorator() {
        let code = r#"
            @ng.Directive({ selector: '[appTest]' })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appTest]");
        });
    }

    #[test]
    fn test_full_directive_decorator() {
        let code = r#"
            @Directive({
                selector: '[appComplete]',
                standalone: true,
                exportAs: 'complete',
                host: {
                    'class': 'complete-directive',
                    '[class.active]': 'isActive',
                    '(click)': 'onClick()'
                },
                hostDirectives: [TooltipDirective]
            })
            class CompleteDirective {
                @Input() inputValue: string;
                @Output() outputEvent = new EventEmitter<string>();
                @HostBinding('attr.role') role = 'button';
                @HostListener('mouseenter') onMouseEnter() {}
            }
        "#;
        assert_directive_metadata(code, |meta| {
            assert_eq!(meta.selector.as_ref().unwrap().as_str(), "[appComplete]");
            assert!(meta.is_standalone);
            assert_eq!(meta.export_as.len(), 1);
            assert_eq!(meta.export_as[0].as_str(), "complete");
            assert_eq!(meta.host.class_attr.as_ref().unwrap().as_str(), "complete-directive");
            // 2 from decorator + 1 from @HostBinding
            assert_eq!(meta.host.properties.len(), 2);
            // 1 from decorator + 1 from @HostListener
            assert_eq!(meta.host.listeners.len(), 2);
            assert_eq!(meta.host_directives.len(), 1);
            assert_eq!(meta.inputs.len(), 1);
            assert_eq!(meta.outputs.len(), 1);
        });
    }

    #[test]
    fn test_directive_without_inheritance() {
        // A directive that does NOT extend any base class
        // should have uses_inheritance = false
        let code = r#"
            @Directive({ selector: '[appTest]' })
            class TestDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert!(!meta.uses_inheritance, "Should not have inheritance");
        });
    }

    #[test]
    fn test_directive_with_inheritance() {
        // A directive that extends a base class
        // should have uses_inheritance = true
        let code = r#"
            @Directive({ selector: '[appChild]' })
            class ChildDirective extends BaseDirective {}
        "#;
        assert_directive_metadata(code, |meta| {
            assert!(meta.uses_inheritance, "Should have inheritance");
        });
    }

    #[test]
    fn test_extract_param_token_returns_read_var_not_read_prop() {
        // Regression test for bug where extract_param_token() returned
        // ReadProp(i0.TypeName) with hardcoded i0, instead of a bare
        // ReadVar(TypeName). The ReadProp prevented resolve_factory_dep_namespaces()
        // from processing the tokens (it only handles ReadVar tokens), causing
        // all directive constructor deps to be assigned the wrong namespace.
        //
        // The fix: Changed extract_param_token to return ReadVar(TypeName)
        // matching the pattern used by injectable, pipe, and ng_module extractors.
        let code = r#"
            @Directive({ selector: '[myDir]' })
            class MyDirective {
                constructor(private store: Store, private svc: SomeService) {}
            }
        "#;
        assert_directive_metadata(code, |meta| {
            // Should have 2 constructor deps
            let deps = meta.deps.as_ref().expect("Directive should have deps");
            assert_eq!(deps.len(), 2, "Should have 2 constructor deps");

            // Each dep token should be a ReadVar (bare identifier), NOT a ReadProp
            // ReadVar tokens can be resolved by resolve_factory_dep_namespaces()
            // to the correct namespace prefix (e.g., i1.Store instead of i0.Store)
            for (i, dep) in deps.iter().enumerate() {
                let token = dep.token.as_ref().unwrap_or_else(|| {
                    panic!("Dep {} should have a token", i);
                });
                assert!(
                    matches!(token, crate::output::ast::OutputExpression::ReadVar(_)),
                    "Dep {} token should be ReadVar (bare identifier), but got ReadProp or other. \
                     This means resolve_factory_dep_namespaces() cannot process it.",
                    i
                );
            }

            // Verify the specific token names
            if let crate::output::ast::OutputExpression::ReadVar(var) =
                deps[0].token.as_ref().unwrap()
            {
                assert_eq!(var.name.as_str(), "Store", "First dep should be Store");
            }
            if let crate::output::ast::OutputExpression::ReadVar(var) =
                deps[1].token.as_ref().unwrap()
            {
                assert_eq!(var.name.as_str(), "SomeService", "Second dep should be SomeService");
            }
        });
    }
}

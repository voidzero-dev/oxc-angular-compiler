//! Angular `@Pipe` decorator parser.
//!
//! This module extracts metadata from `@Pipe({...})` decorators
//! on TypeScript class declarations.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_ast::ast::{
    Argument, Class, ClassElement, Decorator, Expression, MethodDefinitionKind, ObjectPropertyKind,
    PropertyKey,
};
use oxc_span::Span;
use oxc_str::Ident;

use super::metadata::R3PipeMetadata;
use crate::factory::R3DependencyMetadata;
use crate::output::ast::{OutputExpression, ReadVarExpr};

/// Extracted pipe metadata from a `@Pipe` decorator.
///
/// This struct holds the raw metadata extracted from the decorator
/// for use by build tools and analysis.
#[derive(Debug)]
pub struct PipeMetadata<'a> {
    /// The name of the pipe class.
    pub class_name: Ident<'a>,

    /// Span of the class declaration.
    pub class_span: Span,

    /// The pipe name used in templates (from `@Pipe({name: '...'})`).
    pub pipe_name: Option<Ident<'a>>,

    /// Whether the pipe is pure (default: true).
    /// Pure pipes only transform when inputs change.
    pub pure: bool,

    /// Whether this is a standalone pipe (default: true for v19+).
    pub standalone: bool,

    /// Constructor dependencies for factory generation.
    /// `None` means no constructor (use inherited factory).
    /// `Some(vec)` means constructor exists with these dependencies.
    pub deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
}

impl<'a> PipeMetadata<'a> {
    /// Create a new PipeMetadata with defaults.
    pub fn new(
        _allocator: &'a Allocator,
        class_name: Ident<'a>,
        class_span: Span,
        implicit_standalone: bool,
    ) -> Self {
        Self {
            class_name,
            class_span,
            pipe_name: None,
            pure: true, // Angular default is pure
            standalone: implicit_standalone,
            deps: None,
        }
    }

    /// Convert to R3PipeMetadata for compilation.
    ///
    /// This creates the metadata structure needed by the pipe compiler.
    pub fn to_r3_metadata(&self, allocator: &'a Allocator) -> Option<R3PipeMetadata<'a>> {
        // Create type expression: reference to the pipe class
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: self.class_name.clone(), source_span: None },
            allocator,
        ));

        Some(R3PipeMetadata {
            name: self.class_name.clone(),
            pipe_name: self.pipe_name.clone(),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            pure: self.pure,
            is_standalone: self.standalone,
        })
    }
}

/// Extract pipe metadata from a class with decorators.
///
/// Searches for a `@Pipe({...})` decorator and parses its properties.
/// Returns `None` if no `@Pipe` decorator is found.
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
/// @Pipe({
///   name: 'myPipe',
///   pure: true,
///   standalone: true,
/// })
/// export class MyPipe implements PipeTransform {}
/// ```
pub fn extract_pipe_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    implicit_standalone: bool,
    _source_text: Option<&'a str>,
) -> Option<PipeMetadata<'a>> {
    // Get the class name
    let class_name: Ident<'a> = class.id.as_ref()?.name.clone().into();
    let class_span = class.span;

    // Find the @Pipe decorator
    let pipe_decorator = find_pipe_decorator(&class.decorators)?;

    // Get the decorator call arguments
    let call_expr = match &pipe_decorator.expression {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Verify it's calling 'Pipe'
    if !is_pipe_call(&call_expr.callee) {
        return None;
    }

    // Get the first argument (the config object)
    let config_arg = call_expr.arguments.first()?;
    let config_obj = match config_arg {
        Argument::ObjectExpression(obj) => obj,
        _ => return None,
    };

    // Create metadata with defaults
    let mut metadata = PipeMetadata::new(allocator, class_name, class_span, implicit_standalone);

    // Parse each property in the config object
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            let Some(key_name) = get_property_key_name(&prop.key) else {
                continue;
            };

            match key_name.as_str() {
                "name" => {
                    metadata.pipe_name = extract_string_value(&prop.value);
                }
                "pure" => {
                    if let Some(value) = extract_boolean_value(&prop.value) {
                        metadata.pure = value;
                    }
                }
                "standalone" => {
                    if let Some(value) = extract_boolean_value(&prop.value) {
                        metadata.standalone = value;
                    }
                }
                _ => {
                    // Unknown property - ignore
                }
            }
        }
    }

    // Extract constructor dependencies for factory generation
    metadata.deps = extract_constructor_deps(allocator, class);

    Some(metadata)
}

/// Find the @Pipe decorator in a list of decorators.
fn find_pipe_decorator<'a>(decorators: &'a [Decorator<'a>]) -> Option<&'a Decorator<'a>> {
    decorators.iter().find(|d| match &d.expression {
        Expression::CallExpression(call) => is_pipe_call(&call.callee),
        Expression::Identifier(id) => id.name == "Pipe",
        _ => false,
    })
}

/// Find the span of the @Pipe decorator on a class.
///
/// Returns the span including any leading whitespace/newlines that should be removed
/// along with the decorator.
pub fn find_pipe_decorator_span(class: &Class<'_>) -> Option<Span> {
    find_pipe_decorator(&class.decorators).map(|d| d.span)
}

/// Check if a callee expression is a call to 'Pipe'.
fn is_pipe_call(callee: &Expression<'_>) -> bool {
    match callee {
        Expression::Identifier(id) => id.name == "Pipe",
        // Handle namespaced imports like ng.Pipe or core.Pipe
        Expression::StaticMemberExpression(member) => {
            matches!(&member.property.name.as_str(), &"Pipe")
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

/// Extract constructor dependencies from a pipe class.
///
/// Returns `Some(Vec<deps>)` if the class has a constructor,
/// or `None` if there is no constructor (will use inherited factory).
fn extract_constructor_deps<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Option<Vec<'a, R3DependencyMetadata<'a>>> {
    // Find the constructor method
    let constructor = class.body.body.iter().find_map(|element| {
        if let ClassElement::MethodDefinition(method) = element {
            if method.kind == MethodDefinitionKind::Constructor {
                return Some(method);
            }
        }
        None
    })?;

    // Get the constructor's parameters
    let params = &constructor.value.params;
    let mut deps = Vec::with_capacity_in(params.items.len(), allocator);

    for param in &params.items {
        let dep = extract_param_dependency(allocator, param);
        deps.push(dep);
    }

    Some(deps)
}

/// Extract dependency metadata from a single constructor parameter.
fn extract_param_dependency<'a>(
    allocator: &'a Allocator,
    param: &oxc_ast::ast::FormalParameter<'a>,
) -> R3DependencyMetadata<'a> {
    // Extract flags from decorators
    let mut optional = false;
    let mut skip_self = false;
    let mut self_ = false;
    let mut host = false;
    let mut attribute_name: Option<Ident<'a>> = None;

    for decorator in &param.decorators {
        if let Some(name) = get_decorator_name(&decorator.expression) {
            match name.as_str() {
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

    // Extract the token (type annotation or parameter name)
    let token = extract_param_token(allocator, param);

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

/// Extract the injection token from a parameter's type annotation.
fn extract_param_token<'a>(
    allocator: &'a Allocator,
    param: &oxc_ast::ast::FormalParameter<'a>,
) -> Option<OutputExpression<'a>> {
    // First try to get the type annotation (directly on FormalParameter, not on pattern)
    let type_annotation = param.type_annotation.as_ref()?;
    let ts_type = &type_annotation.type_annotation;

    // Handle TSTypeReference: SomeClass, SomeModule, etc.
    if let oxc_ast::ast::TSType::TSTypeReference(type_ref) = ts_type {
        // Get the type name
        let type_name: Ident<'a> = match &type_ref.type_name {
            oxc_ast::ast::TSTypeName::IdentifierReference(id) => id.name.clone().into(),
            oxc_ast::ast::TSTypeName::QualifiedName(_)
            | oxc_ast::ast::TSTypeName::ThisExpression(_) => {
                // Qualified names like Namespace.Type or 'this' type - not valid injection tokens
                return None;
            }
        };

        // Return a reference to the type
        return Some(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: type_name, source_span: None },
            allocator,
        )));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    /// Helper function to parse TypeScript code and extract pipe metadata
    /// from the first @Pipe decorated class found.
    fn with_extracted_metadata<F>(code: &str, implicit_standalone: bool, callback: F)
    where
        F: FnOnce(Option<&PipeMetadata<'_>>),
    {
        let allocator = Allocator::default();
        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

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
                if let Some(metadata) =
                    extract_pipe_metadata(&allocator, class, implicit_standalone, Some(code))
                {
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
        F: FnOnce(&PipeMetadata<'_>),
    {
        with_extracted_metadata(code, true, |meta| {
            let meta = meta.expect("Expected to find @Pipe metadata");
            callback(meta);
        });
    }

    /// Shorthand for tests that expect no metadata to be found.
    fn assert_no_metadata(code: &str) {
        with_extracted_metadata(code, true, |meta| {
            assert!(meta.is_none(), "Expected no @Pipe metadata to be found");
        });
    }

    // =========================================================================
    // Basic extraction tests
    // =========================================================================

    #[test]
    fn test_extract_pipe_name() {
        let code = r#"
            @Pipe({ name: 'myPipe' })
            class MyPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.pipe_name.as_ref().map(|n| n.as_str()), Some("myPipe"));
        });
    }

    #[test]
    fn test_extract_class_name() {
        let code = r#"
            @Pipe({ name: 'test' })
            class MyAwesomePipe {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "MyAwesomePipe");
        });
    }

    // =========================================================================
    // Pure tests
    // =========================================================================

    #[test]
    fn test_extract_pure_true() {
        let code = r#"
            @Pipe({
                name: 'test',
                pure: true
            })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.pure);
        });
    }

    #[test]
    fn test_extract_pure_false() {
        let code = r#"
            @Pipe({
                name: 'test',
                pure: false
            })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.pure);
        });
    }

    #[test]
    fn test_pure_defaults_to_true() {
        let code = r#"
            @Pipe({
                name: 'test'
            })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.pure);
        });
    }

    // =========================================================================
    // Standalone tests
    // =========================================================================

    #[test]
    fn test_extract_standalone_true() {
        let code = r#"
            @Pipe({
                name: 'test',
                standalone: true
            })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.standalone);
        });
    }

    #[test]
    fn test_extract_standalone_false() {
        let code = r#"
            @Pipe({
                name: 'test',
                standalone: false
            })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.standalone);
        });
    }

    #[test]
    fn test_standalone_defaults_to_implicit_value_true() {
        let code = r#"
            @Pipe({
                name: 'test'
            })
            class TestPipe {}
        "#;
        with_extracted_metadata(code, true, |meta| {
            let meta = meta.expect("Expected metadata");
            assert!(meta.standalone);
        });
    }

    #[test]
    fn test_standalone_defaults_to_implicit_value_false() {
        let code = r#"
            @Pipe({
                name: 'test'
            })
            class TestPipe {}
        "#;
        with_extracted_metadata(code, false, |meta| {
            let meta = meta.expect("Expected metadata");
            assert!(!meta.standalone);
        });
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_no_pipe_decorator_returns_none() {
        let code = r#"
            class PlainClass {}
        "#;
        assert_no_metadata(code);
    }

    #[test]
    fn test_pipe_decorator_without_call_returns_none() {
        let code = r#"
            @Pipe
            class TestPipe {}
        "#;
        assert_no_metadata(code);
    }

    #[test]
    fn test_empty_pipe_decorator() {
        let code = r#"
            @Pipe({})
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.pipe_name.is_none());
            assert_eq!(meta.class_name.as_str(), "TestPipe");
            assert!(meta.pure); // default
        });
    }

    #[test]
    fn test_exported_class() {
        let code = r#"
            @Pipe({ name: 'test' })
            export class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.pipe_name.as_ref().map(|n| n.as_str()), Some("test"));
        });
    }

    #[test]
    fn test_namespaced_pipe_decorator() {
        let code = r#"
            @ng.Pipe({ name: 'test' })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.pipe_name.as_ref().map(|n| n.as_str()), Some("test"));
        });
    }

    #[test]
    fn test_full_pipe_configuration() {
        let code = r#"
            @Pipe({
                name: 'myPipe',
                pure: false,
                standalone: true
            })
            export class MyPipe implements PipeTransform {
                transform(value: any): any {
                    return value;
                }
            }
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "MyPipe");
            assert_eq!(meta.pipe_name.as_ref().map(|n| n.as_str()), Some("myPipe"));
            assert!(!meta.pure);
            assert!(meta.standalone);
        });
    }

    #[test]
    fn test_pipe_with_template_literal_name() {
        let code = r#"
            @Pipe({ name: `myPipe` })
            class TestPipe {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.pipe_name.as_ref().map(|n| n.as_str()), Some("myPipe"));
        });
    }
}

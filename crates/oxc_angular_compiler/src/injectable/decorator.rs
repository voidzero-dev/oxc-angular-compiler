//! Angular `@Injectable` decorator parser.
//!
//! This module extracts metadata from `@Injectable({...})` decorators
//! on TypeScript class declarations.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, Class, ClassElement, Decorator, Expression,
    MethodDefinitionKind, ObjectPropertyKind, PropertyKey,
};
use oxc_span::{Atom, Span};

use crate::factory::R3DependencyMetadata;
use crate::output::ast::{OutputExpression, ReadVarExpr};
use crate::output::oxc_converter::convert_oxc_expression;

/// Extracted injectable metadata from a `@Injectable` decorator.
#[derive(Debug)]
pub struct InjectableMetadata<'a> {
    /// The name of the injectable class.
    pub class_name: Atom<'a>,
    /// Span of the class declaration.
    pub class_span: Span,
    /// Where this injectable is provided.
    pub provided_in: Option<ProvidedInValue<'a>>,
    /// useClass - An alternative class to instantiate.
    pub use_class: Option<UseClassMetadata<'a>>,
    /// useFactory - A factory function to call.
    pub use_factory: Option<UseFactoryMetadata<'a>>,
    /// useValue - A literal value.
    pub use_value: Option<OutputExpression<'a>>,
    /// useExisting - An alias to another token.
    pub use_existing: Option<UseExistingMetadata<'a>>,
    /// Constructor dependencies for factory generation.
    /// `None` means no constructor (use inherited factory).
    /// `Some(vec)` means constructor exists with these dependencies.
    pub deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
}

/// The providedIn value for an injectable.
#[derive(Debug)]
pub enum ProvidedInValue<'a> {
    /// providedIn: 'root'
    Root,
    /// providedIn: 'platform'
    Platform,
    /// providedIn: 'any'
    Any,
    /// providedIn: null
    None,
    /// providedIn: SomeModule (module reference)
    Module {
        /// The module expression.
        expression: OutputExpression<'a>,
        /// Whether this is wrapped in forwardRef().
        is_forward_ref: bool,
    },
}

/// Metadata for useClass provider.
#[derive(Debug)]
pub struct UseClassMetadata<'a> {
    /// The class expression.
    pub class_expr: OutputExpression<'a>,
    /// Whether this is wrapped in forwardRef().
    pub is_forward_ref: bool,
    /// Dependencies for the class constructor.
    pub deps: Vec<'a, DependencyMetadata<'a>>,
}

/// Metadata for useFactory provider.
#[derive(Debug)]
pub struct UseFactoryMetadata<'a> {
    /// The factory function expression.
    pub factory: OutputExpression<'a>,
    /// Dependencies for the factory function.
    pub deps: Vec<'a, DependencyMetadata<'a>>,
}

/// Metadata for useExisting provider.
#[derive(Debug)]
pub struct UseExistingMetadata<'a> {
    /// The existing token expression.
    pub existing: OutputExpression<'a>,
    /// Whether this is wrapped in forwardRef().
    pub is_forward_ref: bool,
}

/// Dependency metadata for useClass/useFactory deps.
#[derive(Debug)]
pub struct DependencyMetadata<'a> {
    /// The dependency token.
    pub token: OutputExpression<'a>,
    /// Whether the dependency is optional (@Optional).
    pub optional: bool,
    /// Whether to inject from self (@Self).
    pub self_: bool,
    /// Whether to skip self (@SkipSelf).
    pub skip_self: bool,
    /// Whether to inject from host (@Host).
    pub host: bool,
}

impl<'a> InjectableMetadata<'a> {
    /// Convert to R3InjectableMetadata for compilation.
    pub fn to_r3_metadata(
        &self,
        allocator: &'a Allocator,
    ) -> Option<super::metadata::R3InjectableMetadata<'a>> {
        use super::metadata::R3InjectableMetadataBuilder;

        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: self.class_name.clone(), source_span: None },
            allocator,
        ));

        let mut builder =
            R3InjectableMetadataBuilder::new().name(self.class_name.clone()).r#type(type_expr);

        if let Some(provided_in) = &self.provided_in {
            builder = match provided_in {
                ProvidedInValue::Root => builder.provided_in_root(),
                ProvidedInValue::Platform => builder.provided_in_platform(),
                ProvidedInValue::Any => builder.provided_in_any(),
                ProvidedInValue::None => builder,
                ProvidedInValue::Module { expression, .. } => {
                    builder.provided_in_module(expression.clone_in(allocator))
                }
            };
        }

        if let Some(use_value) = &self.use_value {
            builder = builder.use_value(use_value.clone_in(allocator));
        } else if let Some(use_existing) = &self.use_existing {
            builder = builder.use_existing(
                use_existing.existing.clone_in(allocator),
                use_existing.is_forward_ref,
            );
        } else if let Some(use_class) = &self.use_class {
            let deps = if use_class.deps.is_empty() {
                None
            } else {
                Some(convert_deps_to_r3(allocator, &use_class.deps))
            };
            builder = builder.use_class(
                use_class.class_expr.clone_in(allocator),
                use_class.is_forward_ref,
                deps,
            );
        } else if let Some(use_factory) = &self.use_factory {
            let deps = if use_factory.deps.is_empty() {
                None
            } else {
                Some(convert_deps_to_r3(allocator, &use_factory.deps))
            };
            builder = builder.use_factory(use_factory.factory.clone_in(allocator), deps);
        }

        // Pass constructor dependencies for factory generation
        if let Some(deps) = &self.deps {
            let cloned_deps = clone_r3_deps(allocator, deps);
            builder = builder.deps(Some(cloned_deps));
        }

        builder.build()
    }
}

/// Clone R3DependencyMetadata vector
fn clone_r3_deps<'a>(
    allocator: &'a Allocator,
    deps: &[R3DependencyMetadata<'a>],
) -> Vec<'a, R3DependencyMetadata<'a>> {
    let mut result = Vec::with_capacity_in(deps.len(), allocator);
    for dep in deps {
        result.push(R3DependencyMetadata {
            token: dep.token.as_ref().map(|t| t.clone_in(allocator)),
            attribute_name_type: dep.attribute_name_type.as_ref().map(|a| a.clone_in(allocator)),
            host: dep.host,
            optional: dep.optional,
            self_: dep.self_,
            skip_self: dep.skip_self,
        });
    }
    result
}

fn convert_deps_to_r3<'a>(
    allocator: &'a Allocator,
    deps: &[DependencyMetadata<'a>],
) -> oxc_allocator::Vec<'a, crate::factory::R3DependencyMetadata<'a>> {
    let mut result = oxc_allocator::Vec::new_in(allocator);
    for dep in deps {
        result.push(crate::factory::R3DependencyMetadata {
            token: Some(dep.token.clone_in(allocator)),
            attribute_name_type: None,
            host: dep.host,
            optional: dep.optional,
            self_: dep.self_,
            skip_self: dep.skip_self,
        });
    }
    result
}

/// Find the span of the `@Injectable` decorator on a class.
pub fn find_injectable_decorator_span(class: &Class<'_>) -> Option<Span> {
    for decorator in &class.decorators {
        if is_injectable_decorator(decorator) {
            return Some(decorator.span);
        }
    }
    None
}

/// Extract injectable metadata from a class decorated with `@Injectable`.
pub fn extract_injectable_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    source: &'a str,
) -> Option<InjectableMetadata<'a>> {
    let class_name: Atom<'a> = class.id.as_ref()?.name.clone().into();
    let class_span = class.span;

    // Find the @Injectable decorator
    let decorator = class.decorators.iter().find(|d| is_injectable_decorator(d))?;

    // Get the decorator call expression
    let call_expr = match &decorator.expression {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // If no arguments, return basic metadata with default providedIn: 'root'
    if call_expr.arguments.is_empty() {
        return Some(InjectableMetadata {
            class_name,
            class_span,
            provided_in: Some(ProvidedInValue::Root),
            use_class: None,
            use_factory: None,
            use_value: None,
            use_existing: None,
            deps: extract_constructor_deps(allocator, class, source),
        });
    }

    // Get the config object from the first argument
    let config_obj = match &call_expr.arguments[0] {
        Argument::ObjectExpression(obj) => obj,
        _ => return None,
    };

    // Extract providedIn (default to 'root' if not specified)
    let provided_in = extract_provided_in(allocator, config_obj, source).or(Some(ProvidedInValue::Root));

    // Extract useClass
    let use_class = extract_use_class(allocator, config_obj, source);

    // Extract useFactory
    let use_factory = extract_use_factory(allocator, config_obj, source);

    // Extract useValue
    let use_value = extract_use_value(allocator, config_obj, source);

    // Extract useExisting
    let use_existing = extract_use_existing(allocator, config_obj, source);

    // Extract constructor dependencies
    let deps = extract_constructor_deps(allocator, class, source);

    Some(InjectableMetadata {
        class_name,
        class_span,
        provided_in,
        use_class,
        use_factory,
        use_value,
        use_existing,
        deps,
    })
}

fn is_injectable_decorator(decorator: &Decorator<'_>) -> bool {
    match &decorator.expression {
        Expression::CallExpression(call) => match &call.callee {
            Expression::Identifier(id) => id.name == "Injectable",
            _ => false,
        },
        Expression::Identifier(id) => id.name == "Injectable",
        _ => false,
    }
}

fn get_property_key_name<'a>(key: &'a PropertyKey<'a>) -> Option<Atom<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(s) => Some(s.value.clone()),
        _ => None,
    }
}

fn extract_provided_in<'a>(
    allocator: &'a Allocator,
    config_obj: &'a oxc_ast::ast::ObjectExpression<'a>,
    source: &'a str,
) -> Option<ProvidedInValue<'a>> {
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if let Some(key_name) = get_property_key_name(&prop.key) {
                if key_name.as_str() == "providedIn" {
                    return parse_provided_in_value(allocator, &prop.value, source);
                }
            }
        }
    }
    None
}

fn parse_provided_in_value<'a>(
    allocator: &'a Allocator,
    expr: &'a Expression<'a>,
    source: &'a str,
) -> Option<ProvidedInValue<'a>> {
    match expr {
        Expression::StringLiteral(s) => match s.value.as_str() {
            "root" => Some(ProvidedInValue::Root),
            "platform" => Some(ProvidedInValue::Platform),
            "any" => Some(ProvidedInValue::Any),
            _ => None,
        },
        Expression::NullLiteral(_) => Some(ProvidedInValue::None),
        _ => {
            // Check for forwardRef
            let (expression, is_forward_ref) = extract_forward_ref_or_expression(allocator, expr, source)?;
            Some(ProvidedInValue::Module { expression, is_forward_ref })
        }
    }
}

fn extract_use_class<'a>(
    allocator: &'a Allocator,
    config_obj: &'a oxc_ast::ast::ObjectExpression<'a>,
    source: &'a str,
) -> Option<UseClassMetadata<'a>> {
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if let Some(key_name) = get_property_key_name(&prop.key) {
                if key_name.as_str() == "useClass" {
                    let (class_expr, is_forward_ref) =
                        extract_forward_ref_or_expression(allocator, &prop.value, source)?;
                    let deps = extract_deps_from_config(allocator, config_obj, source);
                    return Some(UseClassMetadata { class_expr, is_forward_ref, deps });
                }
            }
        }
    }
    None
}

fn extract_use_factory<'a>(
    allocator: &'a Allocator,
    config_obj: &'a oxc_ast::ast::ObjectExpression<'a>,
    source: &'a str,
) -> Option<UseFactoryMetadata<'a>> {
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if let Some(key_name) = get_property_key_name(&prop.key) {
                if key_name.as_str() == "useFactory" {
                    let factory = convert_oxc_expression(allocator, &prop.value, source)?;
                    let deps = extract_deps_from_config(allocator, config_obj, source);
                    return Some(UseFactoryMetadata { factory, deps });
                }
            }
        }
    }
    None
}

fn extract_use_value<'a>(
    allocator: &'a Allocator,
    config_obj: &'a oxc_ast::ast::ObjectExpression<'a>,
    source: &'a str,
) -> Option<OutputExpression<'a>> {
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if let Some(key_name) = get_property_key_name(&prop.key) {
                if key_name.as_str() == "useValue" {
                    return convert_oxc_expression(allocator, &prop.value, source);
                }
            }
        }
    }
    None
}

fn extract_use_existing<'a>(
    allocator: &'a Allocator,
    config_obj: &'a oxc_ast::ast::ObjectExpression<'a>,
    source: &'a str,
) -> Option<UseExistingMetadata<'a>> {
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if let Some(key_name) = get_property_key_name(&prop.key) {
                if key_name.as_str() == "useExisting" {
                    let (existing, is_forward_ref) =
                        extract_forward_ref_or_expression(allocator, &prop.value, source)?;
                    return Some(UseExistingMetadata { existing, is_forward_ref });
                }
            }
        }
    }
    None
}

/// Extract an expression, handling forwardRef wrapper.
/// Returns (expression, is_forward_ref).
fn extract_forward_ref_or_expression<'a>(
    allocator: &'a Allocator,
    expr: &'a Expression<'a>,
    source: &'a str,
) -> Option<(OutputExpression<'a>, bool)> {
    // Check if this is forwardRef(() => X)
    if let Expression::CallExpression(call) = expr {
        if let Expression::Identifier(id) = &call.callee {
            if id.name == "forwardRef" {
                if let Some(Argument::ArrowFunctionExpression(arrow)) = call.arguments.first() {
                    // Get the body expression - arrow functions with expression bodies
                    // store the expression as a single ExpressionStatement in body.statements
                    if arrow.expression {
                        if let Some(oxc_ast::ast::Statement::ExpressionStatement(expr_stmt)) =
                            arrow.body.statements.first()
                        {
                            if let Some(expression) =
                                convert_oxc_expression(allocator, &expr_stmt.expression, source)
                            {
                                return Some((expression, true));
                            }
                        }
                    }
                }
            }
        }
    }
    convert_oxc_expression(allocator, expr, source).map(|e| (e, false))
}

fn extract_deps_from_config<'a>(
    allocator: &'a Allocator,
    config_obj: &'a oxc_ast::ast::ObjectExpression<'a>,
    source: &'a str,
) -> Vec<'a, DependencyMetadata<'a>> {
    let mut deps = Vec::new_in(allocator);

    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if let Some(key_name) = get_property_key_name(&prop.key) {
                if key_name.as_str() == "deps" {
                    if let Expression::ArrayExpression(arr) = &prop.value {
                        for element in &arr.elements {
                            if let Some(dep) = extract_dependency(allocator, element, source) {
                                deps.push(dep);
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    deps
}

fn extract_dependency<'a>(
    allocator: &'a Allocator,
    element: &'a ArrayExpressionElement<'a>,
    source: &'a str,
) -> Option<DependencyMetadata<'a>> {
    match element {
        ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_) => None,
        _ => {
            let expr = element.to_expression();
            convert_oxc_expression(allocator, expr, source).map(|t| DependencyMetadata {
                token: t,
                optional: false,
                self_: false,
                skip_self: false,
                host: false,
            })
        }
    }
}

/// Extract constructor dependencies from a class.
///
/// Returns:
/// - `None` if the class has no constructor (use inherited factory pattern)
/// - `Some(vec)` with the dependencies if constructor exists
///
/// Handles parameter decorators like `@Inject()`, `@Optional()`, `@SkipSelf()`, `@Self()`, `@Host()`.
///
/// Example:
/// ```typescript
/// @Injectable()
/// class InitService {
///   constructor(
///     @Inject(WINDOW) private win: Window,
///     private sdkLoadService: SdkLoadService,
///     @Inject(DOCUMENT) private document: Document,
///   ) {}
/// }
/// ```
/// Returns: Some([
///   R3DependencyMetadata { token: WINDOW, ... },
///   R3DependencyMetadata { token: SdkLoadService, ... },
///   R3DependencyMetadata { token: DOCUMENT, ... },
/// ])
pub fn extract_constructor_deps<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
    source: &'a str,
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
        let dep = extract_param_dependency(allocator, param, source);
        deps.push(dep);
    }

    Some(deps)
}

/// Extract dependency metadata from a single constructor parameter.
fn extract_param_dependency<'a>(
    allocator: &'a Allocator,
    param: &oxc_ast::ast::FormalParameter<'a>,
    source: &'a str,
) -> R3DependencyMetadata<'a> {
    // Extract flags and @Inject token from decorators
    let mut optional = false;
    let mut skip_self = false;
    let mut self_ = false;
    let mut host = false;
    let mut inject_token: Option<OutputExpression<'a>> = None;
    let mut attribute_name: Option<Atom<'a>> = None;

    for decorator in &param.decorators {
        if let Some(name) = get_decorator_name(&decorator.expression) {
            match name.as_str() {
                "Inject" => {
                    // @Inject(TOKEN) - extract the token
                    if let Expression::CallExpression(call) = &decorator.expression {
                        if let Some(arg) = call.arguments.first() {
                            inject_token = convert_oxc_expression(allocator, arg.to_expression(), source);
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
                            attribute_name = Some(s.value.clone());
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
fn get_decorator_name<'a>(expr: &'a Expression<'a>) -> Option<Atom<'a>> {
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
        let type_name: Atom<'a> = match &type_ref.type_name {
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn parse_and_extract<'a>(
        allocator: &'a Allocator,
        source: &'a str,
    ) -> Option<InjectableMetadata<'a>> {
        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(allocator, source, source_type).parse();
        let program = allocator.alloc(parser_ret.program);

        for stmt in &program.body {
            if let oxc_ast::ast::Statement::ClassDeclaration(class) = stmt {
                return extract_injectable_metadata(allocator, class, source);
            }
        }
        None
    }

    #[test]
    fn test_extract_basic_injectable() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert_eq!(metadata.class_name.as_str(), "MyService");
        // Default to 'root' when not specified (Angular's default behavior)
        assert!(matches!(metadata.provided_in, Some(ProvidedInValue::Root)));
    }

    #[test]
    fn test_extract_injectable_with_root_provided_in() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'root' })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(matches!(metadata.provided_in, Some(ProvidedInValue::Root)));
    }

    #[test]
    fn test_extract_injectable_with_platform_provided_in() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'platform' })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(matches!(metadata.provided_in, Some(ProvidedInValue::Platform)));
    }

    #[test]
    fn test_extract_injectable_with_empty_object() {
        // @Injectable({}) should default to providedIn: 'root'
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({})
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert_eq!(metadata.class_name.as_str(), "MyService");
        // Default to 'root' when providedIn is not specified in the object
        assert!(matches!(metadata.provided_in, Some(ProvidedInValue::Root)));
    }

    #[test]
    fn test_extract_injectable_with_null_provided_in() {
        // @Injectable({ providedIn: null }) should result in ProvidedInValue::None
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: null })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert_eq!(metadata.class_name.as_str(), "MyService");
        // Explicit null means no providedIn
        assert!(matches!(metadata.provided_in, Some(ProvidedInValue::None)));
    }

    #[test]
    fn test_extract_injectable_with_use_value() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'root', useValue: 42 })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(metadata.use_value.is_some());
    }

    #[test]
    fn test_extract_injectable_with_use_factory() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'root', useFactory: () => new MyService() })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(metadata.use_factory.is_some());
    }

    #[test]
    fn test_extract_injectable_with_use_class() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'root', useClass: OtherService })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(metadata.use_class.is_some());
        let use_class = metadata.use_class.unwrap();
        assert!(!use_class.is_forward_ref);
    }

    #[test]
    fn test_extract_injectable_with_use_existing() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'root', useExisting: OtherService })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(metadata.use_existing.is_some());
        let use_existing = metadata.use_existing.unwrap();
        assert!(!use_existing.is_forward_ref);
    }

    #[test]
    fn test_extract_injectable_with_forward_ref() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable({ providedIn: 'root', useClass: forwardRef(() => OtherService) })
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert!(metadata.use_class.is_some());
        let use_class = metadata.use_class.unwrap();
        assert!(use_class.is_forward_ref);
    }

    // =========================================================================
    // Constructor dependency extraction tests
    // =========================================================================

    #[test]
    fn test_injectable_without_constructor_has_no_deps() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class MyService {}
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        // No constructor = None deps (use inherited factory)
        assert!(metadata.deps.is_none());
    }

    #[test]
    fn test_injectable_with_simple_constructor_dep() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class MyService {
                constructor(private otherService: OtherService) {}
            }
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();

        // Should have deps
        let deps = metadata.deps.as_ref().expect("Should have constructor deps");
        assert_eq!(deps.len(), 1);

        // Check the dependency token is OtherService
        assert!(deps[0].token.is_some());
        assert!(!deps[0].optional);
        assert!(!deps[0].skip_self);
        assert!(!deps[0].self_);
        assert!(!deps[0].host);
    }

    #[test]
    fn test_injectable_with_inject_decorator() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class InitService {
                constructor(
                    @Inject(WINDOW) private win: Window,
                    private sdkLoadService: SdkLoadService,
                    @Inject(DOCUMENT) private document: Document,
                ) {}
            }
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();

        // Should have 3 deps
        let deps = metadata.deps.as_ref().expect("Should have constructor deps");
        assert_eq!(deps.len(), 3);

        // First dep: @Inject(WINDOW) - token should be WINDOW, not Window
        assert!(deps[0].token.is_some(), "First dep should have token");

        // Second dep: no @Inject - token should be SdkLoadService
        assert!(deps[1].token.is_some(), "Second dep should have token");

        // Third dep: @Inject(DOCUMENT) - token should be DOCUMENT, not Document
        assert!(deps[2].token.is_some(), "Third dep should have token");
    }

    #[test]
    fn test_injectable_with_optional_and_skip_self() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class CoreModule {
                constructor(@Optional() @SkipSelf() parentModule?: CoreModule) {}
            }
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();

        // Should have deps
        let deps = metadata.deps.as_ref().expect("Should have constructor deps");
        assert_eq!(deps.len(), 1);

        // Check flags
        let dep = &deps[0];
        assert!(dep.optional, "Should be optional");
        assert!(dep.skip_self, "Should have skip_self");
        assert!(!dep.self_, "Should not have self_");
        assert!(!dep.host, "Should not have host");
        assert!(dep.token.is_some(), "Should have token");
    }

    #[test]
    fn test_injectable_with_self_and_host() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class MyService {
                constructor(
                    @Self() private selfService: SelfService,
                    @Host() private hostService: HostService
                ) {}
            }
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();

        let deps = metadata.deps.as_ref().expect("Should have constructor deps");
        assert_eq!(deps.len(), 2);

        // First dep: @Self()
        assert!(deps[0].self_, "First dep should have self_");
        assert!(!deps[0].host, "First dep should not have host");

        // Second dep: @Host()
        assert!(!deps[1].self_, "Second dep should not have self_");
        assert!(deps[1].host, "Second dep should have host");
    }

    #[test]
    fn test_injectable_with_combined_decorators() {
        let allocator = Allocator::default();
        let source = r#"
            @Injectable()
            class MyService {
                constructor(
                    @Optional() @Inject(TOKEN) private service: SomeService
                ) {}
            }
        "#;

        let metadata = parse_and_extract(&allocator, source);
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();

        let deps = metadata.deps.as_ref().expect("Should have constructor deps");
        assert_eq!(deps.len(), 1);

        // Should have optional flag and token from @Inject
        assert!(deps[0].optional, "Should be optional");
        assert!(deps[0].token.is_some(), "Should have token from @Inject");
    }
}

//! Angular `@NgModule` decorator parser.
//!
//! This module extracts metadata from `@NgModule({...})` decorators
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

/// Extracted NgModule metadata from a `@NgModule` decorator.
///
/// This struct holds the raw metadata extracted from the decorator
/// for use by build tools and analysis.
#[derive(Debug)]
pub struct NgModuleMetadata<'a> {
    /// The name of the NgModule class.
    pub class_name: Atom<'a>,

    /// Span of the class declaration.
    pub class_span: Span,

    /// Declared components, directives, and pipes as class names.
    pub declarations: Vec<'a, Atom<'a>>,

    /// Imported modules as class names (for ɵmod scope resolution).
    pub imports: Vec<'a, Atom<'a>>,

    /// Raw imports array expression (for ɵinj provider resolution).
    /// This preserves call expressions like `StoreModule.forRoot(...)` and spread elements
    /// that are needed by the injector to resolve `ModuleWithProviders`.
    pub raw_imports_expr: Option<OutputExpression<'a>>,

    /// Exported declarations and modules as class names.
    pub exports: Vec<'a, Atom<'a>>,

    /// Providers expression as OutputExpression.
    pub providers: Option<OutputExpression<'a>>,

    /// Bootstrap components as class names.
    pub bootstrap: Vec<'a, Atom<'a>>,

    /// Schema identifiers (e.g., "CUSTOM_ELEMENTS_SCHEMA").
    pub schemas: Vec<'a, Atom<'a>>,

    /// Module ID for registration.
    pub id: Option<Atom<'a>>,

    /// Whether any declarations/imports/exports contain forward references.
    pub contains_forward_decls: bool,

    /// Constructor dependencies for factory generation.
    /// `None` means no constructor (use inherited factory).
    /// `Some(vec)` means constructor exists with these dependencies.
    pub deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
}

impl<'a> NgModuleMetadata<'a> {
    /// Create a new NgModuleMetadata with defaults.
    pub fn new(allocator: &'a Allocator, class_name: Atom<'a>, class_span: Span) -> Self {
        Self {
            class_name,
            class_span,
            declarations: Vec::new_in(allocator),
            imports: Vec::new_in(allocator),
            raw_imports_expr: None,
            exports: Vec::new_in(allocator),
            providers: None,
            bootstrap: Vec::new_in(allocator),
            schemas: Vec::new_in(allocator),
            id: None,
            contains_forward_decls: false,
            deps: None,
        }
    }

    /// Convert to R3NgModuleMetadata for compilation.
    pub fn to_r3_metadata(
        &self,
        allocator: &'a Allocator,
    ) -> Option<super::metadata::R3NgModuleMetadata<'a>> {
        use super::metadata::{R3NgModuleMetadataBuilder, R3Reference, R3SelectorScopeMode};

        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: self.class_name.clone(), source_span: None },
            allocator,
        ));

        let mut builder = R3NgModuleMetadataBuilder::new(allocator)
            .r#type(R3Reference::value_only(type_expr))
            .selector_scope_mode(R3SelectorScopeMode::Inline)
            .contains_forward_decls(self.contains_forward_decls);

        // Add declarations
        for decl in &self.declarations {
            let decl_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: decl.clone(), source_span: None },
                allocator,
            ));
            builder = builder.add_declaration(R3Reference::value_only(decl_expr));
        }

        // Add imports
        for import in &self.imports {
            let import_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: import.clone(), source_span: None },
                allocator,
            ));
            builder = builder.add_import(R3Reference::value_only(import_expr));
        }

        // Add exports
        for export in &self.exports {
            let export_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: export.clone(), source_span: None },
                allocator,
            ));
            builder = builder.add_export(R3Reference::value_only(export_expr));
        }

        // Add bootstrap components
        for bootstrap in &self.bootstrap {
            let bootstrap_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: bootstrap.clone(), source_span: None },
                allocator,
            ));
            builder = builder.add_bootstrap(R3Reference::value_only(bootstrap_expr));
        }

        // Add schemas
        for schema in &self.schemas {
            let schema_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: schema.clone(), source_span: None },
                allocator,
            ));
            builder = builder.add_schema(R3Reference::value_only(schema_expr));
        }

        // Set module ID if present
        if let Some(id) = &self.id {
            let id_expr = OutputExpression::Literal(Box::new_in(
                crate::output::ast::LiteralExpr {
                    value: crate::output::ast::LiteralValue::String(id.clone()),
                    source_span: None,
                },
                allocator,
            ));
            builder = builder.id(id_expr);
        }

        builder.build()
    }
}

/// Extract NgModule metadata from a class with decorators.
///
/// Searches for a `@NgModule({...})` decorator and parses its properties.
/// Returns `None` if no `@NgModule` decorator is found.
///
/// # Example
///
/// ```typescript
/// @NgModule({
///   declarations: [AppComponent, MyDirective],
///   imports: [CommonModule, RouterModule],
///   exports: [AppComponent],
///   providers: [MyService],
///   bootstrap: [AppComponent],
///   schemas: [CUSTOM_ELEMENTS_SCHEMA],
///   id: 'my-module-id',
/// })
/// export class AppModule {}
/// ```
pub fn extract_ng_module_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Option<NgModuleMetadata<'a>> {
    // Get the class name
    let class_name: Atom<'a> = class.id.as_ref()?.name.clone().into();
    let class_span = class.span;

    // Find the @NgModule decorator
    let ng_module_decorator = find_ng_module_decorator(&class.decorators)?;

    // Get the decorator call arguments
    let call_expr = match &ng_module_decorator.expression {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Verify it's calling 'NgModule'
    if !is_ng_module_call(&call_expr.callee) {
        return None;
    }

    // Get the first argument (the config object)
    let config_arg = call_expr.arguments.first()?;
    let config_obj = match config_arg {
        Argument::ObjectExpression(obj) => obj,
        _ => return None,
    };

    // Create metadata with defaults
    let mut metadata = NgModuleMetadata::new(allocator, class_name, class_span);

    // Parse each property in the config object
    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            let Some(key_name) = get_property_key_name(&prop.key) else {
                continue;
            };

            match key_name.as_str() {
                "declarations" => {
                    let (identifiers, has_forward_refs) =
                        extract_reference_array(allocator, &prop.value);
                    metadata.declarations = identifiers;
                    if has_forward_refs {
                        metadata.contains_forward_decls = true;
                    }
                }
                "imports" => {
                    let (identifiers, has_forward_refs) =
                        extract_reference_array(allocator, &prop.value);
                    metadata.imports = identifiers;
                    if has_forward_refs {
                        metadata.contains_forward_decls = true;
                    }
                    // Also store the raw imports expression for ɵinj generation.
                    // This preserves call expressions like StoreModule.forRoot(...)
                    // and spread elements that are dropped by extract_reference_array.
                    metadata.raw_imports_expr = convert_oxc_expression(allocator, &prop.value);
                }
                "exports" => {
                    let (identifiers, has_forward_refs) =
                        extract_reference_array(allocator, &prop.value);
                    metadata.exports = identifiers;
                    if has_forward_refs {
                        metadata.contains_forward_decls = true;
                    }
                }
                "providers" => {
                    metadata.providers = convert_oxc_expression(allocator, &prop.value);
                }
                "bootstrap" => {
                    let (identifiers, has_forward_refs) =
                        extract_reference_array(allocator, &prop.value);
                    metadata.bootstrap = identifiers;
                    if has_forward_refs {
                        metadata.contains_forward_decls = true;
                    }
                }
                "schemas" => {
                    metadata.schemas = extract_identifier_array(allocator, &prop.value);
                }
                "id" => {
                    metadata.id = extract_string_value(&prop.value);
                }
                _ => {
                    // Unknown property - ignore (e.g., "jit")
                }
            }
        }
    }

    // Extract constructor dependencies
    metadata.deps = extract_constructor_deps(allocator, class);

    Some(metadata)
}

/// Find the @NgModule decorator in a list of decorators.
fn find_ng_module_decorator<'a>(decorators: &'a [Decorator<'a>]) -> Option<&'a Decorator<'a>> {
    decorators.iter().find(|d| match &d.expression {
        Expression::CallExpression(call) => is_ng_module_call(&call.callee),
        Expression::Identifier(id) => id.name == "NgModule",
        _ => false,
    })
}

/// Find the span of the @NgModule decorator on a class.
///
/// Returns the span including any leading whitespace/newlines that should be removed
/// along with the decorator.
pub fn find_ng_module_decorator_span(class: &Class<'_>) -> Option<Span> {
    find_ng_module_decorator(&class.decorators).map(|d| d.span)
}

/// Check if a callee expression is a call to 'NgModule'.
fn is_ng_module_call(callee: &Expression<'_>) -> bool {
    match callee {
        Expression::Identifier(id) => id.name == "NgModule",
        // Handle namespaced imports like ng.NgModule or core.NgModule
        Expression::StaticMemberExpression(member) => {
            matches!(&member.property.name.as_str(), &"NgModule")
        }
        _ => false,
    }
}

/// Get the name of a property key as a string.
fn get_property_key_name<'a>(key: &PropertyKey<'a>) -> Option<Atom<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(lit) => Some(lit.value.clone()),
        _ => None,
    }
}

/// Extract a string value from an expression.
fn extract_string_value<'a>(expr: &Expression<'a>) -> Option<Atom<'a>> {
    match expr {
        Expression::StringLiteral(lit) => Some(lit.value.clone()),
        Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
            tpl.quasis.first().and_then(|q| q.value.cooked.clone())
        }
        _ => None,
    }
}

/// Extract an array of identifiers (for declarations, imports, exports, bootstrap).
/// Returns the identifiers and whether any forward references were found.
fn extract_reference_array<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> (Vec<'a, Atom<'a>>, bool) {
    let mut result = Vec::new_in(allocator);
    let mut has_forward_refs = false;

    let Expression::ArrayExpression(arr) = expr else {
        return (result, has_forward_refs);
    };

    for element in &arr.elements {
        match element {
            // Simple identifier: [SomeComponent]
            ArrayExpressionElement::Identifier(id) => {
                result.push(id.name.clone().into());
            }
            // Forward reference: forwardRef(() => SomeComponent)
            // Or method call: StoreModule.forRoot(...), EffectsModule.forRoot([...])
            ArrayExpressionElement::CallExpression(call) => {
                if let Expression::Identifier(id) = &call.callee {
                    if id.name == "forwardRef" {
                        has_forward_refs = true;
                        if let Some(Argument::ArrowFunctionExpression(arrow)) =
                            call.arguments.first()
                        {
                            if let Some(Expression::Identifier(inner_id)) = arrow.get_expression() {
                                result.push(inner_id.name.clone().into());
                            }
                        }
                    }
                } else if let Expression::StaticMemberExpression(member) = &call.callee {
                    // Module.forRoot(...) or Module.forChild(...) pattern
                    // Extract the base class identifier for ɵmod scope resolution
                    if let Expression::Identifier(id) = &member.object {
                        result.push(id.name.clone().into());
                    }
                }
            }
            // Spread element or other complex expressions - skip
            _ => {}
        }
    }

    (result, has_forward_refs)
}

/// Extract an array of simple identifiers (for schemas).
fn extract_identifier_array<'a>(
    allocator: &'a Allocator,
    expr: &Expression<'a>,
) -> Vec<'a, Atom<'a>> {
    let mut result = Vec::new_in(allocator);

    let Expression::ArrayExpression(arr) = expr else {
        return result;
    };

    for element in &arr.elements {
        if let ArrayExpressionElement::Identifier(id) = element {
            result.push(id.name.clone().into());
        }
    }

    result
}

/// Extract constructor dependencies from a class.
///
/// Returns:
/// - `None` if the class has no constructor (use inherited factory pattern)
/// - `Some(vec)` with the dependencies if constructor exists
///
/// Handles parameter decorators like `@Optional()`, `@SkipSelf()`, `@Self()`, `@Host()`.
///
/// Example:
/// ```typescript
/// class CoreModule {
///   constructor(@Optional() @SkipSelf() parentModule?: CoreModule) {}
/// }
/// ```
/// Returns: Some([R3DependencyMetadata { token: CoreModule, optional: true, skip_self: true }])
pub fn extract_constructor_deps<'a>(
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
    let mut attribute_name: Option<Atom<'a>> = None;

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
                            attribute_name = Some(s.value.clone());
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
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    /// Helper function to parse TypeScript code and extract NgModule metadata
    /// from the first @NgModule decorated class found.
    fn with_extracted_metadata<F>(code: &str, callback: F)
    where
        F: FnOnce(Option<&NgModuleMetadata<'_>>),
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
                if let Some(metadata) = extract_ng_module_metadata(&allocator, class) {
                    found_metadata = Some(metadata);
                    break;
                }
            }
        }

        callback(found_metadata.as_ref());
    }

    /// Shorthand for tests that expect metadata to be found.
    fn assert_metadata<F>(code: &str, callback: F)
    where
        F: FnOnce(&NgModuleMetadata<'_>),
    {
        with_extracted_metadata(code, |meta| {
            let meta = meta.expect("Expected to find @NgModule metadata");
            callback(meta);
        });
    }

    /// Shorthand for tests that expect no metadata to be found.
    fn assert_no_metadata(code: &str) {
        with_extracted_metadata(code, |meta| {
            assert!(meta.is_none(), "Expected no @NgModule metadata to be found");
        });
    }

    // =========================================================================
    // Basic extraction tests
    // =========================================================================

    #[test]
    fn test_extract_class_name() {
        let code = r#"
            @NgModule({})
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "AppModule");
        });
    }

    #[test]
    fn test_extract_declarations() {
        let code = r#"
            @NgModule({
                declarations: [AppComponent, MyDirective, MyPipe]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.declarations.len(), 3);
            assert_eq!(meta.declarations[0].as_str(), "AppComponent");
            assert_eq!(meta.declarations[1].as_str(), "MyDirective");
            assert_eq!(meta.declarations[2].as_str(), "MyPipe");
        });
    }

    #[test]
    fn test_extract_imports() {
        let code = r#"
            @NgModule({
                imports: [CommonModule, RouterModule, FormsModule]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.imports.len(), 3);
            assert_eq!(meta.imports[0].as_str(), "CommonModule");
            assert_eq!(meta.imports[1].as_str(), "RouterModule");
            assert_eq!(meta.imports[2].as_str(), "FormsModule");
        });
    }

    #[test]
    fn test_extract_exports() {
        let code = r#"
            @NgModule({
                exports: [SharedComponent, SharedDirective]
            })
            class SharedModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.exports.len(), 2);
            assert_eq!(meta.exports[0].as_str(), "SharedComponent");
            assert_eq!(meta.exports[1].as_str(), "SharedDirective");
        });
    }

    #[test]
    fn test_extract_bootstrap() {
        let code = r#"
            @NgModule({
                bootstrap: [AppComponent]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.bootstrap.len(), 1);
            assert_eq!(meta.bootstrap[0].as_str(), "AppComponent");
        });
    }

    #[test]
    fn test_extract_schemas() {
        let code = r#"
            @NgModule({
                schemas: [CUSTOM_ELEMENTS_SCHEMA, NO_ERRORS_SCHEMA]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.schemas.len(), 2);
            assert_eq!(meta.schemas[0].as_str(), "CUSTOM_ELEMENTS_SCHEMA");
            assert_eq!(meta.schemas[1].as_str(), "NO_ERRORS_SCHEMA");
        });
    }

    #[test]
    fn test_extract_id() {
        let code = r#"
            @NgModule({
                id: 'my-unique-module-id'
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.id.as_ref().map(|s| s.as_str()), Some("my-unique-module-id"));
        });
    }

    #[test]
    fn test_extract_providers() {
        let code = r#"
            @NgModule({
                providers: [MyService, { provide: TOKEN, useClass: MyImpl }]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert!(meta.providers.is_some());
        });
    }

    // =========================================================================
    // Forward reference tests
    // =========================================================================

    #[test]
    fn test_forward_ref_in_declarations() {
        let code = r#"
            @NgModule({
                declarations: [forwardRef(() => MyComponent)]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.declarations.len(), 1);
            assert_eq!(meta.declarations[0].as_str(), "MyComponent");
            assert!(meta.contains_forward_decls);
        });
    }

    #[test]
    fn test_forward_ref_in_imports() {
        let code = r#"
            @NgModule({
                imports: [CommonModule, forwardRef(() => LazyModule)]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.imports.len(), 2);
            assert_eq!(meta.imports[0].as_str(), "CommonModule");
            assert_eq!(meta.imports[1].as_str(), "LazyModule");
            assert!(meta.contains_forward_decls);
        });
    }

    #[test]
    fn test_no_forward_refs() {
        let code = r#"
            @NgModule({
                declarations: [AppComponent],
                imports: [CommonModule]
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert!(!meta.contains_forward_decls);
        });
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_no_ng_module_decorator_returns_none() {
        let code = r#"
            class PlainClass {}
        "#;
        assert_no_metadata(code);
    }

    #[test]
    fn test_ng_module_decorator_without_call_returns_none() {
        let code = r#"
            @NgModule
            class AppModule {}
        "#;
        assert_no_metadata(code);
    }

    #[test]
    fn test_empty_ng_module_decorator() {
        let code = r#"
            @NgModule({})
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "AppModule");
            assert!(meta.declarations.is_empty());
            assert!(meta.imports.is_empty());
            assert!(meta.exports.is_empty());
            assert!(meta.bootstrap.is_empty());
            assert!(meta.schemas.is_empty());
            assert!(meta.id.is_none());
        });
    }

    #[test]
    fn test_exported_class() {
        let code = r#"
            @NgModule({ declarations: [AppComponent] })
            export class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.declarations.len(), 1);
        });
    }

    #[test]
    fn test_namespaced_ng_module_decorator() {
        let code = r#"
            @ng.NgModule({ declarations: [AppComponent] })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.declarations.len(), 1);
        });
    }

    #[test]
    fn test_full_ng_module_decorator() {
        let code = r#"
            @NgModule({
                declarations: [AppComponent, HeaderComponent],
                imports: [BrowserModule, RouterModule],
                exports: [HeaderComponent],
                providers: [AppService],
                bootstrap: [AppComponent],
                schemas: [CUSTOM_ELEMENTS_SCHEMA],
                id: 'app-module'
            })
            class AppModule {}
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "AppModule");
            assert_eq!(meta.declarations.len(), 2);
            assert_eq!(meta.imports.len(), 2);
            assert_eq!(meta.exports.len(), 1);
            assert!(meta.providers.is_some());
            assert_eq!(meta.bootstrap.len(), 1);
            assert_eq!(meta.schemas.len(), 1);
            assert_eq!(meta.id.as_ref().map(|s| s.as_str()), Some("app-module"));
        });
    }

    // =========================================================================
    // Constructor dependency tests
    // =========================================================================

    #[test]
    fn test_constructor_with_optional_typed_param() {
        let code = r#"
            @NgModule({})
            class CoreModule {
                constructor(@Optional() @SkipSelf() parentModule?: CoreModule) {}
            }
        "#;
        assert_metadata(code, |meta| {
            assert_eq!(meta.class_name.as_str(), "CoreModule");

            // Should have deps because constructor exists
            let deps = meta.deps.as_ref().expect("Should have constructor deps");
            assert_eq!(deps.len(), 1);

            // Check the dependency
            let dep = &deps[0];
            assert!(dep.optional, "Should be marked as optional");
            assert!(dep.skip_self, "Should be marked as skip_self");
            assert!(!dep.self_, "Should not be marked as self_");
            assert!(!dep.host, "Should not be marked as host");

            // Most importantly, token should be Some (the CoreModule type)
            assert!(dep.token.is_some(), "Token should be extracted from type annotation");
        });
    }

    #[test]
    fn test_constructor_with_simple_typed_param() {
        let code = r#"
            @NgModule({})
            class AppModule {
                constructor(private service: SomeService) {}
            }
        "#;
        assert_metadata(code, |meta| {
            let deps = meta.deps.as_ref().expect("Should have constructor deps");
            assert_eq!(deps.len(), 1);

            let dep = &deps[0];
            assert!(dep.token.is_some(), "Token should be extracted from type annotation");
        });
    }
}

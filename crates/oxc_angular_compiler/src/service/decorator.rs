//! Angular `@Service` decorator parser (v22+).
//!
//! Extracts metadata from `@Service({...})` decorators on TypeScript class
//! declarations. Ported from `packages/compiler-cli/src/ngtsc/annotations/src/service.ts`.
//!
//! The caller is responsible for confirming that the decorator's local
//! identifier resolves to `Service` in `@angular/core` (via the file's
//! import map) before invoking `extract_service_metadata`. The extractor
//! takes the already-resolved `Decorator` reference rather than searching
//! the class by literal name, so aliased imports like
//! `import { Service as NgService }; @NgService()` flow through correctly.
//! See `component/transform.rs::find_angular_service_decorator` for the
//! import-map gate the AOT caller uses.

use oxc_allocator::{Allocator, Box};
use oxc_ast::ast::{Argument, Class, Decorator, Expression, ObjectPropertyKind, PropertyKey};
use oxc_span::{GetSpan, Span};
use oxc_str::Ident;

use super::metadata::R3ServiceMetadata;
use crate::output::ast::{OutputExpression, ReadVarExpr};

/// Extracted metadata from a `@Service` decorator.
#[derive(Debug)]
pub struct ServiceMetadata<'a> {
    /// The name of the service class.
    pub class_name: Ident<'a>,
    /// Span of the class declaration.
    pub class_span: Span,
    /// `autoProvided: false` flag, if explicitly disabled by the user.
    pub auto_provided: Option<bool>,
    /// User-supplied `factory: ...` expression, captured as raw source text.
    pub factory: Option<&'a str>,
}

/// Find the `@Service` decorator node on a class (by identifier name only).
pub fn find_service_decorator<'a>(decorators: &'a [Decorator<'a>]) -> Option<&'a Decorator<'a>> {
    decorators.iter().find(|d| is_service_decorator(d))
}

/// Find the span of the `@Service` decorator on a class.
pub fn find_service_decorator_span(class: &Class<'_>) -> Option<Span> {
    class.decorators.iter().find(|d| is_service_decorator(d)).map(|d| d.span)
}

/// Extract `@Service` metadata from a class given the already-resolved
/// decorator node.
///
/// The caller (the AOT dispatcher) has already verified via the file's import
/// map that the decorator's local identifier resolves to `Service` in
/// `@angular/core`. Accepting the decorator by reference avoids re-searching
/// the class's decorators by literal name — which would skip aliased imports
/// such as `import { Service as NgService }; @NgService()`.
pub fn extract_service_metadata<'a>(
    _allocator: &'a Allocator,
    class: &'a Class<'a>,
    decorator: &'a Decorator<'a>,
    source_text: Option<&'a str>,
) -> Option<ServiceMetadata<'a>> {
    let class_name: Ident<'a> = class.id.as_ref()?.name.clone().into();
    let class_span = class.span;

    let call_expr = match &decorator.expression {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    if call_expr.arguments.is_empty() {
        return Some(ServiceMetadata {
            class_name,
            class_span,
            auto_provided: None,
            factory: None,
        });
    }

    let config_obj = match &call_expr.arguments[0] {
        Argument::ObjectExpression(obj) => obj,
        _ => return None,
    };

    let mut auto_provided: Option<bool> = None;
    let mut factory: Option<&'a str> = None;

    for prop in &config_obj.properties {
        let ObjectPropertyKind::ObjectProperty(prop) = prop else { continue };
        let Some(key) = get_property_key_name(&prop.key) else { continue };

        match key.as_str() {
            "autoProvided" => {
                if let Expression::BooleanLiteral(bool_lit) = &prop.value {
                    auto_provided = Some(bool_lit.value);
                }
            }
            "factory" => {
                if let Some(src) = source_text {
                    let span = prop.value.span();
                    factory = Some(&src[span.start as usize..span.end as usize]);
                }
            }
            _ => {}
        }
    }

    Some(ServiceMetadata { class_name, class_span, auto_provided, factory })
}

impl<'a> ServiceMetadata<'a> {
    /// Convert to `R3ServiceMetadata` for compilation.
    ///
    /// The factory expression captured from the decorator source text is
    /// re-parsed by the converter so that it lands in the compiler's
    /// OutputExpression form. Falls back to `None` when re-parsing fails.
    pub fn to_r3_metadata(
        &self,
        allocator: &'a Allocator,
        type_argument_count: u32,
    ) -> R3ServiceMetadata<'a> {
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: self.class_name.clone(), source_span: None },
            allocator,
        ));

        let factory = self.factory.and_then(|src| parse_factory_expression(allocator, src));

        R3ServiceMetadata {
            name: self.class_name.clone(),
            r#type: type_expr,
            type_argument_count,
            auto_provided: self.auto_provided,
            factory,
        }
    }
}

fn is_service_decorator(decorator: &Decorator<'_>) -> bool {
    match &decorator.expression {
        Expression::CallExpression(call) => match &call.callee {
            Expression::Identifier(id) => id.name == "Service",
            _ => false,
        },
        Expression::Identifier(id) => id.name == "Service",
        _ => false,
    }
}

fn get_property_key_name<'a>(key: &'a PropertyKey<'a>) -> Option<Ident<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(s) => Some(s.value.clone().into()),
        _ => None,
    }
}

/// Re-parse a user-supplied factory expression so it can flow back into the
/// compiler's output AST. Returns `None` if the source text doesn't parse as
/// a single expression.
fn parse_factory_expression<'a>(
    allocator: &'a Allocator,
    src: &'a str,
) -> Option<OutputExpression<'a>> {
    use crate::output::oxc_converter::convert_oxc_expression;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    // Wrap the expression so the parser treats it as a standalone module.
    let wrapped = allocator.alloc_str(&format!("({});", src));
    let parser_ret = Parser::new(allocator, wrapped, SourceType::ts()).parse();
    if !parser_ret.diagnostics.is_empty() {
        return None;
    }
    let stmt = parser_ret.program.body.first()?;
    let oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) = stmt else {
        return None;
    };
    // Unwrap the parenthesized expression.
    let expr = match &expr_stmt.expression {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    convert_oxc_expression(allocator, expr, Some(wrapped))
}

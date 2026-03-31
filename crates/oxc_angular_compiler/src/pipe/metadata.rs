//! Pipe metadata structures.
//!
//! Ported from Angular's `render3/r3_pipe_compiler.ts`.

use oxc_allocator::{Box, Vec};
use oxc_span::Ident;

use crate::output::ast::OutputExpression;

/// Metadata used to compile a pipe.
///
/// Ported from Angular's `R3PipeMetadata` interface.
#[derive(Debug)]
pub struct R3PipeMetadata<'a> {
    /// The TypeScript class name of the pipe.
    pub name: Ident<'a>,

    /// The actual pipe name used in templates (from `@Pipe({name: '...'})`).
    /// If None, the class name is used.
    pub pipe_name: Option<Ident<'a>>,

    /// Reference to the pipe class itself.
    pub r#type: OutputExpression<'a>,

    /// Number of generic type parameters.
    pub type_argument_count: u32,

    /// Constructor dependencies for dependency injection.
    /// None means use default factory (no-arg constructor).
    pub deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,

    /// Whether the pipe is pure (default: true).
    /// Pure pipes only transform when inputs change.
    pub pure: bool,

    /// Whether the pipe is standalone (Angular 14+).
    /// Standalone pipes can be imported directly without NgModule.
    pub is_standalone: bool,
}

/// Metadata for a single dependency.
///
/// Ported from Angular's `R3DependencyMetadata` interface.
#[derive(Debug)]
pub struct R3DependencyMetadata<'a> {
    /// The injectable token/type.
    /// None indicates an invalid dependency.
    pub token: Option<Box<'a, OutputExpression<'a>>>,

    /// For `@Attribute()` decorator - the attribute name type.
    pub attribute_name_type: Option<Box<'a, OutputExpression<'a>>>,

    /// Whether `@Host()` decorator is present.
    pub host: bool,

    /// Whether `@Optional()` decorator is present.
    pub optional: bool,

    /// Whether `@Self()` decorator is present.
    pub self_: bool,

    /// Whether `@SkipSelf()` decorator is present.
    pub skip_self: bool,
}

impl<'a> R3DependencyMetadata<'a> {
    /// Creates a new dependency with just a token.
    pub fn new(token: Box<'a, OutputExpression<'a>>) -> Self {
        Self {
            token: Some(token),
            attribute_name_type: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        }
    }

    /// Creates an invalid dependency (token is null).
    pub fn invalid() -> Self {
        Self {
            token: None,
            attribute_name_type: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        }
    }
}

/// Builder for creating pipe metadata.
pub struct R3PipeMetadataBuilder<'a> {
    name: Ident<'a>,
    pipe_name: Option<Ident<'a>>,
    r#type: OutputExpression<'a>,
    type_argument_count: u32,
    deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
    pure: bool,
    is_standalone: bool,
}

impl<'a> R3PipeMetadataBuilder<'a> {
    /// Creates a new builder with required fields.
    pub fn new(name: Ident<'a>, r#type: OutputExpression<'a>) -> Self {
        Self {
            name,
            pipe_name: None,
            r#type,
            type_argument_count: 0,
            deps: None,
            pure: true, // default is pure
            is_standalone: false,
        }
    }

    /// Sets the pipe name (as used in templates).
    pub fn pipe_name(mut self, name: Ident<'a>) -> Self {
        self.pipe_name = Some(name);
        self
    }

    /// Sets the type argument count.
    pub fn type_argument_count(mut self, count: u32) -> Self {
        self.type_argument_count = count;
        self
    }

    /// Sets the dependencies.
    pub fn deps(mut self, deps: Vec<'a, R3DependencyMetadata<'a>>) -> Self {
        self.deps = Some(deps);
        self
    }

    /// Sets whether the pipe is pure.
    pub fn pure(mut self, pure: bool) -> Self {
        self.pure = pure;
        self
    }

    /// Sets whether the pipe is standalone.
    pub fn is_standalone(mut self, standalone: bool) -> Self {
        self.is_standalone = standalone;
        self
    }

    /// Builds the pipe metadata.
    pub fn build(self) -> R3PipeMetadata<'a> {
        R3PipeMetadata {
            name: self.name,
            pipe_name: self.pipe_name,
            r#type: self.r#type,
            type_argument_count: self.type_argument_count,
            deps: self.deps,
            pure: self.pure,
            is_standalone: self.is_standalone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::{LiteralExpr, LiteralValue, ReadVarExpr};
    use oxc_allocator::Allocator;

    #[test]
    fn test_pipe_metadata_builder() {
        let allocator = Allocator::default();
        let name = Ident::from("TestPipe");
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestPipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadataBuilder::new(name.clone(), type_expr)
            .pipe_name(Ident::from("test"))
            .pure(true)
            .is_standalone(true)
            .build();

        assert_eq!(metadata.name.as_str(), "TestPipe");
        assert_eq!(metadata.pipe_name.as_ref().map(|n| n.as_str()), Some("test"));
        assert!(metadata.pure);
        assert!(metadata.is_standalone);
    }

    #[test]
    fn test_dependency_metadata() {
        let allocator = Allocator::default();
        let token = Box::new_in(
            OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::String(Ident::from("MyService")),
                    source_span: None,
                },
                &allocator,
            )),
            &allocator,
        );

        let dep = R3DependencyMetadata::new(token);
        assert!(dep.token.is_some());
        assert!(!dep.host);
        assert!(!dep.optional);

        let invalid = R3DependencyMetadata::invalid();
        assert!(invalid.token.is_none());
    }
}

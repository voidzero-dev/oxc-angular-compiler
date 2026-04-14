//! Factory metadata types.
//!
//! Ported from Angular's `render3/r3_factory.ts`.

use oxc_allocator::Vec;
use oxc_str::Ident;

use crate::output::ast::OutputExpression;

/// Target of the factory function being generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactoryTarget {
    /// Component factory.
    Component,
    /// Directive factory.
    Directive,
    /// Pipe factory.
    Pipe,
    /// NgModule factory.
    NgModule,
    /// Injectable factory.
    Injectable,
}

/// Delegate type for delegated factories.
///
/// See: `packages/compiler/src/render3/r3_factory.ts:48-51`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum R3FactoryDelegateType {
    /// Delegate is a class (use `new` to instantiate).
    Class,
    /// Delegate is a function (call directly).
    Function,
}

/// Dependency metadata for factory injection.
#[derive(Debug)]
pub struct R3DependencyMetadata<'a> {
    /// An expression representing the token or value to be injected.
    /// None if the dependency could not be resolved - making it invalid.
    pub token: Option<OutputExpression<'a>>,

    /// If an @Attribute decorator is present, this is the literal type of the attribute name,
    /// or None if no literal type is available (e.g. the attribute name is an expression).
    pub attribute_name_type: Option<OutputExpression<'a>>,

    /// Whether the dependency has an @Host qualifier.
    pub host: bool,

    /// Whether the dependency has an @Optional qualifier.
    pub optional: bool,

    /// Whether the dependency has an @Self qualifier.
    pub self_: bool,

    /// Whether the dependency has an @SkipSelf qualifier.
    pub skip_self: bool,
}

impl<'a> R3DependencyMetadata<'a> {
    /// Creates a simple dependency with just a token.
    pub fn simple(token: OutputExpression<'a>) -> Self {
        Self {
            token: Some(token),
            attribute_name_type: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        }
    }

    /// Creates an optional dependency.
    pub fn optional_dep(token: OutputExpression<'a>) -> Self {
        Self {
            token: Some(token),
            attribute_name_type: None,
            host: false,
            optional: true,
            self_: false,
            skip_self: false,
        }
    }
}

/// Result type for dependencies: valid deps, invalid (error), or none (inherited).
#[derive(Debug)]
pub enum R3FactoryDeps<'a> {
    /// Valid dependencies that can be injected.
    Valid(Vec<'a, R3DependencyMetadata<'a>>),
    /// One or more dependencies couldn't be resolved.
    Invalid,
    /// No constructor, will use inherited factory.
    None,
}

/// Metadata required by the factory generator to generate a `factory` function for a type.
#[derive(Debug)]
pub struct R3ConstructorFactoryMetadata<'a> {
    /// String name of the type being generated (used to name the factory function).
    pub name: Ident<'a>,

    /// An expression representing the interface type being constructed.
    pub type_expr: OutputExpression<'a>,

    /// An expression representing the type for type declarations.
    pub type_decl: OutputExpression<'a>,

    /// Number of generic type parameters.
    pub type_argument_count: u32,

    /// Dependencies of the constructor.
    pub deps: R3FactoryDeps<'a>,

    /// Type of the target being created by the factory.
    pub target: FactoryTarget,
}

/// Metadata for delegated factories (useClass/useFactory with deps).
///
/// See: `packages/compiler/src/render3/r3_factory.ts:53-57`
#[derive(Debug)]
pub struct R3DelegatedFnOrClassMetadata<'a> {
    /// Base metadata (name, type, deps, target).
    pub base: R3ConstructorFactoryMetadata<'a>,
    /// The delegate expression (class or function to call).
    pub delegate: OutputExpression<'a>,
    /// Whether the delegate is a class or function.
    pub delegate_type: R3FactoryDelegateType,
    /// Dependencies to inject when calling the delegate.
    pub delegate_deps: Vec<'a, R3DependencyMetadata<'a>>,
}

/// Metadata for expression-based factories (useValue/useExisting).
///
/// See: `packages/compiler/src/render3/r3_factory.ts:59-61`
#[derive(Debug)]
pub struct R3ExpressionFactoryMetadata<'a> {
    /// Base metadata (name, type, deps, target).
    pub base: R3ConstructorFactoryMetadata<'a>,
    /// The expression to use as the factory result.
    pub expression: OutputExpression<'a>,
}

/// Union type for all factory metadata variants.
///
/// See: `packages/compiler/src/render3/r3_factory.ts:63-66`
#[derive(Debug)]
pub enum R3FactoryMetadata<'a> {
    /// Standard constructor-based factory.
    Constructor(R3ConstructorFactoryMetadata<'a>),
    /// Delegated factory (useClass/useFactory with deps).
    Delegated(R3DelegatedFnOrClassMetadata<'a>),
    /// Expression factory (useValue/useExisting).
    Expression(R3ExpressionFactoryMetadata<'a>),
}

impl<'a> R3FactoryMetadata<'a> {
    /// Get the base metadata for any factory variant.
    pub fn base(&self) -> &R3ConstructorFactoryMetadata<'a> {
        match self {
            Self::Constructor(meta) => meta,
            Self::Delegated(meta) => &meta.base,
            Self::Expression(meta) => &meta.base,
        }
    }

    /// Returns true if this is a delegated factory.
    pub fn is_delegated(&self) -> bool {
        matches!(self, Self::Delegated(_))
    }

    /// Returns true if this is an expression factory.
    pub fn is_expression(&self) -> bool {
        matches!(self, Self::Expression(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::ReadVarExpr;
    use oxc_allocator::{Allocator, Box};

    #[test]
    fn test_dependency_metadata() {
        let allocator = Allocator::default();
        let token = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestService"), source_span: None },
            &allocator,
        ));

        let dep = R3DependencyMetadata::simple(token);
        assert!(!dep.host);
        assert!(!dep.optional);
        assert!(!dep.self_);
        assert!(!dep.skip_self);
        assert!(dep.token.is_some());
    }

    #[test]
    fn test_optional_dependency() {
        let allocator = Allocator::default();
        let token = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("OptionalService"), source_span: None },
            &allocator,
        ));

        let dep = R3DependencyMetadata::optional_dep(token);
        assert!(dep.optional);
    }

    #[test]
    fn test_factory_target() {
        assert_eq!(FactoryTarget::Pipe, FactoryTarget::Pipe);
        assert_ne!(FactoryTarget::Pipe, FactoryTarget::Component);
    }
}

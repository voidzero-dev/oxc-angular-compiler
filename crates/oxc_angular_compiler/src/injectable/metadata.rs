//! Injectable metadata structures.
//!
//! Ported from Angular's `injectable_compiler_2.ts`.

use oxc_allocator::Vec;
use oxc_str::Ident;

use crate::factory::R3DependencyMetadata;
use crate::output::ast::OutputExpression;

/// Provider type for an injectable.
///
/// Exactly one of these can be specified, or none for default behavior.
#[derive(Debug)]
pub enum InjectableProvider<'a> {
    /// Use an alternative class to instantiate.
    /// `useClass: AlternateClass`
    UseClass {
        /// The class expression to instantiate.
        class_expr: OutputExpression<'a>,
        /// Whether this is a forward reference.
        is_forward_ref: bool,
        /// Dependencies for the class constructor.
        deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
    },

    /// Use a factory function.
    /// `useFactory: () => value`
    UseFactory {
        /// The factory function expression.
        factory: OutputExpression<'a>,
        /// Dependencies to inject into the factory.
        deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
    },

    /// Use a literal value.
    /// `useValue: someValue`
    UseValue {
        /// The value expression.
        value: OutputExpression<'a>,
    },

    /// Use an existing token (alias).
    /// `useExisting: OtherToken`
    UseExisting {
        /// The existing token expression.
        existing: OutputExpression<'a>,
        /// Whether this is a forward reference.
        is_forward_ref: bool,
    },

    /// Default: use the injectable class's own constructor.
    Default,
}

/// Scope where the injectable is provided.
#[derive(Debug)]
pub enum ProvidedIn<'a> {
    /// Provided in the root injector (singleton).
    Root,
    /// Provided in a specific module.
    Module(OutputExpression<'a>),
    /// Provided in the platform injector.
    Platform,
    /// Provided in any injector (creates new instance per injector).
    Any,
    /// Not provided by default (must be added to providers array).
    None,
}

/// Metadata needed to compile an injectable.
///
/// Corresponds to Angular's `R3InjectableMetadata` interface.
#[derive(Debug)]
pub struct R3InjectableMetadata<'a> {
    /// Name of the injectable type.
    pub name: Ident<'a>,

    /// An expression representing a reference to the injectable class.
    pub r#type: OutputExpression<'a>,

    /// Number of generic type parameters of the type itself.
    pub type_argument_count: u32,

    /// Where this injectable is provided.
    pub provided_in: ProvidedIn<'a>,

    /// The provider configuration.
    pub provider: InjectableProvider<'a>,

    /// Constructor dependencies for factory generation.
    /// `None` means no constructor (use inherited factory).
    /// `Some(vec)` means constructor exists with these dependencies.
    pub deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
}

impl<'a> R3InjectableMetadata<'a> {
    /// Check if this injectable has a non-default provider.
    pub fn has_custom_provider(&self) -> bool {
        !matches!(self.provider, InjectableProvider::Default)
    }
}

/// Builder for R3InjectableMetadata.
pub struct R3InjectableMetadataBuilder<'a> {
    name: Option<Ident<'a>>,
    r#type: Option<OutputExpression<'a>>,
    type_argument_count: u32,
    provided_in: ProvidedIn<'a>,
    provider: InjectableProvider<'a>,
    deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
}

impl<'a> R3InjectableMetadataBuilder<'a> {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            name: None,
            r#type: None,
            type_argument_count: 0,
            provided_in: ProvidedIn::None,
            provider: InjectableProvider::Default,
            deps: None,
        }
    }

    /// Set the injectable name.
    pub fn name(mut self, name: Ident<'a>) -> Self {
        self.name = Some(name);
        self
    }

    /// Set the injectable type expression.
    pub fn r#type(mut self, type_expr: OutputExpression<'a>) -> Self {
        self.r#type = Some(type_expr);
        self
    }

    /// Set the type argument count.
    pub fn type_argument_count(mut self, count: u32) -> Self {
        self.type_argument_count = count;
        self
    }

    /// Set providedIn to 'root'.
    pub fn provided_in_root(mut self) -> Self {
        self.provided_in = ProvidedIn::Root;
        self
    }

    /// Set providedIn to a specific module.
    pub fn provided_in_module(mut self, module: OutputExpression<'a>) -> Self {
        self.provided_in = ProvidedIn::Module(module);
        self
    }

    /// Set providedIn to 'platform'.
    pub fn provided_in_platform(mut self) -> Self {
        self.provided_in = ProvidedIn::Platform;
        self
    }

    /// Set providedIn to 'any'.
    pub fn provided_in_any(mut self) -> Self {
        self.provided_in = ProvidedIn::Any;
        self
    }

    /// Set provider to useClass.
    pub fn use_class(
        mut self,
        class_expr: OutputExpression<'a>,
        is_forward_ref: bool,
        deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
    ) -> Self {
        self.provider = InjectableProvider::UseClass { class_expr, is_forward_ref, deps };
        self
    }

    /// Set provider to useFactory.
    pub fn use_factory(
        mut self,
        factory: OutputExpression<'a>,
        deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
    ) -> Self {
        self.provider = InjectableProvider::UseFactory { factory, deps };
        self
    }

    /// Set provider to useValue.
    pub fn use_value(mut self, value: OutputExpression<'a>) -> Self {
        self.provider = InjectableProvider::UseValue { value };
        self
    }

    /// Set provider to useExisting.
    pub fn use_existing(mut self, existing: OutputExpression<'a>, is_forward_ref: bool) -> Self {
        self.provider = InjectableProvider::UseExisting { existing, is_forward_ref };
        self
    }

    /// Set constructor dependencies for factory generation.
    pub fn deps(mut self, deps: Option<Vec<'a, R3DependencyMetadata<'a>>>) -> Self {
        self.deps = deps;
        self
    }

    /// Build the metadata.
    ///
    /// Returns None if required fields (name, type) are missing.
    pub fn build(self) -> Option<R3InjectableMetadata<'a>> {
        let name = self.name?;
        let r#type = self.r#type?;

        Some(R3InjectableMetadata {
            name,
            r#type,
            type_argument_count: self.type_argument_count,
            provided_in: self.provided_in,
            provider: self.provider,
            deps: self.deps,
        })
    }
}

impl<'a> Default for R3InjectableMetadataBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

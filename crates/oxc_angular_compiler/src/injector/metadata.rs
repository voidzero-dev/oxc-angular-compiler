//! Injector metadata structures.
//!
//! Ported from Angular's `render3/r3_injector_compiler.ts`.

use oxc_allocator::Vec;
use oxc_str::Ident;

use crate::output::ast::OutputExpression;

/// Metadata needed to compile an injector.
///
/// Corresponds to Angular's `R3InjectorMetadata` interface.
/// This is one of the simplest metadata structures in the compiler.
#[derive(Debug)]
pub struct R3InjectorMetadata<'a> {
    /// Name of the injector type.
    pub name: Ident<'a>,

    /// An expression representing a reference to the injector class.
    pub r#type: OutputExpression<'a>,

    /// The providers array expression.
    /// Can be None if no providers are defined.
    pub providers: Option<OutputExpression<'a>>,

    /// Imported modules/injectors (individual expressions).
    pub imports: Vec<'a, OutputExpression<'a>>,

    /// Pre-built raw imports array expression.
    /// When present, takes precedence over `imports` in the generated output.
    /// This preserves call expressions like `StoreModule.forRoot(...)` and spread elements.
    pub raw_imports: Option<OutputExpression<'a>>,
}

impl<'a> R3InjectorMetadata<'a> {
    /// Check if this injector has any providers.
    pub fn has_providers(&self) -> bool {
        self.providers.is_some()
    }

    /// Check if this injector has any imports.
    pub fn has_imports(&self) -> bool {
        self.raw_imports.is_some() || !self.imports.is_empty()
    }
}

/// Builder for R3InjectorMetadata.
pub struct R3InjectorMetadataBuilder<'a> {
    name: Option<Ident<'a>>,
    r#type: Option<OutputExpression<'a>>,
    providers: Option<OutputExpression<'a>>,
    imports: Vec<'a, OutputExpression<'a>>,
    raw_imports: Option<OutputExpression<'a>>,
}

impl<'a> R3InjectorMetadataBuilder<'a> {
    /// Create a new builder.
    pub fn new(allocator: &'a oxc_allocator::Allocator) -> Self {
        Self {
            name: None,
            r#type: None,
            providers: None,
            imports: Vec::new_in(allocator),
            raw_imports: None,
        }
    }

    /// Set the injector name.
    pub fn name(mut self, name: Ident<'a>) -> Self {
        self.name = Some(name);
        self
    }

    /// Set the injector type expression.
    pub fn r#type(mut self, type_expr: OutputExpression<'a>) -> Self {
        self.r#type = Some(type_expr);
        self
    }

    /// Set the providers expression.
    pub fn providers(mut self, providers: OutputExpression<'a>) -> Self {
        self.providers = Some(providers);
        self
    }

    /// Add an import.
    pub fn add_import(mut self, import: OutputExpression<'a>) -> Self {
        self.imports.push(import);
        self
    }

    /// Set raw imports array expression (takes precedence over individual imports).
    pub fn raw_imports(mut self, raw_imports: OutputExpression<'a>) -> Self {
        self.raw_imports = Some(raw_imports);
        self
    }

    /// Build the metadata.
    ///
    /// Returns None if required fields (name, type) are missing.
    pub fn build(self) -> Option<R3InjectorMetadata<'a>> {
        let name = self.name?;
        let r#type = self.r#type?;

        Some(R3InjectorMetadata {
            name,
            r#type,
            providers: self.providers,
            imports: self.imports,
            raw_imports: self.raw_imports,
        })
    }
}

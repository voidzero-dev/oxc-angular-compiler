//! Class metadata structures.
//!
//! Ported from Angular's `render3/r3_class_metadata_compiler.ts` and `view/api.ts`.

use crate::output::ast::OutputExpression;
use oxc_str::Ident;

/// Metadata of a class which captures the original Angular decorators.
///
/// The original decorators are preserved in the generated code to allow
/// TestBed APIs to recompile the class using the original decorator
/// with a set of overrides applied.
///
/// Corresponds to Angular's `R3ClassMetadata` interface.
#[derive(Debug)]
pub struct R3ClassMetadata<'a> {
    /// The class type for which the metadata is captured.
    pub r#type: OutputExpression<'a>,

    /// An expression representing the Angular decorators that were applied on the class.
    pub decorators: OutputExpression<'a>,

    /// An expression representing the Angular decorators applied to constructor parameters,
    /// or `None` if there is no constructor.
    pub ctor_parameters: Option<OutputExpression<'a>>,

    /// An expression representing the Angular decorators that were applied on the properties
    /// of the class, or `None` if no properties have decorators.
    pub prop_decorators: Option<OutputExpression<'a>>,
}

/// Metadata for a deferred dependency in a component.
///
/// Corresponds to Angular's `R3DeferPerComponentDependency` interface.
#[derive(Debug)]
pub struct R3DeferPerComponentDependency<'a> {
    /// The symbol name of the dependency.
    pub symbol_name: Ident<'a>,

    /// The import path for the dependency.
    pub import_path: Ident<'a>,

    /// Whether this is a default import.
    pub is_default_import: bool,
}

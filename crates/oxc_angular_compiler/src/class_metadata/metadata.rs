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
/// Corresponds to Angular's `R3DeferPerComponentDependency` interface, but
/// with the single `symbolName` field split in two so aliased imports can
/// tree-shake correctly:
///
/// - `param_name` — the local binding (what the decorator metadata literal
///   actually references). Used as the parameter name on the
///   `setClassMetadataAsync` callback so the body's identifier references
///   shadow the static import.
/// - `export_name` — the name under which the symbol is exported from its
///   source module. Used as the property read in the dynamic-import
///   resolver chain (`m.<export_name>`).
///
/// Angular conflates both into one `symbolName` field (always the exported
/// name), which leaves the static `import { Foo as Bar }` declaration pinned
/// for aliased deferrable imports — the callback parameter `Foo` is bound
/// but the body's `Bar` still references the outer scope. Splitting the
/// fields lets the callback parameter shadow the alias and allows bundlers
/// to drop the eager import.
#[derive(Debug)]
pub struct R3DeferPerComponentDependency<'a> {
    /// Local binding for the dependency in the current source file.
    ///
    /// Emitted as the `setClassMetadataAsync` callback parameter name so the
    /// decorator metadata literal's references resolve to the callback
    /// parameter rather than the outer static import.
    pub param_name: Ident<'a>,

    /// Name under which the dependency is exported from `import_path`.
    ///
    /// Emitted as the property read in the dynamic-import resolver
    /// (`m.<export_name>`). For default imports the resolver substitutes
    /// `m.default` based on `is_default_import` and this field is ignored.
    pub export_name: Ident<'a>,

    /// The import path for the dependency.
    pub import_path: Ident<'a>,

    /// Whether this is a default import.
    pub is_default_import: bool,
}

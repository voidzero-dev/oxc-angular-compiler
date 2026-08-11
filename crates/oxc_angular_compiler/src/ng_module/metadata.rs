//! NgModule metadata structures.
//!
//! Ported from Angular's `render3/r3_module_compiler.ts`.

use oxc_allocator::Vec;

use crate::output::ast::OutputExpression;

/// A reference containing both value and type expressions.
///
/// Used for module imports, exports, declarations, etc.
#[derive(Debug)]
pub struct R3Reference<'a> {
    /// The runtime value expression (e.g., the class reference).
    pub value: OutputExpression<'a>,

    /// The type expression for static analysis.
    pub type_expr: Option<OutputExpression<'a>>,
}

impl<'a> R3Reference<'a> {
    /// Create a new reference with just a value.
    pub fn value_only(value: OutputExpression<'a>) -> Self {
        Self { value, type_expr: None }
    }

    /// Create a new reference with value and type.
    pub fn with_type(value: OutputExpression<'a>, type_expr: OutputExpression<'a>) -> Self {
        Self { value, type_expr: Some(type_expr) }
    }
}

/// How the module scope should be emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum R3SelectorScopeMode {
    /// Scope is inlined directly in the ɵɵdefineNgModule call.
    /// Does not support tree-shaking.
    Inline,

    /// Scope is patched via ɵɵsetNgModuleScope in a side effect call.
    /// Supports tree-shaking via ngJitMode guard.
    SideEffect,

    /// No scope information is generated.
    /// Used for AOT-only builds where JIT support is not needed.
    Omit,
}

/// Metadata needed to compile an NgModule.
///
/// Corresponds to Angular's `R3NgModuleMetadata` interface.
#[derive(Debug)]
pub struct R3NgModuleMetadata<'a> {
    /// Reference to the module class.
    pub r#type: R3Reference<'a>,

    /// Bootstrap components.
    pub bootstrap: Vec<'a, R3Reference<'a>>,

    /// Declared directives and pipes.
    pub declarations: Vec<'a, R3Reference<'a>>,

    /// Imported modules.
    pub imports: Vec<'a, R3Reference<'a>>,

    /// Exported classes.
    pub exports: Vec<'a, R3Reference<'a>>,

    /// Custom element schemas.
    pub schemas: Vec<'a, R3Reference<'a>>,

    /// Module ID for registration.
    pub id: Option<OutputExpression<'a>>,

    /// How to emit the selector scope.
    pub selector_scope_mode: R3SelectorScopeMode,

    /// Whether any declarations/imports/exports contain forward references.
    /// If true, arrays are wrapped in arrow functions.
    pub contains_forward_decls: bool,
}

impl<'a> R3NgModuleMetadata<'a> {
    /// Check if this module has any bootstrap components.
    pub fn has_bootstrap(&self) -> bool {
        !self.bootstrap.is_empty()
    }

    /// Check if this module has any declarations.
    pub fn has_declarations(&self) -> bool {
        !self.declarations.is_empty()
    }

    /// Check if this module has any imports.
    pub fn has_imports(&self) -> bool {
        !self.imports.is_empty()
    }

    /// Check if this module has any exports.
    pub fn has_exports(&self) -> bool {
        !self.exports.is_empty()
    }

    /// Check if this module has any schemas.
    pub fn has_schemas(&self) -> bool {
        !self.schemas.is_empty()
    }

    /// Check if scope should be inlined in the definition.
    pub fn should_inline_scope(&self) -> bool {
        self.selector_scope_mode == R3SelectorScopeMode::Inline
    }

    /// Check if scope should be set via side effect.
    pub fn should_set_scope_side_effect(&self) -> bool {
        self.selector_scope_mode == R3SelectorScopeMode::SideEffect
    }
}

/// Builder for R3NgModuleMetadata.
pub struct R3NgModuleMetadataBuilder<'a> {
    r#type: Option<R3Reference<'a>>,
    bootstrap: Vec<'a, R3Reference<'a>>,
    declarations: Vec<'a, R3Reference<'a>>,
    imports: Vec<'a, R3Reference<'a>>,
    exports: Vec<'a, R3Reference<'a>>,
    schemas: Vec<'a, R3Reference<'a>>,
    id: Option<OutputExpression<'a>>,
    selector_scope_mode: R3SelectorScopeMode,
    contains_forward_decls: bool,
}

impl<'a> R3NgModuleMetadataBuilder<'a> {
    /// Create a new builder.
    pub fn new(allocator: &'a oxc_allocator::Allocator) -> Self {
        Self {
            r#type: None,
            bootstrap: Vec::new_in(&allocator),
            declarations: Vec::new_in(&allocator),
            imports: Vec::new_in(&allocator),
            exports: Vec::new_in(&allocator),
            schemas: Vec::new_in(&allocator),
            id: None,
            selector_scope_mode: R3SelectorScopeMode::Inline,
            contains_forward_decls: false,
        }
    }

    /// Set the module type reference.
    pub fn r#type(mut self, type_ref: R3Reference<'a>) -> Self {
        self.r#type = Some(type_ref);
        self
    }

    /// Add a bootstrap component.
    pub fn add_bootstrap(mut self, component: R3Reference<'a>) -> Self {
        self.bootstrap.push(component);
        self
    }

    /// Add a declaration.
    pub fn add_declaration(mut self, decl: R3Reference<'a>) -> Self {
        self.declarations.push(decl);
        self
    }

    /// Add an import.
    pub fn add_import(mut self, import: R3Reference<'a>) -> Self {
        self.imports.push(import);
        self
    }

    /// Add an export.
    pub fn add_export(mut self, export: R3Reference<'a>) -> Self {
        self.exports.push(export);
        self
    }

    /// Add a schema.
    pub fn add_schema(mut self, schema: R3Reference<'a>) -> Self {
        self.schemas.push(schema);
        self
    }

    /// Set the module ID.
    pub fn id(mut self, id: OutputExpression<'a>) -> Self {
        self.id = Some(id);
        self
    }

    /// Set the selector scope mode.
    pub fn selector_scope_mode(mut self, mode: R3SelectorScopeMode) -> Self {
        self.selector_scope_mode = mode;
        self
    }

    /// Set whether the module contains forward declarations.
    pub fn contains_forward_decls(mut self, contains: bool) -> Self {
        self.contains_forward_decls = contains;
        self
    }

    /// Build the metadata.
    ///
    /// Returns None if the required type is missing.
    pub fn build(self) -> Option<R3NgModuleMetadata<'a>> {
        let r#type = self.r#type?;

        Some(R3NgModuleMetadata {
            r#type,
            bootstrap: self.bootstrap,
            declarations: self.declarations,
            imports: self.imports,
            exports: self.exports,
            schemas: self.schemas,
            id: self.id,
            selector_scope_mode: self.selector_scope_mode,
            contains_forward_decls: self.contains_forward_decls,
        })
    }
}

//! NgModule compilation module.
//!
//! This module provides compilation support for Angular `@NgModule` decorators,
//! ported from Angular's `render3/r3_module_compiler.ts`.
//!
//! NgModules define:
//! - declarations: Directives and pipes belonging to this module
//! - imports: Other modules whose exports become available
//! - exports: Declarations that can be used by importing modules
//! - bootstrap: Components to bootstrap the application
//! - providers: DI providers (handled separately via injector)
//! - schemas: Custom element schemas

mod compiler;
mod decorator;
mod definition;
mod metadata;

pub use compiler::{NgModuleCompileResult, compile_ng_module, compile_ng_module_from_metadata};
pub(crate) use decorator::find_ng_module_decorator;
pub use decorator::{NgModuleMetadata, extract_ng_module_metadata, find_ng_module_decorator_span};
pub use definition::{
    FullNgModuleDefinition, NgModuleDefinition, emit_full_ng_module_definition,
    emit_ng_module_definition, generate_full_ng_module_definition, generate_ng_module_definition,
    generate_ng_module_definition_from_decorator,
};
pub use metadata::{
    R3NgModuleMetadata, R3NgModuleMetadataBuilder, R3Reference, R3SelectorScopeMode,
};

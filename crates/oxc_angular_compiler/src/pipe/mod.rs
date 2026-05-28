//! Pipe compilation module.
//!
//! This module provides compilation support for Angular `@Pipe` decorators,
//! ported from Angular's `render3/r3_pipe_compiler.ts`.
//!
//! Pipes are simpler than components/directives:
//! - No template processing
//! - Minimal metadata (name, type, pure, standalone)
//! - No selectors or host bindings
//! - Generates `ɵpipe = ɵɵdefinePipe({...})`
//! - Generates `ɵfac` factory function for dependency injection

mod compiler;
mod decorator;
mod definition;
mod metadata;

pub use compiler::{PipeCompileResult, compile_pipe, compile_pipe_from_metadata};
pub(crate) use decorator::find_pipe_decorator;
pub use decorator::{PipeMetadata, extract_pipe_metadata, find_pipe_decorator_span};
pub use definition::{
    FullPipeDefinition, PipeDefinition, generate_full_pipe_definition_from_decorator,
    generate_pipe_definition, generate_pipe_definition_from_decorator,
};
pub use metadata::{R3DependencyMetadata, R3PipeMetadata, R3PipeMetadataBuilder};

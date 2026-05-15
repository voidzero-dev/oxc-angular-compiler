//! Directive compilation module.
//!
//! This module provides compilation support for Angular `@Directive` decorators,
//! ported from Angular's `render3/view/compiler.ts`.
//!
//! Directives share base functionality with components but don't have templates.
//! They can have:
//! - Selector matching
//! - Host bindings (properties, events, attributes)
//! - Inputs and outputs
//! - Content and view queries
//! - Providers
//! - Lifecycle hooks

mod compiler;
mod decorator;
mod definition;
mod metadata;
mod property_decorators;
mod query;

pub(crate) use compiler::create_host_directive_mappings_array;
pub use compiler::{
    DirectiveCompileResult, compile_directive, compile_directive_from_metadata,
    create_inputs_literal, create_outputs_literal,
};
pub use decorator::{
    StringConsts, collect_string_consts, extract_directive_metadata, find_directive_decorator_span,
};
pub use definition::{DirectiveDefinitions, generate_directive_definitions};
pub use metadata::{
    QueryPredicate, R3DirectiveMetadata, R3DirectiveMetadataBuilder, R3HostDirectiveMetadata,
    R3HostMetadata, R3InputMetadata, R3QueryMetadata,
};
pub use property_decorators::{
    extract_content_queries, extract_host_bindings, extract_host_listeners, extract_input_metadata,
    extract_output_metadata, extract_view_queries,
};
pub use query::{create_content_queries_function, create_view_queries_function};

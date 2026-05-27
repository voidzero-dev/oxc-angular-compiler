//! Injectable compilation module.
//!
//! This module provides compilation support for Angular `@Injectable` decorators,
//! ported from Angular's `injectable_compiler_2.ts`.
//!
//! Injectables are services/providers in Angular's dependency injection system.
//! They support multiple provider types:
//! - Default: Use the injectable class's own constructor
//! - useClass: Instantiate an alternative class
//! - useFactory: Call a factory function
//! - useValue: Return a literal value
//! - useExisting: Alias to another token

mod compiler;
mod decorator;
mod definition;
mod metadata;

pub use compiler::{InjectableCompileResult, compile_injectable, compile_injectable_from_metadata};
pub(crate) use decorator::find_injectable_decorator;
pub use decorator::{
    DependencyMetadata, InjectableMetadata, ProvidedInValue, UseClassMetadata, UseExistingMetadata,
    UseFactoryMetadata, extract_injectable_metadata, find_injectable_decorator_span,
};
pub use definition::{
    InjectableDefinition, generate_injectable_definition,
    generate_injectable_definition_from_decorator,
};
pub use metadata::{InjectableProvider, R3InjectableMetadata, R3InjectableMetadataBuilder};

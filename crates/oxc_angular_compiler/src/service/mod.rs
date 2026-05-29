//! Service compilation module (Angular v22+ `@Service` decorator).
//!
//! Ported from Angular's `service_compiler.ts`. The `@Service` decorator is a
//! lighter alternative to `@Injectable` for root-injector-provided services
//! whose dependencies are resolved via `inject()` calls in the constructor
//! body rather than constructor parameter injection.
//!
//! Unlike `@Injectable`, `@Service` does not support `providedIn` or the
//! `useClass`/`useFactory`/`useValue`/`useExisting` provider variants, and
//! its ɵfac never injects ctor params.

mod compiler;
mod decorator;
mod definition;
mod metadata;

pub use compiler::{ServiceCompileResult, compile_service};
pub use decorator::{
    ServiceMetadata, extract_service_metadata, find_service_decorator,
    find_service_decorator_span,
};
pub use definition::{
    ServiceDefinition, generate_service_definition,
    generate_service_definition_from_decorator,
};
pub use metadata::R3ServiceMetadata;

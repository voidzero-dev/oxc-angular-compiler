//! Class metadata compilation module.
//!
//! Ported from Angular's `render3/r3_class_metadata_compiler.ts`.
//!
//! Generates class metadata for TestBed support:
//! ```javascript
//! (() => {
//!   (typeof ngDevMode === "undefined" || ngDevMode) &&
//!     i0.ɵɵsetClassMetadata(MyComponent, [...], null, null);
//! })();
//! ```

mod builders;
mod compiler;
mod metadata;

pub use builders::{
    build_ctor_params_metadata, build_ctor_params_metadata_in, build_decorator_metadata_array,
    build_prop_decorators_metadata, build_prop_decorators_metadata_in,
};
pub use compiler::{
    compile_class_metadata, compile_component_class_metadata,
    compile_component_metadata_async_resolver, compile_opaque_async_class_metadata,
};
pub use metadata::{R3ClassMetadata, R3DeferPerComponentDependency};

//! Partial-declaration emit for library compilation.
//!
//! Ported from Angular's `packages/compiler/src/render3/partial/`.
//!
//! Each submodule emits one `ɵɵngDeclare*` shape. The linker
//! (`crate::linker`) is the inverse — it expands these calls into the
//! corresponding `ɵɵdefine*` calls at consumer build time.
//!
//! Currently implemented:
//! - `factory` — `ɵɵngDeclareFactory`
//!
//! Not yet implemented (and the dispatch from the per-decorator emit paths
//! falls back to full mode): component, directive, pipe, injectable,
//! injector, ngmodule, classMetadata.

pub mod factory;

pub use factory::compile_declare_factory_function;

/// The literal text used for the `version` field of every partial
/// declaration. Upstream substitutes this at npm publish; we keep the same
/// sentinel so the existing linker version-check logic recognizes it.
///
/// Matches upstream `packages/compiler/src/render3/partial/*.ts` — every
/// emitter writes `"0.0.0-PLACEHOLDER"` directly.
pub const PLACEHOLDER_VERSION: &str = "0.0.0-PLACEHOLDER";

/// Minimum linker version that understands a `ɵɵngDeclareFactory` shape.
///
/// Matches upstream `packages/compiler/src/render3/partial/factory.ts:25`.
pub const MIN_VERSION_FACTORY: &str = "12.0.0";

/// Minimum linker version for the other partial-declaration kinds.
///
/// These constants are used as new partial emitters land. Listed here so
/// every minVersion lives in one place and stays in sync with upstream.
#[allow(dead_code)]
pub(crate) const MIN_VERSION_INJECTABLE: &str = "12.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_INJECTOR: &str = "12.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_CLASS_METADATA: &str = "12.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_PIPE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_NG_MODULE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_DIRECTIVE_BASE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_COMPONENT_BASE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_CLASS_METADATA_ASYNC: &str = "18.0.0";

//! Shared utilities for the Angular compiler.

pub mod chars;
mod deferred_time;
mod parse_util;
mod type_extract;

pub use deferred_time::*;
pub use parse_util::*;
pub use type_extract::*;

/// Whether a property of a decorator's options object is read as metadata.
/// Like ngtsc's `reflectObjectLiteral`, methods and accessors are not.
pub fn is_metadata_property(prop: &oxc_ast::ast::ObjectProperty<'_>) -> bool {
    !prop.method && prop.kind == oxc_ast::ast::PropertyKind::Init
}

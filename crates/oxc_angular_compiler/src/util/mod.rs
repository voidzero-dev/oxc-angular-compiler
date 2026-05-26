//! Shared utilities for the Angular compiler.

pub mod chars;
mod deferred_time;
mod parse_util;
mod type_extract;

pub use deferred_time::*;
pub use parse_util::*;
pub use type_extract::*;

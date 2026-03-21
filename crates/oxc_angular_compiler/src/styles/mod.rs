//! CSS style processing for Angular components.
//!
//! This module provides functionality for:
//! - Style encapsulation (emulating Shadow DOM scoping)
//! - CSS transformation for component-scoped styles

mod encapsulation;
mod minify;

pub use encapsulation::{encapsulate_style, shim_css_text};
pub use minify::{finalize_component_style, minify_component_style};

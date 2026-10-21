//! CSS style processing for Angular components.
//!
//! This module provides functionality for:
//! - Style encapsulation (emulating Shadow DOM scoping)
//! - CSS transformation for component-scoped styles

mod encapsulation;
mod minify;

pub use encapsulation::{
    encapsulate_style, encapsulate_style_for_version, shim_css_text, shim_css_text_for_version,
};
pub use minify::{finalize_component_style, minify_component_style};

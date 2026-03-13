//! R3 (Render3) compiler utilities.
//!
//! This module contains utilities for Angular's Render3 compilation,
//! including runtime identifier constants.

pub mod identifiers;

pub use identifiers::{
    Identifiers, get_attribute_interpolate_instruction, get_class_map_interpolate_instruction,
    get_interpolate_instruction, get_pipe_bind_instruction, get_property_interpolate_instruction,
    get_pure_function_instruction, get_style_map_interpolate_instruction,
    get_style_prop_interpolate_instruction, get_text_interpolate_instruction,
};

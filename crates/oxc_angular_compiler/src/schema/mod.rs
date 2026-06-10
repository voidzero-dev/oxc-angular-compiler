//! Schema definitions for Angular template compilation.
//!
//! This module contains schema information for DOM elements, attributes,
//! and security contexts.

mod dom_security_schema;
mod trusted_types_sinks;

pub use dom_security_schema::{
    calc_security_context_for_unknown_element, compute_security_context,
    extract_element_from_selector, get_security_context, host_binding_security_context,
};
pub use trusted_types_sinks::is_trusted_types_sink;

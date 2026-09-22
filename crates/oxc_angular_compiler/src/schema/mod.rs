//! Schema definitions for Angular template compilation.
//!
//! This module contains schema information for DOM elements, attributes,
//! and security contexts.

mod dom_security_schema;
mod trusted_types_sinks;

pub use dom_security_schema::{
    calc_security_context_for_unknown_element, get_security_context, get_security_context_for,
    host_binding_security_context, host_binding_security_context_for, is_known_element,
    rejects_iframe_src_i18n, strips_namespaced_svg_script,
};
pub use trusted_types_sinks::{is_trusted_types_sink, is_trusted_types_sink_at};

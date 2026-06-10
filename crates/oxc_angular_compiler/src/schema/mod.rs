//! Schema definitions for Angular template compilation.
//!
//! This module contains schema information for DOM elements, attributes,
//! and security contexts.

mod dom_security_schema;

pub use dom_security_schema::{calc_security_context_for_unknown_element, get_security_context};

//! Template transformation module.
//!
//! This module transforms HTML AST nodes to R3 AST nodes.
//!
//! The main entry point is [`html_to_r3`] which provides the
//! [`HtmlToR3Transform`] for converting parsed HTML templates
//! to the R3 intermediate representation.

pub mod boundaries;
pub mod control_flow;
pub mod html_to_r3;

pub use html_to_r3::HtmlToR3Transform;

//! Resolve sanitizers phase.
//!
//! Resolves security sanitizers for bindings based on their security context.
//! This phase determines which sanitizer function should be used to sanitize
//! values before they are bound to DOM properties or attributes.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_sanitizers.ts`.

use oxc_str::Ident;

use crate::ast::r3::SecurityContext;
use crate::ir::ops::{CreateOp, UpdateOp};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use crate::r3::Identifiers;

/// Map a security context to its sanitizer function name.
fn get_sanitizer_fn(security_context: SecurityContext) -> Option<&'static str> {
    match security_context {
        SecurityContext::Html => Some(Identifiers::SANITIZE_HTML),
        SecurityContext::Style => Some(Identifiers::SANITIZE_STYLE),
        SecurityContext::Script => Some(Identifiers::SANITIZE_SCRIPT),
        SecurityContext::Url => Some(Identifiers::SANITIZE_URL),
        SecurityContext::ResourceUrl => Some(Identifiers::SANITIZE_RESOURCE_URL),
        // Special case: When the host element isn't known, some URL attributes
        // (such as "src" and "href") may be part of multiple different security
        // contexts. In this case we use a special sanitization function that
        // selects the actual sanitizer at runtime based on the tag name.
        SecurityContext::UrlOrResourceUrl => Some(Identifiers::SANITIZE_URL_OR_RESOURCE_URL),
        SecurityContext::None => None,
        // AttributeNoBinding means the attribute should not be bound at all.
        // This should produce a compile-time error in the HTML-to-R3 transform.
        // For now, return None but the binding should have been rejected earlier.
        SecurityContext::AttributeNoBinding => None,
    }
}

/// Map a security context to its trusted value function name.
/// Used for constant attributes that need trusted values.
fn get_trusted_value_fn(security_context: SecurityContext) -> Option<&'static str> {
    match security_context {
        SecurityContext::Html => Some(Identifiers::TRUST_CONSTANT_HTML),
        SecurityContext::ResourceUrl => Some(Identifiers::TRUST_CONSTANT_RESOURCE_URL),
        // UrlOrResourceUrl doesn't have a trusted value function - it's resolved at runtime
        SecurityContext::UrlOrResourceUrl => None,
        // Other security contexts don't have trusted value functions
        _ => None,
    }
}

/// Resolves security sanitizers for property bindings.
///
/// This phase:
/// 1. For ExtractedAttribute ops (constant attributes), sets the trusted value function
/// 2. For Property, Attribute, and DomProperty ops, sets the sanitizer function
pub fn resolve_sanitizers(job: &mut ComponentCompilationJob<'_>) {
    // Collect view xrefs to avoid borrow issues
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            // Process create ops - set trusted value functions for extracted attributes
            for op in view.create.iter_mut() {
                if let CreateOp::ExtractedAttribute(attr) = op {
                    if let Some(fn_name) = get_trusted_value_fn(attr.security_context) {
                        attr.trusted_value_fn = Some(Ident::from(fn_name));
                    }
                }
            }

            // Process update ops - set sanitizers for property/attribute bindings
            for op in view.update.iter_mut() {
                match op {
                    UpdateOp::Property(prop) => {
                        if let Some(fn_name) = get_sanitizer_fn(prop.security_context) {
                            prop.sanitizer = Some(Ident::from(fn_name));
                        }
                    }
                    UpdateOp::Attribute(attr) => {
                        if let Some(fn_name) = get_sanitizer_fn(attr.security_context) {
                            attr.sanitizer = Some(Ident::from(fn_name));
                        }
                    }
                    UpdateOp::DomProperty(dom_prop) => {
                        if let Some(fn_name) = get_sanitizer_fn(dom_prop.security_context) {
                            dom_prop.sanitizer = Some(Ident::from(fn_name));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Resolves sanitizers for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn resolve_sanitizers_for_host(job: &mut HostBindingCompilationJob<'_>) {
    // Process create ops - set trusted value functions for extracted attributes
    for op in job.root.create.iter_mut() {
        if let CreateOp::ExtractedAttribute(attr) = op {
            if let Some(fn_name) = get_trusted_value_fn(attr.security_context) {
                attr.trusted_value_fn = Some(Ident::from(fn_name));
            }
        }
    }

    // Process update ops - set sanitizers for property/attribute bindings
    for op in job.root.update.iter_mut() {
        match op {
            UpdateOp::Property(prop) => {
                if let Some(fn_name) = get_sanitizer_fn(prop.security_context) {
                    prop.sanitizer = Some(Ident::from(fn_name));
                }
            }
            UpdateOp::Attribute(attr) => {
                if let Some(fn_name) = get_sanitizer_fn(attr.security_context) {
                    attr.sanitizer = Some(Ident::from(fn_name));
                }
            }
            UpdateOp::DomProperty(dom_prop) => {
                if let Some(fn_name) = get_sanitizer_fn(dom_prop.security_context) {
                    dom_prop.sanitizer = Some(Ident::from(fn_name));
                }
            }
            _ => {}
        }
    }
}

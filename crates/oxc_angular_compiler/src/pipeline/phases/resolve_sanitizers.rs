//! Resolve sanitizers phase.
//!
//! Resolves security sanitizers for bindings based on their security context.
//! This phase determines which sanitizer function should be used to sanitize
//! values before they are bound to DOM properties or attributes.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_sanitizers.ts`.

use oxc_str::Ident;
use rustc_hash::FxHashMap;

use crate::ast::r3::SecurityContext;
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use crate::r3::Identifiers;
use crate::schema::{is_iframe_security_sensitive_attr, uses_iframe_attr_validation};

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
        // `resolve_sanitizers.ts` maps ATTRIBUTE_NO_BINDING to `ɵɵvalidateAttribute`.
        SecurityContext::AttributeNoBinding => Some(Identifiers::VALIDATE_ATTRIBUTE),
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

/// The element-or-container create ops upstream indexes with
/// `createOpXrefMap`, reduced to what the iframe fallback reads: whether the
/// owner op is an `elementStart` whose tag is `iframe`.
///
/// Upstream's `isIframeElement` checks `op.kind === OpKind.ElementStart`, so
/// self-closing `element()` ops are intentionally not matched.
fn create_op_xref_map<'a>(ops: impl Iterator<Item = &'a CreateOp<'a>>) -> FxHashMap<XrefId, bool> {
    let mut elements = FxHashMap::default();
    for op in ops {
        let (xref, is_iframe) = match op {
            CreateOp::ElementStart(op) => (op.xref, op.tag.as_str().eq_ignore_ascii_case("iframe")),
            CreateOp::Element(op) => (op.xref, false),
            CreateOp::ContainerStart(op) => (op.xref, false),
            CreateOp::Container(op) => (op.xref, false),
            CreateOp::Template(op) => (op.xref, false),
            CreateOp::Conditional(op) => (op.xref, false),
            CreateOp::ConditionalBranch(op) => (op.xref, false),
            CreateOp::RepeaterCreate(op) => {
                // Upstream indexes the `@empty` view under the same repeater op.
                if let Some(empty_view) = op.empty_view {
                    elements.insert(empty_view, false);
                }
                (op.xref, false)
            }
            _ => continue,
        };
        elements.insert(xref, is_iframe);
    }
    elements
}

/// Apply upstream's legacy `ɵɵvalidateIframeAttribute` fallback: when a
/// `Property` / `Attribute` / `DomProperty` op got no sanitizer from its
/// security context, a security-sensitive iframe attribute gets the runtime
/// validator. Removed upstream once the schema's `attributeNoBinding` iframe
/// keys covered the same attributes (19.2.17 / 20.3.15 / 21.0.2).
fn resolve_iframe_sanitizer(
    assume_iframe: bool,
    elements: &FxHashMap<XrefId, bool>,
    target: XrefId,
    name: &str,
    sanitizer: &mut Option<Ident<'_>>,
) {
    if sanitizer.is_some() {
        return;
    }
    // For host bindings and `DomProperty` ops the element is not known at
    // compile time, so upstream assumes it may be an iframe; the emitted
    // validator checks the real tag at runtime.
    let is_iframe = if assume_iframe {
        true
    } else {
        *elements
            .get(&target)
            .unwrap_or_else(|| panic!("Property should have an element-like owner"))
    };
    if is_iframe && is_iframe_security_sensitive_attr(name) {
        *sanitizer = Some(Ident::from(Identifiers::VALIDATE_IFRAME_ATTRIBUTE));
    }
}

/// Resolves security sanitizers for property bindings.
///
/// This phase:
/// 1. For ExtractedAttribute ops (constant attributes), sets the trusted value function
/// 2. For Property, Attribute, and DomProperty ops, sets the sanitizer function
pub fn resolve_sanitizers(job: &mut ComponentCompilationJob<'_>) {
    let iframe_validation = uses_iframe_attr_validation(job.angular_version);
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

            let elements = iframe_validation.then(|| create_op_xref_map(view.create.iter()));

            // Process update ops - set sanitizers for property/attribute bindings
            for op in view.update.iter_mut() {
                match op {
                    UpdateOp::Property(prop) => {
                        if let Some(fn_name) = get_sanitizer_fn(prop.security_context) {
                            prop.sanitizer = Some(Ident::from(fn_name));
                        }
                        if let Some(elements) = &elements {
                            resolve_iframe_sanitizer(
                                false,
                                elements,
                                prop.target,
                                prop.name.as_str(),
                                &mut prop.sanitizer,
                            );
                        }
                    }
                    UpdateOp::Attribute(attr) => {
                        if let Some(fn_name) = get_sanitizer_fn(attr.security_context) {
                            attr.sanitizer = Some(Ident::from(fn_name));
                        }
                        if let Some(elements) = &elements {
                            resolve_iframe_sanitizer(
                                false,
                                elements,
                                attr.target,
                                attr.name.as_str(),
                                &mut attr.sanitizer,
                            );
                        }
                    }
                    UpdateOp::DomProperty(dom_prop) => {
                        if let Some(fn_name) = get_sanitizer_fn(dom_prop.security_context) {
                            dom_prop.sanitizer = Some(Ident::from(fn_name));
                        }
                        if let Some(elements) = &elements {
                            resolve_iframe_sanitizer(
                                true,
                                elements,
                                dom_prop.target,
                                dom_prop.name.as_str(),
                                &mut dom_prop.sanitizer,
                            );
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
    let iframe_validation = uses_iframe_attr_validation(job.angular_version);

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
        let (name, sanitizer) = match op {
            UpdateOp::Property(prop) => {
                if let Some(fn_name) = get_sanitizer_fn(prop.security_context) {
                    prop.sanitizer = Some(Ident::from(fn_name));
                }
                (prop.name.as_str(), &mut prop.sanitizer)
            }
            UpdateOp::Attribute(attr) => {
                if let Some(fn_name) = get_sanitizer_fn(attr.security_context) {
                    attr.sanitizer = Some(Ident::from(fn_name));
                }
                (attr.name.as_str(), &mut attr.sanitizer)
            }
            UpdateOp::DomProperty(dom_prop) => {
                if let Some(fn_name) = get_sanitizer_fn(dom_prop.security_context) {
                    dom_prop.sanitizer = Some(Ident::from(fn_name));
                }
                (dom_prop.name.as_str(), &mut dom_prop.sanitizer)
            }
            _ => continue,
        };
        // A host job cannot know its host element at compile time, so upstream
        // assumes iframe and defers the tag check to the runtime validator.
        if iframe_validation && sanitizer.is_none() && is_iframe_security_sensitive_attr(name) {
            *sanitizer = Some(Ident::from(Identifiers::VALIDATE_IFRAME_ATTRIBUTE));
        }
    }
}

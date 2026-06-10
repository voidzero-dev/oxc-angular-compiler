//! DOM Security Schema
//!
//! This module contains the security schema that maps element|property combinations
//! to their appropriate security context for sanitization.
//!
//! Ported from Angular's `schema/dom_security_schema.ts`.
//!
//! DO NOT EDIT THIS LIST OF SECURITY SENSITIVE PROPERTIES WITHOUT A SECURITY REVIEW!

use crate::ast::r3::SecurityContext;
use rustc_hash::FxHashMap;
use std::sync::LazyLock;

/// Security schema mapping `"element|property"` to `SecurityContext`.
/// Properties applying to all elements use `"*"` as the element name.
static SECURITY_SCHEMA: LazyLock<FxHashMap<&'static str, SecurityContext>> = LazyLock::new(|| {
    let mut schema = FxHashMap::default();

    // HTML contexts - content that will be parsed as HTML
    register_context(
        &mut schema,
        SecurityContext::Html,
        &["iframe|srcdoc", "*|innerhtml", "*|outerhtml"],
    );

    // Style contexts - CSS style content
    register_context(&mut schema, SecurityContext::Style, &["*|style"]);

    // URL contexts - URLs that are navigable (less dangerous than resource URLs)
    register_context(
        &mut schema,
        SecurityContext::Url,
        &[
            "*|formaction",
            "area|href",
            "a|href",
            "a|xlink:href",
            "form|action",
            // MathML namespace URLs
            "annotation|href",
            "annotation|xlink:href",
            "annotation-xml|href",
            "annotation-xml|xlink:href",
            "maction|href",
            "maction|xlink:href",
            "malignmark|href",
            "malignmark|xlink:href",
            "math|href",
            "math|xlink:href",
            "mroot|href",
            "mroot|xlink:href",
            "msqrt|href",
            "msqrt|xlink:href",
            "merror|href",
            "merror|xlink:href",
            "mfrac|href",
            "mfrac|xlink:href",
            "mglyph|href",
            "mglyph|xlink:href",
            "msub|href",
            "msub|xlink:href",
            "msup|href",
            "msup|xlink:href",
            "msubsup|href",
            "msubsup|xlink:href",
            "mmultiscripts|href",
            "mmultiscripts|xlink:href",
            "mprescripts|href",
            "mprescripts|xlink:href",
            "mi|href",
            "mi|xlink:href",
            "mn|href",
            "mn|xlink:href",
            "mo|href",
            "mo|xlink:href",
            "mpadded|href",
            "mpadded|xlink:href",
            "mphantom|href",
            "mphantom|xlink:href",
            "mrow|href",
            "mrow|xlink:href",
            "ms|href",
            "ms|xlink:href",
            "mspace|href",
            "mspace|xlink:href",
            "mstyle|href",
            "mstyle|xlink:href",
            "mtable|href",
            "mtable|xlink:href",
            "mtd|href",
            "mtd|xlink:href",
            "mtr|href",
            "mtr|xlink:href",
            "mtext|href",
            "mtext|xlink:href",
            "mover|href",
            "mover|xlink:href",
            "munder|href",
            "munder|xlink:href",
            "munderover|href",
            "munderover|xlink:href",
            "semantics|href",
            "semantics|xlink:href",
            "none|href",
            "none|xlink:href",
            // These are safe but included for compatibility
            "img|src",
            "video|src",
        ],
    );

    // Resource URL contexts - URLs that load executable content
    register_context(
        &mut schema,
        SecurityContext::ResourceUrl,
        &[
            "base|href",
            "embed|src",
            "frame|src",
            "iframe|src",
            "link|href",
            "object|codebase",
            "object|data",
            "script|src",
        ],
    );

    // Attribute no-binding contexts - attributes that should not be bound
    // These are unsafe as `attributeName` can be `href` or `xlink:href`
    register_context(
        &mut schema,
        SecurityContext::AttributeNoBinding,
        &[
            "animate|attributename",
            "set|attributename",
            "animatemotion|attributename",
            "animatetransform|attributename",
            "unknown|attributename",
            "iframe|sandbox",
            "iframe|allow",
            "iframe|allowfullscreen",
            "iframe|referrerpolicy",
            "iframe|csp",
            "iframe|fetchpriority",
            "unknown|sandbox",
            "unknown|allow",
            "unknown|allowfullscreen",
            "unknown|referrerpolicy",
            "unknown|csp",
            "unknown|fetchpriority",
        ],
    );

    schema
});

/// Registers a list of element|property specs with the given security context.
fn register_context(
    schema: &mut FxHashMap<&'static str, SecurityContext>,
    ctx: SecurityContext,
    specs: &[&'static str],
) {
    for spec in specs {
        schema.insert(spec, ctx);
    }
}

/// Gets the security context for a given element and property combination.
///
/// The lookup is case-insensitive. Returns `SecurityContext::None` if no
/// security-sensitive context is found.
///
/// # Arguments
/// * `element` - The element tag name (e.g., "iframe", "a", "script")
/// * `property` - The property name (e.g., "src", "href", "innerHTML")
///
/// # Returns
/// The appropriate `SecurityContext` for sanitization.
pub fn get_security_context(element: &str, property: &str) -> SecurityContext {
    let element_lower = element.to_ascii_lowercase();
    let property_lower = property.to_ascii_lowercase();

    // First try element-specific lookup
    let key = format!("{}|{}", element_lower, property_lower);
    if let Some(&ctx) = SECURITY_SCHEMA.get(key.as_str()) {
        return ctx;
    }

    // Then try wildcard lookup (properties that apply to all elements)
    let wildcard_key = format!("*|{}", property_lower);
    if let Some(&ctx) = SECURITY_SCHEMA.get(wildcard_key.as_str()) {
        return ctx;
    }

    // No security context needed
    SecurityContext::None
}

/// Calculates all possible security contexts for a property when the element is unknown.
///
/// This is used when the host element isn't known at compile time (e.g., for directives
/// that can be applied to multiple element types). The function checks all known elements
/// to find all possible security contexts for the given property.
///
/// # Special Cases
/// - If the property could have both `Url` and `ResourceUrl` contexts on different
///   elements, returns `UrlOrResourceUrl` (resolved at runtime based on element tag)
/// - If all elements have the same context, returns that context
/// - If no elements have a security context, returns `None`
///
/// # Arguments
/// * `property` - The property name (e.g., "src", "href")
///
/// # Returns
/// The appropriate `SecurityContext` for sanitization.
pub fn calc_security_context_for_unknown_element(property: &str) -> SecurityContext {
    let property_lower = property.to_ascii_lowercase();

    // Collect all security contexts for this property across all elements
    let mut has_url = false;
    let mut has_resource_url = false;
    let mut has_other = false;
    let mut other_context = SecurityContext::None;

    for (key, &ctx) in SECURITY_SCHEMA.iter() {
        // Check if this entry is for our property (format: "element|property")
        if let Some(pipe_pos) = key.find('|') {
            let prop = &key[pipe_pos + 1..];
            if prop.eq_ignore_ascii_case(&property_lower) {
                match ctx {
                    SecurityContext::Url => has_url = true,
                    SecurityContext::ResourceUrl => has_resource_url = true,
                    SecurityContext::None => {}
                    _ => {
                        has_other = true;
                        other_context = ctx;
                    }
                }
            }
        }
    }

    // Also check wildcard entries
    let wildcard_key = format!("*|{}", property_lower);
    if let Some(&ctx) = SECURITY_SCHEMA.get(wildcard_key.as_str()) {
        match ctx {
            SecurityContext::Url => has_url = true,
            SecurityContext::ResourceUrl => has_resource_url = true,
            SecurityContext::None => {}
            _ => {
                has_other = true;
                other_context = ctx;
            }
        }
    }

    // Determine the result based on what we found
    if has_url && has_resource_url {
        // Special case: property could be either URL or ResourceURL
        // depending on the element (e.g., "src" on <img> vs <script>)
        SecurityContext::UrlOrResourceUrl
    } else if has_url {
        SecurityContext::Url
    } else if has_resource_url {
        SecurityContext::ResourceUrl
    } else if has_other {
        other_context
    } else {
        SecurityContext::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_context() {
        assert_eq!(get_security_context("iframe", "srcdoc"), SecurityContext::Html);
        assert_eq!(get_security_context("div", "innerHTML"), SecurityContext::Html);
        assert_eq!(get_security_context("span", "outerHTML"), SecurityContext::Html);
    }

    #[test]
    fn test_style_context() {
        assert_eq!(get_security_context("div", "style"), SecurityContext::Style);
        assert_eq!(get_security_context("span", "style"), SecurityContext::Style);
    }

    #[test]
    fn test_url_context() {
        assert_eq!(get_security_context("a", "href"), SecurityContext::Url);
        assert_eq!(get_security_context("form", "action"), SecurityContext::Url);
        assert_eq!(get_security_context("img", "src"), SecurityContext::Url);
    }

    #[test]
    fn test_resource_url_context() {
        assert_eq!(get_security_context("script", "src"), SecurityContext::ResourceUrl);
        assert_eq!(get_security_context("iframe", "src"), SecurityContext::ResourceUrl);
        assert_eq!(get_security_context("embed", "src"), SecurityContext::ResourceUrl);
    }

    #[test]
    fn test_attribute_no_binding_context() {
        assert_eq!(
            get_security_context("animate", "attributeName"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(get_security_context("iframe", "sandbox"), SecurityContext::AttributeNoBinding);
    }

    #[test]
    fn test_no_context() {
        assert_eq!(get_security_context("div", "class"), SecurityContext::None);
        assert_eq!(get_security_context("input", "value"), SecurityContext::None);
    }

    #[test]
    fn test_case_insensitivity() {
        assert_eq!(get_security_context("IFRAME", "SRCDOC"), SecurityContext::Html);
        assert_eq!(get_security_context("Script", "Src"), SecurityContext::ResourceUrl);
    }

    #[test]
    fn test_unknown_element_url_or_resource_url() {
        // "src" can be either URL (img, video) or ResourceURL (script, embed, iframe, frame)
        assert_eq!(
            calc_security_context_for_unknown_element("src"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    #[test]
    fn test_unknown_element_just_url() {
        // "href" on most elements is URL (but "base|href" and "link|href" are ResourceURL)
        // So this should also be UrlOrResourceUrl
        assert_eq!(
            calc_security_context_for_unknown_element("href"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    #[test]
    fn test_unknown_element_html() {
        // "innerHTML" is always Html (wildcard)
        assert_eq!(calc_security_context_for_unknown_element("innerHTML"), SecurityContext::Html);
    }

    #[test]
    fn test_unknown_element_none() {
        // "class" has no security context
        assert_eq!(calc_security_context_for_unknown_element("class"), SecurityContext::None);
    }
}

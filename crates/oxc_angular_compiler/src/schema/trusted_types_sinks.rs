//! Trusted Types sinks
//!
//! Set of `tagName|propertyName` corresponding to Trusted Types sinks. Properties applying to all
//! tags use `*`.
//!
//! Ported from Angular's `schema/trusted_types_sinks.ts`. Extracted from, and should be kept in
//! sync with <https://www.w3.org/TR/trusted-types/#integrations>.
//!
//! DO NOT EDIT THIS LIST OF SECURITY SENSITIVE SINKS WITHOUT A SECURITY REVIEW!

use rustc_hash::FxHashSet;
use std::sync::LazyLock;

/// Set of `"tagName|propertyName"` Trusted Types sinks. Properties applying to all tags use `"*"`.
///
/// NOTE: All strings in this set *must* be lowercase!
static TRUSTED_TYPES_SINKS: LazyLock<FxHashSet<&'static str>> = LazyLock::new(|| {
    FxHashSet::from_iter([
        // TrustedHTML
        "iframe|srcdoc",
        "*|innerhtml",
        "*|outerhtml",
        // NB: no TrustedScript here, as the corresponding tags are stripped by the compiler.
        // TrustedScriptURL
        "embed|src",
        "iframe|src",
        "object|codebase",
        "object|data",
    ])
});

/// Returns `true` if the given property on the given DOM tag is a Trusted Types sink.
///
/// In that case, use [`crate::schema::get_security_context`] to determine which particular
/// Trusted Type is required for values passed to the sink:
/// - [`crate::ast::r3::SecurityContext::Html`] corresponds to `TrustedHTML`
/// - [`crate::ast::r3::SecurityContext::ResourceUrl`] corresponds to `TrustedScriptURL`
///
/// The lookup is case-insensitive, so that case differences between attribute and property names
/// do not have a security impact.
pub fn is_trusted_types_sink(tag_name: &str, prop_name: &str) -> bool {
    let tag = tag_name.to_ascii_lowercase();
    let prop = prop_name.to_ascii_lowercase();

    TRUSTED_TYPES_SINKS.contains(format!("{tag}|{prop}").as_str())
        || TRUSTED_TYPES_SINKS.contains(format!("*|{prop}").as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_html_sinks() {
        assert!(is_trusted_types_sink("iframe", "srcdoc"));
        assert!(is_trusted_types_sink("p", "innerHTML"));
        assert!(is_trusted_types_sink("div", "outerHTML"));
    }

    #[test]
    fn detects_resource_url_sinks() {
        assert!(is_trusted_types_sink("embed", "src"));
        assert!(is_trusted_types_sink("object", "codebase"));
        assert!(is_trusted_types_sink("object", "data"));
    }

    #[test]
    fn detects_iframe_src() {
        // Issue #315 sub-gap 1 / upstream commit 78dea55351: `iframe|src` is a sink.
        assert!(is_trusted_types_sink("iframe", "src"));
    }

    #[test]
    fn is_case_insensitive() {
        assert!(is_trusted_types_sink("IFRAME", "SRC"));
        assert!(is_trusted_types_sink("P", "iNnErHtMl"));
    }

    #[test]
    fn rejects_non_sinks() {
        assert!(!is_trusted_types_sink("a", "href"));
        assert!(!is_trusted_types_sink("base", "href"));
        assert!(!is_trusted_types_sink("div", "style"));
        // `img|src` is a navigable URL, not a Trusted Types sink.
        assert!(!is_trusted_types_sink("img", "src"));
        assert!(!is_trusted_types_sink("p", "formaction"));
    }
}

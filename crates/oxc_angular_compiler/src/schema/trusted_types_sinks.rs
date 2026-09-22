//! Trusted Types sinks.
//!
//! Ported from `@angular/compiler` `schema/trusted_types_sinks.ts`.
//! Tag and property names are lowercased. Namespace prefixes are not stripped:
//! `:svg:iframe|src` is not the `iframe|src` sink.

/// `tag|property` pairs that must not be rewritten by i18n.
/// Every entry is lowercase. `*` applies to every tag.
const TRUSTED_TYPES_SINKS: &[&str] = &[
    "iframe|srcdoc",
    "*|innerhtml",
    "*|outerhtml",
    "embed|src",
    "iframe|src",
    "object|codebase",
    "object|data",
];

/// Whether `prop_name` on `tag_name` is a Trusted Types sink.
pub fn is_trusted_types_sink(tag_name: &str, prop_name: &str) -> bool {
    let tag_name = tag_name.to_ascii_lowercase();
    let prop_name = prop_name.to_ascii_lowercase();
    let specific = format!("{tag_name}|{prop_name}");
    let wildcard = format!("*|{prop_name}");
    TRUSTED_TYPES_SINKS.iter().any(|sink| *sink == specific || *sink == wildcard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iframe_src_is_a_sink() {
        assert!(is_trusted_types_sink("iframe", "src"));
        assert!(is_trusted_types_sink("IFRAME", "SRC"));
        assert!(is_trusted_types_sink("div", "innerHTML"));
    }

    #[test]
    fn namespaced_tag_is_not_rewritten_to_the_local_name() {
        assert!(!is_trusted_types_sink(":svg:iframe", "src"));
        assert!(!is_trusted_types_sink("div", "title"));
    }
}

//! HTML tag definitions.
//!
//! Ported from Angular's `ml_parser/html_tags.ts` and `ml_parser/tags.ts`.

use crate::schema::is_known_element;

/// Content type for HTML elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagContentType {
    /// Raw text content (script, style).
    RawText,
    /// Escapable raw text (textarea, title).
    EscapableRawText,
    /// Parsable content (most elements).
    Parsable,
}

/// Namespace-aware content type.
/// Some elements like `<title>` have different content types depending on namespace.
#[derive(Debug, Clone, Copy)]
pub enum ContentType {
    /// Simple content type that applies to all namespaces.
    Static(TagContentType),
    /// Title element: ESCAPABLE_RAW_TEXT in HTML, PARSABLE in SVG.
    Title,
}

impl ContentType {
    /// Get the content type for the given namespace prefix.
    pub fn get_content_type(&self, prefix: Option<&str>) -> TagContentType {
        match self {
            ContentType::Static(ct) => *ct,
            ContentType::Title => {
                // SVGTitleElement uses PARSABLE, HTMLTitleElement uses ESCAPABLE_RAW_TEXT
                if prefix == Some("svg") {
                    TagContentType::Parsable
                } else {
                    TagContentType::EscapableRawText
                }
            }
        }
    }
}

/// Tag definition for an HTML element.
#[derive(Debug, Clone)]
pub struct HtmlTagDefinition {
    /// Content type (may be namespace-dependent).
    pub content_type: ContentType,
    /// Whether this is a void element.
    pub is_void: bool,
    /// Whether the first line feed should be ignored.
    pub ignore_first_lf: bool,
    /// Whether this element can be implicitly closed by its parent.
    pub closed_by_parent: bool,
    /// Tags that implicitly close this element.
    pub closed_by_children: &'static [&'static str],
    /// Implicit namespace prefix (e.g., "svg" for svg elements).
    pub implicit_namespace_prefix: Option<&'static str>,
    /// Whether this element can self-close (e.g., `<div/>`).
    pub can_self_close: bool,
    /// Whether children should not inherit this element's namespace.
    pub prevent_namespace_inheritance: bool,
}

impl Default for HtmlTagDefinition {
    fn default() -> Self {
        Self {
            content_type: ContentType::Static(TagContentType::Parsable),
            is_void: false,
            ignore_first_lf: false,
            closed_by_parent: false,
            closed_by_children: &[],
            implicit_namespace_prefix: None,
            can_self_close: false,
            prevent_namespace_inheritance: false,
        }
    }
}

impl HtmlTagDefinition {
    /// Check if this element is closed by the given child tag.
    pub fn is_closed_by_child(&self, name: &str) -> bool {
        if self.is_void {
            return true;
        }
        let lower = name.to_lowercase();
        self.closed_by_children.iter().any(|&child| child == lower)
    }

    /// Get the content type for the given namespace prefix.
    pub fn get_content_type(&self, prefix: Option<&str>) -> TagContentType {
        self.content_type.get_content_type(prefix)
    }
}

/// Default tag definition for unknown tags (canSelfClose: true).
fn default_tag_definition() -> HtmlTagDefinition {
    HtmlTagDefinition { can_self_close: true, ..Default::default() }
}

/// Returns the tag definition for the given tag name.
pub fn get_html_tag_definition(tag_name: &str) -> HtmlTagDefinition {
    // Check case-sensitive first (for SVG), then case-insensitive
    match tag_name {
        "foreignObject" => {
            return HtmlTagDefinition {
                // Usually the implicit namespace here would be redundant since it will be inherited
                // from the parent `svg`, but we have to do it for `foreignObject`, because the way
                // the parser works is that the parent node of an end tag is its own start tag which
                // means that the `preventNamespaceInheritance` on `foreignObject` would have it
                // default to the implicit namespace which is `html`, unless specified otherwise.
                implicit_namespace_prefix: Some("svg"),
                // We want to prevent children of foreignObject from inheriting its namespace,
                // because the point of the element is to allow nodes from other namespaces.
                prevent_namespace_inheritance: true,
                ..Default::default()
            };
        }
        _ => {}
    }

    let lower = tag_name.to_lowercase();
    match lower.as_str() {
        // Void elements (isVoid: true implies closedByParent: true, canSelfClose: true)
        "base" | "meta" | "area" | "embed" | "link" | "img" | "input" | "param" | "hr" | "br"
        | "source" | "track" | "wbr" | "col" => HtmlTagDefinition {
            is_void: true,
            closed_by_parent: true,
            can_self_close: true,
            ..Default::default()
        },

        // <p> element
        "p" => HtmlTagDefinition {
            closed_by_children: &[
                "address",
                "article",
                "aside",
                "blockquote",
                "div",
                "dl",
                "fieldset",
                "footer",
                "form",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "header",
                "hgroup",
                "hr",
                "main",
                "nav",
                "ol",
                "p",
                "pre",
                "section",
                "table",
                "ul",
            ],
            closed_by_parent: true,
            ..Default::default()
        },

        // Table section elements
        "thead" => {
            HtmlTagDefinition { closed_by_children: &["tbody", "tfoot"], ..Default::default() }
        }
        "tbody" => HtmlTagDefinition {
            closed_by_children: &["tbody", "tfoot"],
            closed_by_parent: true,
            ..Default::default()
        },
        "tfoot" => HtmlTagDefinition {
            closed_by_children: &["tbody"],
            closed_by_parent: true,
            ..Default::default()
        },
        "tr" => HtmlTagDefinition {
            closed_by_children: &["tr"],
            closed_by_parent: true,
            ..Default::default()
        },
        "td" => HtmlTagDefinition {
            closed_by_children: &["td", "th"],
            closed_by_parent: true,
            ..Default::default()
        },
        "th" => HtmlTagDefinition {
            closed_by_children: &["td", "th"],
            closed_by_parent: true,
            ..Default::default()
        },

        // SVG and MathML namespace elements
        "svg" => HtmlTagDefinition { implicit_namespace_prefix: Some("svg"), ..Default::default() },
        "math" => {
            HtmlTagDefinition { implicit_namespace_prefix: Some("math"), ..Default::default() }
        }

        // List elements
        "li" => HtmlTagDefinition {
            closed_by_children: &["li"],
            closed_by_parent: true,
            ..Default::default()
        },

        // Definition list elements
        "dt" => HtmlTagDefinition {
            closed_by_children: &["dt", "dd"],
            // Note: dt does NOT have closedByParent in Angular
            ..Default::default()
        },
        "dd" => HtmlTagDefinition {
            closed_by_children: &["dt", "dd"],
            closed_by_parent: true,
            ..Default::default()
        },

        // Ruby elements
        "rb" => HtmlTagDefinition {
            closed_by_children: &["rb", "rt", "rtc", "rp"],
            closed_by_parent: true,
            ..Default::default()
        },
        "rt" => HtmlTagDefinition {
            closed_by_children: &["rb", "rt", "rtc", "rp"],
            closed_by_parent: true,
            ..Default::default()
        },
        "rtc" => HtmlTagDefinition {
            // Note: rtc does NOT have "rt" in closedByChildren
            closed_by_children: &["rb", "rtc", "rp"],
            closed_by_parent: true,
            ..Default::default()
        },
        "rp" => HtmlTagDefinition {
            closed_by_children: &["rb", "rt", "rtc", "rp"],
            closed_by_parent: true,
            ..Default::default()
        },

        // Select elements
        "optgroup" => HtmlTagDefinition {
            closed_by_children: &["optgroup"],
            closed_by_parent: true,
            ..Default::default()
        },
        "option" => HtmlTagDefinition {
            closed_by_children: &["option", "optgroup"],
            closed_by_parent: true,
            ..Default::default()
        },

        // Preformatted text
        "pre" | "listing" => HtmlTagDefinition { ignore_first_lf: true, ..Default::default() },

        // Raw text elements
        "style" | "script" => HtmlTagDefinition {
            content_type: ContentType::Static(TagContentType::RawText),
            ..Default::default()
        },

        // Title has namespace-dependent content type
        "title" => HtmlTagDefinition { content_type: ContentType::Title, ..Default::default() },

        // Textarea
        "textarea" => HtmlTagDefinition {
            content_type: ContentType::Static(TagContentType::EscapableRawText),
            ignore_first_lf: true,
            ..Default::default()
        },

        // The other elements of Angular's DOM schema. Like `html_tags.ts`, every name of
        // `allKnownElementNames()` without a namespace gets `canSelfClose: false`, so `<div/>`
        // is an error.
        _ if !lower.starts_with(':') && is_known_element(&lower) => HtmlTagDefinition::default(),

        // Default: unknown tags get canSelfClose: true
        _ => default_tag_definition(),
    }
}

/// Splits a name in `:namespace:name` format into namespace and local parts.
///
/// Angular uses `:svg:rect` format internally for namespaced elements.
/// Returns `(None, name)` if the name doesn't start with `:`.
pub fn split_ns_name(element_name: &str) -> (Option<&str>, &str) {
    if !element_name.starts_with(':') {
        return (None, element_name);
    }

    // Find the second colon
    if let Some(colon_index) = element_name[1..].find(':') {
        let namespace = &element_name[1..colon_index + 1];
        let local_name = &element_name[colon_index + 2..];
        (Some(namespace), local_name)
    } else {
        // Malformed: only one colon, treat as no namespace
        (None, element_name)
    }
}

/// Merges a namespace prefix and local name into `:namespace:name` format.
///
/// Angular uses `:svg:rect` format internally for namespaced elements.
pub fn merge_ns_and_name(prefix: Option<&str>, local_name: &str) -> String {
    match prefix {
        Some(p) if !p.is_empty() => format!(":{p}:{local_name}"),
        _ => local_name.to_string(),
    }
}

/// Returns the namespace prefix from a `:namespace:name` formatted string.
pub fn get_ns_prefix(full_name: &str) -> Option<&str> {
    split_ns_name(full_name).0
}

/// Returns whether the given tag name is a void element.
pub fn is_void_element(name: &str) -> bool {
    get_html_tag_definition(name).is_void
}

/// Returns the namespace URI for the given namespace key.
pub fn namespace_uri(namespace_key: &str) -> Option<&'static str> {
    match namespace_key {
        "svg" => Some("http://www.w3.org/2000/svg"),
        "xlink" => Some("http://www.w3.org/1999/xlink"),
        "xml" => Some("http://www.w3.org/XML/1998/namespace"),
        "xmlns" => Some("http://www.w3.org/2000/xmlns/"),
        "math" => Some("http://www.w3.org/1998/Math/MathML"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_void_elements() {
        assert!(get_html_tag_definition("br").is_void);
        assert!(get_html_tag_definition("input").is_void);
        assert!(!get_html_tag_definition("div").is_void);
    }

    #[test]
    fn test_void_elements_have_can_self_close() {
        let br = get_html_tag_definition("br");
        assert!(br.is_void);
        assert!(br.can_self_close);
        assert!(br.closed_by_parent);
    }

    #[test]
    fn test_raw_text() {
        assert_eq!(
            get_html_tag_definition("script").get_content_type(None),
            TagContentType::RawText
        );
        assert_eq!(
            get_html_tag_definition("style").get_content_type(None),
            TagContentType::RawText
        );
    }

    #[test]
    fn test_title_namespace_content_type() {
        // HTML title is escapable raw text
        assert_eq!(
            get_html_tag_definition("title").get_content_type(None),
            TagContentType::EscapableRawText
        );
        // SVG title is parsable (can contain child elements)
        assert_eq!(
            get_html_tag_definition("title").get_content_type(Some("svg")),
            TagContentType::Parsable
        );
    }

    #[test]
    fn test_split_ns_name() {
        // Angular format: :namespace:name
        assert_eq!(split_ns_name(":svg:rect"), (Some("svg"), "rect"));
        assert_eq!(split_ns_name(":xlink:href"), (Some("xlink"), "href"));
        assert_eq!(split_ns_name("div"), (None, "div"));
        assert_eq!(split_ns_name(":malformed"), (None, ":malformed"));
    }

    #[test]
    fn test_merge_ns_and_name() {
        assert_eq!(merge_ns_and_name(Some("svg"), "rect"), ":svg:rect");
        assert_eq!(merge_ns_and_name(Some("xlink"), "href"), ":xlink:href");
        assert_eq!(merge_ns_and_name(None, "div"), "div");
        assert_eq!(merge_ns_and_name(Some(""), "div"), "div");
    }

    #[test]
    fn test_foreign_object() {
        let def = get_html_tag_definition("foreignObject");
        assert_eq!(def.implicit_namespace_prefix, Some("svg"));
        assert!(def.prevent_namespace_inheritance);
    }

    #[test]
    fn test_is_closed_by_child() {
        let p = get_html_tag_definition("p");
        assert!(p.is_closed_by_child("div"));
        assert!(p.is_closed_by_child("p"));
        assert!(!p.is_closed_by_child("span"));
    }

    #[test]
    fn test_dt_dd_closed_by_parent() {
        // dt does NOT have closedByParent
        assert!(!get_html_tag_definition("dt").closed_by_parent);
        // dd DOES have closedByParent
        assert!(get_html_tag_definition("dd").closed_by_parent);
    }

    #[test]
    fn test_rtc_closed_by_children() {
        let rtc = get_html_tag_definition("rtc");
        // rtc does NOT have "rt" in closedByChildren
        assert!(rtc.is_closed_by_child("rb"));
        assert!(rtc.is_closed_by_child("rtc"));
        assert!(rtc.is_closed_by_child("rp"));
        assert!(!rtc.is_closed_by_child("rt")); // rt is NOT in the list
    }

    #[test]
    fn test_table_elements() {
        // thead
        let thead = get_html_tag_definition("thead");
        assert!(!thead.closed_by_parent);
        assert!(thead.is_closed_by_child("tbody"));
        assert!(thead.is_closed_by_child("tfoot"));
        assert!(!thead.is_closed_by_child("thead"));

        // tbody
        let tbody = get_html_tag_definition("tbody");
        assert!(tbody.closed_by_parent);

        // tfoot
        let tfoot = get_html_tag_definition("tfoot");
        assert!(tfoot.closed_by_parent);
        assert!(tfoot.is_closed_by_child("tbody"));
        assert!(!tfoot.is_closed_by_child("tfoot"));

        // tr
        let tr = get_html_tag_definition("tr");
        assert!(tr.closed_by_parent);

        // td/th
        assert!(get_html_tag_definition("td").closed_by_parent);
        assert!(get_html_tag_definition("th").closed_by_parent);
    }

    #[test]
    fn test_unknown_tags_can_self_close() {
        let custom = get_html_tag_definition("my-custom-element");
        assert!(custom.can_self_close);
    }

    #[test]
    fn test_known_dom_elements_cannot_self_close() {
        for name in ["div", "span", "button", "section", "table", "DIV", "video", "slot"] {
            assert!(!get_html_tag_definition(name).can_self_close, "{name}");
        }
    }
}

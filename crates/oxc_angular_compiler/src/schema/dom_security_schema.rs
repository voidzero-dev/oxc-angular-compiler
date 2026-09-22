//! DOM Security Schema
//!
//! Maps `element|property` pairs to the sanitizer Angular applies.
//! Ported from `@angular/compiler` v22 `schema/dom_security_schema.ts`.
//!
//! DO NOT EDIT THIS LIST OF SECURITY SENSITIVE PROPERTIES WITHOUT A SECURITY REVIEW!

use std::sync::LazyLock;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast::r3::SecurityContext;
use crate::parser::html::split_ns_name;
use crate::pipeline::selector::CssSelector;

/// Security schema mapping `"element|property"` to `SecurityContext`.
///
/// Keys follow `registerContext` in `dom_security_schema.ts`: an `svg` or `math`
/// namespace is stored as `:svg:tag|attr` / `:math:tag|attr`. `*` and `unknown`
/// stay un-namespaced. Lookup lowercases both sides.
static SECURITY_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> = LazyLock::new(|| {
    let mut schema = FxHashMap::default();

    register(
        &mut schema,
        SecurityContext::Html,
        None,
        &[("iframe", &["srcdoc"]), ("*", &["innerHTML", "outerHTML"])],
    );
    register(&mut schema, SecurityContext::Style, None, &[("*", &["style"])]);

    // No SCRIPT contexts: the parser strips `<script>` and `:svg:script`.
    register(
        &mut schema,
        SecurityContext::Url,
        None,
        &[
            ("*", &["formAction"]),
            ("area", &["href"]),
            ("a", &["href", "xlink:href"]),
            ("form", &["action"]),
            // Kept for compatibility with upstream; safe in practice.
            ("img", &["src"]),
            ("video", &["src"]),
        ],
    );

    register_uniform(
        &mut schema,
        SecurityContext::Url,
        Some("math"),
        MATHML_URL_ELEMENTS,
        &["href", "xlink:href"],
    );

    register(
        &mut schema,
        SecurityContext::ResourceUrl,
        None,
        &[
            ("base", &["href"]),
            ("embed", &["src"]),
            ("frame", &["src"]),
            ("iframe", &["src"]),
            ("link", &["href"]),
            ("object", &["codebase", "data"]),
        ],
    );

    register(&mut schema, SecurityContext::Url, Some("svg"), &[("a", &["href", "xlink:href"])]);

    // SVG animation value attributes can retarget `href` / `xlink:href`.
    // Upstream registers them under the SVG namespace as ATTRIBUTE_NO_BINDING.
    register(
        &mut schema,
        SecurityContext::AttributeNoBinding,
        Some("svg"),
        &[
            ("animate", &["attributeName", "values", "to", "from"]),
            ("set", &["to", "attributeName"]),
            ("animateMotion", &["attributeName"]),
            ("animateTransform", &["attributeName"]),
        ],
    );

    register(
        &mut schema,
        SecurityContext::AttributeNoBinding,
        None,
        &[
            (
                "unknown",
                &[
                    "attributeName",
                    "values",
                    "to",
                    "from",
                    "sandbox",
                    "allow",
                    "allowFullscreen",
                    "referrerPolicy",
                    "csp",
                    "fetchPriority",
                ],
            ),
            (
                "iframe",
                &["sandbox", "allow", "allowFullscreen", "referrerPolicy", "csp", "fetchPriority"],
            ),
        ],
    );

    schema
});

/// MathML elements whose `href` / `xlink:href` are URL sinks in the security schema.
/// `annotation`, `malignmark`, `mglyph`, `mprescripts`, and `none` are not in the
/// DOM element schema, so the host-element scan never reaches them. Template lookup
/// still hits the `:math:` key when the parser names the element that way.
const MATHML_URL_ELEMENTS: &[&str] = &[
    "annotation",
    "annotation-xml",
    "maction",
    "malignmark",
    "math",
    "mroot",
    "msqrt",
    "merror",
    "mfrac",
    "mglyph",
    "msub",
    "msup",
    "msubsup",
    "mmultiscripts",
    "mprescripts",
    "mi",
    "mn",
    "mo",
    "mpadded",
    "mphantom",
    "mrow",
    "ms",
    "mspace",
    "mstyle",
    "mtable",
    "mtd",
    "mtr",
    "mtext",
    "mover",
    "munder",
    "munderover",
    "semantics",
    "none",
];

/// Lowercased tag names from `DomElementSchemaRegistry`'s `SCHEMA` array.
/// This is `allKnownElementNames()` for the host-binding scan.
const KNOWN_ELEMENT_NAMES: &[&str] = &[
    "[element]",
    "[htmlelement]",
    "abbr",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "cite",
    "content",
    "code",
    "dd",
    "dfn",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "i",
    "kbd",
    "main",
    "mark",
    "nav",
    "noscript",
    "rb",
    "rp",
    "rt",
    "rtc",
    "ruby",
    "s",
    "samp",
    "search",
    "section",
    "small",
    "strong",
    "sub",
    "sup",
    "u",
    "var",
    "wbr",
    "media",
    ":svg:",
    ":svg:graphics",
    ":svg:animation",
    ":svg:geometry",
    ":svg:componenttransferfunction",
    ":svg:gradient",
    ":svg:textcontent",
    ":svg:textpositioning",
    "a",
    "area",
    "audio",
    "br",
    "base",
    "body",
    "button",
    "canvas",
    "dl",
    "data",
    "datalist",
    "details",
    "dialog",
    "dir",
    "div",
    "embed",
    "fieldset",
    "font",
    "form",
    "frame",
    "frameset",
    "geolocation",
    "hr",
    "head",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "html",
    "iframe",
    "img",
    "input",
    "li",
    "label",
    "legend",
    "link",
    "map",
    "marquee",
    "menu",
    "meta",
    "meter",
    "ins",
    "del",
    "ol",
    "object",
    "optgroup",
    "option",
    "output",
    "p",
    "param",
    "picture",
    "pre",
    "progress",
    "q",
    "blockquote",
    "script",
    "select",
    "selectedcontent",
    "slot",
    "source",
    "span",
    "style",
    "caption",
    "th",
    "td",
    "col",
    "colgroup",
    "table",
    "tr",
    "tfoot",
    "thead",
    "tbody",
    "template",
    "textarea",
    "time",
    "title",
    "track",
    "ul",
    "unknown",
    "video",
    ":svg:a",
    ":svg:animate",
    ":svg:animatemotion",
    ":svg:animatetransform",
    ":svg:circle",
    ":svg:clippath",
    ":svg:defs",
    ":svg:desc",
    ":svg:discard",
    ":svg:ellipse",
    ":svg:feblend",
    ":svg:fecolormatrix",
    ":svg:fecomponenttransfer",
    ":svg:fecomposite",
    ":svg:feconvolvematrix",
    ":svg:fediffuselighting",
    ":svg:fedisplacementmap",
    ":svg:fedistantlight",
    ":svg:fedropshadow",
    ":svg:feflood",
    ":svg:fefunca",
    ":svg:fefuncb",
    ":svg:fefuncg",
    ":svg:fefuncr",
    ":svg:fegaussianblur",
    ":svg:feimage",
    ":svg:femerge",
    ":svg:femergenode",
    ":svg:femorphology",
    ":svg:feoffset",
    ":svg:fepointlight",
    ":svg:fespecularlighting",
    ":svg:fespotlight",
    ":svg:fetile",
    ":svg:feturbulence",
    ":svg:filter",
    ":svg:foreignobject",
    ":svg:g",
    ":svg:image",
    ":svg:line",
    ":svg:lineargradient",
    ":svg:mpath",
    ":svg:marker",
    ":svg:mask",
    ":svg:metadata",
    ":svg:path",
    ":svg:pattern",
    ":svg:polygon",
    ":svg:polyline",
    ":svg:radialgradient",
    ":svg:rect",
    ":svg:svg",
    ":svg:script",
    ":svg:set",
    ":svg:stop",
    ":svg:style",
    ":svg:switch",
    ":svg:symbol",
    ":svg:tspan",
    ":svg:text",
    ":svg:textpath",
    ":svg:title",
    ":svg:use",
    ":svg:view",
    "keygen",
    "menuitem",
    "summary",
    ":svg:cursor",
    ":math:",
    ":math:math",
    ":math:maction",
    ":math:menclose",
    ":math:merror",
    ":math:mfenced",
    ":math:mfrac",
    ":math:mi",
    ":math:mmultiscripts",
    ":math:mn",
    ":math:mo",
    ":math:mover",
    ":math:mpadded",
    ":math:mphantom",
    ":math:mroot",
    ":math:mrow",
    ":math:ms",
    ":math:mspace",
    ":math:msqrt",
    ":math:mstyle",
    ":math:msub",
    ":math:msubsup",
    ":math:msup",
    ":math:mtable",
    ":math:mtd",
    ":math:mtext",
    ":math:mtr",
    ":math:munder",
    ":math:munderover",
    ":math:semantics",
];

static KNOWN_ELEMENT_SET: LazyLock<FxHashSet<&'static str>> =
    LazyLock::new(|| KNOWN_ELEMENT_NAMES.iter().copied().collect());

fn register(
    schema: &mut FxHashMap<String, SecurityContext>,
    ctx: SecurityContext,
    namespace: Option<&str>,
    specs: &[(&str, &[&str])],
) {
    for &(element, attrs) in specs {
        register_uniform(schema, ctx, namespace, &[element], attrs);
    }
}

fn register_uniform(
    schema: &mut FxHashMap<String, SecurityContext>,
    ctx: SecurityContext,
    namespace: Option<&str>,
    elements: &[&str],
    attrs: &[&str],
) {
    for element in elements {
        let tag = match namespace {
            Some(ns) if *element != "*" && *element != "unknown" => {
                format!(":{ns}:{element}").to_ascii_lowercase()
            }
            _ => (*element).to_ascii_lowercase(),
        };
        for attr in attrs {
            schema.insert(format!("{tag}|{}", attr.to_ascii_lowercase()), ctx);
        }
    }
}

fn is_known_element(name: &str) -> bool {
    KNOWN_ELEMENT_SET.contains(name.to_ascii_lowercase().as_str())
}

/// `normalizeTagName` from `dom_element_schema_registry.ts`.
/// `:svg:` and `:math:` are kept; every other namespace is stripped.
fn normalize_tag_name(tag_name: &str) -> String {
    let lower = tag_name.to_ascii_lowercase();
    let (ns, name) = split_ns_name(&lower);
    match ns {
        Some(ns @ ("svg" | "math")) => format!(":{ns}:{name}"),
        _ => name.to_string(),
    }
}

/// Security context for one element and property.
///
/// Case-insensitive. Returns `SecurityContext::None` when the pair is not a sink.
pub fn get_security_context(element: &str, property: &str) -> SecurityContext {
    let tag = normalize_tag_name(element);
    let property_lower = property.to_ascii_lowercase();

    let key = format!("{tag}|{property_lower}");
    if let Some(&ctx) = SECURITY_SCHEMA.get(&key) {
        return ctx;
    }

    let wildcard_key = format!("*|{property_lower}");
    if let Some(&ctx) = SECURITY_SCHEMA.get(&wildcard_key) {
        return ctx;
    }

    SecurityContext::None
}

/// Union of security contexts for `property` across every known element.
///
/// `Url` together with `ResourceUrl` collapses to `UrlOrResourceUrl`.
pub fn calc_security_context_for_unknown_element(property: &str) -> SecurityContext {
    host_binding_security_context("", property)
}

/// Security context of a host binding.
///
/// Mirrors `calcPossibleSecurityContexts` plus the host ingest filter that drops
/// `NONE` and the `{URL, RESOURCE_URL}` pair in `resolve_sanitizers.ts`.
/// `style` / `class` / animation bindings are classified by the caller; this
/// function is the element-selector lookup for attribute and property bindings.
pub fn host_binding_security_context(selector: &str, prop_name: &str) -> SecurityContext {
    reduce_security_contexts(&collect_security_contexts(selector, prop_name))
}

fn collect_security_contexts(selector: &str, prop_name: &str) -> Vec<SecurityContext> {
    let selector = selector.trim();
    if selector.is_empty() {
        return KNOWN_ELEMENT_NAMES.iter().map(|el| get_security_context(el, prop_name)).collect();
    }

    let (namespace_key, base_selector) = split_ns_name(selector);
    let mut contexts = Vec::new();
    for css in CssSelector::parse(base_selector) {
        let excluded = not_element_names(&css);
        let element_names: Vec<String> = if let Some(element) = &css.element {
            resolve_concrete_element(element)
        } else {
            KNOWN_ELEMENT_NAMES.iter().map(|name| (*name).to_string()).collect()
        };

        for element_name in element_names {
            if is_not_excluded(&element_name, &excluded) {
                let full_name = qualify_with_selector_namespace(&element_name, namespace_key);
                contexts.push(get_security_context(&full_name, prop_name));
            }
        }
    }
    contexts
}

/// A bare selector element that is not in the DOM schema is rewritten to
/// `:svg:name` or `:math:name` when that namespaced element exists.
fn resolve_concrete_element(element: &str) -> Vec<String> {
    if element == "*" || is_known_element(element) {
        return vec![element.to_string()];
    }
    let lower = element.to_ascii_lowercase();
    let svg = format!(":svg:{lower}");
    let math = format!(":math:{lower}");
    if is_known_element(&svg) {
        vec![svg]
    } else if is_known_element(&math) {
        vec![math]
    } else {
        vec![element.to_string()]
    }
}

fn qualify_with_selector_namespace(element_name: &str, namespace_key: Option<&str>) -> String {
    let lower = element_name.to_ascii_lowercase();
    let (own_ns, name) = split_ns_name(&lower);
    let ns =
        own_ns.map(str::to_ascii_lowercase).or_else(|| namespace_key.map(str::to_ascii_lowercase));
    match ns {
        Some(ns) if !ns.is_empty() => format!(":{ns}:{name}"),
        _ => name.to_string(),
    }
}

fn not_element_names(css: &CssSelector) -> FxHashSet<String> {
    css.not_selectors
        .iter()
        .filter(|sel| {
            sel.element.is_some()
                && sel.class_names.is_empty()
                && sel.attrs.is_empty()
                && sel.not_selectors.is_empty()
        })
        .filter_map(|sel| sel.element.as_ref().map(|name| name.to_ascii_lowercase()))
        .collect()
}

fn is_not_excluded(element_name: &str, excluded: &FxHashSet<String>) -> bool {
    let lower = element_name.to_ascii_lowercase();
    if excluded.contains(&lower) {
        return false;
    }
    let local = split_ns_name(&lower).1.to_ascii_lowercase();
    !excluded.contains(&local)
}

fn reduce_security_contexts(contexts: &[SecurityContext]) -> SecurityContext {
    let mut present = Vec::new();
    for ctx in contexts {
        match *ctx {
            SecurityContext::None => {}
            SecurityContext::UrlOrResourceUrl => {
                push_unique(&mut present, SecurityContext::Url);
                push_unique(&mut present, SecurityContext::ResourceUrl);
            }
            other => push_unique(&mut present, other),
        }
    }

    let has_url = present.contains(&SecurityContext::Url);
    let has_resource_url = present.contains(&SecurityContext::ResourceUrl);
    match present.as_slice() {
        [] => SecurityContext::None,
        [single] => *single,
        _ if has_url && has_resource_url && present.len() == 2 => SecurityContext::UrlOrResourceUrl,
        // Upstream throws on any other mix. Those mixes are not produced by the
        // current schema; keep the lowest enum value so compilation still finishes.
        many => many
            .iter()
            .copied()
            .min_by_key(|ctx| security_context_rank(*ctx))
            .unwrap_or(SecurityContext::None),
    }
}

fn push_unique(contexts: &mut Vec<SecurityContext>, ctx: SecurityContext) {
    if !contexts.contains(&ctx) {
        contexts.push(ctx);
    }
}

fn security_context_rank(ctx: SecurityContext) -> u8 {
    match ctx {
        SecurityContext::None => 0,
        SecurityContext::Html => 1,
        SecurityContext::Style => 2,
        SecurityContext::Script => 3,
        SecurityContext::Url => 4,
        SecurityContext::ResourceUrl => 5,
        SecurityContext::AttributeNoBinding => 6,
        SecurityContext::UrlOrResourceUrl => 7,
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
        assert_eq!(get_security_context(":svg:a", "href"), SecurityContext::Url);
        assert_eq!(get_security_context(":math:mi", "href"), SecurityContext::Url);
        // Bare MathML local names are not the schema key.
        assert_eq!(get_security_context("mi", "href"), SecurityContext::None);
    }

    #[test]
    fn test_resource_url_context() {
        assert_eq!(get_security_context("iframe", "src"), SecurityContext::ResourceUrl);
        assert_eq!(get_security_context("embed", "src"), SecurityContext::ResourceUrl);
        // v22 dropped `script|src`. Script elements are stripped before binding.
        assert_eq!(get_security_context("script", "src"), SecurityContext::None);
    }

    #[test]
    fn test_svg_animation_attribute_no_binding() {
        assert_eq!(get_security_context(":svg:animate", "to"), SecurityContext::AttributeNoBinding);
        assert_eq!(
            get_security_context(":svg:animate", "from"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context(":svg:animate", "values"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context(":svg:animate", "attributeName"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(get_security_context(":svg:set", "to"), SecurityContext::AttributeNoBinding);
        // The un-namespaced key is not registered. HTML `<animate>` is not the SVG element.
        assert_eq!(get_security_context("animate", "to"), SecurityContext::None);
        assert_eq!(get_security_context("animate", "attributeName"), SecurityContext::None);
    }

    #[test]
    fn test_attribute_no_binding_context() {
        assert_eq!(get_security_context("iframe", "sandbox"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("unknown", "to"), SecurityContext::AttributeNoBinding);
    }

    #[test]
    fn test_no_context() {
        assert_eq!(get_security_context("div", "class"), SecurityContext::None);
        assert_eq!(get_security_context("input", "value"), SecurityContext::None);
    }

    #[test]
    fn test_case_insensitivity() {
        assert_eq!(get_security_context("IFRAME", "SRCDOC"), SecurityContext::Html);
        assert_eq!(get_security_context(":SVG:Animate", "TO"), SecurityContext::AttributeNoBinding);
    }

    #[test]
    fn test_unknown_element_url_or_resource_url() {
        assert_eq!(
            calc_security_context_for_unknown_element("src"),
            SecurityContext::UrlOrResourceUrl
        );
        assert_eq!(
            calc_security_context_for_unknown_element("href"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    #[test]
    fn test_unknown_element_html() {
        assert_eq!(calc_security_context_for_unknown_element("innerHTML"), SecurityContext::Html);
    }

    #[test]
    fn test_unknown_element_none() {
        assert_eq!(calc_security_context_for_unknown_element("class"), SecurityContext::None);
    }

    #[test]
    fn test_unknown_element_svg_animation_values() {
        assert_eq!(
            calc_security_context_for_unknown_element("to"),
            SecurityContext::AttributeNoBinding
        );
    }

    #[test]
    fn test_host_selector_promotes_svg_animate() {
        assert_eq!(
            host_binding_security_context("animate", "to"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(host_binding_security_context("a", "href"), SecurityContext::Url);
        assert_eq!(host_binding_security_context("img", "src"), SecurityContext::Url);
        assert_eq!(host_binding_security_context("iframe", "src"), SecurityContext::ResourceUrl);
    }

    #[test]
    fn test_host_not_selector_drops_the_only_contributor() {
        assert_eq!(host_binding_security_context("[x]:not(object)", "data"), SecurityContext::None);
        assert_eq!(
            host_binding_security_context("[x]:not(img):not(video)", "src"),
            SecurityContext::ResourceUrl
        );
    }

    #[test]
    fn test_host_comma_selector_merges_url_kinds() {
        assert_eq!(
            host_binding_security_context("a, base", "href"),
            SecurityContext::UrlOrResourceUrl
        );
    }
}

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

/// Which Angular security schema a compilation is targeting.
///
/// `None` means the latest schema (v22). The v22 schema namespaces SVG and
/// MathML keys. Earlier versions store bare `tag|attr` keys and gained the
/// SVG animation and Trusted Types entries on the v21 line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemaKind {
    /// Before 21.1. `script|src` is a resource URL. No SVG animation sinks.
    Legacy,
    /// 21.1.0 through 21.2.6. Adds `script|href`, MathML hrefs, and
    /// `attributeName` no-binding keys. No animation value attributes yet.
    V21_1,
    /// 21.2.7 through 21.x. Animation `to` / `from` / `values` are bare keys.
    V21_2_7,
    /// 22+. Namespaced keys. `script|src` and `script|href` are gone.
    V22,
}

struct SecurityProfile {
    kind: SchemaKind,
    /// v22 `normalizeTagName` keeps `:svg:` and `:math:`.
    namespaced: bool,
    /// v22 preparser strips `:svg:script` as well as `script`.
    strip_svg_script: bool,
    /// `iframe|src` joined Trusted Types sinks in 21.2.4.
    iframe_src_i18n: bool,
}

fn security_profile(version: Option<crate::AngularVersion>) -> SecurityProfile {
    let Some(version) = version else {
        return v22_profile();
    };
    if version.major >= 22 {
        return v22_profile();
    }
    let on_21 = version.major == 21;
    let v21_1 = on_21 && version.minor >= 1;
    let v21_2_4 = on_21 && (version.minor > 2 || (version.minor == 2 && version.patch >= 4));
    let v21_2_7 = on_21 && (version.minor > 2 || (version.minor == 2 && version.patch >= 7));
    let kind = if v21_2_7 {
        SchemaKind::V21_2_7
    } else if v21_1 {
        SchemaKind::V21_1
    } else {
        SchemaKind::Legacy
    };
    SecurityProfile { kind, namespaced: false, strip_svg_script: false, iframe_src_i18n: v21_2_4 }
}

fn v22_profile() -> SecurityProfile {
    SecurityProfile {
        kind: SchemaKind::V22,
        namespaced: true,
        strip_svg_script: true,
        iframe_src_i18n: true,
    }
}

/// Whether this Angular version strips `:svg:script` during template lowering.
pub fn strips_namespaced_svg_script(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).strip_svg_script
}

/// Whether i18n must reject `iframe` `src` as a Trusted Types sink.
pub fn rejects_iframe_src_i18n(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).iframe_src_i18n
}

fn build_v22_schema() -> FxHashMap<String, SecurityContext> {
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
}

static V22_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> = LazyLock::new(build_v22_schema);
static V21_27_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V21_2_7));
static V21_1_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V21_1));
static LEGACY_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::Legacy));

fn schema_for(kind: SchemaKind) -> &'static FxHashMap<String, SecurityContext> {
    match kind {
        SchemaKind::Legacy => &LEGACY_SCHEMA,
        SchemaKind::V21_1 => &V21_1_SCHEMA,
        SchemaKind::V21_2_7 => &V21_27_SCHEMA,
        SchemaKind::V22 => &V22_SCHEMA,
    }
}

/// Bare-key schema used before Angular 22.
///
/// 21.1 adds MathML hrefs, `script|href`, iframe sandbox keys, and
/// `attributeName` no-binding. 21.2.7 adds the animation value attributes.
fn build_pren22_schema(kind: SchemaKind) -> FxHashMap<String, SecurityContext> {
    let mut schema = FxHashMap::default();
    register_base_html_style_and_url(&mut schema);
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
            ("script", &["src"]),
        ],
    );

    let extended = matches!(kind, SchemaKind::V21_1 | SchemaKind::V21_2_7);
    if extended {
        register_uniform(
            &mut schema,
            SecurityContext::Url,
            None,
            MATHML_URL_ELEMENTS,
            &["href", "xlink:href"],
        );
        register(
            &mut schema,
            SecurityContext::ResourceUrl,
            None,
            &[("script", &["href", "xlink:href"])],
        );
        register(
            &mut schema,
            SecurityContext::AttributeNoBinding,
            None,
            &[
                ("animate", &["attributeName"]),
                ("set", &["attributeName"]),
                ("animateMotion", &["attributeName"]),
                ("animateTransform", &["attributeName"]),
                ("unknown", &["attributeName"]),
                (
                    "iframe",
                    &[
                        "sandbox",
                        "allow",
                        "allowFullscreen",
                        "referrerPolicy",
                        "csp",
                        "fetchPriority",
                    ],
                ),
                (
                    "unknown",
                    &[
                        "sandbox",
                        "allow",
                        "allowFullscreen",
                        "referrerPolicy",
                        "csp",
                        "fetchPriority",
                    ],
                ),
            ],
        );
    }
    if matches!(kind, SchemaKind::V21_2_7) {
        register(
            &mut schema,
            SecurityContext::AttributeNoBinding,
            None,
            &[
                ("animate", &["values", "to", "from"]),
                ("set", &["to"]),
                ("unknown", &["values", "to", "from"]),
            ],
        );
    }
    schema
}

fn register_base_html_style_and_url(schema: &mut FxHashMap<String, SecurityContext>) {
    register(
        schema,
        SecurityContext::Html,
        None,
        &[("iframe", &["srcdoc"]), ("*", &["innerHTML", "outerHTML"])],
    );
    register(schema, SecurityContext::Style, None, &[("*", &["style"])]);
    register(
        schema,
        SecurityContext::Url,
        None,
        &[
            ("*", &["formAction"]),
            ("area", &["href"]),
            ("a", &["href", "xlink:href"]),
            ("form", &["action"]),
            ("img", &["src"]),
            ("video", &["src"]),
        ],
    );
}

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

/// Whether `name` is in the DOM element schema (`allKnownElementNames`),
/// including `:svg:`/`:math:`-prefixed entries.
pub fn is_known_element(name: &str) -> bool {
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

/// Security context for one element and property on the latest schema (v22).
///
/// Case-insensitive. Returns `SecurityContext::None` when the pair is not a sink.
pub fn get_security_context(element: &str, property: &str) -> SecurityContext {
    get_security_context_for(element, property, None)
}

/// Security context for the Angular version being compiled.
///
/// v22 keeps `:svg:` and `:math:` in the lookup key. Earlier versions lowercase
/// the tag as written and look up a bare `tag|attr` key, so `:svg:animate|to`
/// misses and `animate|to` hits on 21.2.7.
pub fn get_security_context_for(
    element: &str,
    property: &str,
    version: Option<crate::AngularVersion>,
) -> SecurityContext {
    let profile = security_profile(version);
    let tag =
        if profile.namespaced { normalize_tag_name(element) } else { element.to_ascii_lowercase() };
    let property_lower = property.to_ascii_lowercase();
    let schema = schema_for(profile.kind);

    let key = format!("{tag}|{property_lower}");
    if let Some(&ctx) = schema.get(&key) {
        return ctx;
    }

    let wildcard_key = format!("*|{property_lower}");
    if let Some(&ctx) = schema.get(&wildcard_key) {
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

/// Security context of a host binding on the latest schema (v22).
///
/// Mirrors `calcPossibleSecurityContexts` plus the host ingest filter that drops
/// `NONE` and the `{URL, RESOURCE_URL}` pair in `resolve_sanitizers.ts`.
pub fn host_binding_security_context(selector: &str, prop_name: &str) -> SecurityContext {
    host_binding_security_context_for(selector, prop_name, None)
}

/// Host-binding security context for a specific Angular version.
pub fn host_binding_security_context_for(
    selector: &str,
    prop_name: &str,
    version: Option<crate::AngularVersion>,
) -> SecurityContext {
    let contexts = if security_profile(version).namespaced {
        collect_namespaced_contexts(selector, prop_name, version)
    } else {
        collect_bare_contexts(selector, prop_name, version)
    };
    reduce_security_contexts(&contexts)
}

fn collect_namespaced_contexts(
    selector: &str,
    prop_name: &str,
    version: Option<crate::AngularVersion>,
) -> Vec<SecurityContext> {
    let selector = selector.trim();
    if selector.is_empty() {
        return KNOWN_ELEMENT_NAMES
            .iter()
            .map(|el| get_security_context_for(el, prop_name, version))
            .collect();
    }

    // `splitNsName` treats any leading `:x:` as a namespace, including the
    // `:not(` of a pseudo-class. Only a plain identifier is a namespace, so a
    // selector like `:not(img):not(video)` reaches `CssSelector::parse` whole
    // instead of parsing `not(video)` as the element.
    let (namespace_key, base_selector) = match split_ns_name(selector) {
        (Some(ns), _) if !ns.is_empty() && ns.bytes().all(is_selector_ident_byte) => {
            split_ns_name(selector)
        }
        _ => (None, selector),
    };
    let mut contexts = Vec::new();
    for css in CssSelector::parse(base_selector) {
        let excluded = not_element_names(&css);
        let element_names: Vec<String> = if let Some(element) = &css.element {
            if element == "*" && !excluded.is_empty() {
                // `*` on a `:not(...)` selector means "any element". Expanding it
                // lets the exclusions apply; a literal `*|attr` lookup would
                // silently drop the sanitizer.
                KNOWN_ELEMENT_NAMES.iter().map(|name| (*name).to_string()).collect()
            } else {
                resolve_concrete_element(element)
            }
        } else {
            KNOWN_ELEMENT_NAMES.iter().map(|name| (*name).to_string()).collect()
        };

        for element_name in element_names {
            if is_not_excluded(&element_name, &excluded) {
                let full_name = qualify_with_selector_namespace(&element_name, namespace_key);
                contexts.push(get_security_context_for(&full_name, prop_name, version));
            }
        }
    }
    contexts
}

/// v21 `calcPossibleSecurityContexts`: no namespace rewrite, and `:not(element)`
/// matches the element string exactly, including case.
fn collect_bare_contexts(
    selector: &str,
    prop_name: &str,
    version: Option<crate::AngularVersion>,
) -> Vec<SecurityContext> {
    let selector = selector.trim();
    if selector.is_empty() {
        return KNOWN_ELEMENT_NAMES
            .iter()
            .map(|el| get_security_context_for(el, prop_name, version))
            .collect();
    }

    let mut contexts = Vec::new();
    for css in CssSelector::parse(selector) {
        let excluded: FxHashSet<String> = css
            .not_selectors
            .iter()
            .filter(|sel| {
                sel.element.is_some()
                    && sel.class_names.is_empty()
                    && sel.attrs.is_empty()
                    && sel.not_selectors.is_empty()
            })
            .filter_map(|sel| sel.element.clone())
            .collect();
        let element_names: Vec<String> = if let Some(element) = &css.element {
            vec![element.clone()]
        } else {
            KNOWN_ELEMENT_NAMES.iter().map(|name| (*name).to_string()).collect()
        };
        for element_name in element_names {
            if excluded.contains(&element_name) {
                continue;
            }
            contexts.push(get_security_context_for(&element_name, prop_name, version));
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

fn is_selector_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
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
    fn test_host_selector_leading_not_is_not_a_namespace() {
        // `:not(...)` is a pseudo-class, not a `:ns:` prefix.
        assert_eq!(
            host_binding_security_context(":not(img):not(video)", "src"),
            SecurityContext::ResourceUrl
        );
        // A real `:svg:` prefix still namespaces the lookup.
        assert_eq!(
            host_binding_security_context(":svg:animate", "to"),
            SecurityContext::AttributeNoBinding
        );
    }

    #[test]
    fn test_host_comma_selector_merges_url_kinds() {
        assert_eq!(
            host_binding_security_context("a, base", "href"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    fn v21_2_7() -> Option<crate::AngularVersion> {
        Some(crate::AngularVersion::new(21, 2, 7))
    }

    #[test]
    fn v21_2_7_uses_bare_keys() {
        let version = v21_2_7();
        assert_eq!(
            get_security_context_for("animate", "to", version),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(get_security_context_for(":svg:animate", "to", version), SecurityContext::None);
        assert_eq!(
            get_security_context_for("script", "src", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(
            get_security_context_for("script", "href", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(get_security_context_for("mi", "href", version), SecurityContext::Url);
        assert_eq!(get_security_context_for(":math:mi", "href", version), SecurityContext::None);
        assert_eq!(
            host_binding_security_context_for("animate", "to", version),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            host_binding_security_context_for("[x]", "to", version),
            SecurityContext::AttributeNoBinding
        );
    }

    #[test]
    fn v21_2_6_has_no_animation_value_sinks() {
        let version = Some(crate::AngularVersion::new(21, 2, 6));
        assert_eq!(get_security_context_for("animate", "to", version), SecurityContext::None);
        assert_eq!(
            get_security_context_for("animate", "attributeName", version),
            SecurityContext::AttributeNoBinding
        );
        assert!(rejects_iframe_src_i18n(version));
        assert_eq!(host_binding_security_context_for("[x]", "to", version), SecurityContext::None);
    }

    #[test]
    fn v21_0_has_script_src_only() {
        let version = Some(crate::AngularVersion::new(21, 0, 0));
        assert_eq!(
            get_security_context_for("script", "src", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(get_security_context_for("script", "href", version), SecurityContext::None);
        assert_eq!(
            get_security_context_for("animate", "attributeName", version),
            SecurityContext::None
        );
        assert!(!rejects_iframe_src_i18n(version));
        assert!(!strips_namespaced_svg_script(version));
    }

    #[test]
    fn v21_2_3_still_allows_iframe_src_i18n() {
        assert!(!rejects_iframe_src_i18n(Some(crate::AngularVersion::new(21, 2, 3))));
        assert!(rejects_iframe_src_i18n(Some(crate::AngularVersion::new(21, 2, 4))));
    }
}

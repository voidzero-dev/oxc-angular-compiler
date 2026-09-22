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
/// `None` means the latest schema (v22). Security fixes were backported per
/// release line, so the cutovers are not monotonic: e.g. `attributeName`
/// no-binding reached 20.3.15 and 21.0.2 but `script|href` only reached
/// 20.3.16 and 21.0.7, and the namespaced schema landed on 20.3.22 and
/// 21.2.14 while 21.0.x / 21.1.x never received it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemaKind {
    /// 21.0.0 / 21.0.1 and everything below 20.3.15. Old URL set with
    /// `*|ping`, `*|cite`, `applet|code`, `media|src`, etc.
    Legacy,
    /// 19.2.17, 20.3.15, 21.0.2–21.0.5. Adds MathML hrefs, `attributeName`
    /// no-binding, and iframe sandbox keys on top of the old URL set.
    V20_3_15,
    /// 21.0.6 only. The hardening without the legacy URL keys, and before
    /// `script|href` landed in 21.0.7.
    V21_0_6,
    /// 20.3.16–20.3.21 and 21.0.7 through 21.2.6. Adds `script|href`; the
    /// `ping`/`cite`/`applet`/`media` keys are gone from 21.0.6 on.
    V21_1,
    /// Same as `V21_1` but still carrying the legacy URL keys: 19.2.18–19.2.22
    /// and 20.3.16–20.3.21 (the key removal was never backported to either
    /// maintained line).
    V20_3_16,
    /// 21.2.7 through 21.2.13. Animation `to` / `from` / `values` are bare keys.
    V21_2_7,
    /// 21.2.14 only. Namespaced keys but no `:svg:a|href` yet.
    V21_2_14,
    /// 19.2.23+, 20.3.22+, 21.2.15+, 22+. Namespaced keys with `:svg:a|href`.
    /// `script|src` and `script|href` are gone.
    V22,
}

struct SecurityProfile {
    kind: SchemaKind,
    /// The schema keys keep `:svg:` / `:math:` prefixes (21.2.14+, 19.2.23+,
    /// 20.3.22+). `calcPossibleSecurityContexts` rewrites selectors for these
    /// versions.
    namespaced: bool,
    /// `securityContext` runs `normalizeTagName`, stripping non-svg/math
    /// prefixes from the element before the lookup. Upstream added the
    /// normalizer one release after the namespaced schema: 21.2.14 looks the
    /// verbatim tag up, so `:xml:iframe|src` misses there but hits `iframe|src`
    /// at 21.2.15+.
    normalizes_tag_names: bool,
    /// The preparser strips `:svg:script` as well as `script`.
    strip_svg_script: bool,
    /// The preparser also strips `:svg:style`. Only 19.2.23, 20.3.22 and
    /// 21.2.14 did this; it was reverted everywhere else.
    strip_svg_style: bool,
    /// `iframe|src` joined Trusted Types sinks (19.2.20+, 20.3.18–21, 21.2.4+;
    /// never on the 21.0.x / 21.1.x lines).
    iframe_src_i18n: bool,
    /// `resolve_sanitizers` falls back to `ɵɵvalidateIframeAttribute` for
    /// security-sensitive iframe attributes with no other sanitizer. Upstream
    /// removed this when the iframe `attributeNoBinding` keys landed
    /// (19.2.17 / 20.3.15 / 21.0.2), so it only exists on the legacy schema.
    iframe_attr_validation: bool,
}

fn security_profile(version: Option<crate::AngularVersion>) -> SecurityProfile {
    let Some(version) = version else {
        return v22_profile();
    };
    if version.major >= 22 {
        return v22_profile();
    }

    let (kind, iframe_src_i18n) = match version.major {
        19 if version.minor >= 2 => match version.patch {
            0..=16 => (SchemaKind::Legacy, false),
            17 => (SchemaKind::V20_3_15, false),
            18..=19 => (SchemaKind::V20_3_16, false),
            20..=22 => (SchemaKind::V20_3_16, true),
            // 19.2.23+ has the namespaced schema with `:svg:a|href`.
            _ => (SchemaKind::V22, true),
        },
        20 if version.minor >= 3 => match version.patch {
            0..=14 => (SchemaKind::Legacy, false),
            15 => (SchemaKind::V20_3_15, false),
            16..=17 => (SchemaKind::V20_3_16, false),
            18..=21 => (SchemaKind::V20_3_16, true),
            // 20.3.22+ has the namespaced schema with `:svg:a|href`.
            _ => (SchemaKind::V22, true),
        },
        21 => match (version.minor, version.patch) {
            (0, 0..=1) => (SchemaKind::Legacy, false),
            (0, 2..=5) => (SchemaKind::V20_3_15, false),
            (0, 6) => (SchemaKind::V21_0_6, false),
            (0, _) => (SchemaKind::V21_1, false),
            (1, _) => (SchemaKind::V21_1, false),
            (2, 0..=3) => (SchemaKind::V21_1, false),
            (2, 4..=6) => (SchemaKind::V21_1, true),
            (2, 7..=13) => (SchemaKind::V21_2_7, true),
            (2, 14) => (SchemaKind::V21_2_14, true),
            // 21.2.15+ and any later 21.x minor use the namespaced schema.
            _ => (SchemaKind::V22, true),
        },
        _ => (SchemaKind::Legacy, false),
    };

    let namespaced = matches!(kind, SchemaKind::V21_2_14 | SchemaKind::V22);
    // `:svg:style` stripping existed only in 19.2.23, 20.3.22 and 21.2.14.
    let strip_svg_style = matches!(kind, SchemaKind::V21_2_14)
        || (version.major == 20 && version.minor == 3 && version.patch == 22)
        || (version.major == 19 && version.minor == 2 && version.patch == 23);
    SecurityProfile {
        kind,
        namespaced,
        // `normalizeTagName` in `securityContext` landed with the schema
        // backports that carried `:svg:a|href` (19.2.23 / 20.3.22 / 21.2.15);
        // 21.2.14 has namespaced keys but no normalizer.
        normalizes_tag_names: matches!(kind, SchemaKind::V22),
        strip_svg_script: namespaced,
        strip_svg_style,
        iframe_src_i18n,
        iframe_attr_validation: matches!(kind, SchemaKind::Legacy),
    }
}

fn v22_profile() -> SecurityProfile {
    SecurityProfile {
        kind: SchemaKind::V22,
        namespaced: true,
        normalizes_tag_names: true,
        strip_svg_script: true,
        strip_svg_style: false,
        iframe_src_i18n: true,
        iframe_attr_validation: false,
    }
}

/// Whether this Angular version strips `:svg:script` during template lowering.
pub fn strips_namespaced_svg_script(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).strip_svg_script
}

/// Whether this Angular version's preparser classifies `:svg:style` as a style
/// element (its text is collected into component styles and the element is
/// dropped). Only 20.3.22 and 21.2.14 did this.
pub fn strips_namespaced_svg_style(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).strip_svg_style
}

/// Whether this Angular version's schema keys keep `:svg:` / `:math:` prefixes.
/// `calcPossibleSecurityContexts` only promotes bare selector elements to
/// their `:svg:` / `:math:` forms on the namespaced schema.
pub fn uses_namespaced_schema(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).namespaced
}

/// Whether i18n must reject `iframe` `src` as a Trusted Types sink.
pub fn rejects_iframe_src_i18n(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).iframe_src_i18n
}

/// Whether `resolve_sanitizers` applies the `ɵɵvalidateIframeAttribute`
/// fallback. Upstream kept it on versions without the iframe
/// `attributeNoBinding` schema keys (everything before 19.2.17 / 20.3.15 /
/// 21.0.2) and dropped it once those keys covered the same attributes.
pub fn uses_iframe_attr_validation(version: Option<crate::AngularVersion>) -> bool {
    security_profile(version).iframe_attr_validation
}

/// Whether `attr_name` is a security-sensitive `<iframe>` attribute
/// (`IFRAME_SECURITY_SENSITIVE_ATTRS`). The comparison is case-insensitive
/// because `setAttribute` lowercases names.
pub fn is_iframe_security_sensitive_attr(attr_name: &str) -> bool {
    matches!(
        attr_name.to_ascii_lowercase().as_str(),
        "sandbox" | "allow" | "allowfullscreen" | "referrerpolicy" | "csp" | "fetchpriority"
    )
}

fn build_v22_schema(with_svg_a: bool) -> FxHashMap<String, SecurityContext> {
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

    // `:svg:a|href` landed in 20.3.22 / 21.2.15; 21.2.14 did not have it.
    if with_svg_a {
        register(&mut schema, SecurityContext::Url, Some("svg"), &[("a", &["href", "xlink:href"])]);
    }

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

static V22_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_v22_schema(true));
static V21_2_14_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_v22_schema(false));
static V21_27_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V21_2_7));
static V21_1_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V21_1));
static V20_3_16_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V20_3_16));
static V20_3_15_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V20_3_15));
static V21_0_6_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::V21_0_6));
static LEGACY_SCHEMA: LazyLock<FxHashMap<String, SecurityContext>> =
    LazyLock::new(|| build_pren22_schema(SchemaKind::Legacy));

fn schema_for(kind: SchemaKind) -> &'static FxHashMap<String, SecurityContext> {
    match kind {
        SchemaKind::Legacy => &LEGACY_SCHEMA,
        SchemaKind::V20_3_15 => &V20_3_15_SCHEMA,
        SchemaKind::V21_0_6 => &V21_0_6_SCHEMA,
        SchemaKind::V20_3_16 => &V20_3_16_SCHEMA,
        SchemaKind::V21_1 => &V21_1_SCHEMA,
        SchemaKind::V21_2_7 => &V21_27_SCHEMA,
        SchemaKind::V21_2_14 => &V21_2_14_SCHEMA,
        SchemaKind::V22 => &V22_SCHEMA,
    }
}
/// Pre-v22 schemas look up bare `tag|attr` keys (`normalizeTagName` did not keep
/// namespaces yet), so nothing here registers `:svg:` or `:math:` keys.
fn build_pren22_schema(kind: SchemaKind) -> FxHashMap<String, SecurityContext> {
    let mut schema = FxHashMap::default();
    register_base_html_style_and_url(&mut schema);

    let hardened = kind != SchemaKind::Legacy;
    let legacy_url_keys =
        matches!(kind, SchemaKind::Legacy | SchemaKind::V20_3_15 | SchemaKind::V20_3_16);
    // `script|href` / `script|xlink:href` landed in 20.3.16 and 21.0.7.
    let script_href =
        matches!(kind, SchemaKind::V20_3_16 | SchemaKind::V21_1 | SchemaKind::V21_2_7);

    if hardened {
        // `a|xlink:href` was added with the MathML hardening; the Legacy URL
        // set has only `a|href` / `a|ping`.
        register(&mut schema, SecurityContext::Url, None, &[("a", &["xlink:href"])]);
        register_uniform(
            &mut schema,
            SecurityContext::Url,
            None,
            MATHML_URL_ELEMENTS,
            &["href", "xlink:href"],
        );
    }

    if legacy_url_keys {
        register(
            &mut schema,
            SecurityContext::Url,
            None,
            &[
                ("area", &["ping"]),
                ("audio", &["src"]),
                ("a", &["ping"]),
                ("blockquote", &["cite"]),
                ("body", &["background"]),
                ("del", &["cite"]),
                ("input", &["src"]),
                ("ins", &["cite"]),
                ("q", &["cite"]),
                ("source", &["src"]),
                ("track", &["src"]),
                ("video", &["poster"]),
            ],
        );
        register(
            &mut schema,
            SecurityContext::ResourceUrl,
            None,
            &[
                ("applet", &["code", "codebase"]),
                ("head", &["profile"]),
                ("html", &["manifest"]),
                ("media", &["src"]),
            ],
        );
    }

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
    if script_href {
        register(
            &mut schema,
            SecurityContext::ResourceUrl,
            None,
            &[("script", &["href", "xlink:href"])],
        );
    }

    if hardened {
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
            // `a|xlink:href` is not in the Legacy URL set; callers add it for
            // hardened kinds.
            ("a", &["href"]),
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
/// Callers pass the resolved (possibly `:ns:`-prefixed) element name. Versions
/// with `normalizeTagName` (19.2.23+, 20.3.22+, 21.2.15+) strip non-svg/math
/// prefixes; every earlier version lowercases the tag as written, so
/// `:xml:iframe|src` misses the schema while `:svg:animate|to` hits its
/// namespaced key at 21.2.14.
pub fn get_security_context_for(
    element: &str,
    property: &str,
    version: Option<crate::AngularVersion>,
) -> SecurityContext {
    let profile = security_profile(version);
    let tag = if profile.normalizes_tag_names {
        normalize_tag_name(element)
    } else {
        element.to_ascii_lowercase()
    };
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

/// Security context of a property/attribute binding on a concrete element,
/// for a specific Angular version.
///
/// Upstream routes element bindings through `calcPossibleSecurityContexts`
/// with the element name as the selector, so a bare element missing from the
/// DOM schema is promoted to its `:svg:`/`:math:` form (e.g. `<animate>` →
/// `:svg:animate`) on the namespaced schema, and a `tagName === null`
/// selectorless host expands over every known element. The bound attribute
/// keeps `securityContexts[0]` — the numerically lowest context after
/// upstream's sort, which keeps `NONE` ahead of `URL`/`RESOURCE_URL`.
pub fn element_security_context_for(
    element: &str,
    prop_name: &str,
    version: Option<crate::AngularVersion>,
) -> SecurityContext {
    let contexts = if security_profile(version).namespaced {
        collect_namespaced_contexts(element, prop_name, version)
    } else {
        collect_bare_contexts(element, prop_name, version)
    };
    contexts
        .iter()
        .copied()
        .min_by_key(|ctx| security_context_rank(*ctx))
        .unwrap_or(SecurityContext::None)
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
        // A literal `*` element (only produced for a `:not(...)`-only selector)
        // is looked up as the `*|attr` key, matching upstream: upstream expands
        // `element === null` over all known elements but keeps `*` verbatim.
        let element_names: Vec<String> = if let Some(element) = &css.element {
            if element == "*" { vec![element.clone()] } else { resolve_concrete_element(element) }
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

/// Pre-namespaced `calcPossibleSecurityContexts` (everything before the
/// 19.2.23 / 20.3.22 / 21.2.14 / v22 schema): no `splitNsName`, no element
/// promotion — the whole selector goes through `CssSelector.parse`, so
/// `:svg:animate` parses to element `animate` and hits the bare
/// `animate|to` key, and `:not(element)` matches the element string
/// exactly, including case.
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
    if is_known_element(element) {
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
    fn test_host_universal_selector_expands_to_known_elements() {
        // `*` never produces an element name upstream, so `*` / `*[x]` behave
        // like an attribute-only selector and scan every known element.
        for selector in ["*", "*[x]", "[x]"] {
            assert_eq!(
                host_binding_security_context(selector, "src"),
                SecurityContext::UrlOrResourceUrl,
                "{selector}"
            );
            assert_eq!(
                host_binding_security_context(selector, "formAction"),
                SecurityContext::Url,
                "{selector}"
            );
        }
        // Same expansion on the pre-namespaced lookup path.
        let version = Some(crate::AngularVersion::new(21, 0, 1));
        assert_eq!(
            host_binding_security_context_for("*[x]", "src", version),
            SecurityContext::UrlOrResourceUrl
        );
        // A `:not(...)`-only selector parses to a literal `*` element upstream
        // and is looked up as the `*|attr` key, which has no `src` entry.
        assert_eq!(host_binding_security_context("*:not(img)", "src"), SecurityContext::None);
        assert_eq!(
            host_binding_security_context(":not(img):not(video)", "src"),
            SecurityContext::None
        );
    }

    #[test]
    fn test_host_selector_leading_not_is_not_a_namespace() {
        // `:not(...)` is a pseudo-class, not a `:ns:` prefix.
        assert_eq!(
            host_binding_security_context(":not(img):not(video)", "src"),
            SecurityContext::None
        );
        // A real `:svg:` prefix still namespaces the lookup.
        assert_eq!(
            host_binding_security_context(":svg:animate", "to"),
            SecurityContext::AttributeNoBinding
        );
    }

    #[test]
    fn test_element_binding_promotes_unknown_bare_element() {
        // Element bindings go through `calcPossibleSecurityContexts` upstream:
        // a bare element missing from the DOM schema is looked up as its
        // `:svg:`/`:math:` form on the namespaced schema.
        assert_eq!(
            element_security_context_for("animate", "to", None),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            element_security_context_for("set", "to", None),
            SecurityContext::AttributeNoBinding
        );
        // The local name promotes even under a different namespace, matching
        // upstream (`hasElement(':math:animate')` misses, so it tries
        // `:svg:animate` / `:math:animate` and keeps the hit's own prefix).
        assert_eq!(
            element_security_context_for(":math:animate", "to", None),
            SecurityContext::AttributeNoBinding
        );
        // No promotion before the namespaced schema (19.2.23 / 20.3.22 /
        // 21.2.14 / v22): `animate|to` is a verbatim miss.
        let version = Some(crate::AngularVersion::new(21, 0, 1));
        assert_eq!(element_security_context_for("animate", "to", version), SecurityContext::None);
        // Known elements resolve directly on every version.
        assert_eq!(
            element_security_context_for("iframe", "src", None),
            SecurityContext::ResourceUrl
        );
        assert_eq!(
            element_security_context_for("iframe", "src", version),
            SecurityContext::ResourceUrl
        );
        // Unknown in every namespace stays a miss.
        assert_eq!(element_security_context_for("bogus", "src", None), SecurityContext::None);
        // `tagName === null` expands over every known element upstream, and
        // the binding keeps `securityContexts[0]` — the numerically lowest,
        // so `NONE` wins over URL/RESOURCE_URL.
        assert_eq!(element_security_context_for("", "src", None), SecurityContext::None);
        // `formAction` hits the `*|formAction` URL wildcard key, so the
        // lowest context is URL.
        assert_eq!(element_security_context_for("", "formAction", None), SecurityContext::Url);
        // `innerHTML` hits the `*|innerHTML` HTML key for every element, so
        // the lowest context is HTML.
        assert_eq!(element_security_context_for("", "innerHTML", None), SecurityContext::Html);
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

    #[test]
    fn v21_0_1_is_the_legacy_schema() {
        let version = Some(crate::AngularVersion::new(21, 0, 1));
        // Legacy URL keys exist, but the hardening had not landed yet.
        assert_eq!(get_security_context_for("a", "ping", version), SecurityContext::Url);
        assert_eq!(get_security_context_for("media", "src", version), SecurityContext::ResourceUrl);
        assert_eq!(
            get_security_context_for("applet", "code", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(
            get_security_context_for("script", "src", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(get_security_context_for("a", "xlink:href", version), SecurityContext::None);
        assert_eq!(get_security_context_for("mi", "href", version), SecurityContext::None);
        assert_eq!(
            get_security_context_for("animate", "attributeName", version),
            SecurityContext::None
        );
        assert_eq!(get_security_context_for("iframe", "sandbox", version), SecurityContext::None);
        assert_eq!(get_security_context_for("script", "href", version), SecurityContext::None);
    }

    #[test]
    fn v21_0_2_hardens_but_keeps_legacy_keys() {
        let version = Some(crate::AngularVersion::new(21, 0, 2));
        assert_eq!(get_security_context_for("a", "xlink:href", version), SecurityContext::Url);
        assert_eq!(get_security_context_for("mi", "href", version), SecurityContext::Url);
        assert_eq!(
            get_security_context_for("animate", "attributeName", version),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(get_security_context_for("media", "src", version), SecurityContext::ResourceUrl);
        // `script|href` only landed in 21.0.7.
        assert_eq!(get_security_context_for("script", "href", version), SecurityContext::None);
    }

    #[test]
    fn v21_0_6_drops_legacy_keys_without_script_href() {
        let version = Some(crate::AngularVersion::new(21, 0, 6));
        assert_eq!(get_security_context_for("a", "ping", version), SecurityContext::None);
        assert_eq!(get_security_context_for("media", "src", version), SecurityContext::None);
        assert_eq!(get_security_context_for("script", "href", version), SecurityContext::None);
        assert_eq!(
            get_security_context_for("script", "src", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(get_security_context_for("mi", "href", version), SecurityContext::Url);
        assert_eq!(
            get_security_context_for("animate", "attributeName", version),
            SecurityContext::AttributeNoBinding
        );
    }

    #[test]
    fn v20_3_16_has_script_href_and_legacy_keys() {
        let version = Some(crate::AngularVersion::new(20, 3, 16));
        // The legacy-key removal was never backported to 20.3.
        assert_eq!(get_security_context_for("media", "src", version), SecurityContext::ResourceUrl);
        assert_eq!(
            get_security_context_for("script", "href", version),
            SecurityContext::ResourceUrl
        );
        assert_eq!(
            get_security_context_for("script", "xlink:href", version),
            SecurityContext::ResourceUrl
        );
        assert!(!rejects_iframe_src_i18n(version));
        assert!(rejects_iframe_src_i18n(Some(crate::AngularVersion::new(20, 3, 18))));
    }

    #[test]
    fn v21_2_14_is_namespaced_without_svg_a() {
        let version = Some(crate::AngularVersion::new(21, 2, 14));
        assert_eq!(
            get_security_context_for(":svg:animate", "to", version),
            SecurityContext::AttributeNoBinding
        );
        // Bare `animate|to` is not a key in the namespaced schema.
        assert_eq!(get_security_context_for("animate", "to", version), SecurityContext::None);
        // `:svg:a|href` arrived in 21.2.15.
        assert_eq!(get_security_context_for(":svg:a", "href", version), SecurityContext::None);
        assert_eq!(get_security_context_for("script", "src", version), SecurityContext::None);
        assert!(strips_namespaced_svg_script(version));
        assert!(strips_namespaced_svg_style(version));
    }

    #[test]
    fn v21_2_15_and_v20_3_22_add_svg_a() {
        for version in [
            crate::AngularVersion::new(21, 2, 15),
            crate::AngularVersion::new(20, 3, 22),
            crate::AngularVersion::new(20, 3, 23),
        ] {
            let version = Some(version);
            assert_eq!(get_security_context_for(":svg:a", "href", version), SecurityContext::Url);
            assert!(strips_namespaced_svg_script(version));
        }
        // `:svg:style` classification was reverted after 21.2.14 / 20.3.22.
        assert!(!strips_namespaced_svg_style(Some(crate::AngularVersion::new(21, 2, 15))));
        assert!(strips_namespaced_svg_style(Some(crate::AngularVersion::new(20, 3, 22))));
        assert!(!strips_namespaced_svg_style(Some(crate::AngularVersion::new(20, 3, 23))));
        assert!(!strips_namespaced_svg_style(None));
    }

    #[test]
    fn v19_2_follows_the_same_backported_cutovers() {
        // The 19.2 line received the same security backports as 20.3/21.x.
        let v19_2_16 = Some(crate::AngularVersion::new(19, 2, 16));
        assert_eq!(
            get_security_context_for("media", "src", v19_2_16),
            SecurityContext::ResourceUrl
        );
        assert_eq!(get_security_context_for("a", "xlink:href", v19_2_16), SecurityContext::None);
        assert_eq!(get_security_context_for("script", "href", v19_2_16), SecurityContext::None);

        // 19.2.17 hardened (MathML hrefs, attrNoBinding) but kept legacy keys.
        let v19_2_17 = Some(crate::AngularVersion::new(19, 2, 17));
        assert_eq!(get_security_context_for("a", "xlink:href", v19_2_17), SecurityContext::Url);
        assert_eq!(get_security_context_for("mi", "href", v19_2_17), SecurityContext::Url);
        assert_eq!(
            get_security_context_for("iframe", "sandbox", v19_2_17),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context_for("media", "src", v19_2_17),
            SecurityContext::ResourceUrl
        );
        assert_eq!(get_security_context_for("script", "href", v19_2_17), SecurityContext::None);
        assert!(!rejects_iframe_src_i18n(v19_2_17));

        // 19.2.18+ added `script|href`; `iframe|src` joined the Trusted Types
        // sinks at 19.2.20.
        let v19_2_19 = Some(crate::AngularVersion::new(19, 2, 19));
        assert_eq!(
            get_security_context_for("script", "href", v19_2_19),
            SecurityContext::ResourceUrl
        );
        assert!(!rejects_iframe_src_i18n(v19_2_19));
        assert!(rejects_iframe_src_i18n(Some(crate::AngularVersion::new(19, 2, 20))));

        // 19.2.23+ has the namespaced schema with `:svg:a|href`; `:svg:style`
        // was stripped in exactly 19.2.23.
        let v19_2_23 = Some(crate::AngularVersion::new(19, 2, 23));
        assert!(uses_namespaced_schema(v19_2_23));
        assert_eq!(get_security_context_for(":svg:a", "href", v19_2_23), SecurityContext::Url);
        assert_eq!(get_security_context_for("script", "src", v19_2_23), SecurityContext::None);
        assert!(strips_namespaced_svg_script(v19_2_23));
        assert!(strips_namespaced_svg_style(v19_2_23));
        let v19_2_24 = Some(crate::AngularVersion::new(19, 2, 24));
        assert!(uses_namespaced_schema(v19_2_24));
        assert!(!strips_namespaced_svg_style(v19_2_24));

        // 19.0 / 19.1 are the legacy schema.
        for version in [crate::AngularVersion::new(19, 0, 0), crate::AngularVersion::new(19, 1, 4)]
        {
            let version = Some(version);
            assert!(!uses_namespaced_schema(version));
            assert_eq!(
                get_security_context_for("media", "src", version),
                SecurityContext::ResourceUrl
            );
        }
    }

    #[test]
    fn iframe_attr_validation_only_on_the_legacy_schema() {
        assert!(uses_iframe_attr_validation(Some(crate::AngularVersion::new(19, 0, 0))));
        assert!(uses_iframe_attr_validation(Some(crate::AngularVersion::new(19, 2, 16))));
        assert!(!uses_iframe_attr_validation(Some(crate::AngularVersion::new(19, 2, 17))));
        assert!(uses_iframe_attr_validation(Some(crate::AngularVersion::new(20, 3, 14))));
        assert!(!uses_iframe_attr_validation(Some(crate::AngularVersion::new(20, 3, 15))));
        assert!(uses_iframe_attr_validation(Some(crate::AngularVersion::new(21, 0, 1))));
        assert!(!uses_iframe_attr_validation(Some(crate::AngularVersion::new(21, 0, 2))));
        assert!(!uses_iframe_attr_validation(None));
        assert!(!uses_iframe_attr_validation(Some(crate::AngularVersion::new(22, 0, 0))));
    }

    #[test]
    fn v21_2_14_looks_up_the_tag_verbatim() {
        // 21.2.14 has the namespaced schema keys but `securityContext` had no
        // `normalizeTagName` yet: the tag is lowercased verbatim.
        let v21_2_14 = Some(crate::AngularVersion::new(21, 2, 14));
        assert_eq!(get_security_context_for(":xml:iframe", "src", v21_2_14), SecurityContext::None);
        assert_eq!(
            get_security_context_for(":svg:animate", "to", v21_2_14),
            SecurityContext::AttributeNoBinding
        );
        // 21.2.15+ normalizes `:xml:iframe` down to `iframe`.
        let v21_2_15 = Some(crate::AngularVersion::new(21, 2, 15));
        assert_eq!(
            get_security_context_for(":xml:iframe", "src", v21_2_15),
            SecurityContext::ResourceUrl
        );
        // Pre-namespaced versions never normalized either.
        let v21_2_13 = Some(crate::AngularVersion::new(21, 2, 13));
        assert_eq!(get_security_context_for(":xml:iframe", "src", v21_2_13), SecurityContext::None);
        assert_eq!(get_security_context_for(":svg:animate", "to", v21_2_13), SecurityContext::None);
    }

    #[test]
    fn iframe_security_sensitive_attrs_match_case_insensitively() {
        for attr in
            ["sandbox", "allow", "allowfullscreen", "referrerpolicy", "csp", "fetchpriority"]
        {
            assert!(is_iframe_security_sensitive_attr(attr));
        }
        // `setAttribute` lowercases the name, so upstream compares lowercase.
        assert!(is_iframe_security_sensitive_attr("SandBox"));
        assert!(is_iframe_security_sensitive_attr("allowFullScreen"));
        assert!(!is_iframe_security_sensitive_attr("src"));
        assert!(!is_iframe_security_sensitive_attr("srcdoc"));
    }
}

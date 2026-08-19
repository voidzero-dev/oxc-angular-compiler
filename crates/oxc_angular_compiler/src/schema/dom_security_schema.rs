//! DOM Security Schema
//!
//! This module contains the security schema that maps element|property combinations
//! to their appropriate security context for sanitization.
//!
//! Ported from Angular's `schema/dom_security_schema.ts`.
//!
//! DO NOT EDIT THIS LIST OF SECURITY SENSITIVE PROPERTIES WITHOUT A SECURITY REVIEW!

use crate::ast::expression::BindingType;
use crate::ast::r3::SecurityContext;
use crate::parser::html::split_ns_name;
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
            // SVGScriptElement href sinks. v21.2.7 dom_security_schema.ts:122-125
            // registers `script|href` and `script|xlink:href` as RESOURCE_URL with a
            // comment pointing at SVGScriptElement.href. Because a namespaced
            // `<svg:script>` is stored `:svg:script` and `get_security_context`
            // strips the `:ns:` prefix before lookup, `:svg:script|href` resolves to
            // these bare `script|...` keys. This pairs with the transform keeping
            // `:svg:script` alive (G4): the SVG script element survives and its href
            // is sanitized as a resource URL — the actual XSS mechanism for an
            // SVGScriptElement.
            "script|href",
            "script|xlink:href",
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
            // SVG animation *value* attributes. Binding these animates the
            // referenced attribute's value at runtime, so they are an XSS vector
            // identical to `attributeName` and must be validated. Latest upstream
            // `dom_security_schema.ts` registers them under SVG_NAMESPACE as
            // ATTRIBUTE_NO_BINDING:
            //   ['animate', ['attributeName', 'values', 'to', 'from']]
            //   ['set', ['to', 'attributeName']]
            // OXC stores schema keys non-namespaced (the `:ns:` prefix is stripped
            // in `get_security_context`), so they are added here as bare lowercase
            // `tag|attr` keys. (Issue #315 sub-gap 2: "SVG animation value
            // attributes bypass sanitization".)
            "animate|to",
            "animate|from",
            "animate|values",
            "set|to",
            // The no-namespace `unknown` element aggregates every value attribute
            // upstream registers, so bindings on an unknown host still validate
            // these. (Issue #315 sub-gap 2 / upstream `unknown` entry in
            // `dom_security_schema.ts`.)
            "unknown|to",
            "unknown|from",
            "unknown|values",
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

    // Strip any explicit `:ns:` prefix so namespaced element names resolve to the
    // same security context as the bare local name. Angular's template pipeline
    // looks security up by the element's *local* name (namespace tracked
    // separately), so e.g. `<svg:animate>` (stored as `:svg:animate`) must resolve
    // like `animate`. Schema keys are never namespaced. (Issue #315 sub-gap 2 /
    // Codex namespaced-lookup finding.)
    let (_, element_normalized) = split_ns_name(&element_lower);

    // First try element-specific lookup
    let key = format!("{}|{}", element_normalized, property_lower);
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

/// `SECURITY_SCHEMA` element segments that are NOT a BARE entry of upstream
/// `DomElementSchemaRegistry.allKnownElementNames()` — the complete "phantom"
/// set. These bare keys exist in `SECURITY_SCHEMA` (and upstream
/// `dom_security_schema.ts`) but the matching element name is registered in the
/// element SCHEMA only under a namespace (e.g. `:svg:animate`, `:math:math`) or
/// is absent from the element schema entirely (e.g. `none`, `annotation`,
/// `malignmark`, `mglyph`, `mprescripts`, `annotation-xml`).
///
/// Upstream's host-unknown scan (`calcPossibleSecurityContexts`,
/// `binding_parser.ts:888-896`) iterates the REAL `allKnownElementNames()` and
/// maps each through `securityContext(name, prop)` WITHOUT stripping the
/// namespace (`dom_element_schema_registry.ts:449-456`). So for a namespaced-only
/// element, `securityContext(':svg:animate','to')`/`securityContext(':math:math',
/// 'href')` looks up `:svg:animate|to`/`:math:math|href` -> NOT FOUND -> NONE,
/// and the bare keys (`animate|to`, `math|href`, …) are UNREACHABLE from the host
/// path; an absent element name is never iterated at all. OXC stores the security
/// schema non-namespaced (the `:ns:` prefix is stripped at lookup), so without
/// this skip these bare keys would contribute to the host-unknown aggregation as
/// if a real `animate`/`math`/`semantics`/… element existed.
///
/// The only OBSERVABLE divergence this causes is when a `:not(element)` selector
/// excludes EVERY real bare contributor of a property while a phantom segment
/// still supplies it. Concretely, the MathML `*|href`/`*|xlink:href` URL keys
/// (`math|href`, `mi|href`, `annotation|href`, `semantics|href`, …) made
/// `[x]:not(a):not(area):not(base):not(link):not(script)` + `[attr.href]` resolve
/// to `Url` in OXC where @angular/compiler@21.2.7 yields `NONE` (all the real
/// bare `href` contributors — `a`, `area`, `base`, `link`, `script` — are
/// excluded, and the MathML elements exist only as `:math:*`). Skipping the full
/// phantom set makes OXC's host-unknown scan byte-for-byte match upstream's
/// real-element-name iteration for every property, since each phantom prop is
/// also supplied by a bare-known element (`unknown` for the SVG animation
/// value/`attributeName` props; `a`/`area`/`base`/`link`/`script` for the
/// MathML href props), so the only difference is the unreachable-upstream
/// phantom contribution.
///
/// Derived from the @angular/compiler@21.2.7 oracle: every `SECURITY_SCHEMA`
/// element segment minus the bare `allKnownElementNames()` entries. The real bare
/// elements (`a`, `area`, `base`, `embed`, `form`, `frame`, `iframe`, `img`,
/// `link`, `object`, `script`, `video`, `unknown`) and the `*` wildcard are NOT
/// in this set and keep contributing.
const ELEMENTS_ONLY_KNOWN_NAMESPACED: &[&str] = &[
    // SVG animation elements (registered only as `:svg:*`).
    "animate",
    "set",
    "animatemotion",
    "animatetransform",
    // MathML elements (registered only as `:math:*`).
    "maction",
    "math",
    "merror",
    "mfrac",
    "mi",
    "mmultiscripts",
    "mn",
    "mo",
    "mover",
    "mpadded",
    "mphantom",
    "mroot",
    "mrow",
    "ms",
    "mspace",
    "msqrt",
    "mstyle",
    "msub",
    "msubsup",
    "msup",
    "mtable",
    "mtd",
    "mtext",
    "mtr",
    "munder",
    "munderover",
    "semantics",
    // Names absent from the element SCHEMA entirely (never iterated upstream).
    "annotation",
    "annotation-xml",
    "malignmark",
    "mglyph",
    "mprescripts",
    "none",
];

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
    calc_security_context_for_unknown_element_excluding(property, &[])
}

/// Like [`calc_security_context_for_unknown_element`], but excludes any schema
/// entry whose element name appears in `excluded_elements` via a CASE-SENSITIVE
/// exact match. This mirrors upstream `notElementNames.has(elName)`, a `Set.has`
/// over the LOWERCASE `allKnownElementNames()` vs the case-PRESERVED `:not()`
/// element names (`CssSelector.setElement` does not lowercase). The schema keys
/// are lowercase, so `:not(object)` excludes `object` but `:not(OBJECT)` does
/// not.
///
/// This mirrors the attribute-only / wildcard branch of upstream
/// `calcPossibleSecurityContexts` (`binding_parser.ts:888-896`), where the
/// candidate element set is `registry.allKnownElementNames()` minus the
/// `:not(element)` exclusions before each name is mapped through
/// `registry.securityContext(name, propName)`:
///
/// ```ts
/// const elementNames = selector.element ? [selector.element] : registry.allKnownElementNames();
/// const notElementNames = new Set(
///   selector.notSelectors.filter((s) => s.isElementSelector()).map((s) => s.element),
/// );
/// const possibleElementNames = elementNames.filter((elName) => !notElementNames.has(elName));
/// ctxs.push(...possibleElementNames.map(nameToContext));
/// ```
///
/// OXC scans the schema map keyed `"element|property"` rather than iterating an
/// explicit element list, so the exclusion is applied by skipping element-specific
/// entries whose `element` segment is excluded. Wildcard entries (`"*|property"`)
/// are never excluded: upstream a `*|prop` context applies to *every* known
/// element name, so it survives unless every element is excluded — and `*` is
/// never itself an element `:not()` name. (With `excluded_elements` empty this is
/// identical to the original all-elements scan.)
fn calc_security_context_for_unknown_element_excluding(
    property: &str,
    excluded_elements: &[String],
) -> SecurityContext {
    let property_lower = property.to_ascii_lowercase();

    // Collect all security contexts for this property across all (non-excluded)
    // elements.
    let mut has_url = false;
    let mut has_resource_url = false;
    let mut has_other = false;
    let mut other_context = SecurityContext::None;

    for (key, &ctx) in SECURITY_SCHEMA.iter() {
        // Check if this entry is for our property (format: "element|property")
        if let Some(pipe_pos) = key.find('|') {
            let elem = &key[..pipe_pos];
            let prop = &key[pipe_pos + 1..];
            if prop.eq_ignore_ascii_case(&property_lower) {
                // Skip element-specific entries whose element is excluded by a
                // `:not(element)` selector (upstream `possibleElementNames`
                // filter). Wildcard `*|prop` entries are handled separately
                // below and never excluded.
                if elem == "*" {
                    continue;
                }
                // Skip schema keys whose element segment is NOT a real bare
                // `allKnownElementNames()` entry (the complete phantom set; see
                // `ELEMENTS_ONLY_KNOWN_NAMESPACED`). Upstream's host-unknown scan
                // iterates `DomElementSchemaRegistry.allKnownElementNames()` and
                // maps each through `securityContext(name, prop)` WITHOUT
                // stripping the namespace (`dom_element_schema_registry.ts:449`).
                // The SVG animation elements (`:svg:animate`, `:svg:set`,
                // `:svg:animateMotion`, `:svg:animateTransform`) and the MathML
                // elements (`:math:math`, `:math:mi`, `:math:semantics`, …) are
                // registered in the element schema ONLY as namespaced keys; a few
                // others (`none`, `annotation`, `annotation-xml`, `malignmark`,
                // `mglyph`, `mprescripts`) are absent from the element schema
                // entirely. So e.g. `securityContext(':svg:animate','to')` and
                // `securityContext(':math:math','href')` resolve to NONE, and the
                // bare keys (`animate|to`, `set|to`, `math|href`, `mi|href`,
                // `semantics|href`, …) are UNREACHABLE from upstream's
                // host-unknown scan. OXC stores the security schema non-namespaced
                // (with ns stripped at lookup), which would otherwise make these
                // bare keys contribute here as if a real `animate`/`math`/… element
                // existed. The observable bug: `[x]:not(a):not(area):not(base)
                // :not(link):not(script)` + `[attr.href]` reduced to `Url` in OXC
                // via the phantom MathML href keys, where @angular/compiler@21.2.7
                // yields NONE (all real bare `href` contributors excluded).
                // Skipping the full phantom set restores faithfulness: each phantom
                // prop is also supplied by a real bare element (`unknown` for the
                // SVG animation value/`attributeName` props; `a`/`area`/`base`/
                // `link`/`script` for the MathML href props), so the only behavior
                // change is removing the unreachable-upstream phantom contribution.
                // The `iframe|*` sandbox-family keys are kept (real `iframe`).
                if ELEMENTS_ONLY_KNOWN_NAMESPACED.contains(&elem) {
                    continue;
                }
                // CASE-SENSITIVE exact match, mirroring upstream
                // `notElementNames.has(elName)` (a `Set.has` over LOWERCASE
                // `allKnownElementNames()` vs the case-PRESERVED `:not()` name).
                // The schema keys (`elem`) are lowercase, so a lowercase
                // `:not(object)` still excludes `object`, while `:not(OBJECT)`
                // does NOT (no exact match) — matching v21.2.7.
                if excluded_elements.iter().any(|ex| ex == elem) {
                    continue;
                }
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

    // Also check wildcard entries (apply to all known elements -> never excluded).
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

/// Collects the element names excluded by an alternate's `:not(...)` selectors,
/// mirroring upstream's `notSelectors.filter((s) => s.isElementSelector())`.
///
/// Upstream `isElementSelector()` (`directive_matching.ts:168-175`) is `true`
/// only when the negated selector is a *pure* element selector: it has an
/// element AND no classes, no attributes, and no nested `:not()`. So
/// `:not(object)` contributes `object`, but `:not(object.foo)`,
/// `:not(object[x])`, `:not(.foo)`, and `:not([y])` contribute nothing.
///
/// The element name is returned with its ORIGINAL case PRESERVED, matching
/// upstream `CssSelector.setElement` (`directive_matching.ts:181-183`), which
/// stores the element verbatim (NO `.toLowerCase()`). The exclusion downstream
/// is a CASE-SENSITIVE exact match against the lowercase known/schema element
/// names (upstream `notElementNames.has(...)` over `allKnownElementNames()`),
/// so `:not(object)` excludes `object` but `:not(OBJECT)` does not.
fn not_element_names(css: &crate::pipeline::selector::CssSelector) -> Vec<String> {
    css.not_selectors
        .iter()
        .filter_map(|n| {
            // Pure element selector only (upstream `isElementSelector()`).
            if n.class_names.is_empty() && n.attrs.is_empty() && n.not_selectors.is_empty() {
                n.element.as_deref().map(str::to_string)
            } else {
                None
            }
        })
        .collect()
}

/// Computes the security context for a HOST binding given a directive/component
/// selector and the bound attribute/property name.
///
/// This mirrors the upstream HOST pipeline, which differs from the template
/// pipeline in how it reduces the candidate contexts to a single result:
///
/// 1. `calcPossibleSecurityContexts(registry, selector, propName, isAttribute)`
///    (`binding_parser.ts:874-900`) parses ALL comma-separated alternates via
///    `CssSelector.parse(selector)` (one `CssSelector` per alternate) and, for
///    each alternate, looks up `registry.securityContext(elName, ...)` for every
///    `possibleElementName`. When the alternate names a concrete element the set
///    is just that element; when it is attribute-only
///    (`selector.element === null`) it is ALL known element names. In BOTH cases
///    the `:not(element)` element exclusions are filtered out first
///    (`possibleElementNames = elementNames.filter((n) => !notElementNames.has(n))`),
///    so e.g. `[x]:not(object)` excludes `object` from the all-elements scan.
///    The collected contexts are de-duplicated and returned.
///
/// 2. The HOST consumer in `ingest.ts:117-130` then
///    `.filter(context => context !== SecurityContext.NONE)` (DROP `NONE`).
///
/// 3. `resolve_sanitizers.ts:60-99` resolves the filtered array to a sanitizer:
///    if it is EXACTLY `{URL, RESOURCE_URL}` -> `sanitizeUrlOrResourceUrl`;
///    otherwise `getOnlySecurityContext` (which throws on length > 1 and returns
///    the single remaining context, or `NONE` when empty).
///
/// In this codebase, `resolve_sanitizers.rs` consumes a single
/// `SecurityContext` (not an array), with the special `UrlOrResourceUrl`
/// variant standing in for the `{URL, RESOURCE_URL}` pair. So this function
/// performs steps 1-3 here and returns that single reduced context:
///
/// - empty (after dropping `NONE`)            -> `SecurityContext::None`
/// - exactly `{Url, ResourceUrl}`             -> `SecurityContext::UrlOrResourceUrl`
/// - a single surviving context               -> that context
/// - more than one (and not the URL pair)     -> the lowest-enum-order context
///   (see the fallback note below)
///
/// The attribute-vs-property (`isAttribute`) distinction is carried by the
/// `name` passed in: callers pass the `attr.`-stripped name for attribute
/// bindings and the plain property name for property bindings, matching the
/// schema keys (e.g. `animate|to`, `a|href`, `*|innerhtml`). This is the same
/// convention the template path uses in `html_to_r3.rs`.
///
/// NOTE: this helper implements the HOST-path reduction only. The template path
/// uses `get_security_context` directly with the concrete element name (see
/// `html_to_r3.rs`) and the upstream `securityContexts[0]` rule; it does NOT go
/// through this function.
pub fn compute_security_context(selector: &str, name: &str) -> SecurityContext {
    use crate::pipeline::selector::CssSelector;

    // Step 1: collect a context for every possible element across all
    // comma-separated alternates (mirrors `calcPossibleSecurityContexts`).
    let mut contexts: Vec<SecurityContext> = Vec::new();
    let mut push_unique = |ctx: SecurityContext| {
        if !contexts.contains(&ctx) {
            contexts.push(ctx);
        }
    };

    for css in CssSelector::parse(selector) {
        // `:not(element)` exclusions for this alternate (upstream
        // `notElementNames`). Only *pure* element `:not()` selectors count, per
        // upstream `isElementSelector()`.
        let excluded = not_element_names(&css);
        match &css.element {
            // Concrete element alternate (e.g. `img` in `img[x]`).
            Some(element_name) if element_name != "*" => {
                // Honor `:not(element)` element exclusions, like upstream's
                // `possibleElementNames = elementNames.filter(...)`: a concrete
                // element that is itself excluded contributes no context
                // (upstream's single-element list becomes empty). CASE-SENSITIVE
                // exact match (upstream `notElementNames.has(elName)` over the
                // case-preserved `[selector.element]`), so `object:not(OBJECT)`
                // does NOT self-exclude.
                let is_excluded = excluded.iter().any(|n| n == element_name);
                if !is_excluded {
                    push_unique(get_security_context(element_name, name));
                }
            }
            // Attribute-only alternate (`selector.element === null`) or the `*`
            // wildcard: aggregate across ALL known elements, but first drop the
            // `:not(element)` exclusions, mirroring upstream's
            // `possibleElementNames = registry.allKnownElementNames().filter(
            //   (elName) => !notElementNames.has(elName))`. Without this filter
            // a directive like `[x]:not(object)` with host `[attr.data]` would
            // still see `object|data` and over-sanitize as RESOURCE_URL, whereas
            // upstream excludes `object` and yields no sanitizer.
            _ => {
                push_unique(calc_security_context_for_unknown_element_excluding(name, &excluded));
            }
        }
    }

    // `calc_security_context_for_unknown_element` already collapses the
    // all-elements URL/RESOURCE_URL pair into the `UrlOrResourceUrl` variant.
    // Expand it back to its constituents so the merge below can combine it with
    // contexts contributed by other (concrete-element) alternates and re-derive
    // the pair faithfully.
    let mut expanded: Vec<SecurityContext> = Vec::new();
    for ctx in contexts {
        match ctx {
            SecurityContext::UrlOrResourceUrl => {
                if !expanded.contains(&SecurityContext::Url) {
                    expanded.push(SecurityContext::Url);
                }
                if !expanded.contains(&SecurityContext::ResourceUrl) {
                    expanded.push(SecurityContext::ResourceUrl);
                }
            }
            other if !expanded.contains(&other) => expanded.push(other),
            _ => {}
        }
    }

    // Step 2: drop `NONE` (the HOST-path filter in `ingest.ts`).
    expanded.retain(|ctx| *ctx != SecurityContext::None);

    // Step 3: reduce to a single context, mirroring `resolve_sanitizers.ts`.
    match expanded.as_slice() {
        // empty -> NONE (no sanitizer)
        [] => SecurityContext::None,
        // exactly one surviving context
        [single] => *single,
        // exactly the {URL, RESOURCE_URL} pair -> runtime-resolved sanitizer
        two if two.len() == 2
            && two.contains(&SecurityContext::Url)
            && two.contains(&SecurityContext::ResourceUrl) =>
        {
            SecurityContext::UrlOrResourceUrl
        }
        // More than one surviving context that is NOT the URL/RESOURCE_URL pair.
        // Upstream `getOnlySecurityContext` THROWS an `AssertionError` here
        // ("Ambiguous security context") — its own comment says this is believed
        // to never happen in practice outside the URL/RESOURCE_URL case. A
        // compiler should not panic on user input, so we pick the lowest context
        // by enum order (matching upstream's ascending sort + TDB's historical
        // "take the first one" behavior). This branch is effectively unreachable.
        many => many
            .iter()
            .copied()
            .min_by_key(|ctx| security_context_rank(*ctx))
            .unwrap_or(SecurityContext::None),
    }
}

/// Rank a `SecurityContext` by upstream Angular's `SecurityContext` enum order
/// (`core.ts`): NONE=0, HTML=1, STYLE=2, SCRIPT=3, URL=4, RESOURCE_URL=5,
/// ATTRIBUTE_NO_BINDING=6. Used only for the unreachable >1-context fallback in
/// `compute_security_context` to deterministically pick the lowest context,
/// matching upstream's ascending sort. `UrlOrResourceUrl` is an OXC-only
/// composite and never appears in the ranked set (it is expanded beforehand).
fn security_context_rank(ctx: SecurityContext) -> u8 {
    match ctx {
        SecurityContext::None => 0,
        SecurityContext::Html => 1,
        SecurityContext::Style => 2,
        SecurityContext::Script => 3,
        SecurityContext::Url => 4,
        SecurityContext::ResourceUrl => 5,
        SecurityContext::AttributeNoBinding => 6,
        // Composite variant; not part of upstream's enum. Rank after all real
        // contexts so it never wins the `min_by_key` (it is expanded away).
        SecurityContext::UrlOrResourceUrl => 7,
    }
}

/// Extracts the element name from a CSS selector, if the selector begins with
/// a concrete element name.
///
/// Examples:
/// - `"a[myDirective]"` → `Some("a")`
/// - `"div.my-class"` → `Some("div")`
/// - `"[myDirective]"` → `None`
/// - `".my-class"` → `None`
/// - `""` → `None` (no element; mirrors upstream's `selector === null`)
pub fn extract_element_from_selector(selector: &str) -> Option<String> {
    let s = selector.trim();

    // A leading `[`, `.`, `:`, or `#` means there is no element in this selector.
    if s.starts_with('[') || s.starts_with('.') || s.starts_with(':') || s.starts_with('#') {
        return None;
    }

    // The element name runs until the first non-identifier character.
    let mut element_end = 0;
    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            element_end = i + c.len_utf8();
        } else {
            break;
        }
    }

    if element_end > 0 { Some(s[..element_end].to_lowercase()) } else { None }
}

/// Computes the security context for a `host` binding, given its binding type,
/// the bound (already prefix-stripped) name, and the directive/component
/// selector used as the element context.
///
/// Mirrors upstream `BindingParser.createBoundElementProperty`
/// (`binding_parser.ts`): attribute bindings use the attribute security lookup
/// (`isAttribute = true`) and property bindings the property lookup
/// (`isAttribute = false`), while `class`/`style`/animation bindings carry their
/// fixed contexts (`NONE`/`STYLE`/`NONE`). In this codebase the `isAttribute`
/// distinction is encoded by the schema key (the `attr.`-stripped name passed in
/// for attribute bindings), so both attribute and property bindings resolve via
/// the shared `compute_security_context`.
pub fn host_binding_security_context(
    binding_type: BindingType,
    name: &str,
    selector: &str,
) -> SecurityContext {
    match binding_type {
        // Attribute (`[attr.X]`) and property (`[domProp]`) bindings get a real
        // security context derived from the selector's element.
        BindingType::Attribute | BindingType::Property | BindingType::TwoWay => {
            compute_security_context(selector, name)
        }
        // `[style.X]` bindings are always the STYLE context (upstream uses
        // `[SecurityContext.STYLE]` for the `style` prefix).
        BindingType::Style => SecurityContext::Style,
        // `[class.X]` and animation bindings are never sanitized.
        BindingType::Class | BindingType::Animation | BindingType::LegacyAnimation => {
            SecurityContext::None
        }
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
    fn test_svg_script_href_resource_url_context() {
        // v21.2.7 dom_security_schema.ts:122-125 registers `script|href` and
        // `script|xlink:href` as RESOURCE_URL (the SVGScriptElement.href sinks).
        // Plain (bare) lookups resolve directly.
        assert_eq!(get_security_context("script", "href"), SecurityContext::ResourceUrl);
        assert_eq!(get_security_context("script", "xlink:href"), SecurityContext::ResourceUrl);
        // A namespaced `<svg:script>` is stored `:svg:script`; the `:ns:` prefix is
        // stripped before lookup, so it resolves to the same RESOURCE_URL context.
        // This is reachable in templates now that the transform keeps `:svg:script`
        // alive (G4).
        assert_eq!(get_security_context(":svg:script", "href"), SecurityContext::ResourceUrl);
        assert_eq!(get_security_context(":svg:script", "xlink:href"), SecurityContext::ResourceUrl);
        // Lookup is case-insensitive.
        assert_eq!(get_security_context("SCRIPT", "HREF"), SecurityContext::ResourceUrl);
        assert_eq!(get_security_context(":svg:script", "XLINK:HREF"), SecurityContext::ResourceUrl);
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
    fn test_svg_animation_value_attribute_no_binding() {
        // Latest upstream `dom_security_schema.ts` registers SVG animation *value*
        // attributes as ATTRIBUTE_NO_BINDING (under SVG_NAMESPACE):
        //   ['animate', ['attributeName', 'values', 'to', 'from']]
        //   ['set', ['to', 'attributeName']]
        // plus the no-namespace `unknown` aggregate (which includes 'values', 'to',
        // 'from'). Binding these animates an attribute's *value* at runtime, so
        // leaving them unsanitized is the XSS vector this closes. OXC stores keys
        // non-namespaced (the `:ns:` prefix is stripped before lookup), so they
        // resolve to AttributeNoBinding -> `ɵɵvalidateAttribute` at runtime.
        // (Issue #315 sub-gap 2.)
        assert_eq!(get_security_context("animate", "to"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("animate", "from"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("animate", "values"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("set", "to"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("unknown", "to"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("unknown", "from"), SecurityContext::AttributeNoBinding);
        assert_eq!(get_security_context("unknown", "values"), SecurityContext::AttributeNoBinding);
        // The namespaced element form resolves identically (the `:svg:` prefix is
        // stripped before lookup).
        assert_eq!(get_security_context(":svg:animate", "to"), SecurityContext::AttributeNoBinding);
        // Lookup is case-insensitive (matching the SVG `<animate>` element name).
        assert_eq!(get_security_context("ANIMATE", "TO"), SecurityContext::AttributeNoBinding);
        // The pre-existing `attributeName` registration is unaffected.
        assert_eq!(
            get_security_context("animate", "attributeName"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context("ANIMATE", "ATTRIBUTENAME"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context("unknown", "attributeName"),
            SecurityContext::AttributeNoBinding
        );
    }

    #[test]
    fn test_namespaced_svg_element_security_lookup() {
        // Issue #315 sub-gap 2 / Codex namespaced-lookup finding: an explicitly
        // namespaced element is stored with a `:ns:` prefix (e.g. `:svg:animate`).
        // The lookup strips the prefix so the namespaced form resolves to the same
        // context as the bare local name.
        assert_eq!(
            get_security_context(":svg:animate", "attributeName"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context(":svg:set", "attributeName"),
            SecurityContext::AttributeNoBinding
        );
        assert_eq!(
            get_security_context(":svg:iframe", "sandbox"),
            SecurityContext::AttributeNoBinding
        );
        // Namespaced URL contexts (wildcard and element-specific) still resolve.
        assert_eq!(get_security_context(":svg:a", "href"), SecurityContext::Url);
        assert_eq!(get_security_context(":svg:iframe", "srcdoc"), SecurityContext::Html);
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

    // -----------------------------------------------------------------------
    // G1: `compute_security_context` (HOST path) must consider EVERY
    // comma-separated selector alternate and merge their contexts, mirroring
    // upstream `calcPossibleSecurityContexts` + the host NONE-filter +
    // `resolve_sanitizers` URL/RESOURCE_URL special case.
    // -----------------------------------------------------------------------
    #[test]
    fn test_compute_sc_single_concrete_element() {
        // Single alternate, concrete element -> element-specific lookup.
        assert_eq!(compute_security_context("img[x]", "src"), SecurityContext::Url);
        assert_eq!(compute_security_context("iframe[x]", "src"), SecurityContext::ResourceUrl);
        assert_eq!(compute_security_context("a[appLink]", "href"), SecurityContext::Url);
    }

    #[test]
    fn test_compute_sc_attribute_only_aggregates() {
        // Attribute-only selector (`element === null`) -> unknown-element scan.
        assert_eq!(
            compute_security_context("[appHref]", "href"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    #[test]
    fn test_compute_sc_merges_url_and_resource_url() {
        // img|src = URL, iframe|src = RESOURCE_URL -> {URL, RESOURCE_URL} merge.
        assert_eq!(
            compute_security_context("img[x],iframe[x]", "src"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    #[test]
    fn test_compute_sc_filters_none_single_survivor() {
        // div|src = NONE (filtered), iframe|src = RESOURCE_URL -> single survivor.
        assert_eq!(
            compute_security_context("div[x],iframe[x]", "src"),
            SecurityContext::ResourceUrl
        );
        // Order must not matter.
        assert_eq!(
            compute_security_context("iframe[x],div[x]", "src"),
            SecurityContext::ResourceUrl
        );
    }

    #[test]
    fn test_compute_sc_all_none_is_none() {
        // Neither a|title nor b|title is sensitive -> empty after filter -> NONE.
        assert_eq!(compute_security_context("a[x],b[x]", "title"), SecurityContext::None);
    }

    #[test]
    fn test_compute_sc_dedupes_same_context() {
        // Two URL alternates collapse to a single URL context (not the pair).
        assert_eq!(compute_security_context("a[x],area[x]", "href"), SecurityContext::Url);
    }

    #[test]
    fn test_compute_sc_not_excludes_concrete_element() {
        // `:not(iframe)` removes the only RESOURCE_URL contributor, leaving just
        // img|src = URL.
        assert_eq!(
            compute_security_context("img[x],iframe[x]:not(iframe)", "src"),
            SecurityContext::Url
        );
    }

    // -----------------------------------------------------------------------
    // v21.2.7 faithfulness (Codex iteration-10): the attribute-only / wildcard
    // branch must filter `:not(element)` names out of the all-elements scan,
    // mirroring upstream `possibleElementNames = elementNames.filter(...)` in
    // `binding_parser.ts:888-896`. Previously OXC ignored `:not(...)` here and
    // over-sanitized.
    //
    // Schema facts (dom_security_schema.ts): `data`/`codebase` are ONLY on
    // `object` (RESOURCE_URL); `srcdoc` is ONLY on `iframe` (HTML). So excluding
    // those elements removes the sole contributor and yields NONE.
    // -----------------------------------------------------------------------
    #[test]
    fn test_compute_sc_attr_only_not_object_excludes_data() {
        // `object|data` is the ONLY `data` sink. `[x]:not(object)` excludes it,
        // so nothing else contributes `data` -> NONE (upstream: no sanitizer).
        // Sanity: `object|data` really is RESOURCE_URL and `data` is object-only.
        assert_eq!(get_security_context("object", "data"), SecurityContext::ResourceUrl);
        assert_eq!(calc_security_context_for_unknown_element("data"), SecurityContext::ResourceUrl);
        assert_eq!(compute_security_context("[x]:not(object)", "data"), SecurityContext::None);
    }

    #[test]
    fn test_compute_sc_attr_only_not_object_excludes_codebase() {
        // `object|codebase` is the ONLY `codebase` sink. Excluding `object`
        // leaves nothing -> NONE.
        assert_eq!(get_security_context("object", "codebase"), SecurityContext::ResourceUrl);
        assert_eq!(compute_security_context("[x]:not(object)", "codebase"), SecurityContext::None);
    }

    #[test]
    fn test_compute_sc_attr_only_not_iframe_excludes_srcdoc() {
        // `iframe|srcdoc` is the ONLY `srcdoc` sink (HTML). Excluding `iframe`
        // leaves nothing -> NONE.
        assert_eq!(get_security_context("iframe", "srcdoc"), SecurityContext::Html);
        assert_eq!(calc_security_context_for_unknown_element("srcdoc"), SecurityContext::Html);
        assert_eq!(compute_security_context("[x]:not(iframe)", "srcdoc"), SecurityContext::None);
    }

    #[test]
    fn test_compute_sc_attr_only_no_not_still_sanitizes() {
        // CONTROL: without `:not`, `object|data` is still in the set -> RESOURCE_URL.
        assert_eq!(compute_security_context("[x]", "data"), SecurityContext::ResourceUrl);
        assert_eq!(compute_security_context("[x]", "srcdoc"), SecurityContext::Html);
    }

    #[test]
    fn test_compute_sc_attr_only_not_unrelated_element_still_sanitizes() {
        // CONTROL: excluding a non-sink element (`div`) leaves `object` in the
        // set -> still RESOURCE_URL for `data`.
        assert_eq!(compute_security_context("[x]:not(div)", "data"), SecurityContext::ResourceUrl);
    }

    #[test]
    fn test_compute_sc_attr_only_non_element_not_does_not_filter() {
        // CONTROL: a NON-element `:not()` (class / attribute) is NOT an
        // `isElementSelector()`, so it excludes nothing -> still RESOURCE_URL.
        assert_eq!(compute_security_context("[x]:not(.foo)", "data"), SecurityContext::ResourceUrl);
        assert_eq!(compute_security_context("[x]:not([y])", "data"), SecurityContext::ResourceUrl);
    }

    #[test]
    fn test_compute_sc_wildcard_not_object_excludes_data() {
        // The `*` wildcard alternate aggregates over all elements just like the
        // attribute-only case, and must also honor `:not(object)`.
        assert_eq!(compute_security_context("*:not(object)", "data"), SecurityContext::None);
    }

    #[test]
    fn test_compute_sc_attr_only_not_object_keeps_wildcard_prop() {
        // Excluding `object` must NOT drop a wildcard `*|prop` context, which
        // upstream treats as applying to every (remaining) element. `innerhtml`
        // is `*|innerhtml` = HTML, so `[x]:not(object)` still yields HTML.
        assert_eq!(compute_security_context("[x]:not(object)", "innerHTML"), SecurityContext::Html);
    }

    // -----------------------------------------------------------------------
    // v21.2.7 faithfulness (Finding 1): the `:not(element)` exclusion is a
    // CASE-SENSITIVE exact match. Upstream `CssSelector.setElement`
    // (directive_matching.ts:181-183) stores the element verbatim (no
    // `.toLowerCase()`), while `allKnownElementNames()` and the schema keys are
    // LOWERCASE. So `notElementNames.has(name)` only excludes when the `:not()`
    // name is exactly the lowercase known name.
    //
    // Oracle (faithful reimpl of calcPossibleSecurityContexts over the real
    // @angular/compiler@21.2.7 DomElementSchemaRegistry + CssSelector):
    //   [x]:not(object) + data => [NONE]                    -> None
    //   [x]:not(OBJECT) + data => [NONE, RESOURCE_URL]      -> ResourceUrl
    //   [x]:not(IFRAME) + src  => [NONE, URL, RESOURCE_URL] -> UrlOrResourceUrl
    //
    // Previously OXC lowercased the `:not()` name and compared
    // case-insensitively, so `:not(OBJECT)` wrongly excluded `object` -> None,
    // an UNDER-sanitization XSS gap.
    // -----------------------------------------------------------------------
    #[test]
    fn test_compute_sc_not_uppercase_object_does_not_exclude() {
        // Uppercase `:not(OBJECT)` does NOT exclude lowercase `object`, so
        // `object|data` (RESOURCE_URL) survives.
        assert_eq!(
            compute_security_context("[x]:not(OBJECT)", "data"),
            SecurityContext::ResourceUrl
        );
    }

    #[test]
    fn test_compute_sc_not_lowercase_object_excludes() {
        // CONTROL companion: lowercase `:not(object)` DOES exclude `object`.
        assert_eq!(compute_security_context("[x]:not(object)", "data"), SecurityContext::None);
    }

    #[test]
    fn test_compute_sc_not_uppercase_iframe_does_not_exclude() {
        // Uppercase `:not(IFRAME)` does NOT exclude lowercase `iframe`; the
        // across-all-elements `src` set is {URL, RESOURCE_URL}.
        assert_eq!(
            compute_security_context("[x]:not(IFRAME)", "src"),
            SecurityContext::UrlOrResourceUrl
        );
    }

    #[test]
    fn test_compute_sc_concrete_uppercase_not_self_does_not_exclude() {
        // A concrete-element alternate whose `:not()` is the SAME element but in
        // a different case must NOT self-exclude (case-sensitive). `object` is a
        // concrete element here and `:not(OBJECT)` does not match it, so
        // `object|data` = RESOURCE_URL survives.
        assert_eq!(
            compute_security_context("object:not(OBJECT)", "data"),
            SecurityContext::ResourceUrl
        );
        // CONTROL: same-case self-exclusion DOES drop it -> None.
        assert_eq!(compute_security_context("object:not(object)", "data"), SecurityContext::None);
    }

    #[test]
    fn test_calc_excluding_helper_directly() {
        // The exclusion helper drops the named element from the all-elements scan.
        assert_eq!(
            calc_security_context_for_unknown_element_excluding("data", &[]),
            SecurityContext::ResourceUrl
        );
        assert_eq!(
            calc_security_context_for_unknown_element_excluding("data", &["object".to_string()]),
            SecurityContext::None
        );
        // Excluding `object` does not affect a wildcard context.
        assert_eq!(
            calc_security_context_for_unknown_element_excluding(
                "innerhtml",
                &["object".to_string()]
            ),
            SecurityContext::Html
        );
    }

    // Finding 1 (issue #315): host-unknown `:not(<animation element>)` faithfulness.
    //
    // Upstream `calcPossibleSecurityContexts` iterates
    // `DomElementSchemaRegistry.allKnownElementNames()` and maps each through
    // `securityContext(name, prop)` WITHOUT stripping the namespace. The SVG
    // animation elements exist in the element schema ONLY as namespaced keys
    // (`:svg:animate`, `:svg:set`, `:svg:animatemotion`, `:svg:animatetransform`),
    // so they resolve to NONE; the bare ATTRIBUTE_NO_BINDING security keys
    // (`animate|to`, `set|to`, …) are reachable from the host-unknown scan ONLY
    // via the real `unknown` element. Verified against @angular/compiler@21.2.7:
    //   calc('[x]','to',true)            -> [NONE, ATTRIBUTE_NO_BINDING] -> host ATTRIBUTE_NO_BINDING
    //   calc('[x]:not(unknown)','to',true) -> [NONE]                     -> host NONE
    //   calc('[x]:not(animate)','to',true) -> [NONE, ATTRIBUTE_NO_BINDING] -> host ATTRIBUTE_NO_BINDING
    #[test]
    fn host_unknown_animation_props_match_v21_2_7() {
        // Base attribute-only host selector: ATTRIBUTE_NO_BINDING via real `unknown`.
        for prop in ["to", "from", "values", "attributeName"] {
            assert_eq!(
                compute_security_context("[x]", prop),
                SecurityContext::AttributeNoBinding,
                "base [x] + {prop} should be ATTRIBUTE_NO_BINDING (via real `unknown` element)",
            );
        }
        // `:not(unknown)` removes the ONLY real contributor -> NONE upstream. The
        // phantom bare `animate`/`set` keys must NOT keep it ATTRIBUTE_NO_BINDING.
        for prop in ["to", "from", "values", "attributeName"] {
            assert_eq!(
                compute_security_context("[x]:not(unknown)", prop),
                SecurityContext::None,
                "[x]:not(unknown) + {prop} should be NONE to match @angular/compiler@21.2.7",
            );
        }
        // `:not(animate)` excludes a phantom (non-)element; the real `unknown`
        // element still contributes -> ATTRIBUTE_NO_BINDING (matches upstream).
        assert_eq!(
            compute_security_context("[x]:not(animate)", "to"),
            SecurityContext::AttributeNoBinding,
        );
        // The iframe sandbox-family ATTRIBUTE_NO_BINDING props are kept under
        // `:not(unknown)` because the real `iframe` element still contributes
        // (upstream parity).
        for prop in ["sandbox", "allow", "allowfullscreen", "csp", "fetchpriority"] {
            assert_eq!(
                compute_security_context("[x]:not(unknown)", prop),
                SecurityContext::AttributeNoBinding,
                "[x]:not(unknown) + {prop} stays ATTRIBUTE_NO_BINDING via real `iframe`",
            );
        }
    }

    // Finding (iteration-21): the host-unknown scan must skip the COMPLETE phantom
    // element set, not just the four SVG animation names. The MathML `*|href` /
    // `*|xlink:href` URL keys (`math|href`, `mi|href`, `annotation|href`,
    // `semantics|href`, …) name elements registered in the element schema ONLY as
    // `:math:*` (or absent entirely), so upstream's `allKnownElementNames()`
    // iteration never reaches them. Verified against @angular/compiler@21.2.7:
    //   calc('[x]:not(a):not(area):not(base):not(link):not(script)','href',true)
    //      -> [NONE] -> host NONE
    // (all real bare `href` contributors are `a`/`area`/`base`/`link`/`script`).
    // Previously OXC scanned bare SECURITY_SCHEMA keys and the phantom MathML
    // `*|href` keys kept it `Url`.
    #[test]
    fn host_unknown_mathml_href_phantom_match_v21_2_7() {
        // Excluding EVERY real bare `href` contributor leaves only phantom MathML
        // elements -> upstream NONE. OXC must not return `Url` via phantom keys.
        assert_eq!(
            compute_security_context(
                "[x]:not(a):not(area):not(base):not(link):not(script)",
                "href"
            ),
            SecurityContext::None,
            "all real bare `href` sinks excluded; phantom MathML keys must not contribute",
        );
        // Same for `xlink:href` (real bare contributors are only `a` and `script`).
        assert_eq!(
            compute_security_context("[x]:not(a):not(script)", "xlink:href"),
            SecurityContext::None,
            "all real bare `xlink:href` sinks excluded; phantom MathML keys must not contribute",
        );
        // CONTROL: without the `:not()` exclusions the real bare `href` sinks make
        // `[x]` + `href` the {URL, RESOURCE_URL} pair (`a`/`area` = URL,
        // `base`/`link` = RESOURCE_URL), matching upstream `[NONE,URL,RESOURCE_URL]`.
        assert_eq!(compute_security_context("[x]", "href"), SecurityContext::UrlOrResourceUrl,);
        // CONTROL: excluding only some real contributors still leaves `a` (URL),
        // so the phantom skip does not over-reduce. `:not(area):not(base)
        // :not(link):not(script)` keeps `a|href` = URL.
        assert_eq!(
            compute_security_context("[x]:not(area):not(base):not(link):not(script)", "href"),
            SecurityContext::Url,
            "`a|href` (real bare URL sink) survives",
        );
        // CONTROL: a concrete MathML-ish phantom element selector resolves via the
        // bare-key element-specific lookup in `get_security_context` (the template
        // path), which is unchanged — only the host-unknown aggregation skips
        // phantoms. `math[x]` + `href` is still URL (concrete element path).
        assert_eq!(compute_security_context("math[x]", "href"), SecurityContext::Url);
    }
}

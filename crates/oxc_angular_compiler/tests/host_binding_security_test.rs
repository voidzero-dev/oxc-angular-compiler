//! Issue #315 finding F1: host bindings must compute a real security context.
//!
//! `@Component`/`@Directive` `host: { '[attr.href]': '...', '[innerHTML]': '...' }`
//! bindings previously hard-coded `SecurityContext::None`, so no sanitizer or
//! validator was ever emitted — an XSS gap. Upstream Angular computes the
//! security context for host bindings using the directive/component SELECTOR as
//! the element context (`createHostBindingsFunction` →
//! `calcPossibleSecurityContexts(registry, selector, name, isAttribute)`).
//!
//! These tests drive the full public pipeline (`transform_angular_file`) and
//! assert that the generated host-bindings update block contains (or, for the
//! control case, omits) the expected sanitizer/validator identifier.

use oxc_allocator::Allocator;
use oxc_angular_compiler::transform_angular_file;

/// Compile an Angular `.ts` source and return generated JS, asserting no errors.
fn compile(source: &str, file: &str) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, file, source, None, None);
    assert!(!result.has_errors(), "compile errors: {:?}", result.diagnostics);
    result.code
}

// ---------------------------------------------------------------------------
// URL context — `[attr.href]` on a selector whose element is `<a>`.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_href_on_anchor_emits_url_sanitizer() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: 'a[appLink]',
    host: { '[attr.href]': 'url' }
})
export class LinkDirective {
    url = '';
}
";
    let code = compile(source, "link.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeUrl"),
        "expected ɵɵsanitizeUrl for host [attr.href] on a[appLink].\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// HTML context — `[innerHTML]` DOM-property host binding.
// ---------------------------------------------------------------------------
#[test]
fn component_host_inner_html_emits_html_sanitizer() {
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-card',
    template: '',
    host: { '[innerHTML]': 'content' }
})
export class CardComponent {
    content = '';
}
";
    let code = compile(source, "card.component.ts");
    assert!(
        code.contains("ɵɵsanitizeHtml"),
        "expected ɵɵsanitizeHtml for host [innerHTML].\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// AttributeNoBinding — `[attr.to]` on an attribute-only selector (unknown
// element). The unknown-element fallback aggregates the SVG-animation value
// attributes, which resolve to AttributeNoBinding -> ɵɵvalidateAttribute.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_to_unknown_element_emits_validate_attribute() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[appAnim]',
    host: { '[attr.to]': 'target' }
})
export class AnimDirective {
    target = '';
}
";
    let code = compile(source, "anim.directive.ts");
    assert!(
        code.contains("ɵɵvalidateAttribute"),
        "expected ɵɵvalidateAttribute for host [attr.to] on unknown element.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Finding 1 (issue #315): host-unknown `:not(unknown)` + animation attr.
//
// Upstream `calcPossibleSecurityContexts` iterates `allKnownElementNames()`
// (the SVG animation elements are registered ONLY as namespaced names like
// `:svg:animate`, which resolve to NONE) and reaches ATTRIBUTE_NO_BINDING for
// `to`/`from`/`values`/`attributeName` ONLY via the real `unknown` element.
// A `:not(unknown)` selector therefore excludes the SOLE contributor and the
// host binding yields NONE — NO `ɵɵvalidateAttribute`. Verified by running
// @angular/compiler@21.2.7:
//   calc('[x]:not(unknown)','to',true).filter(c=>c!==NONE) === []  -> NONE.
#[test]
fn directive_host_attr_to_not_unknown_emits_no_validator() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[appAnim]:not(unknown)',
    host: { '[attr.to]': 'target' }
})
export class AnimDirective {
    target = '';
}
";
    let code = compile(source, "anim-not-unknown.directive.ts");
    assert!(
        !code.contains("ɵɵvalidateAttribute") && !code.contains("ɵɵsanitize"),
        "host [attr.to] on `[appAnim]:not(unknown)` must NOT gain a validator \
         (matches @angular/compiler@21.2.7: excluding `unknown` removes the only \
         ATTRIBUTE_NO_BINDING source).\nGot:\n{code}"
    );
}

// `:not(animate)` excludes a phantom (non-)element — the real `unknown` element
// still contributes, so the validator IS retained (upstream parity).
#[test]
fn directive_host_attr_to_not_animate_keeps_validator() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[appAnim]:not(animate)',
    host: { '[attr.to]': 'target' }
})
export class AnimDirective {
    target = '';
}
";
    let code = compile(source, "anim-not-animate.directive.ts");
    assert!(
        code.contains("ɵɵvalidateAttribute"),
        "host [attr.to] on `[appAnim]:not(animate)` must KEEP ɵɵvalidateAttribute \
         (the real `unknown` element still contributes).\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// UrlOrResourceUrl — `[attr.href]` on an attribute-only selector. `href` is a
// URL context on <a>/<area> and a ResourceUrl context on <base>/<link>, so the
// unknown-element fallback yields the runtime-resolved sanitizer.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_href_unknown_element_emits_url_or_resource_url() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[appHref]',
    host: { '[attr.href]': 'url' }
})
export class HrefDirective {
    url = '';
}
";
    let code = compile(source, "href.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeUrlOrResourceUrl"),
        "expected ɵɵsanitizeUrlOrResourceUrl for host [attr.href] on unknown element.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Control — a non-sensitive host attribute must NOT gain any sanitizer.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_title_emits_no_sanitizer() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[appTitle]',
    host: { '[attr.title]': 'title' }
})
export class TitleDirective {
    title = '';
}
";
    let code = compile(source, "title.directive.ts");
    assert!(
        !code.contains("ɵɵsanitize") && !code.contains("ɵɵvalidateAttribute"),
        "host [attr.title] must not gain a sanitizer/validator.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// G1 — multi-selector merge to URL/RESOURCE_URL.
//
// `selector: 'img[x],iframe[x]'` with host `[attr.src]`: `img|src` is URL and
// `iframe|src` is RESOURCE_URL. Upstream `calcPossibleSecurityContexts`
// considers BOTH comma alternates and the host pipeline merges the
// {URL, RESOURCE_URL} pair into the runtime-resolved sanitizer
// `ɵɵsanitizeUrlOrResourceUrl`. The old single-leading-element behavior wrongly
// used only `img|src` -> `ɵɵsanitizeUrl`.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_src_img_iframe_merges_to_url_or_resource_url() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: 'img[x],iframe[x]',
    host: { '[attr.src]': 'v' }
})
export class SrcDirective {
    v = '';
}
";
    let code = compile(source, "src.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeUrlOrResourceUrl"),
        "expected ɵɵsanitizeUrlOrResourceUrl for host [attr.src] on 'img[x],iframe[x]' \
         (img|src=URL + iframe|src=RESOURCE_URL).\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// G1 — multi-selector with a NONE alternate filtered out, single survivor.
//
// `selector: 'div[x],iframe[x]'` with host `[attr.src]`: `div|src` is NONE
// (filtered by the host pipeline) and `iframe|src` is RESOURCE_URL. The merged
// set is exactly {RESOURCE_URL} -> `ɵɵsanitizeResourceUrl`. The old behavior
// used only `div|src` -> NONE -> NO sanitizer at all.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_src_div_iframe_emits_resource_url() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: 'div[x],iframe[x]',
    host: { '[attr.src]': 'v' }
})
export class DivIframeDirective {
    v = '';
}
";
    let code = compile(source, "div-iframe.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeResourceUrl"),
        "expected ɵɵsanitizeResourceUrl for host [attr.src] on 'div[x],iframe[x]' \
         (div|src=NONE filtered, iframe|src=RESOURCE_URL).\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// G1 control — single-element selector behavior is unchanged.
//
// `selector: 'img[x]'` host `[attr.src]` -> `img|src` = URL -> `ɵɵsanitizeUrl`.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_src_single_img_emits_url() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: 'img[x]',
    host: { '[attr.src]': 'v' }
})
export class ImgDirective {
    v = '';
}
";
    let code = compile(source, "img.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeUrl") && !code.contains("ɵɵsanitizeUrlOrResourceUrl"),
        "expected plain ɵɵsanitizeUrl for host [attr.src] on single 'img[x]'.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// G1 control — non-sensitive multi-selector must NOT gain any sanitizer.
//
// `selector: 'a[x],b[x]'` host `[attr.title]`: neither `a|title` nor `b|title`
// is security-sensitive, so the merged set is empty -> NONE -> no sanitizer.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_title_multi_selector_emits_no_sanitizer() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: 'a[x],b[x]',
    host: { '[attr.title]': 't' }
})
export class TitleMultiDirective {
    t = '';
}
";
    let code = compile(source, "title-multi.directive.ts");
    assert!(
        !code.contains("ɵɵsanitize") && !code.contains("ɵɵvalidateAttribute"),
        "host [attr.title] on 'a[x],b[x]' must not gain a sanitizer/validator.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Control — class/style host bindings must NOT gain a content sanitizer.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_class_binding_emits_no_content_sanitizer() {
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: 'a[appActive]',
    host: { '[class.active]': 'isActive' }
})
export class ActiveDirective {
    isActive = false;
}
";
    let code = compile(source, "active.directive.ts");
    assert!(
        !code.contains("ɵɵsanitizeUrl")
            && !code.contains("ɵɵsanitizeHtml")
            && !code.contains("ɵɵvalidateAttribute"),
        "host [class.active] must not gain a content sanitizer.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// v21.2.7 faithfulness (Codex iteration-10): the attribute-only / wildcard
// selector branch of the host security aggregator must honor `:not(element)`
// exclusions before scanning all known elements, mirroring upstream
// `calcPossibleSecurityContexts` (`binding_parser.ts:888-896`):
//
//   const elementNames = selector.element ? [selector.element]
//                                          : registry.allKnownElementNames();
//   const notElementNames = new Set(
//     selector.notSelectors.filter((s) => s.isElementSelector()).map((s) => s.element));
//   const possibleElementNames = elementNames.filter((n) => !notElementNames.has(n));
//
// Schema facts (`dom_security_schema.ts`): `object|data`/`object|codebase` are
// the ONLY `data`/`codebase` sinks (RESOURCE_URL) and `iframe|srcdoc` is the
// ONLY `srcdoc` sink (HTML). Excluding those elements removes the sole
// contributor, so the host binding gets NO sanitizer. The previous OXC code
// ignored `:not(...)` here and over-sanitized.
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_data_not_object_emits_no_sanitizer() {
    // `[x]:not(object)` excludes the only `data` sink (`object|data`).
    // Upstream yields no sanitizer; the old OXC behavior emitted
    // ɵɵsanitizeResourceUrl (over-sanitization).
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(object)',
    host: { '[attr.data]': 'v' }
})
export class NotObjectDirective {
    v = '';
}
";
    let code = compile(source, "not-object.directive.ts");
    assert!(
        !code.contains("ɵɵsanitize") && !code.contains("ɵɵvalidateAttribute"),
        "host [attr.data] on '[x]:not(object)' must NOT gain a sanitizer (object \
         is the only `data` sink and is excluded).\nGot:\n{code}"
    );
}

#[test]
fn directive_host_attr_srcdoc_not_iframe_emits_no_sanitizer() {
    // `[x]:not(iframe)` excludes the only `srcdoc` sink (`iframe|srcdoc`).
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(iframe)',
    host: { '[attr.srcdoc]': 'v' }
})
export class NotIframeDirective {
    v = '';
}
";
    let code = compile(source, "not-iframe.directive.ts");
    assert!(
        !code.contains("ɵɵsanitize") && !code.contains("ɵɵvalidateAttribute"),
        "host [attr.srcdoc] on '[x]:not(iframe)' must NOT gain a sanitizer \
         (iframe is the only `srcdoc` sink and is excluded).\nGot:\n{code}"
    );
}

#[test]
fn directive_host_attr_data_no_not_still_emits_resource_url() {
    // CONTROL: without `:not`, the attribute-only scan still sees `object|data`
    // -> ɵɵsanitizeResourceUrl.
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]',
    host: { '[attr.data]': 'v' }
})
export class PlainDataDirective {
    v = '';
}
";
    let code = compile(source, "plain-data.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeResourceUrl"),
        "host [attr.data] on '[x]' (no :not) must still emit ɵɵsanitizeResourceUrl \
         (object|data present).\nGot:\n{code}"
    );
}

#[test]
fn directive_host_attr_data_not_unrelated_element_still_emits_resource_url() {
    // CONTROL: `:not(div)` excludes a NON-sink element, so `object` stays in
    // the set -> ɵɵsanitizeResourceUrl.
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(div)',
    host: { '[attr.data]': 'v' }
})
export class NotDivDirective {
    v = '';
}
";
    let code = compile(source, "not-div.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeResourceUrl"),
        "host [attr.data] on '[x]:not(div)' must still emit ɵɵsanitizeResourceUrl \
         (div is not a `data` sink; object remains).\nGot:\n{code}"
    );
}

#[test]
fn directive_host_attr_data_non_element_not_still_emits_resource_url() {
    // CONTROL: a NON-element `:not()` (class selector) is not an
    // `isElementSelector()`, so it excludes nothing -> ɵɵsanitizeResourceUrl.
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(.foo)',
    host: { '[attr.data]': 'v' }
})
export class NotClassDirective {
    v = '';
}
";
    let code = compile(source, "not-class.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeResourceUrl"),
        "host [attr.data] on '[x]:not(.foo)' must still emit ɵɵsanitizeResourceUrl \
         (non-element :not does not filter).\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// v21.2.7 faithfulness (Finding 1): the `:not(element)` exclusion is a
// CASE-SENSITIVE exact match, mirroring upstream's `Set.has` over LOWERCASE
// `allKnownElementNames()` vs the case-PRESERVED `:not()` element name.
//
// Upstream `CssSelector.setElement` (directive_matching.ts:181-183) stores the
// element name verbatim (no `.toLowerCase()`), so `:not(OBJECT)` parses to
// element `"OBJECT"`. `notElementNames.has("object")` is then FALSE, so
// `object` is NOT excluded and `object|data` (RESOURCE_URL) survives.
//
// Oracle (faithful reimpl of calcPossibleSecurityContexts driving the real
// @angular/compiler@21.2.7 DomElementSchemaRegistry + CssSelector):
//   [x]:not(object) + data => [NONE]                -> None (no sanitizer)
//   [x]:not(OBJECT) + data => [NONE, RESOURCE_URL]  -> ResourceUrl
//   [x]:not(IFRAME) + src  => [NONE, URL, RESOURCE_URL] -> UrlOrResourceUrl
//
// The previous OXC code lowercased the `:not()` name AND compared
// case-insensitively, so `:not(OBJECT)` wrongly excluded `object` -> None ->
// UNDER-SANITIZATION (an XSS gap on a reachable host-binding path).
// ---------------------------------------------------------------------------
#[test]
fn directive_host_attr_data_not_uppercase_object_still_emits_resource_url() {
    // `[x]:not(OBJECT)` does NOT exclude lowercase `object` (case-sensitive
    // exact match), so `object|data` survives -> ɵɵsanitizeResourceUrl.
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(OBJECT)',
    host: { '[attr.data]': 'v' }
})
export class NotUpperObjectDirective {
    v = '';
}
";
    let code = compile(source, "not-upper-object.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeResourceUrl"),
        "host [attr.data] on '[x]:not(OBJECT)' must STILL emit ɵɵsanitizeResourceUrl \
         (uppercase :not does NOT exclude lowercase `object`; case-sensitive match).\nGot:\n{code}"
    );
}

#[test]
fn directive_host_attr_data_not_lowercase_object_emits_no_sanitizer() {
    // CONTROL companion: lowercase `[x]:not(object)` DOES exclude `object`
    // (exact match) -> no sanitizer. (Mirrors the existing
    // `directive_host_attr_data_not_object_emits_no_sanitizer` but kept next to
    // the uppercase case for the case-sensitivity contrast.)
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(object)',
    host: { '[attr.data]': 'v' }
})
export class NotLowerObjectDirective {
    v = '';
}
";
    let code = compile(source, "not-lower-object.directive.ts");
    assert!(
        !code.contains("ɵɵsanitize") && !code.contains("ɵɵvalidateAttribute"),
        "host [attr.data] on '[x]:not(object)' must NOT gain a sanitizer \
         (lowercase exact match excludes `object`).\nGot:\n{code}"
    );
}

#[test]
fn directive_host_attr_src_not_uppercase_iframe_emits_url_or_resource_url() {
    // `[x]:not(IFRAME)` does NOT exclude lowercase `iframe`. Per the oracle,
    // the surviving set across all elements is {URL, RESOURCE_URL} (iframe is
    // not the only `src` RESOURCE_URL sink) -> ɵɵsanitizeUrlOrResourceUrl.
    let source = r"
import { Directive } from '@angular/core';

@Directive({
    selector: '[x]:not(IFRAME)',
    host: { '[attr.src]': 'v' }
})
export class NotUpperIframeDirective {
    v = '';
}
";
    let code = compile(source, "not-upper-iframe.directive.ts");
    assert!(
        code.contains("ɵɵsanitizeUrlOrResourceUrl"),
        "host [attr.src] on '[x]:not(IFRAME)' must emit ɵɵsanitizeUrlOrResourceUrl \
         (uppercase :not does NOT exclude `iframe`; URL+RESOURCE_URL survive).\nGot:\n{code}"
    );
}

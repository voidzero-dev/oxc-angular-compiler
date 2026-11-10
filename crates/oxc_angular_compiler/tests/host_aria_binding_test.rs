//! Host `aria` property bindings (#704). Angular has three eras for them:
//!
//! - A, before 20.2.0-next.2: a DOM property (`ɵɵhostProperty` before 20).
//! - B, 20.2.0-next.2 to 20.3.2 and 21.0.0-next.0 to next.4: when reified,
//!   any `aria*` name longer than `aria` is written as
//!   `ɵɵattribute(ariaAttrName(name))` in the place of its DOM property.
//! - C, 20.3.3+ and 21.0.0-next.5+: an `aria-*` property becomes an
//!   attribute binding, so it comes after the `ɵɵdomProperty` calls.
//!
//! Expected output is ngtsc's (19.2.15, 20.1.8, 20.2.0 and 22.2.4 full
//! builds) and Angular's linker for the other versions.

use oxc_allocator::Allocator;
use oxc_angular_compiler::linker::{LinkOptions, link_with_options};
use oxc_angular_compiler::{AngularVersion, TransformOptions, transform_angular_file};

fn version(v: &str) -> AngularVersion {
    AngularVersion::parse(v).unwrap()
}

fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile(source: &str, angular_version: Option<AngularVersion>) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { angular_version, ..Default::default() };
    let result = transform_angular_file(&allocator, "/x/a.ts", source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "unexpected diagnostics: {:?}", result.diagnostics);
    compact(&result.code)
}

fn link(code: &str, angular_version: Option<AngularVersion>) -> String {
    let allocator = Allocator::default();
    let options = LinkOptions { angular_version, ..LinkOptions::default() };
    let result = link_with_options(&allocator, code, "test.mjs", &options);
    assert!(result.linked);
    compact(&result.code)
}

/// The update block of the host bindings function.
fn update_block(code: &str) -> &str {
    let start = code.find("if((rf&2)){").expect("host update block") + "if((rf&2)){".len();
    let end = code[start..].find('}').unwrap() + start;
    &code[start..end]
}

/// The issue's repro: ngtsc 22.2.4 emits both bindings as attributes.
#[test]
fn host_aria_property_is_an_attribute() {
    let code = compile(
        "import { Directive, HostBinding } from '@angular/core';
@Directive({ selector: '[d]', host: { '[aria-label]': 'l' } })
export class D { l = 'a'; @HostBinding('aria-hidden') h = true; }",
        None,
    );
    assert!(code.contains("hostVars:2"), "{code}");
    assert_eq!(update_block(&code), r#"i0.ɵɵattribute("aria-label",ctx.l)("aria-hidden",ctx.h);"#);
}

/// ngtsc 22.2.4 (with host type-checking off: `[aria]` and `[aria-]` are not
/// known properties): only `aria-` and a longer name is an attribute.
#[test]
fn only_aria_dash_names_become_attributes() {
    let code = compile(
        "import { Component, HostBinding } from '@angular/core';
@Component({ selector: 'c', template: '', host: { '[aria-label]': 'l', '[attr.aria-x]': 'l', '[ariaLabel]': 'l', '[aria]': 'l', '[aria-]': 'l' } })
export class C { l = 'a'; @HostBinding('aria-hidden') h = true; }",
        None,
    );
    assert!(code.contains("hostVars:6"), "{code}");
    assert_eq!(
        update_block(&code),
        r#"i0.ɵɵdomProperty("ariaLabel",ctx.l)("aria",ctx.l)("aria-",ctx.l);i0.ɵɵattribute("aria-label",ctx.l)("aria-x",ctx.l)("aria-hidden",ctx.h);"#
    );
}

/// A constant `aria-*` binding stays a binding (ngtsc 22.2.4), like
/// `[attr.title]`; it is not moved to `hostAttrs`.
#[test]
fn constant_aria_binding_is_not_a_host_attribute() {
    let code = compile(
        "import { Directive } from '@angular/core';
@Directive({ selector: '[d]', host: { '[aria-label]': \"'x'\", '[attr.title]': \"'y'\", '[ariaLabel]': \"'z'\" } })
export class D {}",
        None,
    );
    assert!(code.contains("hostVars:3"), "{code}");
    assert!(!code.contains("hostAttrs"), "{code}");
    assert_eq!(
        update_block(&code),
        r#"i0.ɵɵdomProperty("ariaLabel","z");i0.ɵɵattribute("aria-label","x")("title","y");"#
    );
}

/// Angular drops the `@` of a legacy animation before it checks the name, so
/// ngtsc 22.2.4 writes `[@aria-t]` as the attribute `aria-t`.
#[test]
fn legacy_animation_with_an_aria_name_is_an_attribute() {
    let code = compile(
        "import { Directive } from '@angular/core';
@Directive({ selector: '[d]', host: { '[@aria-t]': 'l', '[aria-label]': 'l' } })
export class D { l = 1 }",
        None,
    );
    assert_eq!(update_block(&code), r#"i0.ɵɵattribute("aria-t",ctx.l)("aria-label",ctx.l);"#);

    let code = compile(
        "import { Directive, HostBinding } from '@angular/core';
@Directive({ selector: '[d]' })
export class D { @HostBinding('@aria-t') t = 1; @HostBinding('attr.aria-z') z = 1; @HostBinding('ariaLabel') y = 1; }",
        None,
    );
    assert!(code.contains("hostVars:3"), "{code}");
    assert_eq!(
        update_block(&code),
        r#"i0.ɵɵdomProperty("ariaLabel",ctx.y);i0.ɵɵattribute("aria-t",ctx.t)("aria-z",ctx.z);"#
    );
}

/// Angular's compliance golden `host_bindings/aria_bindings`.
#[test]
fn angular_aria_bindings_golden() {
    let code = compile(
        "import {Component} from '@angular/core';
@Component({
  template: ``,
  host: {
    '[attr.aria-disabled]': 'disabled',
    '[aria-readonly]': 'readonly',
    '[ariaLabel]': 'label',
  },
})
export class MyComponent {
  disabled = '';
  readonly = '';
  label = '';
}",
        None,
    );
    assert_eq!(
        update_block(&code),
        r#"i0.ɵɵdomProperty("ariaLabel",ctx.label);i0.ɵɵattribute("aria-disabled",ctx.disabled)("aria-readonly",ctx.readonly);"#
    );
}

const ERA_A_19: &str = r#"i0.ɵɵhostProperty("aria-label",ctx.l)("ariaLabel",ctx.l)("aria",ctx.l)("title",ctx.l);i0.ɵɵsyntheticHostProperty("@aria-t",ctx.l);i0.ɵɵattribute("aria-x",ctx.l);"#;
const ERA_A: &str = r#"i0.ɵɵdomProperty("aria-label",ctx.l)("ariaLabel",ctx.l)("aria",ctx.l)("title",ctx.l);i0.ɵɵsyntheticHostProperty("@aria-t",ctx.l);i0.ɵɵattribute("aria-x",ctx.l);"#;
const ERA_B: &str = r#"i0.ɵɵattribute("aria-label",ctx.l)("aria-label",ctx.l);i0.ɵɵdomProperty("aria",ctx.l)("title",ctx.l);i0.ɵɵsyntheticHostProperty("@aria-t",ctx.l);i0.ɵɵattribute("aria-x",ctx.l);"#;
const ERA_C: &str = r#"i0.ɵɵdomProperty("ariaLabel",ctx.l)("aria",ctx.l)("title",ctx.l);i0.ɵɵattribute("aria-label",ctx.l)("aria-x",ctx.l)("aria-t",ctx.l);"#;

/// The versions of each era, with Angular's output for the host
/// `{ "aria-label", "ariaLabel", "aria", "attr.aria-x", "title", "@aria-t" }`.
const ERAS: &[(&str, &str)] = &[
    ("19.2.15", ERA_A_19),
    ("20.1.8", ERA_A),
    ("20.2.0-next.0", ERA_A),
    ("20.2.0-next.1", ERA_A),
    ("20.2.0-next.2", ERA_B),
    ("20.2.0", ERA_B),
    ("20.2.4", ERA_B),
    ("20.3.2", ERA_B),
    ("21.0.0-next.4", ERA_B),
    ("20.3.3", ERA_C),
    ("21.0.0-next.5", ERA_C),
    ("21.0.0", ERA_C),
    ("22.2.4", ERA_C),
];

#[test]
fn full_compile_follows_the_angular_version() {
    let source = "import { Directive } from '@angular/core';
@Directive({ selector: '[d]', host: { '[aria-label]': 'l', '[ariaLabel]': 'l', '[aria]': 'l', '[attr.aria-x]': 'l', '[title]': 'l', '[@aria-t]': 'l' } })
export class D { l: any; }";
    for &(v, expected) in ERAS {
        let code = compile(source, Some(version(v)));
        assert_eq!(update_block(&code), expected, "{v}");
    }
    assert_eq!(update_block(&compile(source, None)), ERA_C);
}

#[test]
fn linker_follows_the_angular_version() {
    for &(v, expected) in ERAS {
        let code = format!(
            r#"import * as i0 from "@angular/core";
class D {{
}}
D.ɵdir = i0.ɵɵngDeclareDirective({{ minVersion: "14.0.0", version: "{v}", type: D, isStandalone: true, selector: "[d]", host: {{ properties: {{ "aria-label": "l", "ariaLabel": "l", "aria": "l", "attr.aria-x": "l", "title": "l", "@aria-t": "l" }} }}, ngImport: i0 }});
"#
        );
        let linked = link(&code, Some(version(v)));
        assert_eq!(update_block(&linked), expected, "{v}");
    }
}

/// Era B (Angular 20.3.2's linker): every `aria*` name longer than `aria` is
/// an attribute, a camel-case name lower-cased after `aria-`, kept in its
/// DOM property's place.
#[test]
fn reified_aria_attribute_names() {
    let code = r#"import * as i0 from "@angular/core";
class D {
}
D.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "20.3.2", type: D, isStandalone: true, selector: "[d]", host: { properties: { "ariaLabelledByElements": "l", "aria-Busy": "l", "ARIA-x": "l", "aria": "l", "innerHTML": "l", "attr.aria-q": "l" }, listeners: { "click": "b()" } }, ngImport: i0 });
"#;
    let linked = link(code, Some(version("20.3.2")));
    assert_eq!(
        update_block(&linked),
        r#"i0.ɵɵattribute("aria-labelledbyelements",ctx.l)("aria-Busy",ctx.l);i0.ɵɵdomProperty("ARIA-x",ctx.l)("aria",ctx.l)("innerHTML",ctx.l,i0.ɵɵsanitizeHtml);i0.ɵɵattribute("aria-q",ctx.l);"#
    );
    // Era C (22.2.4) for the same host.
    let linked = link(code, Some(version("22.2.4")));
    assert_eq!(
        update_block(&linked),
        r#"i0.ɵɵdomProperty("ariaLabelledByElements",ctx.l)("ARIA-x",ctx.l)("aria",ctx.l)("innerHTML",ctx.l,i0.ɵɵsanitizeHtml);i0.ɵɵattribute("aria-Busy",ctx.l)("aria-q",ctx.l);"#
    );
}

//! Sanitizers that ngtsc emits and the port used to skip.
//!
//! Expected fragments are ngtsc 22.2.2's output with the whitespace removed.
//! Each fix is gated on the releases that shipped it upstream.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{AngularVersion, TransformOptions, transform_angular_file};

fn compile(source: &str, version: Option<&str>) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions {
        angular_version: version.map(|v| AngularVersion::parse(v).unwrap()),
        ..TransformOptions::default()
    };
    let result = transform_angular_file(&allocator, "t.ts", source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn component(template: &str) -> String {
    format!(
        "import {{ Component }} from '@angular/core';\n\
         @Component({{ selector: 'app-t', template: `{template}` }})\n\
         export class T {{ u = 'x'; }}\n"
    )
}

fn directive(selector: &str, host: &str) -> String {
    format!(
        "import {{ Directive }} from '@angular/core';\n\
         @Directive({{ selector: '{selector}', host: {host} }})\n\
         export class D {{ u = 'x'; }}\n"
    )
}

/// Asserts `needle` is in the output for every version in `with`, and
/// `without` is there instead for every version in `before`.
fn assert_gated(source: &str, needle: &str, without: &str, with: &[&str], before: &[&str]) {
    for version in with.iter().map(|v| Some(*v)).chain([None]) {
        let code = compile(source, version);
        assert!(code.contains(needle), "{version:?}: missing `{needle}` in\n{code}");
    }
    for version in before {
        let code = compile(source, Some(version));
        assert!(code.contains(without), "{version}: missing `{without}` in\n{code}");
    }
}

#[test]
fn two_way_properties_are_sanitized() {
    let with = ["20.3.25", "21.2.17", "22.0.1", "22.1.0-next.0", "22.2.2"];
    let before = ["19.2.25", "20.3.24", "21.2.16", "22.0.0"];
    assert_gated(
        &component(r#"<div [(innerHTML)]="u"></div>"#),
        r#"ɵɵtwoWayProperty("innerHTML",ctx.u,i0.ɵɵsanitizeHtml)"#,
        r#"ɵɵtwoWayProperty("innerHTML",ctx.u)"#,
        &with,
        &before,
    );
    assert_gated(
        &component(r#"<iframe [(src)]="u"></iframe>"#),
        r#"ɵɵtwoWayProperty("src",ctx.u,i0.ɵɵsanitizeResourceUrl)"#,
        r#"ɵɵtwoWayProperty("src",ctx.u)"#,
        &with,
        &before,
    );
    assert_gated(
        &component(r#"<a [(href)]="u"></a>"#),
        r#"ɵɵtwoWayProperty("href",ctx.u,i0.ɵɵsanitizeUrl)"#,
        r#"ɵɵtwoWayProperty("href",ctx.u)"#,
        &with,
        &before,
    );
    assert_gated(
        &component(r#"<ng-template [(innerHTML)]="u"></ng-template>"#),
        r#"ɵɵtwoWayProperty("innerHTML",ctx.u,i0.ɵɵsanitizeHtml)"#,
        r#"ɵɵtwoWayProperty("innerHTML",ctx.u)"#,
        &with,
        &before,
    );
}

#[test]
fn host_bindings_are_sanitized_for_any_concrete_host() {
    // The directive can land on another element through `hostDirectives`, so
    // a selector without a URL sink no longer turns the sanitizer off.
    let with = ["20.3.28", "21.2.20", "22.1.0", "22.2.0-next.0"];
    let before = ["20.3.27", "21.2.19", "22.0.8", "22.1.0-rc.0"];
    assert_gated(
        &directive("div[d]", r"{ '[href]': 'u', '[attr.src]': 'u' }"),
        r#"ɵɵdomProperty("href",ctx.u,i0.ɵɵsanitizeUrlOrResourceUrl)"#,
        r#"ɵɵdomProperty("href",ctx.u)"#,
        &with,
        &before,
    );
    assert_gated(
        &directive("div[d]", r"{ '[attr.src]': 'u' }"),
        r#"ɵɵattribute("src",ctx.u,i0.ɵɵsanitizeUrlOrResourceUrl)"#,
        r#"ɵɵattribute("src",ctx.u)"#,
        &with,
        &before,
    );
    // A selector that is itself a URL sink still widens: `a[d]` may run on
    // `<link>`, whose `href` is a resource URL.
    assert_gated(
        &directive("a[d]", r"{ '[href]': 'u' }"),
        r#"ɵɵdomProperty("href",ctx.u,i0.ɵɵsanitizeUrlOrResourceUrl)"#,
        r#"ɵɵdomProperty("href",ctx.u,i0.ɵɵsanitizeUrl)"#,
        &with,
        &before,
    );
    assert_gated(
        &directive("div[d]", r"{ '[attr.srcdoc]': 'u', '[attr.sandbox]': 'u' }"),
        r#"ɵɵattribute("srcdoc",ctx.u,i0.ɵɵsanitizeHtml)("sandbox",ctx.u,i0.ɵɵvalidateAttribute)"#,
        r#"ɵɵattribute("srcdoc",ctx.u)("sandbox",ctx.u)"#,
        &with,
        &before,
    );

    // A context every element shares keeps its own sanitizer, and a name no
    // element sanitizes stays bare.
    let code = compile(&directive("my-el", r"{ '[innerHTML]': 'u', '[title]': 'u' }"), None);
    assert!(
        code.contains(r#"ɵɵdomProperty("innerHTML",ctx.u,i0.ɵɵsanitizeHtml)("title",ctx.u)"#),
        "{code}"
    );
}

#[test]
fn any_mathml_element_href_is_a_url() {
    assert_gated(
        &component(
            r#"<math><foo [attr.href]="u"></foo><mfenced [attr.href]="u"></mfenced></math>"#,
        ),
        r#"ɵɵattribute("href",ctx.u,i0.ɵɵsanitizeUrl);i0.ɵɵadvance();i0.ɵɵattribute("href",ctx.u,i0.ɵɵsanitizeUrl)"#,
        r#"ɵɵattribute("href",ctx.u);i0.ɵɵadvance();i0.ɵɵattribute("href",ctx.u)"#,
        &["22.0.1", "22.1.0-next.0"],
        &["21.2.25", "22.0.0"],
    );
}

#[test]
fn namespaced_elements_fall_back_to_the_html_schema() {
    let source =
        component(r#"<svg><iframe [src]="u"></iframe><iframe [srcdoc]="u"></iframe></svg>"#);
    let with = ["22.0.2", "22.1.0-next.1"];
    let before = ["22.0.1", "22.1.0-next.0"];
    assert_gated(
        &source,
        r#"("src",ctx.u,i0.ɵɵsanitizeResourceUrl)"#,
        r#"("src",ctx.u)"#,
        &with,
        &before,
    );
    assert_gated(
        &source,
        r#"("srcdoc",ctx.u,i0.ɵɵsanitizeHtml)"#,
        r#"("srcdoc",ctx.u)"#,
        &with,
        &before,
    );
}

#[test]
fn iframe_credentialless_is_validated() {
    assert_gated(
        &component(r#"<iframe [attr.credentialless]="u"></iframe>"#),
        r#"ɵɵattribute("credentialless",ctx.u,i0.ɵɵvalidateAttribute)"#,
        r#"ɵɵattribute("credentialless",ctx.u)"#,
        &["22.0.2", "22.1.0-next.1"],
        &["22.0.1", "22.1.0-next.0"],
    );
}

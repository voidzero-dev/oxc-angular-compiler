//! Finding 2: namespaced `attr.` bindings must emit the v21.2.7-faithful
//! `ɵɵattribute(name, value, sanitizer?, namespace?)` form.
//!
//! TEMPLATE path: upstream `createBoundElementProperty` stores the merged
//! `:ns:local` name (`mergeNsAndName`), so `binding_specialization`'s
//! `splitNsName` splits it and codegen emits the namespace argument with the
//! LOCAL name. This works for ANY namespace prefix (not just well-known ones).
//!
//! HOST path: upstream host ingest stores the PLAIN name (no merge), so
//! `splitNsName("xlink:href") = [null, "xlink:href"]` does NOT split and codegen
//! emits `ɵɵattribute("xlink:href", ...)` with NO namespace argument.
//!
//! All expected `ɵɵattribute(...)` forms below were captured from
//! @angular/compiler@21.2.7 via `parseTemplate` + `compileComponentFromMetadata`
//! / `compileDirectiveFromMetadata` (the oracle), NOT from OXC.

use oxc_allocator::Allocator;
use oxc_angular_compiler::transform_angular_file;

fn compile(source: &str, file: &str) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, file, source, None, None);
    assert!(!result.has_errors(), "compile errors: {:?}", result.diagnostics);
    result.code
}

fn component_with_template(template: &str) -> String {
    let source = format!(
        r#"
import {{ Component }} from '@angular/core';

@Component({{
    selector: 'test-cmp',
    template: '{template}'
}})
export class TestCmp {{
    u = '';
}}
"#
    );
    compile(&source, "test.component.ts")
}

fn directive_with_host(selector: &str, host: &str) -> String {
    let source = format!(
        r#"
import {{ Directive }} from '@angular/core';

@Directive({{
    selector: '{selector}',
    host: {{ {host} }}
}})
export class TestDir {{
    url = '';
}}
"#
    );
    compile(&source, "test.directive.ts")
}

// ---------------------------------------------------------------------------
// TEMPLATE path: well-known prefix `xlink` -> local name + namespace arg.
// Upstream: ɵɵattribute("href", ctx.u, ɵɵsanitizeUrl, "xlink")
// ---------------------------------------------------------------------------
#[test]
fn template_attr_xlink_href_emits_local_name_and_namespace() {
    let code = component_with_template(r#"<svg><a [attr.xlink:href]=\"u\"></a></svg>"#);
    assert!(
        code.contains(r#"ɵɵattribute("href",ctx.u,i0.ɵɵsanitizeUrl,"xlink")"#),
        "expected ɵɵattribute(\"href\", ..., ɵɵsanitizeUrl, \"xlink\").\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// TEMPLATE path: ARBITRARY (non-well-known) prefix must ALSO split, because
// upstream stores the merged `:custom:foo` form and `splitNsName` is prefix-
// agnostic. Upstream: ɵɵattribute("foo", ctx.u, null, "custom")
// (This is the case the old OXC `KNOWN_NS_PREFIXES`-only split got WRONG.)
// ---------------------------------------------------------------------------
#[test]
fn template_attr_custom_prefix_emits_local_name_and_namespace() {
    let code = component_with_template(r#"<svg><a [attr.custom:foo]=\"u\"></a></svg>"#);
    assert!(
        code.contains(r#"ɵɵattribute("foo",ctx.u,null,"custom")"#),
        "expected ɵɵattribute(\"foo\", ctx.u, null, \"custom\") for an arbitrary ns prefix.\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// TEMPLATE control: non-namespaced attr stays a plain 2-arg call.
// Upstream: ɵɵattribute("foo", ctx.u)
// ---------------------------------------------------------------------------
#[test]
fn template_attr_plain_no_namespace_arg() {
    let code = component_with_template(r#"<div [attr.foo]=\"u\"></div>"#);
    assert!(
        code.contains(r#"ɵɵattribute("foo",ctx.u)"#),
        "expected plain ɵɵattribute(\"foo\", ctx.u).\nGot:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// HOST path: name kept PLAIN, NO namespace argument (upstream host ingest never
// merges; splitNsName("xlink:href") does not split).
// Upstream: ɵɵattribute("xlink:href", ctx.url, ɵɵsanitizeUrl)
// ---------------------------------------------------------------------------
#[test]
fn host_attr_xlink_href_keeps_plain_name_no_namespace() {
    let code = directive_with_host("a[appLink]", r#"'[attr.xlink:href]': 'url'"#);
    assert!(
        code.contains(r#"ɵɵattribute("xlink:href",ctx.url,i0.ɵɵsanitizeUrl)"#),
        "expected host ɵɵattribute(\"xlink:href\", ctx.url, ɵɵsanitizeUrl) with NO namespace arg.\nGot:\n{code}"
    );
    assert!(
        !code.contains(r#"ɵɵattribute("href",ctx.url"#),
        "host path must NOT split xlink:href into local name + namespace.\nGot:\n{code}"
    );
}

#[test]
fn host_attr_custom_prefix_keeps_plain_name_no_namespace() {
    let code = directive_with_host("[appLink]", r#"'[attr.custom:foo]': 'url'"#);
    assert!(
        code.contains(r#"ɵɵattribute("custom:foo",ctx.url)"#),
        "expected host ɵɵattribute(\"custom:foo\", ctx.url) with NO namespace arg.\nGot:\n{code}"
    );
}

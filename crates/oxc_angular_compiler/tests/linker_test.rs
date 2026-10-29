//! Tests for the Angular linker: key quoting and version-dependent emit.

use oxc_allocator::Allocator;
use oxc_angular_compiler::AngularVersion;
use oxc_angular_compiler::linker::{LinkOptions, link, link_with_options};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Helper to build a ɵɵngDeclareDirective source with a given inputs block.
fn make_directive_source(inputs_block: &str) -> String {
    format!(
        r#"import * as i0 from "@angular/core";
export class MyDir {{}}
MyDir.ɵdir = i0.ɵɵngDeclareDirective({{ minVersion: "14.0.0", version: "17.0.0", type: MyDir, selector: "[myDir]", inputs: {{ {inputs_block} }} }});"#
    )
}

/// Helper to build a ɵɵngDeclareDirective source with a given outputs block.
fn make_directive_source_with_outputs(outputs_block: &str) -> String {
    format!(
        r#"import * as i0 from "@angular/core";
export class MyDir {{}}
MyDir.ɵdir = i0.ɵɵngDeclareDirective({{ minVersion: "14.0.0", version: "17.0.0", type: MyDir, selector: "[myDir]", outputs: {{ {outputs_block} }} }});"#
    )
}

#[test]
fn test_link_inputs_dotted_key() {
    let allocator = Allocator::default();
    let code = make_directive_source(r#""fxFlexAlign.xs": "fxFlexAlignXs""#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_inputs_hyphenated_key() {
    let allocator = Allocator::default();
    let code = make_directive_source(r#""fxFlexAlign.lt-sm": "fxFlexAlignLtSm""#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_inputs_simple_identifier() {
    let allocator = Allocator::default();
    let code = make_directive_source(r#"fxFlexAlign: "fxFlexAlign""#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_inputs_object_format_dotted_key() {
    let allocator = Allocator::default();
    let code = make_directive_source(
        r#""fxFlexAlign.xs": { classPropertyName: "fxFlexAlignXs", publicName: "fxFlexAlign.xs", isRequired: false, isSignal: false }"#,
    );
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_inputs_array_format_dotted_key() {
    let allocator = Allocator::default();
    let code = make_directive_source(r#""fxFlexAlign.xs": ["fxFlexAlign.xs", "fxFlexAlignXs"]"#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_outputs_dotted_key() {
    let allocator = Allocator::default();
    let code = make_directive_source_with_outputs(r#""activate.xs": "activateXs""#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_outputs_hyphenated_key() {
    let allocator = Allocator::default();
    let code = make_directive_source_with_outputs(r#""activate.lt-sm": "activateLtSm""#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_outputs_simple_identifier() {
    let allocator = Allocator::default();
    let code = make_directive_source_with_outputs(r#"activate: "activate""#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_inputs_array_format_with_transform_function() {
    let allocator = Allocator::default();
    let code =
        make_directive_source(r#"push: ["cdkConnectedOverlayPush", "push", i0.booleanAttribute]"#);
    let result = link(&allocator, &code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

/// Regression: signal form FormField directive declares
/// `controlCreate: { passThroughInput: "formField" }` in its partial metadata.
/// The linker must emit `ɵɵControlFeature("formField")` in the features array,
/// otherwise `DirectiveDef.controlDef` is never set and the runtime
/// `ɵɵcontrolCreate()` / `ɵɵcontrol()` instructions become no-ops.
/// See voidzero-dev/oxc-angular-compiler#229.
#[test]
fn test_link_control_feature_pass_through_input() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
export class FormField {}
FormField.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.2.8", type: FormField, selector: "[formField]", inputs: { field: { classPropertyName: "field", publicName: "formField", isRequired: true, isSignal: true } }, controlCreate: { passThroughInput: "formField" }, isStandalone: true, isSignal: true });"#;
    let result = link(&allocator, code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

#[test]
fn test_link_control_feature_null_pass_through_input() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
export class MyControl {}
MyControl.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.2.8", type: MyControl, selector: "[myControl]", controlCreate: { passThroughInput: null }, isStandalone: true, isSignal: true });"#;
    let result = link(&allocator, code, "test.mjs");
    insta::assert_snapshot!(result.code);
}

/// Helper to build a ɵɵngDeclareComponent source with a given `styles` array.
fn make_component_source_with_styles(styles: &str) -> String {
    format!(
        r#"import * as i0 from "@angular/core";
export class MyCmp {{}}
MyCmp.ɵcmp = i0.ɵɵngDeclareComponent({{ minVersion: "14.0.0", version: "17.0.0", type: MyCmp, selector: "my-cmp", template: "<div></div>", styles: [{styles}] }});"#
    )
}

#[test]
fn test_link_styles_template_literal_matches_string_literal() {
    for (template_styles, string_styles) in [
        ("`.a { color: red }`", r#"".a { color: red }""#),
        (r#"`.a { content: "\\f101\n" }`"#, r#"".a { content: \"\\f101\n\" }""#),
    ] {
        let allocator = Allocator::default();
        let template =
            link(&allocator, &make_component_source_with_styles(template_styles), "test.mjs");
        let string =
            link(&allocator, &make_component_source_with_styles(string_styles), "test.mjs");
        assert!(template.code.contains("styles: ["), "styles dropped:\n{}", template.code);
        assert_eq!(template.code, string.code);
    }
}

#[test]
fn test_link_host_bindings_template_literal_matches_string_literal() {
    let make = |prop: &str, listener: &str| {
        format!(
            r#"import * as i0 from "@angular/core";
export class MyDir {{}}
MyDir.ɵdir = i0.ɵɵngDeclareDirective({{ minVersion: "14.0.0", version: "17.0.0", type: MyDir, selector: "[myDir]", host: {{ properties: {{ "id": {prop} }}, listeners: {{ "click": {listener} }} }} }});"#
        )
    };
    let allocator = Allocator::default();
    let template = link(&allocator, &make("`this.dirId`", "`onClick($event)`"), "test.mjs");
    let string = link(&allocator, &make(r#""this.dirId""#, r#""onClick($event)""#), "test.mjs");
    assert!(template.code.contains("dirId"), "host property dropped:\n{}", template.code);
    assert!(template.code.contains("onClick"), "host listener dropped:\n{}", template.code);
    assert_eq!(template.code, string.code);
}

/// Static `host.attributes` values rewritten as template literals keep their
/// cooked value and emit byte-identical hostAttrs to the quoted form.
#[test]
fn test_link_host_attributes_template_literal_matches_string_literal() {
    let make = |value: &str| {
        format!(
            r#"import * as i0 from "@angular/core";
export class MyDir {{}}
MyDir.ɵdir = i0.ɵɵngDeclareDirective({{ minVersion: "14.0.0", version: "17.0.0", type: MyDir, selector: "[myDir]", host: {{ attributes: {{ "aria-label": {value} }} }} }});"#
        )
    };
    let allocator = Allocator::default();
    let template = link(&allocator, &make(r#"`say "hi"`"#), "test.mjs");
    let string = link(&allocator, &make(r#""say \"hi\"""#), "test.mjs");
    assert!(
        template.code.contains(r#"hostAttrs: ["aria-label", "say \"hi\""]"#),
        "host attribute dropped or unescaped:\n{}",
        template.code
    );
    assert_eq!(template.code, string.code);
}

/// `inputs` values rewritten as template literals are treated like string
/// literals. For the aliased array form `[publicName, classPropertyName]` this
/// preserves the declaration-format detection — otherwise the array would be
/// passed through verbatim and the runtime would read the public name as
/// `InputFlags`. Array elements keep their original source spelling, so only
/// the simple and object forms are asserted byte-identical.
#[test]
fn test_link_inputs_template_literal_matches_string_literal() {
    // Simple value form: same emitted output either way.
    let allocator = Allocator::default();
    let template = link(&allocator, &make_directive_source(r#"prop: `publicProp`"#), "test.mjs");
    let string = link(&allocator, &make_directive_source(r#"prop: "publicProp""#), "test.mjs");
    assert_eq!(template.code, string.code);

    // Aliased array form: the first template-literal element must be recognised
    // as a string so the [0, ...] flags element is inserted.
    let allocator = Allocator::default();
    let template =
        link(&allocator, &make_directive_source(r#"prop: [`publicProp`, `prop`]"#), "test.mjs");
    assert!(
        template.code.contains(r#"prop: [0, `publicProp`, `prop`]"#),
        "aliased input lost its InputFlags element:\n{}",
        template.code
    );
    let string =
        link(&allocator, &make_directive_source(r#"prop: ["publicProp", "prop"]"#), "test.mjs");
    assert!(
        string.code.contains(r#"prop: [0, "publicProp", "prop"]"#),
        "quoted input changed behaviour:\n{}",
        string.code
    );

    // Object format: publicName/classPropertyName may also be template literals.
    let allocator = Allocator::default();
    let template = link(
        &allocator,
        &make_directive_source(
            r#"prop: { classPropertyName: `prop`, publicName: `publicProp`, isRequired: false, isSignal: false }"#,
        ),
        "test.mjs",
    );
    let string = link(
        &allocator,
        &make_directive_source(
            r#"prop: { classPropertyName: "prop", publicName: "publicProp", isRequired: false, isSignal: false }"#,
        ),
        "test.mjs",
    );
    assert_eq!(template.code, string.code);
}

/// `deps: "invalid"` rewritten as a template literal still selects the
/// ɵɵinvalidFactory path instead of falling through to `new Type()`.
#[test]
fn test_link_factory_invalid_deps_template_literal() {
    let allocator = Allocator::default();
    let code = r#"
import * as i0 from "@angular/core";
class MyService {
}
MyService.ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyService, deps: `invalid`, target: i0.ɵɵFactoryTarget.Injectable });
"#;
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    assert!(result.code.contains("ɵɵinvalidFactory"), "deps not invalid:\n{}", result.code);
}

/// A component with `[src]` bindings on `img`, `video` and `a` (control), plus a safe read.
const SRC_BINDINGS_COMPONENT: &str = r#"
import * as i0 from "@angular/core";
class Media {
}
Media.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.0", type: Media, isStandalone: true, selector: "x-media", ngImport: i0, template: '<img [src]="url"><video [src]="url"></video><a [href]="url">{{ item?.label }}</a>' });
"#;

/// A directive binding `src` on its `img` host element.
const SRC_HOST_DIRECTIVE: &str = r#"
import * as i0 from "@angular/core";
class ImgSrc {
}
ImgSrc.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.0.0", type: ImgSrc, isStandalone: true, selector: "img[imgSrc]", host: { properties: { "src": "url" } }, ngImport: i0 });
"#;

/// A component whose host binding and template both read `item?.label`.
const SAFE_NAV_HOST_COMPONENT: &str = r#"
import * as i0 from "@angular/core";
class Item {
}
Item.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.0", type: Item, isStandalone: true, selector: "x-item", host: { properties: { "title": "item?.label" } }, ngImport: i0, template: '<span>{{ item?.label }}</span>' });
"#;

/// A component whose `@if` block has a single namespaced root element.
const SVG_BLOCK_ROOT_COMPONENT: &str = r#"
import * as i0 from "@angular/core";
class Shape {
}
Shape.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.0", type: Shape, isStandalone: true, selector: "x-shape", ngImport: i0, template: '@if (show) { <svg:rect></svg:rect> }' });
"#;

fn link_for_version(code: &str, version: Option<AngularVersion>) -> String {
    let allocator = Allocator::default();
    let options = LinkOptions { angular_version: version, ..LinkOptions::default() };
    let result = link_with_options(&allocator, code, "test.mjs", &options);
    assert!(result.linked);
    result.code
}

/// Angular before 22.2.2 sanitizes `img|src` and `video|src` as URLs, and its runtime relies
/// on that sanitizer to unwrap `SafeUrl`s. The linker must honor the application's version.
#[test]
fn test_link_sanitizes_img_video_src_before_22_2_2() {
    for version in [AngularVersion::new(21, 2, 0), AngularVersion::new(22, 2, 1)] {
        let code = link_for_version(SRC_BINDINGS_COMPONENT, Some(version));
        assert_eq!(
            code.matches(r#"ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrl)"#).count(),
            2,
            "{version:?}: img and video src must be sanitized:\n{code}"
        );
        assert!(code.contains(r#"ɵɵdomProperty("href",ctx.url,i0.ɵɵsanitizeUrl)"#), "{code}");
    }
}

/// Angular 22.2.2 removed `img|src` and `video|src` from the URL security context
/// (angular/angular@439ff1bdf4). An unset version keeps assuming the latest Angular.
#[test]
fn test_link_does_not_sanitize_img_video_src_from_22_2_2() {
    for version in [Some(AngularVersion::new(22, 2, 2)), None] {
        let code = link_for_version(SRC_BINDINGS_COMPONENT, version);
        assert_eq!(
            code.matches(r#"ɵɵdomProperty("src",ctx.url)"#).count(),
            2,
            "{version:?}: img and video src must not be sanitized:\n{code}"
        );
        assert!(code.contains(r#"ɵɵdomProperty("href",ctx.url,i0.ɵɵsanitizeUrl)"#), "{code}");
    }

    let allocator = Allocator::default();
    let default_link = link(&allocator, SRC_BINDINGS_COMPONENT, "test.mjs").code;
    assert_eq!(default_link, link_for_version(SRC_BINDINGS_COMPONENT, None));
}

/// Checks that the application's version reaches the directive host path. 22.0.8 still
/// picks the sanitizer from the `img` selector (`img|src` is a URL there); from 22.1.0 the
/// binding is checked against every concrete host (angular/angular@d06e3748b7), so `src`
/// defers the URL / resource URL choice to runtime.
#[test]
fn test_link_host_src_binding_respects_angular_version() {
    let code = link_for_version(SRC_HOST_DIRECTIVE, Some(AngularVersion::new(22, 0, 8)));
    assert!(
        code.contains(r#"ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrl)"#),
        "22.0.8 must sanitize `img|src` from the selector:\n{code}"
    );
    for version in [Some(AngularVersion::new(22, 2, 1)), None] {
        let code = link_for_version(SRC_HOST_DIRECTIVE, version);
        assert!(
            code.contains(r#"ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrlOrResourceUrl)"#),
            "{version:?} must sanitize `src` for any concrete host:\n{code}"
        );
    }
}

/// Component host bindings follow the application's version (`ɵɵhostProperty` before 20,
/// `ɵɵdomProperty` from 20) but keep legacy `?.`, matching Angular's linker on a 21.0.0
/// declaration with `@angular/compiler` 19.2.0 and 22.2.1.
#[test]
fn test_link_component_host_respects_angular_version() {
    let v19 = link_for_version(SAFE_NAV_HOST_COMPONENT, Some(AngularVersion::new(19, 2, 0)));
    assert!(
        v19.contains(r#"ɵɵhostProperty("title",((ctx.item == null)? null: ctx.item.label))"#),
        "19.2.0: component host must use ɵɵhostProperty with legacy `?.`:\n{v19}"
    );
    let v22 = link_for_version(SAFE_NAV_HOST_COMPONENT, Some(AngularVersion::new(22, 2, 1)));
    assert!(
        v22.contains(r#"ɵɵdomProperty("title",((ctx.item == null)? null: ctx.item.label))"#),
        "22.2.1: component host must use ɵɵdomProperty with legacy `?.`:\n{v22}"
    );
}

/// The application's version reaches template ingest: Angular 22.2.2 strips the namespace
/// from a control-flow block's root tag (angular/angular@0244a1cb3), earlier versions keep it.
#[test]
fn test_link_block_root_tag_respects_angular_version() {
    let before = link_for_version(SVG_BLOCK_ROOT_COMPONENT, Some(AngularVersion::new(22, 2, 1)));
    assert!(
        before.contains(r#"ɵɵconditionalCreate(0,Shape_Conditional_0_Template,1,0,":svg:rect")"#),
        "22.2.1: block root tag must keep its namespace:\n{before}"
    );
    let after = link_for_version(SVG_BLOCK_ROOT_COMPONENT, Some(AngularVersion::new(22, 2, 2)));
    assert!(
        after.contains(r#"ɵɵconditionalCreate(0,Shape_Conditional_0_Template,1,0,"rect")"#),
        "22.2.2: block root tag must drop its namespace:\n{after}"
    );
}

/// Angular's linker derives `?.` semantics from the declaration's own `version`, so passing the
/// application's version must not switch linked templates to native optional chaining.
#[test]
fn test_link_angular_version_keeps_legacy_safe_navigation() {
    for version in [Some(AngularVersion::new(22, 2, 1)), None] {
        let code = link_for_version(SRC_BINDINGS_COMPONENT, version);
        assert!(code.contains("== null)? null: ctx.item.label"), "{version:?}:\n{code}");
        assert!(!code.contains("ctx.item?.label"), "{version:?}:\n{code}");
    }
}

/// Keys and values with a lone surrogate (`'\uD800'`) have no `&str`: the
/// linker keeps them as written instead of a lossy rendering.
#[test]
fn test_link_lone_surrogate_keys_kept_as_written() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
export class MyDir {}
MyDir.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "17.0.0", type: MyDir, selector: "[myDir]", inputs: { '\uD800': "pub", ok: '\uDC00', '\uD801': ["pub", "priv"], '\uD802': { classPropertyName: '\uD802', publicName: "p", isSignal: true }, '\uD803': { isRequired: true, isSignal: true }, named: { classPropertyName: "named", publicName: '\uDC01', isSignal: false } }, host: { attributes: { '\uD800': "v", title: '\uDBFF' } } });"#;
    let result = link(&allocator, code, "test.mjs");
    assert!(
        result.code.contains(
            r#"inputs: { '\uD800': "pub", ok: '\uDC00', '\uD801': [0, "pub", "priv"], '\uD802': [1, "p", '\uD802'], '\uD803': [1, '\uD803', '\uD803'], named: [0, '\uDC01', "named"] }"#
        ),
        "inputs not kept as written:\n{}",
        result.code
    );
    assert!(
        result.code.contains(r#"hostAttrs: ['\uD800', "v", "title", '\uDBFF']"#),
        "hostAttrs not kept as written:\n{}",
        result.code
    );
}

/// Babel's linker links every declaration call; Deepkit's runtime types keep a copy of each static initializer in a
/// `__type` array, whose declarations must not stay unlinked (their `ngImport: i0` keeps all of @angular/core)
#[test]
fn test_link_declarations_nested_in_arrays_and_objects() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
class Cmp {
    static { this.ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.4", ngImport: i0, type: Cmp, deps: [], target: i0.ɵɵFactoryTarget.Component }); }
    static { this.__type = ['ɵfac', function () { return i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.4", ngImport: i0, type: Cmp, deps: [], target: i0.ɵɵFactoryTarget.Component }); }]; }
}
const table = { factory: () => i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.4", ngImport: i0, type: Cmp, deps: [], target: i0.ɵɵFactoryTarget.Component }) };
"#;
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    assert!(!result.code.contains("ɵɵngDeclare"), "every declaration links:\n{}", result.code);
    assert!(!result.code.contains("ngImport"), "{}", result.code);
}

/// Like Babel's linker, every call is visited, whatever expression or statement holds it.
#[test]
fn test_link_declarations_in_any_position() {
    let d = r#"i0.ɵɵngDeclareInjectable({ minVersion: "12.0.0", version: "22.0.0", ngImport: i0, type: Svc, providedIn: "root" })"#;
    let positions = [
        format!("(function () {{ Svc.ɵprov = {d}; }})();"),
        format!("(() => {{ Svc.ɵprov = {d}; }})();"),
        format!("f(...[{d}]);"),
        format!("if ({d}) {{}}"),
        format!("for (let x = {d}; {d}; {d}) break;"),
        format!("for (const x of [{d}]);"),
        format!("switch ({d}) {{ case {d}: break; }}"),
        format!("class A extends ({d}, Object) {{ [{d}] = 1; static accessor p = {d}; }}"),
        format!("({{ [{d}]: 1 }});"),
        format!("tag`${{{d}}}`;"),
        format!("f?.({d}); a?.[{d}];"),
        format!("function* g(a = {d}) {{ yield {d}; }}"),
        format!("const {{ a = {d} }} = {{}}; x[{d}] = 1;"),
    ];
    for position in positions {
        let allocator = Allocator::default();
        let code = format!("import * as i0 from \"@angular/core\";\nclass Svc {{}}\n{position}\n");
        let result = link(&allocator, &code, "test.mjs");
        assert!(!result.code.contains("ɵɵngDeclare"), "every declaration links:\n{}", result.code);
    }
}

/// A linked factory is a `function` expression, so where it starts a longer statement it gets parentheses, like
/// Babel's output: `function X_Factory() {}.x;` would not parse, and `export default function … + ""` would
/// export the function
#[test]
fn test_link_factory_starting_a_statement_keeps_its_meaning() {
    let d = r#"i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.4", ngImport: i0, type: X, deps: [], target: i0.ɵɵFactoryTarget.Injectable })"#;
    for (position, expected) in [
        (format!("let y = 1\n{d}.x;"), "let y = 1\n;(function X_Factory("),
        (format!("{d} + 1;"), "\n;(function X_Factory("),
        (format!("{d}.call(null);"), "\n;(function X_Factory("),
        (format!("export default {d} + \"\";"), "export default (function X_Factory("),
        // a bare body gets no `;`, which would become the body itself
        (format!("if (c) {d}.x;"), "if (c) (function X_Factory("),
        (format!("if (c) {{}} else {d}.x;"), "else (function X_Factory("),
        (format!("l: {d}.x;"), "l: (function X_Factory("),
        (format!("switch (c) {{ case 1: {d}.x; }}"), "case 1: ;(function X_Factory("),
        (format!("const f = () => {d}.x;"), "const f = () => function X_Factory("),
        // nor does `@function …`
        (format!("@{d} class Y {{}}"), "@(function X_Factory("),
        // the whole statement, or not its start: as before
        (format!("{d};"), "\nfunction X_Factory("),
        (format!("const z = {d}.x;"), "const z = function X_Factory("),
    ] {
        let allocator = Allocator::default();
        let code = format!("import * as i0 from \"@angular/core\";\nclass X {{}}\n{position}\n");
        let result = link(&allocator, &code, "test.mjs");
        assert!(result.code.contains(expected), "expected `{expected}`:\n{}", result.code);
        let reparsed = Parser::new(&allocator, &result.code, SourceType::mjs()).parse();
        assert!(reparsed.diagnostics.is_empty(), "{}", result.code);
    }
}

/// Every `ɵɵdefine*` call carries `/*@__PURE__*/`, like the output of Angular's
/// babel linker (`@angular/compiler-cli/linker/babel` 22.2.3) on these declarations.
/// Factories (function expressions) and `ɵsetClassMetadata` get no annotation.
#[test]
fn test_link_define_calls_are_pure() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
class Svc {
  static ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "21.0.0", ngImport: i0, type: Svc, deps: [], target: i0.ɵɵFactoryTarget.Injectable });
  static ɵprov = i0.ɵɵngDeclareInjectable({ minVersion: "12.0.0", version: "21.0.0", ngImport: i0, type: Svc, providedIn: "root" });
}
class Auto {
  static ɵprov = i0.ɵɵngDeclareService({ minVersion: "21.0.0", version: "21.0.0", ngImport: i0, type: Auto });
}
class Mod {
  static ɵmod = i0.ɵɵngDeclareNgModule({ minVersion: "14.0.0", version: "21.0.0", ngImport: i0, type: Mod, id: "foo" });
  static ɵinj = i0.ɵɵngDeclareInjector({ minVersion: "12.0.0", version: "21.0.0", ngImport: i0, type: Mod });
}
class P {
  static ɵpipe = i0.ɵɵngDeclarePipe({ minVersion: "14.0.0", version: "21.0.0", ngImport: i0, type: P, isStandalone: true, name: "p" });
}
class D {
  static ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.0.0", type: D, isStandalone: true, selector: "[d]", ngImport: i0 });
}
class C {
  static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.0", type: C, isStandalone: true, selector: "c", ngImport: i0, template: "<b>{{x}}</b>" });
}
i0.ɵɵngDeclareClassMetadata({ minVersion: "12.0.0", version: "21.0.0", ngImport: i0, type: C, decorators: [{ type: i0.Component, args: [{ selector: "c" }] }] });
"#;
    let result = link(&allocator, code, "test.mjs");
    for define in [
        "defineInjectable",
        "defineService",
        "defineNgModule",
        "defineInjector",
        "definePipe",
        "defineDirective",
        "defineComponent",
    ] {
        let pure = format!("/*@__PURE__*/ i0.ɵɵ{define}(");
        assert!(result.code.contains(&pure), "missing `{pure}`:\n{}", result.code);
        assert_eq!(
            result.code.matches(&format!("ɵɵ{define}(")).count(),
            1,
            "unexpected `{define}` calls:\n{}",
            result.code
        );
    }
    // `registerNgModuleType` is a side effect: only the define call inside the IIFE is pure.
    assert!(
        result.code.contains(
            r#"(() => { i0.ɵɵregisterNgModuleType(Mod, "foo"); return /*@__PURE__*/ i0.ɵɵdefineNgModule("#
        ),
        "{}",
        result.code
    );
    assert!(result.code.contains("static ɵfac = function Svc_Factory("), "{}", result.code);
    assert!(result.code.contains("ngDevMode) && i0.ɵsetClassMetadata(C,"), "{}", result.code);
}

/// When the linker wraps a define call in an IIFE to declare template or host
/// binding functions and pooled constants (where `ngImport` is bound in an arrow
/// function, which cannot hold shared constants), the IIFE itself is annotated:
/// an unannotated IIFE would keep unused classes alive.
#[test]
fn test_link_declaration_iife_is_pure() {
    let allocator = Allocator::default();
    let code = r#"((i0) => {
class C {
  static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.0", type: C, isStandalone: true, selector: "c", ngImport: i0, template: "<b>{{x}}</b><ng-template><i>{{y}}</i></ng-template>" });
}
class D {
  static ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.0.0", type: D, isStandalone: true, selector: "[d]", ngImport: i0, host: { properties: { "style": "{ width: w }" } } });
}
})(core);
"#;
    let result = link(&allocator, code, "test.mjs");
    assert!(result.code.contains("static ɵcmp = /*@__PURE__*/ (() => {"), "{}", result.code);
    assert!(result.code.contains("static ɵdir = /*@__PURE__*/ (() => {"), "{}", result.code);
    assert!(result.code.contains("return /*@__PURE__*/ i0.ɵɵdefineComponent("), "{}", result.code);
    assert!(result.code.contains("return /*@__PURE__*/ i0.ɵɵdefineDirective("), "{}", result.code);
}

/// `styleAttribute` is parsed like Angular's `parse_extracted_styles` phase: `;` and `:`
/// inside quotes or parentheses do not split, and property names are hyphenated. The
/// expected `hostAttrs` are Angular 22.2.3's babel linker output for the same declaration.
#[test]
fn test_link_host_style_attribute_respects_quotes_and_parens() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
class D {
  static ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.0.0", type: D, isStandalone: true, selector: "[d]", host: { styleAttribute: "background: url('a;b'); font-family: \"Foo;Bar\", serif; backgroundColor: blue" }, ngImport: i0 });
}
"#;
    let result = link(&allocator, code, "test.mjs");
    assert!(
        result.code.contains(
            r#"hostAttrs: [2, "background", "url('a;b')", "font-family", "\"Foo;Bar\", serif", "background-color", "blue"]"#
        ),
        "{}",
        result.code
    );
}

/// Zero-based line of the first line starting with `prefix`.
fn line_starting_with(text: &str, prefix: &str) -> u32 {
    text.lines().position(|line| line.starts_with(prefix)).unwrap() as u32
}

/// With `sourcemap`, the linker returns a map from the linked code back to the
/// input; code after a replaced declaration maps to its original line even when
/// the replacement spans more lines than the declaration.
#[test]
fn test_link_sourcemap() {
    let code = r#"import * as i0 from "@angular/core";
class P {
  static ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "21.0.0", ngImport: i0, type: P, deps: [], target: i0.ɵɵFactoryTarget.Pipe });
  static ɵpipe = i0.ɵɵngDeclarePipe({ minVersion: "14.0.0", version: "21.0.0", ngImport: i0, type: P, isStandalone: true, name: "p" });
}
export { P };
"#;
    let allocator = Allocator::default();
    assert!(link(&allocator, code, "p.mjs").map.is_none(), "no map unless requested");

    let options = LinkOptions { sourcemap: true, ..LinkOptions::default() };
    let result = link_with_options(&allocator, code, "p.mjs", &options);
    assert!(result.linked);
    let map = result.map.expect("sourcemap requested");
    let map = oxc_sourcemap::SourceMap::from_json_string(&map).unwrap();
    assert_eq!(map.get_sources().collect::<Vec<_>>(), ["p.mjs"]);

    let src_line = line_starting_with(code, "export");
    let out_line = line_starting_with(&result.code, "export");
    assert_ne!(src_line, out_line, "the factory should add lines:\n{}", result.code);
    assert!(
        map.get_tokens().any(|t| t.get_dst_line() == out_line
            && t.get_dst_col() == 0
            && t.get_src_line() == src_line
            && t.get_src_col() == 0),
        "`export` line {out_line} should map to source line {src_line}:\n{}",
        result.code
    );
}

#[test]
fn test_link_escapes_metadata_strings() {
    let code = r#"import * as i0 from "@angular/core";
export class P {}
P.ɵpipe = i0.ɵɵngDeclarePipe({ minVersion: "14.0.0", version: "21.0.0", ngImport: i0, type: P, isStandalone: true, name: 'a"+x+"' });
export class D {}
D.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.0.0", ngImport: i0, type: D, isStandalone: true, selector: "[t='a\\b']", inputs: { 'a.b"c': "c" }, host: { classAttribute: 'k"+x+"' }, controlCreate: { passThroughInput: 'f"+x+"' } });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    for expected in [
        r#"name: "a\"+x+\"""#,
        r#"selectors: [["", "t", "a\\b"]]"#,
        r#"inputs: { "a.b\"c": "c" }"#,
        r#"hostAttrs: [1, "k\"+x+\""]"#,
        r#"ɵɵControlFeature("f\"+x+\"")"#,
    ] {
        assert!(result.code.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
}

#[test]
fn test_link_defer_block_dependencies() {
    // `deferBlockDependencies` follows the binder's block order: the outer
    // block, the block nested in it, the block in its placeholder, then the
    // next top-level block.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "22.2.2", ngImport: i0, type: C, isStandalone: true, selector: "c", template: "@defer { <lazy-cmp/> @defer { <b-b></b-b> } } @placeholder { @defer { <i-i></i-i> } } @defer (on idle) { <x-y/> }", deferBlockDependencies: [() => [import("./lazy").then(m => m.LazyCmp)], null, () => [import("./b").then(m => m.B)], () => [import("./x").then(m => m.X), import("./y").then(m => m.Y)]] });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    // Fragments of the TS linker's output.
    for expected in [
        r#"constC_Defer_2_DepsFn=()=>[import("./lazy").then(m=>m.LazyCmp)];"#,
        r#"constC_Defer_5_DepsFn=()=>[import("./x").then(m=>m.X),import("./y").then(m=>m.Y)];"#,
        r#"constC_DeferPlaceholder_1_Defer_1_DepsFn=()=>[import("./b").then(m=>m.B)];"#,
        "i0.ɵɵdefer(2,1);",
        "i0.ɵɵdefer(1,0,C_DeferPlaceholder_1_Defer_1_DepsFn);",
        "i0.ɵɵdefer(2,0,C_Defer_2_DepsFn,null,1);",
        "i0.ɵɵdefer(5,4,C_Defer_5_DepsFn);",
    ] {
        assert!(linked.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
}

#[test]
fn test_link_defer_block_dependencies_in_parentheses() {
    // Babel's AST has no parentheses: `(null)` is a null entry, and an entry is
    // emitted without the parentheses around it, as the TS linker does.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "22.2.2", ngImport: i0, type: C, isStandalone: true, selector: "c", template: "@defer { m1 } @defer { m2 }", deferBlockDependencies: [(() => [import("./a")]), (null)] });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    // Fragments of the TS linker's output.
    for expected in [
        r#"constC_Defer_1_DepsFn=()=>[import("./a")];"#,
        "i0.ɵɵdefer(1,0,C_Defer_1_DepsFn);",
        "i0.ɵɵdefer(4,3);",
    ] {
        assert!(linked.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
    assert!(!linked.contains("C_Defer_4_DepsFn"), "{}", result.code);
}

#[test]
fn test_link_query_predicate_splits_refs() {
    // Angular's `getQueryPredicate` splits each string of the predicate on `,`
    // and trims each part, keeping the empty ones, as ngtsc 22.2.3 partial
    // output linked by Angular 22.2.3's linker shows.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", ngImport: i0, type: C, isStandalone: true, selector: "c", queries: [{ propertyName: "a", predicate: ["y, z"] }], viewQueries: [{ propertyName: "b", first: true, predicate: ["x, "], descendants: true }], template: "" });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(linked.contains(r#"const_c0=["y","z"];const_c1=["x",""];"#), "{}", result.code);
    assert!(linked.contains("ɵɵcontentQuery(dirIndex,_c0,4)"), "{}", result.code);
    assert!(linked.contains("ɵɵviewQuery(_c1,5)"), "{}", result.code);
}

#[test]
fn test_link_component_pools_query_predicates() {
    // Angular's linker pools string predicates like ngtsc, with the
    // component's one pool: content queries, then view queries, then the host
    // bindings' constants, then the template's, which reuses `["*"]`.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.2.0", version: "22.2.2", type: C, isStandalone: true, selector: "c", host: { properties: { "attr.a": "[1, 2]" } }, queries: [{ propertyName: "c", first: true, predicate: ["y"], descendants: true }], viewQueries: [{ propertyName: "s", first: true, predicate: ["z"], descendants: true, isSignal: true }, { propertyName: "q", first: true, predicate: ["*"], descendants: true }, { propertyName: "t", first: true, predicate: TemplateRef, descendants: true }, { propertyName: "r", first: true, predicate: ["z"], descendants: true }], ngImport: i0, template: '<ng-content></ng-content>', isInline: true });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    // Fragments of the TS linker's output.
    for expected in [
        r#"const_c0=["y"];const_c1=["z"];const_c2=["*"];const_c3=()=>[1,2];"#,
        "i0.ɵɵcontentQuery(dirIndex,_c0,5);",
        "i0.ɵɵviewQuerySignal(ctx.s,_c1,5);",
        "i0.ɵɵviewQuery(_c2,5)(TemplateRef,5)(_c1,5);",
        "i0.ɵɵattribute(\"a\",i0.ɵɵpureFunction0(1,_c3));",
        "ngContentSelectors:_c2,",
    ] {
        assert!(linked.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
}

#[test]
fn test_link_directive_pools_query_predicates() {
    // As in a component, the predicates are pooled before the host bindings'
    // constants.
    let code = r#"import * as i0 from "@angular/core";
export class D {}
D.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "22.2.2", type: D, isStandalone: true, selector: "[d]", host: { properties: { "attr.a": "[1, 2]" } }, queries: [{ propertyName: "c", first: true, predicate: ["y"], descendants: true }, { propertyName: "cs", predicate: ["y, z"] }], viewQueries: [{ propertyName: "q", first: true, predicate: ["y"], descendants: true }], ngImport: i0 });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    // Fragments of the TS linker's output.
    for expected in [
        r#"const_c0=["y"];const_c1=["y","z"];const_c2=()=>[1,2];"#,
        "i0.ɵɵcontentQuery(dirIndex,_c0,5)(dirIndex,_c1,4);",
        "i0.ɵɵviewQuery(_c0,5);",
        "i0.ɵɵattribute(\"a\",i0.ɵɵpureFunction0(1,_c2));",
    ] {
        assert!(linked.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
}

#[test]
fn test_link_query_predicate_pools_lone_surrogates() {
    // A predicate with a lone surrogate is split and pooled like any other, as
    // Angular 22.2.3's linker shows, so the later constants keep their names.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: C, isStandalone: true, selector: "c", host: { properties: { "attr.a": "[1, 2]" } }, viewQueries: [{ propertyName: "q", first: true, predicate: ["\uD800"], descendants: true }, { propertyName: "r", first: true, predicate: ["y"], descendants: true }, { propertyName: "s", first: true, predicate: ["\uD800, z"], descendants: true }], ngImport: i0, template: "", isInline: true });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    for expected in [
        r#"const_c0=["\uD800"];const_c1=["y"];const_c2=["\uD800","z"];const_c3=()=>[1,2];"#,
        "i0.ɵɵviewQuery(_c0,5)(_c1,5)(_c2,5);",
        "i0.ɵɵattribute(\"a\",i0.ɵɵpureFunction0(1,_c3));",
    ] {
        assert!(linked.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
}

#[test]
fn test_link_query_predicate_trims_like_js() {
    // The parts are trimmed with `String.prototype.trim`, which strips U+FEFF
    // but not U+0085, as Angular 22.2.3's linker shows.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", ngImport: i0, type: C, isStandalone: true, selector: "c", viewQueries: [{ propertyName: "q", first: true, predicate: ["\uFEFFx"], descendants: true }, { propertyName: "r", first: true, predicate: ["y\u0085"], descendants: true }], template: "" });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "test.mjs");
    assert!(result.linked);
    assert!(result.code.contains(r#"["x"]"#), "{}", result.code);
    assert!(result.code.contains("[\"y\u{85}\"]"), "{}", result.code);
}

/// A component (standalone by default at version 22) with the given extra metadata.
fn dom_only_component(extra: &str) -> String {
    format!(
        r#"import * as i0 from "@angular/core";
export class A {{}}
A.ɵcmp = i0.ɵɵngDeclareComponent({{ minVersion: "14.0.0", version: "22.2.2", type: A, selector: "a", ngImport: i0, template: '<div [title]="t" (click)="go()"></div>@defer {{<i></i>}}', isInline: true{extra} }});
"#
    )
}

/// Angular's linker compiles a standalone template in DomOnly mode unless it has a
/// directive, component or NgModule dependency, or a `@defer` block with dependencies,
/// as Angular 22.2.3's linker shows. DomOnly needs Angular 20.1.
#[test]
fn test_link_dom_only_template() {
    let dom_only = [
        r#"ɵɵdomElementStart(0,"div",0);"#,
        r#"ɵɵdomListener("click",functionA_Template_div_click_0_listener()"#,
        r#"ɵɵdomProperty("title",ctx.t);"#,
        r#"ɵɵdomElement(0,"i");"#,
    ];
    let full = [
        r#"ɵɵelementStart(0,"div",0);"#,
        r#"ɵɵlistener("click",functionA_Template_div_click_0_listener()"#,
        r#"ɵɵproperty("title",ctx.t);"#,
        r#"ɵɵelement(0,"i");"#,
    ];
    let cases = [
        ("", None, &dom_only),
        (r#", dependencies: [{ kind: "pipe", type: P, name: "p" }]"#, None, &dom_only),
        (", deferBlockDependencies: [null]", None, &dom_only),
        ("", Some(AngularVersion::new(20, 1, 0)), &dom_only),
        (r#", dependencies: [{ kind: "directive", type: D, selector: "d" }]"#, None, &full),
        (r#", dependencies: [{ kind: "component", type: D, selector: "d" }]"#, None, &full),
        (r#", dependencies: [{ kind: "ngmodule", type: M }]"#, None, &full),
        (r#", deferBlockDependencies: [() => [import("./x").then(m => m.X)]]"#, None, &full),
        (", isStandalone: false", None, &full),
        ("", Some(AngularVersion::new(20, 0, 0)), &full),
    ];
    for (extra, version, expected) in cases {
        let code = link_for_version(&dom_only_component(extra), version);
        let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        for expected in expected {
            assert!(
                linked.contains(expected),
                "{extra} {version:?}: missing `{expected}` in\n{code}"
            );
        }
    }
}

/// Partial declarations of three classes with equal constants, as ngtsc 22.2.3 emits them.
const SHARED_CONSTANTS_FILE: &str = r#"import * as i0 from "@angular/core";
export class A {}
A.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: A, isStandalone: true, selector: "a", viewQueries: [{ propertyName: "f", first: true, predicate: ["foo"], descendants: true }, { propertyName: "b", first: true, predicate: ["bar"], descendants: true }], ngImport: i0, template: '<p i18n>Hi {{n}}</p><i [attr.x]="[1,2]"></i>@for (x of xs; track x.id) {<b>{{x}}</b>}', isInline: true });
export class B {}
B.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: B, isStandalone: true, selector: "b", viewQueries: [{ propertyName: "f", first: true, predicate: ["foo"], descendants: true }, { propertyName: "q", first: true, predicate: ["qux"], descendants: true }], ngImport: i0, template: '<p i18n>Bye {{n}}</p><i [attr.x]="[1,2]"></i>@for (x of xs; track x.id) {<u>{{x}}</u>}', isInline: true });
export class D {}
D.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "22.2.2", type: D, isStandalone: true, selector: "[d]", host: { properties: { "attr.y": "[1,2]" } }, ngImport: i0 });
"#;

/// Like Angular's linker, the declarations of a module share one constant pool: the names
/// go on across the file, equal constants are declared once, and the constants and child
/// template functions go after the imports, in the order Angular 22.2.3's linker emits them,
/// with the main template function inline.
#[test]
fn test_link_shares_constants_across_a_module() {
    let allocator = Allocator::default();
    let result = link(&allocator, SHARED_CONSTANTS_FILE, "/x/cases/main.js");
    assert!(result.linked);
    let code = &result.code;
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let mut last = 0;
    for expected in [
        r#"import*asi0from"@angular/core";const_c0=["foo"];const_c1=["bar"];const_c2=()=>[1,2];const_forTrack0=($index,$item)=>$item.id;functionA_For_4_Template(rf,ctx){"#,
        r#"const_c3=["qux"];functionB_For_4_Template(rf,ctx){"#,
        "exportclassA{}A.ɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent({type:A,",
        "i0.ɵɵviewQuery(_c0,5)(_c1,5);",
        "i18n_0=MSG__X_CASES_MAIN_JS_0",
        "template:functionA_Template(rf,ctx){",
        r#"i0.ɵɵrepeaterCreate(3,A_For_4_Template,2,1,"b",null,_forTrack0);"#,
        r#"i0.ɵɵattribute("x",i0.ɵɵpureFunction0(2,_c2));"#,
        "exportclassB{}B.ɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent({type:B,",
        "i0.ɵɵviewQuery(_c0,5)(_c3,5);",
        "i18n_1=MSG__X_CASES_MAIN_JS_1",
        r#"i0.ɵɵrepeaterCreate(3,B_For_4_Template,2,1,"u",null,_forTrack0);"#,
        r#"i0.ɵɵattribute("x",i0.ɵɵpureFunction0(2,_c2));"#,
        "exportclassD{}D.ɵdir=/*@__PURE__*/i0.ɵɵdefineDirective({type:D,",
        r#"i0.ɵɵattribute("y",i0.ɵɵpureFunction0(1,_c2));"#,
    ] {
        let Some(at) = linked[last..].find(expected) else {
            panic!("missing `{expected}` after byte {last} in\n{code}");
        };
        last += at + expected.len();
    }
    assert!(!code.contains("(() =>"), "no IIFE:\n{code}");
}

/// A long style is inline on its first use and shared from its second, across the
/// declarations of a module: as in Angular 22.2.3's linker output, a first use in an
/// earlier declaration stays inline, and the constants go after the imports.
#[test]
fn test_link_shares_long_styles_across_a_module() {
    let a = ".a { color: red; background: blue; border: 1px solid green; padding: 4px; }";
    let b = ".b { color: red; background: blue; border: 1px solid green; padding: 4px; }";
    let component = |name: &str, styles: &[&str]| {
        let styles: Vec<String> = styles.iter().map(|style| format!("{style:?}")).collect();
        format!(
            r#"export class {name} {{}}
{name}.ɵcmp = i0.ɵɵngDeclareComponent({{ minVersion: "14.0.0", version: "22.2.2", type: {name}, isStandalone: true, selector: "{name}", ngImport: i0, template: "<p></p>", isInline: true, styles: [{}] }});
"#,
            styles.join(", ")
        )
    };
    let code = format!(
        "import * as i0 from \"@angular/core\";\n// comment\n{}{}{}",
        component("A", &[a, b]),
        component("B", &[b, b, a]),
        component("C", &[a])
    );
    let allocator = Allocator::default();
    let result = link(&allocator, &code, "test.mjs");
    assert!(result.linked);
    let scoped_a = r#"".a[_ngcontent-%COMP%] { color: red; background: blue; border: 1px solid green; padding: 4px; }""#;
    let scoped_b = r#"".b[_ngcontent-%COMP%] { color: red; background: blue; border: 1px solid green; padding: 4px; }""#;
    let expected = format!(
        "import * as i0 from \"@angular/core\";\n// comment\nconst _c0 = {scoped_b};\nconst _c1 = {scoped_a};\nexport class A {{}}"
    );
    assert!(result.code.starts_with(&expected), "{}", result.code);
    for expected in [
        format!("styles: [{scoped_a}, {scoped_b}]"),
        "styles: [_c0, _c0, _c1]".to_string(),
        "styles: [_c1]".to_string(),
    ] {
        assert!(result.code.contains(&expected), "missing `{expected}` in\n{}", result.code);
    }
}

/// Angular's linker shares the constants of the declarations whose `ngImport` is bound in
/// a function declaration or expression, at the start of its body (after the directives
/// and the comments after them), and keeps the others' local in an IIFE: an arrow
/// function or a method cannot hold them. As Angular 22.2.3's linker shows for a UMD bundle.
#[test]
fn test_link_constant_scopes() {
    let component = |name: &str| {
        format!(
            r#"class {name} {{}} {name}.ɵcmp = i0.ɵɵngDeclareComponent({{ minVersion: "14.0.0", version: "22.2.2", type: {name}, isStandalone: true, selector: "{name}", ngImport: i0, template: "<i [attr.y]=\"[1,2]\"></i>", isInline: true }});"#
        )
    };
    let code = format!(
        r#"(function (global, factory) {{ factory(exports, require("@angular/core")); }})(this, (function (exports, i0) {{ 'use strict';
  /** c */
  {}
  {}
  const f = function (i0) {{ {} }};
  const g = (i0) => {{ {} }};
  class E {{ m(i0) {{ {} }} }}
}}));
"#,
        component("A"),
        component("B"),
        component("F"),
        component("G"),
        component("M")
    );
    let allocator = Allocator::default();
    let result = link(&allocator, &code, "test.js");
    assert!(result.linked);
    let code = &result.code;
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let mut last = 0;
    for expected in [
        "(function(exports,i0){'usestrict';/**c*/const_c0=()=>[1,2];classA{}A.ɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent(",
        "i0.ɵɵpureFunction0(1,_c0)",
        "classB{}B.ɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent(",
        "i0.ɵɵpureFunction0(1,_c0)",
        "constf=function(i0){const_c0=()=>[1,2];classF{}F.ɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent(",
        "constg=(i0)=>{classG{}G.ɵcmp=/*@__PURE__*/(()=>{const_c0=()=>[1,2];",
        "classE{m(i0){classM{}M.ɵcmp=/*@__PURE__*/(()=>{const_c0=()=>[1,2];",
    ] {
        let Some(at) = linked[last..].find(expected) else {
            panic!("missing `{expected}` after byte {last} in\n{code}");
        };
        last += at + expected.len();
    }
}

/// The host binding functions are named with the scope's pool, like the template
/// functions: as Angular 22.2.3's linker shows, a second class `A` of the module gets
/// `A_HostBindings1`.
#[test]
fn test_link_names_host_bindings_across_a_module() {
    let code = r#"import * as i0 from "@angular/core";
export function f1() { class A {} A.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "22.2.2", type: A, isStandalone: true, selector: "cmp-a", ngImport: i0, template: '@if (x) {<p>one</p>}', isInline: true, host: { listeners: { click: "go()" } } }); return A; }
export function f2() { class A {} A.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "22.2.2", type: A, isStandalone: true, selector: "cmp-a2", ngImport: i0, template: '@if (x) {<p>two</p>}', isInline: true, host: { listeners: { click: "go()" } } }); return A; }
export function f3() { class A {} A.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "17.0.0", version: "22.2.2", type: A, isStandalone: true, selector: "[d]", ngImport: i0, host: { properties: { title: "t" } } }); return A; }
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "/x/main.js");
    assert!(result.linked);
    let code = &result.code;
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let mut last = 0;
    for expected in [
        "hostBindings:functionA_HostBindings(rf,ctx){",
        "template:functionA_Template(rf,ctx){",
        "hostBindings:functionA_HostBindings1(rf,ctx){",
        "template:functionA_Template1(rf,ctx){",
        "hostBindings:functionA_HostBindings2(rf,ctx){",
    ] {
        let Some(at) = linked[last..].find(expected) else {
            panic!("missing `{expected}` after byte {last} in\n{code}");
        };
        last += at + expected.len();
    }
}

/// Angular's linker names Closure messages after the linked file's path, without external
/// ids even for a custom id, as Angular 22.2.3's linker shows for `/x/lib/a.component.mjs`.
#[test]
fn test_link_i18n_closure_names_use_file_path() {
    let code = r#"import * as i0 from "@angular/core";
export class A {}
A.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: A, selector: "a", ngImport: i0, template: '<p i18n>Hello</p><p i18n="@@custom">Bye</p>', isInline: true });
"#;
    let allocator = Allocator::default();
    let result = link(&allocator, code, "/x/lib/a.component.mjs");
    assert!(result.linked);
    let linked: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    for expected in [
        r#"constMSG__X_LIB_A_COMPONENT_MJS_0=goog.getMsg("Hello");"#,
        "i18n_0=MSG__X_LIB_A_COMPONENT_MJS_0",
        r#"constMSG__X_LIB_A_COMPONENT_MJS_1=goog.getMsg("Bye");"#,
        "i18n_1=MSG__X_LIB_A_COMPONENT_MJS_1",
    ] {
        assert!(linked.contains(expected), "missing `{expected}` in\n{}", result.code);
    }
}

/// `ɵɵngDeclareDirective` of ngtsc 22.2.3 for two signal and two decorator
/// view queries, and one signal and two decorator content queries.
const CHAINED_QUERIES_DIRECTIVE: &str = r#"import * as i0 from "@angular/core";
export class D {}
D.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "17.2.0", version: "22.2.2", type: D, isStandalone: true, selector: "[d]", queries: [{ propertyName: "cs", predicate: ["a"], isSignal: true }, { propertyName: "b", predicate: ["b"] }, { propertyName: "c", predicate: ["c"] }], viewQueries: [{ propertyName: "s", first: true, predicate: ["s"], descendants: true, isSignal: true }, { propertyName: "r", first: true, predicate: ["r"], descendants: true, isSignal: true }, { propertyName: "q", first: true, predicate: ["q"], descendants: true }, { propertyName: "p", first: true, predicate: ["p"], descendants: true }], ngImport: i0 });
"#;

/// Consecutive query creation calls of one instruction are chained, as Angular
/// 22.2.3's linker does, except for runtimes before v21.0.4, whose query
/// instructions return nothing. Consecutive signal queries advance once.
#[test]
fn test_link_chains_query_creation() {
    for version in [None, Some(AngularVersion::new(21, 0, 4))] {
        let code = link_for_version(CHAINED_QUERIES_DIRECTIVE, version);
        let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        for expected in [
            "{i0.ɵɵcontentQuerySignal(dirIndex,ctx.cs,_c0,4);i0.ɵɵcontentQuery(dirIndex,_c1,4)(dirIndex,_c2,4);}",
            "{i0.ɵɵviewQuerySignal(ctx.s,_c3,5)(ctx.r,_c4,5);i0.ɵɵviewQuery(_c5,5)(_c6,5);}",
            "{i0.ɵɵqueryAdvance();let_t;i0.ɵɵqueryRefresh(_t=i0.ɵɵloadQuery())&&(ctx.b=_t);",
            "{i0.ɵɵqueryAdvance(2);let_t;i0.ɵɵqueryRefresh(_t=i0.ɵɵloadQuery())&&(ctx.q=_t.first);",
        ] {
            assert!(linked.contains(expected), "{version:?}: missing `{expected}` in\n{code}");
        }
    }

    let code = link_for_version(CHAINED_QUERIES_DIRECTIVE, Some(AngularVersion::new(21, 0, 3)));
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(linked.contains("i0.ɵɵviewQuery(_c5,5);i0.ɵɵviewQuery(_c6,5);"), "{code}");
    assert!(!linked.contains(",4)(") && !linked.contains(",5)("), "{code}");
}

/// `ɵɵngDeclareComponent` of ngtsc 21.2.25 for
/// `@Component({ selector: 'span[bitBadge]', template: '<ng-content></ng-content>' })`.
const BADGE_COMPONENT: &str = r#"import * as i0 from "@angular/core";
export class BadgeComponent {}
BadgeComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.2.25", type: BadgeComponent, isStandalone: true, selector: "span[bitBadge]", ngImport: i0, template: '<ng-content></ng-content>', isInline: true });
"#;

/// Angular before v22.0.0-next.4 sets `attrs` to the first selector's
/// attributes, pooled after the host bindings' constants and before the
/// template's, as Angular 21.2.25's linker does.
#[test]
fn test_link_selector_attrs_before_v22() {
    let code = link_for_version(BADGE_COMPONENT, Some(AngularVersion::new(21, 2, 25)));
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(linked.contains(r#"const_c0=["bitBadge",""];const_c1=["*"];"#), "{code}");
    assert!(linked.contains("attrs:_c0,ngContentSelectors:_c1,"), "{code}");

    for version in [Some(AngularVersion::new(22, 0, 0)), None] {
        let code = link_for_version(BADGE_COMPONENT, version);
        assert!(!code.contains("attrs:"), "{version:?}:\n{code}");
        assert!(code.contains(r#"const _c0 = ["*"];"#), "{version:?}:\n{code}");
    }

    // After the query predicates and the host bindings' constants.
    let code = r#"import * as i0 from "@angular/core";
export class C {}
C.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.2.25", type: C, isStandalone: true, selector: "span[bitBadge]", host: { properties: { "attr.a": "[1, 2]" } }, viewQueries: [{ propertyName: "q", first: true, predicate: ["x"], descendants: true }], ngImport: i0, template: '<ng-content></ng-content>', isInline: true });
"#;
    let code = link_for_version(code, Some(AngularVersion::new(21, 2, 25)));
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        linked.contains(
            r#"const_c0=["x"];const_c1=()=>[1,2];const_c2=["bitBadge",""];const_c3=["*"];"#
        ),
        "{code}"
    );
    assert!(linked.contains("attrs:_c2,ngContentSelectors:_c3,"), "{code}");

    // Shared with an equal predicate.
    let code = r#"import * as i0 from "@angular/core";
export class D {}
D.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.2.25", type: D, isStandalone: true, selector: "[x]", viewQueries: [{ propertyName: "q", first: true, predicate: ["x, "], descendants: true }], ngImport: i0, template: '', isInline: true });
"#;
    let code = link_for_version(code, Some(AngularVersion::new(21, 2, 25)));
    let linked: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(linked.matches("const_c").count(), 1, "{code}");
    assert!(linked.contains(r#"const_c0=["x",""];"#), "{code}");
    assert!(linked.contains("ɵɵviewQuery(_c0,5)"), "{code}");
    assert!(linked.contains("},attrs:_c0,decls:0,"), "{code}");
}

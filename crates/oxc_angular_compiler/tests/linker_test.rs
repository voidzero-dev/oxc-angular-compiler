//! Tests for the Angular linker: key quoting and version-dependent emit.

use oxc_allocator::Allocator;
use oxc_angular_compiler::AngularVersion;
use oxc_angular_compiler::linker::{LinkOptions, link, link_with_options};

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
            code.matches(r#"ɵɵproperty("src",ctx.url,i0.ɵɵsanitizeUrl)"#).count(),
            2,
            "{version:?}: img and video src must be sanitized:\n{code}"
        );
        assert!(code.contains(r#"ɵɵproperty("href",ctx.url,i0.ɵɵsanitizeUrl)"#), "{code}");
    }
}

/// Angular 22.2.2 removed `img|src` and `video|src` from the URL security context
/// (angular/angular@439ff1bdf4). An unset version keeps assuming the latest Angular.
#[test]
fn test_link_does_not_sanitize_img_video_src_from_22_2_2() {
    for version in [Some(AngularVersion::new(22, 2, 2)), None] {
        let code = link_for_version(SRC_BINDINGS_COMPONENT, version);
        assert_eq!(
            code.matches(r#"ɵɵproperty("src",ctx.url)"#).count(),
            2,
            "{version:?}: img and video src must not be sanitized:\n{code}"
        );
        assert!(code.contains(r#"ɵɵproperty("href",ctx.url,i0.ɵɵsanitizeUrl)"#), "{code}");
    }

    let allocator = Allocator::default();
    let default_link = link(&allocator, SRC_BINDINGS_COMPONENT, "test.mjs").code;
    assert_eq!(default_link, link_for_version(SRC_BINDINGS_COMPONENT, None));
}

/// Checks that the application's version reaches the directive host path: before 22.2.2,
/// `img|src` is in the URL schema, so the host binding must be sanitized.
/// Only "sanitized" is asserted, not which sanitizer, and 22.2.2 is not asserted at all:
/// Angular emits `ɵɵsanitizeUrlOrResourceUrl` here for both versions (concrete host security
/// contexts, angular/angular@d06e3748b7), which OXC does not port yet.
#[test]
fn test_link_host_src_binding_respects_angular_version() {
    let code = link_for_version(SRC_HOST_DIRECTIVE, Some(AngularVersion::new(22, 2, 1)));
    assert!(
        code.contains(r#"ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitize"#),
        "22.2.1 must reach the directive host path and sanitize `img|src`:\n{code}"
    );
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
/// binding functions and pooled constants, the IIFE itself is annotated. Angular's
/// linker hoists those declarations and leaves `/* @__PURE__ */ i0.ɵɵdefine*(...)`
/// as the static field value, so an unannotated IIFE would keep unused classes alive.
#[test]
fn test_link_declaration_iife_is_pure() {
    let allocator = Allocator::default();
    let code = r#"import * as i0 from "@angular/core";
class C {
  static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.0", type: C, isStandalone: true, selector: "c", ngImport: i0, template: "<b>{{x}}</b><ng-template><i>{{y}}</i></ng-template>" });
}
class D {
  static ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "21.0.0", type: D, isStandalone: true, selector: "[d]", ngImport: i0, host: { properties: { "style": "{ width: w }" } } });
}
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

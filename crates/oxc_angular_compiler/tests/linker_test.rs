//! Tests for Angular linker input/output key quoting.

use oxc_allocator::Allocator;
use oxc_angular_compiler::linker::link;

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

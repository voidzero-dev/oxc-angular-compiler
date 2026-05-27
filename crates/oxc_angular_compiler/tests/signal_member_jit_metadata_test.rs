//! Tests for synthesizing prop-decorator metadata for initializer-API members
//! (`input()`, `output()`, `model()`, `viewChild()`/`contentChild()`/…) into
//! `ɵsetClassMetadata`.
//!
//! Angular's AOT `ɵcmp` carries signal members, but `TestBed.overrideComponent`
//! discards `ɵcmp` and recompiles via JIT, which reconstructs inputs/outputs/
//! queries ONLY from decorator/prop metadata reflected off the class. Angular's
//! own CLI applies a JIT transform (compiler-cli `initializer_api_transforms/`)
//! that adds synthetic `@Input`/`@Output`/query decorators for signal members so
//! JIT can see them; without it, `setInput`/router-binding fail (NG0315/NG0950).
//! These tests assert OXC emits the equivalent synthetic prop decorators.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

/// Compile `source` with class metadata enabled and return the full output.
fn compile(source: &str) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { emit_class_metadata: true, ..TransformOptions::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "compile errored: {:?}", result.diagnostics);
    result.code
}

/// The slice of output from `ɵsetClassMetadata` onward (contains the decorators
/// array and the prop-decorators object). Asserting against this avoids matching
/// identifiers in the import statements or template.
fn metadata_region(code: &str) -> String {
    let start = code
        .find("\u{275}setClassMetadata")
        .unwrap_or_else(|| panic!("ɵsetClassMetadata not present:\n{code}"));
    code[start..].to_string()
}

fn component(body: &str, imports: &str) -> String {
    format!(
        "import {{ Component, {imports} }} from '@angular/core';\n\n\
         @Component({{ selector: 'c', template: '<span>x</span>', standalone: true }})\n\
         export class C {{\n{body}\n}}\n"
    )
}

#[test]
fn signal_input_emits_input_prop_decorator_with_is_signal() {
    let md = metadata_region(&compile(&component("  readonly value = input(0);", "input")));
    assert!(md.contains("value"), "prop key missing:\n{md}");
    assert!(md.contains("Input"), "synthetic Input decorator missing:\n{md}");
    assert!(md.contains("isSignal:true"), "isSignal flag missing:\n{md}");
    assert!(md.contains("required:false"), "required flag missing:\n{md}");
    assert!(md.contains("alias:\"value\""), "alias missing:\n{md}");
    // ngc emits a three-field config `{isSignal, alias, required}` for signal inputs —
    // NO `transform` key (verified against @angular/compiler-cli output).
    assert!(!md.contains("transform"), "signal input must NOT emit a transform key:\n{md}");
}

#[test]
fn synthetic_decorator_uses_core_namespace_reference() {
    // Angular references the synthetic decorator through the @angular/core namespace
    // import (createSyntheticAngularCoreDecoratorAccess → `i0.Input`), not a bare `Input`
    // identifier (a signal component imports `input`, not `Input`).
    let md = metadata_region(&compile(&component("  readonly value = input(0);", "input")));
    assert!(
        md.contains(".Input"),
        "expected a namespaced `<ns>.Input` reference, not a bare identifier:\n{md}"
    );
}

#[test]
fn signal_query_with_options_spreads_source_options() {
    let md = metadata_region(&compile(&component(
        "  readonly items = contentChildren('item', { descendants: true });",
        "contentChildren",
    )));
    assert!(md.contains("ContentChildren"), "ContentChildren decorator missing:\n{md}");
    assert!(md.contains("isSignal:true"), "query isSignal missing:\n{md}");
    // The source options object is spread verbatim (matching Angular's
    // `{ ...callArgs[1], isSignal: true }`).
    assert!(md.contains("descendants"), "source query options not preserved:\n{md}");
}

#[test]
fn required_signal_input_marks_required() {
    let md = metadata_region(&compile(&component(
        "  readonly value = input.required<string>();",
        "input",
    )));
    assert!(md.contains("isSignal:true"), "isSignal missing:\n{md}");
    assert!(md.contains("required:true"), "required:true missing:\n{md}");
}

#[test]
fn aliased_signal_input_uses_alias() {
    let md = metadata_region(&compile(&component(
        "  readonly value = input(0, { alias: 'publicName' });",
        "input",
    )));
    assert!(md.contains("alias:\"publicName\""), "alias not applied:\n{md}");
}

#[test]
fn signal_output_emits_output_prop_decorator() {
    let md = metadata_region(&compile(&component("  readonly changed = output<string>();", "output")));
    assert!(md.contains("changed"), "prop key missing:\n{md}");
    assert!(md.contains("Output"), "synthetic Output decorator missing:\n{md}");
    // output() lowers to `Output("<bindingName>")` (a single string arg).
    assert!(md.contains("\"changed\""), "output binding name missing:\n{md}");
}

#[test]
fn signal_model_emits_input_and_output() {
    let md = metadata_region(&compile(&component("  readonly open = model(false);", "model")));
    assert!(md.contains("Input"), "model Input decorator missing:\n{md}");
    assert!(md.contains("isSignal:true"), "model Input isSignal missing:\n{md}");
    assert!(md.contains("Output"), "model Output decorator missing:\n{md}");
    assert!(md.contains("\"openChange\""), "model output binding `openChange` missing:\n{md}");
}

#[test]
fn signal_view_query_emits_view_child_with_is_signal() {
    let md = metadata_region(&compile(&component(
        "  readonly ref = viewChild<string>('tpl');",
        "viewChild",
    )));
    assert!(md.contains("ViewChild"), "synthetic ViewChild decorator missing:\n{md}");
    assert!(md.contains("isSignal:true"), "query isSignal missing:\n{md}");
}

#[test]
fn signal_content_query_emits_content_child_with_is_signal() {
    let md = metadata_region(&compile(&component(
        "  readonly ref = contentChild<string>('tpl');",
        "contentChild",
    )));
    assert!(md.contains("ContentChild"), "synthetic ContentChild decorator missing:\n{md}");
    assert!(md.contains("isSignal:true"), "query isSignal missing:\n{md}");
}

#[test]
fn classic_input_output_unchanged_and_not_signal() {
    let md = metadata_region(&compile(&component(
        "  @Input() foo = 1;\n  @Output() bar = new EventEmitter();",
        "Input, Output, EventEmitter",
    )));
    assert!(md.contains("foo"), "classic @Input prop key missing:\n{md}");
    assert!(md.contains("bar"), "classic @Output prop key missing:\n{md}");
    assert!(md.contains("type:Input"), "classic Input type missing:\n{md}");
    assert!(
        !md.contains("isSignal"),
        "classic decorators must not gain an isSignal flag:\n{md}"
    );
}

#[test]
fn no_metadata_when_emit_disabled() {
    let allocator = Allocator::default();
    let source = component("  readonly value = input(0);", "input");
    // emit_class_metadata is default-on, so disable it explicitly.
    let options = TransformOptions { emit_class_metadata: false, ..TransformOptions::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", &source, Some(&options), None);
    assert!(
        !result.code.contains("\u{275}setClassMetadata"),
        "no setClassMetadata should be emitted when disabled:\n{}",
        result.code
    );
}

// ─── ngc parity: detector edge cases ───────────────────────────────────────────

#[test]
fn signal_input_detected_through_as_cast() {
    // ngc unwraps `as` expressions when detecting initializer APIs:
    // `foo = input(0) as any` is still recognized as a signal input.
    let md = metadata_region(&compile(&component("  readonly value = input(0) as any;", "input")));
    assert!(md.contains("Input") && md.contains("isSignal:true"), "input behind `as` cast not detected:\n{md}");
}

#[test]
fn signal_input_detected_through_parentheses() {
    // ngc unwraps parenthesized initializers: `foo = (input(0))`.
    let md = metadata_region(&compile(&component("  readonly value = (input(0));", "input")));
    assert!(md.contains("Input") && md.contains("isSignal:true"), "parenthesized input not detected:\n{md}");
}

#[test]
fn namespaced_required_signal_input_detected() {
    // ngc handles `core.input.required()` (namespace import + `.required()`).
    let allocator = Allocator::default();
    let source = "import * as core from '@angular/core';\n\n\
        @core.Component({ selector: 'c', template: '<span>x</span>', standalone: true })\n\
        export class C { readonly value = core.input.required<string>(); }\n";
    let options = TransformOptions { emit_class_metadata: true, ..TransformOptions::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "compile errored: {:?}", result.diagnostics);
    let md = metadata_region(&result.code);
    assert!(md.contains("Input") && md.contains("isSignal:true"), "core.input.required not detected:\n{md}");
    assert!(md.contains("required:true"), "required flag missing for core.input.required:\n{md}");
}

#[test]
fn directive_emits_set_class_metadata_with_signal_input() {
    // ngc emits setClassMetadata for @Directive too (incl. signal-member prop
    // decorators), so signal inputs survive TestBed.overrideDirective.
    let allocator = Allocator::default();
    let source = "import { Directive, input, Input } from '@angular/core';\n\n\
        @Directive({ selector: '[appFoo]', standalone: true })\n\
        export class FooDirective {\n\
          readonly value = input('x');\n\
          @Input() classic = 0;\n\
        }\n";
    let options = TransformOptions { emit_class_metadata: true, ..TransformOptions::default() };
    let result =
        transform_angular_file(&allocator, "foo.directive.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "compile errored: {:?}", result.diagnostics);
    assert!(
        result.code.contains("\u{275}setClassMetadata"),
        "directive should emit setClassMetadata:\n{}",
        result.code
    );
    let md = metadata_region(&result.code);
    assert!(md.contains("Directive"), "Directive decorator metadata missing:\n{md}");
    assert!(
        md.contains("Input") && md.contains("isSignal:true"),
        "signal input prop decorator missing for directive:\n{md}"
    );
    assert!(md.contains("classic"), "classic @Input prop decorator missing:\n{md}");
}

#[test]
fn directive_no_metadata_when_emit_disabled() {
    let allocator = Allocator::default();
    let source = "import { Directive, input } from '@angular/core';\n\n\
        @Directive({ selector: '[appFoo]', standalone: true })\n\
        export class FooDirective { readonly value = input('x'); }\n";
    let options = TransformOptions { emit_class_metadata: false, ..TransformOptions::default() };
    let result =
        transform_angular_file(&allocator, "foo.directive.ts", source, Some(&options), None);
    assert!(
        !result.code.contains("\u{275}setClassMetadata"),
        "no setClassMetadata when disabled:\n{}",
        result.code
    );
}

#[test]
fn namespaced_required_signal_model_detected() {
    // ngc handles `core.model.required()` → Input(isSignal) + Output.
    let allocator = Allocator::default();
    let source = "import * as core from '@angular/core';\n\n\
        @core.Component({ selector: 'c', template: '<span>x</span>', standalone: true })\n\
        export class C { readonly value = core.model.required<string>(); }\n";
    let options = TransformOptions { emit_class_metadata: true, ..TransformOptions::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "compile errored: {:?}", result.diagnostics);
    let md = metadata_region(&result.code);
    assert!(md.contains("Input") && md.contains("isSignal:true"), "core.model.required input not detected:\n{md}");
    assert!(md.contains("Output"), "core.model.required output not detected:\n{md}");
    assert!(md.contains("required:true"), "required flag missing for core.model.required:\n{md}");
}

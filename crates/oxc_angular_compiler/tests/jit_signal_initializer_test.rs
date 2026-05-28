//! JIT downlevel parity for signal initializer APIs.
//!
//! When `jit: true`, OXC must synthesize `@Input`/`@Output`/`@ViewChild`/etc.
//! `propDecorators` entries for fields initialized via `input()`, `output()`,
//! `model()`, `viewChild()`, `viewChildren()`, `contentChild()`,
//! `contentChildren()`. The runtime JIT facade
//! (`compileComponent`/`compileDirective` in `@angular/compiler`) discovers
//! inputs/outputs/queries via decorator metadata only — never via field
//! initializers — so without lowering, a JIT-compiled signal component has
//! no inputs/outputs/queries at runtime.
//!
//! Mirrors `packages/compiler-cli/src/ngtsc/transform/jit/src/initializer_api_transforms/`.
//! Issue #312.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions as ComponentTransformOptions, transform_angular_file};

/// Compile in JIT mode and return the emitted code.
fn compile_jit(source: &str) -> String {
    let allocator = Allocator::default();
    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "compile errored: {:?}", result.diagnostics);
    result.code
}

fn component(body: &str, imports: &str) -> String {
    format!(
        "import {{ Component, {imports} }} from '@angular/core';\n\n\
         @Component({{ selector: 'c', template: '', standalone: true }})\n\
         export class C {{\n{body}\n}}\n"
    )
}

#[test]
fn input_initializer_emits_synthesized_input_prop_decorator() {
    let out = compile_jit(&component("  readonly value = input(0);", "input"));
    assert!(
        out.contains("propDecorators"),
        "JIT output should emit propDecorators. Got:\n{out}"
    );
    assert!(out.contains("value:"), "field name should be a propDecorators key. Got:\n{out}");
    assert!(out.contains("type: Input"), "synthesized @Input missing. Got:\n{out}");
    assert!(out.contains("isSignal: true"), "isSignal flag missing. Got:\n{out}");
    assert!(out.contains("required: false"), "required: false missing. Got:\n{out}");
    assert!(out.contains("alias: \"value\""), "default alias = field name missing. Got:\n{out}");
}

#[test]
fn input_required_marks_required_true() {
    let out = compile_jit(&component("  readonly value = input.required<string>();", "input"));
    assert!(out.contains("type: Input"), "Got:\n{out}");
    assert!(out.contains("required: true"), "input.required() should set required: true. Got:\n{out}");
}

#[test]
fn input_with_alias_option_uses_provided_alias() {
    let out = compile_jit(&component(
        "  readonly value = input(0, { alias: 'publicName' });",
        "input",
    ));
    assert!(out.contains("alias: \"publicName\""), "alias from options not honored. Got:\n{out}");
}

#[test]
fn explicit_input_decorator_blocks_input_synthesis() {
    // The user explicitly chose @Input — the decorator wins, no synthesized version
    // should appear (mirrors upstream signalInputsTransform's early return).
    let out = compile_jit(&component("  @Input() value = input(0);", "input, Input"));
    // The explicit @Input decorator survives via the existing propDecorators path…
    assert!(out.contains("type: Input"), "explicit @Input lost. Got:\n{out}");
    // …but the synthesized entry must NOT add isSignal. If both ran, isSignal would
    // appear in the args. Its absence proves the synthesis was skipped.
    assert!(
        !out.contains("isSignal"),
        "synthesis must be skipped when explicit @Input is present. Got:\n{out}"
    );
}

#[test]
fn output_initializer_emits_synthesized_output_prop_decorator() {
    let out = compile_jit(&component("  readonly clicked = output<void>();", "output"));
    assert!(out.contains("type: Output"), "synthesized @Output missing. Got:\n{out}");
    // Default alias is the field name.
    assert!(out.contains("\"clicked\""), "default alias missing. Got:\n{out}");
}

#[test]
fn output_with_alias_option() {
    let out = compile_jit(&component(
        "  readonly ready = output<void>({ alias: 'readyPub' });",
        "output",
    ));
    assert!(out.contains("\"readyPub\""), "alias from options not honored. Got:\n{out}");
}

#[test]
fn output_from_observable_options_live_in_args_1() {
    // outputFromObservable(source, options?) — options is args[1], not args[0].
    // Regression: an earlier OXC AOT bug pulled options from args[0] and lost the alias.
    let out = compile_jit(&format!(
        "import {{ Component, outputFromObservable }} from '@angular/core';\n\
         import {{ of }} from 'rxjs';\n\
         @Component({{ selector: 'c', template: '', standalone: true }})\n\
         export class C {{\n  ready = outputFromObservable(of(1), {{ alias: 'readyPub' }});\n}}\n",
    ));
    assert!(out.contains("\"readyPub\""), "alias from args[1] not honored. Got:\n{out}");
}

#[test]
fn model_synthesizes_input_and_output_pair() {
    let out = compile_jit(&component("  readonly value = model(0);", "model"));
    // Both decorators on the same field.
    assert!(out.contains("type: Input"), "@Input half of model() missing. Got:\n{out}");
    assert!(out.contains("type: Output"), "@Output half of model() missing. Got:\n{out}");
    // Output alias is `<input>Change` (matches ngc behavior).
    assert!(out.contains("\"valueChange\""), "model's output alias `<name>Change` missing. Got:\n{out}");
}

#[test]
fn model_required_propagates_required_to_input() {
    let out = compile_jit(&component("  readonly value = model.required<string>();", "model"));
    assert!(out.contains("required: true"), "model.required() should set Input.required. Got:\n{out}");
}

#[test]
fn view_child_emits_synthesized_view_child_with_is_signal() {
    let out = compile_jit(&component(
        "  readonly el = viewChild<ElementRef>('ref');",
        "viewChild, ElementRef",
    ));
    assert!(out.contains("type: ViewChild"), "synthesized @ViewChild missing. Got:\n{out}");
    assert!(out.contains("isSignal: true"), "isSignal flag missing on query. Got:\n{out}");
    // OXC's TS stripper normalizes single-quoted string literals to double-quoted, so
    // assert against the post-strip form. The locator text just needs to survive.
    assert!(out.contains("\"ref\""), "locator (positional arg 0) lost. Got:\n{out}");
}

#[test]
fn view_child_with_options_spreads_existing_options() {
    let out = compile_jit(&component(
        "  readonly el = viewChild<ElementRef>('ref', { read: ElementRef });",
        "viewChild, ElementRef",
    ));
    assert!(out.contains("...{ read: ElementRef }"), "options spread missing. Got:\n{out}");
    assert!(out.contains("isSignal: true"), "isSignal still folded in. Got:\n{out}");
}

#[test]
fn view_children_emits_view_children_decorator() {
    let out = compile_jit(&component(
        "  readonly els = viewChildren<ElementRef>('ref');",
        "viewChildren, ElementRef",
    ));
    assert!(out.contains("type: ViewChildren"), "Got:\n{out}");
}

#[test]
fn content_child_required_emits_content_child() {
    let out = compile_jit(&component(
        "  readonly el = contentChild.required<ElementRef>('ref');",
        "contentChild, ElementRef",
    ));
    // .required() variant maps to the same decorator name (ContentChild), with isSignal.
    assert!(out.contains("type: ContentChild"), "Got:\n{out}");
    assert!(out.contains("isSignal: true"), "Got:\n{out}");
}

#[test]
fn explicit_view_child_decorator_blocks_query_synthesis() {
    // Mirrors the AOT regression test in component.spec.ts:272 — explicit @ViewChild
    // wins over the signal-derived query so the field isn't registered twice.
    let out = compile_jit(&component(
        "  @ViewChild('r') r = viewChild<ElementRef>('r');",
        "viewChild, ViewChild, ElementRef",
    ));
    assert!(out.contains("type: ViewChild"), "explicit @ViewChild lost. Got:\n{out}");
    assert!(
        !out.contains("isSignal"),
        "synthesis must skip when explicit query decorator coexists. Got:\n{out}"
    );
}

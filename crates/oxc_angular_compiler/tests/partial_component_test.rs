//! End-to-end tests for the partial-declaration Component emitter
//! (`ɵɵngDeclareComponent`). Unit-shape tests of the emitter live alongside
//! E2E tests because constructing a `ComponentMetadata` directly is
//! verbose; going through `transform_angular_file` is closer to how the
//! emitter is actually used.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, link, transform_angular_file};

fn compile_partial(allocator: &Allocator, filename: &str, source: &str) -> String {
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result = transform_angular_file(allocator, filename, source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    result.code
}

#[test]
fn minimal_component_emits_template_as_string_literal() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-hello', template: '<h1>Hello</h1>', standalone: true })
export class HelloComponent {}
";
    let code = compile_partial(&allocator, "hello.component.ts", source);

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareComponent"),
        "expected ɵɵngDeclareComponent, got:\n{code}"
    );
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareFactory"),
        "expected ɵɵngDeclareFactory, got:\n{code}"
    );
    // Template is emitted as a string literal — the linker re-parses.
    assert!(
        code.contains(r#"template:"<h1>Hello</h1>""#),
        "expected template as string literal, got:\n{code}"
    );
    // Inline template marker.
    assert!(code.contains("isInline:true"), "expected isInline:true, got:\n{code}");
    assert!(
        !code.contains("\u{0275}\u{0275}defineComponent"),
        "ɵɵdefineComponent should not appear in partial mode, got:\n{code}"
    );
}

#[test]
fn component_no_template_pipeline_runs_in_partial_mode() {
    // Even templates that would normally trigger the IR pipeline (block
    // syntax, control flow) compile in partial mode — the linker handles
    // parsing.
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({
  selector: 'app-list',
  template: '@if (show) { <p>shown</p> } @for (i of items; track i) { <span>{{ i }}</span> }',
  standalone: true
})
export class ListComponent {
  show = true;
  items = [1, 2, 3];
}
";
    let code = compile_partial(&allocator, "list.component.ts", source);

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareComponent"),
        "expected ɵɵngDeclareComponent, got:\n{code}"
    );
    // Block syntax in template bumps minVersion to 17.0.0.
    assert!(
        code.contains(r#"minVersion:"17.0.0""#),
        "expected minVersion:\"17.0.0\" (block syntax), got:\n{code}"
    );
    // The template stays a string — no instruction emission, no @if
    // parsing on our side.
    assert!(
        code.contains(r#"template:"@if"#),
        "expected raw @if in template literal, got:\n{code}"
    );
}

#[test]
fn component_styles_emitted_as_string_array() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({
  selector: 'app-styled',
  template: '<div></div>',
  styles: ['div { color: red; }', 'p { font: 12px; }'],
  standalone: true
})
export class StyledComponent {}
";
    let code = compile_partial(&allocator, "styled.component.ts", source);

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareComponent"),
        "expected ɵɵngDeclareComponent, got:\n{code}"
    );
    // Styles preserved as raw strings.
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains(r#"styles:["div{color:red;}","p{font:12px;}"]"#),
        "expected styles array of raw strings, got:\n{code}"
    );
}

#[test]
fn component_change_detection_omitted_when_default() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-foo', template: '', standalone: true })
export class FooComponent {}
";
    let code = compile_partial(&allocator, "foo.component.ts", source);
    assert!(
        !code.contains("changeDetection"),
        "Default change detection should be omitted, got:\n{code}"
    );
}

#[test]
fn component_change_detection_on_push_emitted() {
    let allocator = Allocator::default();
    let source = "import { Component, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-push',
  template: '',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: true
})
export class PushComponent {}
";
    let code = compile_partial(&allocator, "push.component.ts", source);
    assert!(
        code.contains("\u{0275}\u{0275}ChangeDetectionStrategy.OnPush")
            || code.contains("ChangeDetectionStrategy.OnPush"),
        "expected i0.ChangeDetectionStrategy.OnPush, got:\n{code}"
    );
}

#[test]
fn component_encapsulation_omitted_when_emulated() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-emu', template: '', standalone: true })
export class EmuComponent {}
";
    let code = compile_partial(&allocator, "emu.component.ts", source);
    assert!(
        !code.contains("encapsulation"),
        "Emulated encapsulation should be omitted, got:\n{code}"
    );
}

#[test]
fn component_encapsulation_none_emitted() {
    let allocator = Allocator::default();
    let source = "import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-noenc',
  template: '',
  encapsulation: ViewEncapsulation.None,
  standalone: true
})
export class NoEncComponent {}
";
    let code = compile_partial(&allocator, "noenc.component.ts", source);
    assert!(
        code.contains("ViewEncapsulation.None"),
        "expected i0.ViewEncapsulation.None, got:\n{code}"
    );
}

#[test]
fn component_round_trips_through_linker_to_full_define_component() {
    // The full E2E test: partial output → our own linker →
    // ɵɵdefineComponent with no ngDeclare residue.
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-rt', template: '<h1>{{ title }}</h1>', standalone: true })
export class RtComponent {
  title = 'round-tripped';
}
";
    let code = compile_partial(&allocator, "rt.component.ts", source);

    let linked = link(&allocator, &code, "rt.component.mjs");
    assert!(linked.linked, "linker should accept the partial component output. emitted:\n{code}");
    assert!(
        linked.code.contains("\u{0275}\u{0275}defineComponent"),
        "linked output should contain ɵɵdefineComponent, got:\n{}",
        linked.code
    );
    assert!(
        !linked.code.contains("\u{0275}\u{0275}ngDeclare"),
        "linked output should have no ngDeclare* calls left, got:\n{}",
        linked.code
    );
}

#[test]
fn full_mode_component_unchanged() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-full', template: '<p>hi</p>', standalone: true })
export class FullComponent {}
";
    let result = transform_angular_file(&allocator, "full.component.ts", source, None, None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    assert!(
        result.code.contains("\u{0275}\u{0275}defineComponent"),
        "default-mode (Full) should still emit ɵɵdefineComponent, got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("\u{0275}\u{0275}ngDeclareComponent"),
        "Full mode must not leak ngDeclare calls, got:\n{}",
        result.code
    );
}

#[test]
fn component_preserve_whitespaces_emitted() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({
  selector: 'app-ws',
  template: '<p>   spaced   </p>',
  preserveWhitespaces: true,
  standalone: true
})
export class WsComponent {}
";
    let code = compile_partial(&allocator, "ws.component.ts", source);
    assert!(
        code.contains("preserveWhitespaces:true"),
        "expected preserveWhitespaces:true, got:\n{code}"
    );
}

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

/// Regression test for the codex P1 review on PR #325:
///
/// Standalone components with `imports: [...]` are extracted with
/// `declaration_list_emit_mode = RuntimeResolved`, which means
/// `metadata.declarations` stays empty (full mode emits
/// `ɵɵgetComponentDepsFactory` at runtime instead). Partial mode can't
/// do that — the linker emits a static `dependencies: [...]` array. The
/// pre-fix behavior omitted the `dependencies` field entirely, so any
/// standalone library component that imported `CommonModule`, child
/// components, or pipes would render with the directives/pipes silently
/// not registered.
///
/// Fix: in partial mode, when `meta.declarations.is_empty()` but
/// `meta.imports` is non-empty, lower each import to a synthetic
/// directive-shape dep on the fly. Our linker reads only the `type`
/// expression from each entry, so the resulting `dependencies: [Import1,
/// Import2, …]` array is byte-equivalent to what we'd get if the
/// analyzer had populated `declarations` directly.
#[test]
fn standalone_component_with_imports_emits_dependencies_in_partial_mode() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MyDir } from './my.directive';

@Component({
  selector: 'app-foo',
  template: '<div *ngIf=\"show\" myDir>hi</div>',
  standalone: true,
  imports: [CommonModule, MyDir]
})
export class FooComponent {
  show = true;
}
";
    let code = compile_partial(&allocator, "foo.component.ts", source);

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareComponent"),
        "expected ɵɵngDeclareComponent, got:\n{code}"
    );
    // The partial declaration should include both imports as deps.
    assert!(code.contains("dependencies:"), "expected dependencies field, got:\n{code}");
    assert!(
        code.contains("type:CommonModule"),
        "expected CommonModule in dependencies, got:\n{code}"
    );
    assert!(code.contains("type:MyDir"), "expected MyDir in dependencies, got:\n{code}");

    // Round-trip through the linker — the full ɵcmp should carry both
    // imports in its dependencies array.
    let linked = link(&allocator, &code, "foo.component.mjs");
    assert!(linked.linked, "linker should accept the partial output, emitted:\n{code}");
    assert!(
        linked.code.contains("\u{0275}\u{0275}defineComponent"),
        "linked output should contain ɵɵdefineComponent, got:\n{}",
        linked.code
    );
    // After linking, both import types must be in the final dependencies array.
    assert!(
        linked.code.contains("dependencies: [")
            && linked.code.contains("CommonModule")
            && linked.code.contains("MyDir"),
        "linked dependencies array should preserve both imports, got:\n{}",
        linked.code
    );
}

/// Sanity: a non-standalone component or a standalone component with no
/// imports should NOT get a synthetic `dependencies` field.
#[test]
fn standalone_component_with_no_imports_omits_dependencies() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-bar', template: '<p>hi</p>', standalone: true })
export class BarComponent {}
";
    let code = compile_partial(&allocator, "bar.component.ts", source);
    assert!(!code.contains("dependencies:"), "no imports → no dependencies field, got:\n{code}");
}

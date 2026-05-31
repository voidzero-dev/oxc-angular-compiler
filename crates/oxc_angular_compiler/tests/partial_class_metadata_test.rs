//! End-to-end tests for the partial-declaration ClassMetadata emitter
//! (`ɵɵngDeclareClassMetadata`).

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
fn partial_mode_emits_bare_ng_declare_class_metadata_for_injectable() {
    let allocator = Allocator::default();
    let source = "import { Injectable } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class MyService {}
";
    let code = compile_partial(&allocator, "my.service.ts", source);

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareClassMetadata"),
        "expected ɵɵngDeclareClassMetadata, got:\n{code}"
    );
    // No ngDevMode guard, no IIFE.
    assert!(
        !code.contains("ngDevMode"),
        "partial ClassMetadata should NOT carry the ngDevMode guard, got:\n{code}"
    );
    assert!(
        !code.contains("\u{0275}setClassMetadata"),
        "partial mode should NOT emit ɵsetClassMetadata, got:\n{code}"
    );
}

#[test]
fn partial_mode_class_metadata_for_component() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-foo', template: '<p>hi</p>', standalone: true })
export class FooComponent {}
";
    let code = compile_partial(&allocator, "foo.component.ts", source);

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareClassMetadata"),
        "expected ɵɵngDeclareClassMetadata for component, got:\n{code}"
    );
    assert!(
        !code.contains("\u{0275}setClassMetadata"),
        "partial mode should NOT emit ɵsetClassMetadata, got:\n{code}"
    );
}

#[test]
fn partial_class_metadata_round_trips_through_linker() {
    // The whole partial output (component + class metadata) should link
    // cleanly back into the full ɵsetClassMetadata IIFE form.
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-rt', template: '<h1>round trip</h1>', standalone: true })
export class RtCmp {}
";
    let code = compile_partial(&allocator, "rt.component.ts", source);
    let linked = link(&allocator, &code, "rt.component.mjs");
    assert!(linked.linked, "linker should accept the partial output. emitted:\n{code}");
    assert!(
        linked.code.contains("\u{0275}setClassMetadata"),
        "linked output should contain ɵsetClassMetadata, got:\n{}",
        linked.code
    );
    assert!(
        !linked.code.contains("\u{0275}\u{0275}ngDeclare"),
        "linked output should have no ngDeclare* calls left, got:\n{}",
        linked.code
    );
}

#[test]
fn full_mode_class_metadata_unchanged() {
    let allocator = Allocator::default();
    let source = "import { Component } from '@angular/core';

@Component({ selector: 'app-full', template: '<p>hi</p>', standalone: true })
export class FullCmp {}
";
    let result = transform_angular_file(&allocator, "full.component.ts", source, None, None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    assert!(
        result.code.contains("\u{0275}setClassMetadata"),
        "Full mode should still emit ɵsetClassMetadata, got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("\u{0275}\u{0275}ngDeclareClassMetadata"),
        "Full mode must not leak ngDeclareClassMetadata, got:\n{}",
        result.code
    );
}

//! End-to-end test for partial-compilation emit via `transform_angular_file`.
//!
//! Exercises the full pipeline from TS source → emitted JS, with
//! `compilation_mode: Partial` set. Asserts that the output contains
//! `ɵɵngDeclareInjectable` + `ɵɵngDeclareFactory` and none of the
//! `ɵɵdefine*` calls full mode would emit.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, link, transform_angular_file};

#[test]
fn partial_mode_emits_ng_declare_for_root_injectable() {
    let allocator = Allocator::default();
    let source = "import { Injectable } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class MyService {}
";

    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result = transform_angular_file(&allocator, "my.service.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let code = &result.code;

    // Partial-form calls present.
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareFactory"),
        "expected ɵɵngDeclareFactory in partial-mode output, got:\n{code}"
    );
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareInjectable"),
        "expected ɵɵngDeclareInjectable in partial-mode output, got:\n{code}"
    );

    // Full-form calls absent.
    assert!(
        !code.contains("\u{0275}\u{0275}defineInjectable"),
        "ɵɵdefineInjectable should not appear in partial mode, got:\n{code}"
    );

    // Both declarations carry the partial version envelope.
    assert!(
        code.contains("minVersion") && code.contains("\"0.0.0-PLACEHOLDER\""),
        "expected minVersion + placeholder version, got:\n{code}"
    );

    // Round-trip the entire emitted file through our linker — it should
    // expand back into a working full-Ivy Injectable.
    let linked = link(&allocator, code, "my.service.mjs");
    assert!(linked.linked, "linker should accept the partial output");
    assert!(
        linked.code.contains("\u{0275}\u{0275}defineInjectable"),
        "linked output should contain ɵɵdefineInjectable, got:\n{}",
        linked.code
    );
    assert!(
        !linked.code.contains("\u{0275}\u{0275}ngDeclareFactory")
            && !linked.code.contains("\u{0275}\u{0275}ngDeclareInjectable"),
        "linked output should have no ngDeclare calls left, got:\n{}",
        linked.code
    );
}

#[test]
fn full_mode_unchanged_for_root_injectable() {
    // Sanity: with the default (Full) compilation mode, output continues to
    // use ɵɵdefineInjectable. This pins that adding the option didn't
    // accidentally change default behavior.
    let allocator = Allocator::default();
    let source = "import { Injectable } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class MyService {}
";

    let result = transform_angular_file(&allocator, "my.service.ts", source, None, None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);

    assert!(
        result.code.contains("\u{0275}\u{0275}defineInjectable"),
        "default-mode (Full) should still emit ɵɵdefineInjectable, got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("\u{0275}\u{0275}ngDeclareInjectable"),
        "Full mode must not leak ngDeclare calls, got:\n{}",
        result.code
    );
}

#[test]
fn partial_mode_with_use_class_provider() {
    let allocator = Allocator::default();
    let source = "import { Injectable } from '@angular/core';

class ConcreteService {}

@Injectable({ providedIn: 'root', useClass: ConcreteService })
export class AbstractService {}
";

    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "abstract.service.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);

    assert!(
        result.code.contains("useClass:ConcreteService")
            || result.code.contains("useClass: ConcreteService"),
        "expected useClass:ConcreteService in partial output, got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("\u{0275}\u{0275}ngDeclareInjectable"),
        "expected ɵɵngDeclareInjectable, got:\n{}",
        result.code
    );
}

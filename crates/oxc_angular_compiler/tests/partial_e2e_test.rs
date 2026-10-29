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

/// Transforms `source` in partial mode, then links the result. Returns the
/// partial output and the linked output, both with whitespace removed.
fn partial_and_linked(source: &str) -> (String, String) {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result = transform_angular_file(&allocator, "case.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let linked = link(&allocator, &result.code, "case.ts");
    assert!(linked.linked, "linker should accept the partial output:\n{}", result.code);
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    (strip(&result.code), strip(&linked.code))
}

#[test]
fn partial_pipe_factory_keeps_constructor_deps() {
    // ngtsc: ɵɵngDeclareFactory({..., type: P, deps: [{ token: TOK },
    // { token: Ok, optional: true }], target: i0.ɵɵFactoryTarget.Pipe })
    let (partial, linked) = partial_and_linked(
        "import { Pipe, Inject, Optional, InjectionToken } from '@angular/core';
const TOK = new InjectionToken<string>('t');
class Ok {}
@Pipe({ name: 'p' })
export class P {
  constructor(@Inject(TOK) t: string, @Optional() o: Ok) {}
  transform(v: any) { return v; }
}
",
    );
    assert!(
        partial.contains("type:P,deps:[{token:TOK")
            && partial.contains("},{token:Ok,optional:true}],target:i0.ɵɵFactoryTarget.Pipe"),
        "pipe factory should keep its constructor deps, got:\n{partial}"
    );
    assert!(
        linked.contains(
            "new(__ngFactoryType__||P)(i0.ɵɵdirectiveInject(TOK,16),i0.ɵɵdirectiveInject(Ok,24))"
        ),
        "linked pipe factory should inject TOK and Ok, got:\n{linked}"
    );
}

#[test]
fn partial_pipe_factory_without_constructor() {
    // ngtsc: a pipe with no constructor and no superclass gets `deps: []`;
    // one that extends a base class gets `deps: null` (inherited factory).
    let (partial, _) = partial_and_linked(
        "import { Pipe } from '@angular/core';
@Pipe({ name: 'p' })
export class P { transform(v: any) { return v; } }
",
    );
    assert!(
        partial.contains("type:P,deps:[],target:i0.ɵɵFactoryTarget.Pipe"),
        "pipe without a constructor should get deps:[], got:\n{partial}"
    );

    let (partial, linked) = partial_and_linked(
        "import { Pipe } from '@angular/core';
class Ok {}
class Base { constructor(o: Ok) {} }
@Pipe({ name: 'p' })
export class P extends Base { transform(v: any) { return v; } }
",
    );
    assert!(
        partial.contains("type:P,deps:null,target:i0.ɵɵFactoryTarget.Pipe"),
        "pipe inheriting its constructor should get deps:null, got:\n{partial}"
    );
    assert!(
        linked.contains("i0.ɵɵgetInheritedFactory(P)"),
        "linked pipe factory should be inherited, got:\n{linked}"
    );
}

#[test]
fn partial_use_class_keeps_empty_deps() {
    // ngtsc: ɵɵngDeclareInjectable({..., useClass: Holder.cls(), deps: [] }),
    // which links to `new (Holder.cls())()` rather than `Holder.cls().ɵfac`.
    let (partial, linked) = partial_and_linked(
        "import { Injectable } from '@angular/core';
export const Holder: any = { cls: () => class {} };
@Injectable({ providedIn: 'root', useClass: Holder.cls(), deps: [] })
export class F1 {}
",
    );
    assert!(
        partial.contains("providedIn:\"root\",useClass:Holder.cls(),deps:[]})"),
        "useClass should keep its empty deps, got:\n{partial}"
    );
    assert!(
        linked.contains("new(Holder.cls())()"),
        "linked useClass should be instantiated, got:\n{linked}"
    );
}

#[test]
fn partial_use_factory_keeps_empty_deps() {
    // ngtsc: ɵɵngDeclareInjectable({..., useFactory: make, deps: [] }),
    // which links to the conditional factory rather than `() => (make)()`.
    let (partial, linked) = partial_and_linked(
        "import { Injectable } from '@angular/core';
export function make() { return {}; }
@Injectable({ providedIn: 'root', useFactory: make, deps: [] })
export class F1 {}
",
    );
    assert!(
        partial.contains("providedIn:\"root\",useFactory:make,deps:[]})"),
        "useFactory should keep its empty deps, got:\n{partial}"
    );
    assert!(
        linked.contains("__ngConditionalFactory__=(make)()"),
        "linked useFactory should build the conditional factory, got:\n{linked}"
    );
}

#[test]
fn partial_injectable_factory_inherits_constructor() {
    // ngtsc: ɵɵngDeclareFactory({..., type: S, deps: null,
    // target: i0.ɵɵFactoryTarget.Injectable })
    let (partial, linked) = partial_and_linked(
        "import { Injectable } from '@angular/core';
class Ok {}
@Injectable() export class B { constructor(o: Ok) {} }
@Injectable() export class S extends B {}
",
    );
    assert!(
        partial.contains("type:S,deps:null,target:i0.ɵɵFactoryTarget.Injectable"),
        "injectable inheriting its constructor should get deps:null, got:\n{partial}"
    );
    assert!(
        linked.contains("new(__ngFactoryType__||B)(i0.ɵɵinject(Ok))"),
        "linked base injectable factory should inject Ok, got:\n{linked}"
    );
    assert!(
        linked.contains("i0.ɵɵgetInheritedFactory(S)"),
        "linked injectable factory should be inherited, got:\n{linked}"
    );
}

#[test]
fn partial_ng_module_factory_inherits_constructor() {
    // ngtsc: ɵɵngDeclareFactory({..., type: M, deps: null,
    // target: i0.ɵɵFactoryTarget.NgModule })
    let (partial, linked) = partial_and_linked(
        "import { NgModule } from '@angular/core';
class Ok {}
export class B { constructor(o: Ok) {} }
@NgModule({}) export class M extends B {}
",
    );
    assert!(
        partial.contains("type:M,deps:null,target:i0.ɵɵFactoryTarget.NgModule"),
        "NgModule inheriting its constructor should get deps:null, got:\n{partial}"
    );
    assert!(
        linked.contains("i0.ɵɵgetInheritedFactory(M)"),
        "linked NgModule factory should be inherited, got:\n{linked}"
    );
}

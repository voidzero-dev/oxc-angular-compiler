//! Tests for the partial-declaration Service linker (`ɵɵngDeclareService`),
//! the partial form of Angular v22's `@Service` decorator.
//!
//! Each case pins the linked output against Angular's `compileService`
//! (packages/compiler/src/service_compiler.ts) goldens: the factory delegates
//! to the class `ɵfac` unless an explicit `factory` is declared, and
//! `autoProvided` is emitted only when explicitly `false`.

use oxc_allocator::Allocator;
use oxc_angular_compiler::linker::link;

/// Build a `ɵɵngDeclareService` partial declaration with extra metadata props
/// appended after `type` (e.g. `, factory: ...` or `, autoProvided: false`).
fn declare_service(extra: &str) -> String {
    format!(
        r#"import * as i0 from "@angular/core";
export class MyService {{}}
MyService.ɵfac = function MyService_Factory(__ngFactoryType__) {{ return new (__ngFactoryType__ || MyService)(); }};
MyService.ɵprov = i0.ɵɵngDeclareService({{ minVersion: "22.0.0", version: "0.0.0-PLACEHOLDER", ngImport: i0, type: MyService{extra} }});"#
    )
}

#[test]
fn links_default_service_to_define_service() {
    let allocator = Allocator::default();
    let result = link(&allocator, &declare_service(""), "test.mjs");
    assert!(
        result
            .code
            .contains("i0.\u{0275}\u{0275}defineService({ token: MyService, factory: MyService.\u{0275}fac })"),
        "expected default defineService delegating to ɵfac, got: {}",
        result.code
    );
    // The partial declaration must be linked away (no JIT fallback at runtime).
    assert!(
        !result.code.contains("ngDeclareService"),
        "ɵɵngDeclareService must be replaced, got: {}",
        result.code
    );
}

#[test]
fn links_service_with_explicit_factory() {
    let allocator = Allocator::default();
    let result = link(
        &allocator,
        &declare_service(", factory: () => new Alternate()"),
        "test.mjs",
    );
    assert!(
        result.code.contains("factory: () => (() => new Alternate())()"),
        "expected the declared factory wrapped in an invoking arrow, got: {}",
        result.code
    );
    assert!(!result.code.contains("ngDeclareService"), "got: {}", result.code);
}

#[test]
fn links_service_with_auto_provided_false() {
    let allocator = Allocator::default();
    let result = link(
        &allocator,
        &declare_service(", autoProvided: false"),
        "test.mjs",
    );
    assert!(
        result.code.contains(
            "i0.\u{0275}\u{0275}defineService({ token: MyService, factory: MyService.\u{0275}fac, autoProvided: false })"
        ),
        "expected autoProvided: false to be preserved, got: {}",
        result.code
    );
}

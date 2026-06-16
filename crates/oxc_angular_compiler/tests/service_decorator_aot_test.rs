//! Regression: a class whose only Angular decorator is `@Service` (Angular v22)
//! must be AOT-compiled (emitting `ɵfac` + `ɵɵdefineService`), not left for the
//! JIT compiler.
//!
//! The cheap pre-check `has_angular_decorator` previously omitted `Service`, so
//! a `@Service`-only file skipped Angular compilation entirely and the decorator
//! was downleveled to a runtime `_decorate([...])` call — which triggers
//! `ɵɵngDeclareService` → JIT fallback at runtime
//! ("service needs the JIT compiler, '@angular/compiler' is not available").

use oxc_allocator::Allocator;
use oxc_angular_compiler::transform_angular_file;

#[test]
fn compiles_service_only_file_to_aot_definitions() {
    let allocator = Allocator::default();
    let source = r#"
import { ErrorHandler, Service, inject } from '@angular/core';
import { AnalyticsService } from './analytics';

@Service({ autoProvided: false })
export class AnalyticsErrorReportHandler extends ErrorHandler {
  private _analytics = inject(AnalyticsService);
}
"#;

    let result = transform_angular_file(&allocator, "error-report-handler.ts", source, None, None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    let code = &result.code;

    // AOT definitions must be emitted.
    assert!(
        code.contains("\u{0275}\u{0275}defineService"),
        "expected ɵɵdefineService (AOT), got:\n{code}"
    );
    assert!(code.contains("\u{0275}fac"), "expected ɵfac factory, got:\n{code}");
    // Emitted JS is compact (no space after the colon).
    assert!(
        code.contains("autoProvided:false"),
        "expected autoProvided:false to be preserved, got:\n{code}"
    );

    // The `@Service` decorator must be compiled away — not downleveled to a
    // runtime decorator call (which would fall back to JIT at runtime).
    assert!(
        !code.contains("_decorate("),
        "@Service must be compiled away, not left as a runtime decorator, got:\n{code}"
    );
}

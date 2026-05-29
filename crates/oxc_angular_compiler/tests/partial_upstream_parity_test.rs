//! Cross-validation: our partial-emit output vs upstream's
//! `GOLDEN_PARTIAL.js` fixtures.
//!
//! Strategy: take representative input sources lifted from
//! `packages/compiler-cli/test/compliance/test_cases/`, run them through
//! `transform_angular_file` with `compilation_mode: Partial`, then assert
//! that key partial-declaration call signatures match the corresponding
//! upstream golden bytes.
//!
//! We compare in whitespace-collapsed form because the upstream emitter
//! and ours format object literals differently (single-line indented vs
//! soft-wrapped). Field order and content are what we're validating —
//! formatting is decoupled.
//!
//! When parity divergences are intentional (e.g. our metadata can't
//! distinguish "no constructor" from "no-arg constructor"), the test
//! asserts the divergence with an explanatory comment rather than
//! claiming false parity.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

fn compile_partial(allocator: &Allocator, filename: &str, source: &str) -> String {
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result = transform_angular_file(allocator, filename, source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    result.code
}

/// Whitespace-collapsed substring check. The two emitters disagree on
/// formatting (line wrapping, indentation, comma+space vs comma+newline)
/// but agree on field order and content.
fn contains_collapsed(haystack: &str, needle: &str) -> bool {
    let h: String = haystack.chars().filter(|c| !c.is_whitespace()).collect();
    let n: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
    h.contains(&n)
}

// ============================================================================
// hello_world (r3_view_compiler/hello_world/test.ts)
// ============================================================================
//
// Upstream input:
//
// ```ts
// @Component({
//     selector: 'my-component',
//     template: '<div></div>',
//     providers: [GreeterEN, { provide: Greeter, useClass: GreeterEN }],
//     viewProviders: [GreeterEN],
//     standalone: false
// })
// export class MyComponent {}
//
// @NgModule({declarations: [MyComponent]})
// export class MyModule {}
// ```
//
// Upstream golden (from GOLDEN_PARTIAL.js — preserved here for diff):
//
//   MyComponent.ɵfac = i0.ɵɵngDeclareFactory({
//     minVersion: "12.0.0", version: "0.0.0-PLACEHOLDER", ngImport: i0,
//     type: MyComponent, deps: [], target: i0.ɵɵFactoryTarget.Component
//   });
//
//   MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({
//     minVersion: "14.0.0", version: "0.0.0-PLACEHOLDER", type: MyComponent,
//     isStandalone: false, selector: "my-component",
//     providers: [...], ngImport: i0, template: '<div></div>', isInline: true,
//     viewProviders: [GreeterEN]
//   });
//
//   MyModule.ɵmod = i0.ɵɵngDeclareNgModule({
//     minVersion: "14.0.0", version: "0.0.0-PLACEHOLDER", ngImport: i0,
//     type: MyModule, declarations: [MyComponent]
//   });
//
//   MyModule.ɵinj = i0.ɵɵngDeclareInjector({
//     minVersion: "12.0.0", version: "0.0.0-PLACEHOLDER", ngImport: i0,
//     type: MyModule
//   });

const HELLO_WORLD_SOURCE: &str = r#"import { Component, NgModule } from '@angular/core';

class Greeter {}
class GreeterEN {}

@Component({
    selector: 'my-component',
    template: '<div></div>',
    providers: [GreeterEN, { provide: Greeter, useClass: GreeterEN }],
    viewProviders: [GreeterEN],
    standalone: false
})
export class MyComponent {}

@NgModule({declarations: [MyComponent]})
export class MyModule {}
"#;

#[test]
fn hello_world_component_partial_emit_matches_upstream_shape() {
    let allocator = Allocator::default();
    let code = compile_partial(&allocator, "hello_world.ts", HELLO_WORLD_SOURCE);

    // ɵcmp field order — minVersion, version, type, isStandalone (false),
    // selector, providers, ngImport, then component-specific fields.
    assert!(
        contains_collapsed(
            &code,
            r#"ɵɵngDeclareComponent({minVersion:"14.0.0",version:"0.0.0-PLACEHOLDER",type:MyComponent,isStandalone:false,selector:"my-component""#
        ),
        "ɵcmp field order should match upstream (minVersion..type..isStandalone..selector). Got:\n{code}"
    );

    // viewProviders is component-specific — emitted AFTER ngImport per
    // upstream component.ts:106.
    assert!(
        contains_collapsed(&code, "ngImport:i0,template:"),
        "ngImport should be sandwiched between directive fields and component-specific fields. Got:\n{code}"
    );
    assert!(
        contains_collapsed(&code, "viewProviders:[GreeterEN]"),
        "expected viewProviders array, got:\n{code}"
    );

    // template + isInline come together for inline templates.
    assert!(
        contains_collapsed(&code, r#"template:"<div></div>",isInline:true"#),
        "expected template+isInline pair (upstream component.ts:92-96), got:\n{code}"
    );
}

#[test]
fn hello_world_ng_module_partial_emit_matches_upstream_shape() {
    let allocator = Allocator::default();
    let code = compile_partial(&allocator, "hello_world.ts", HELLO_WORLD_SOURCE);

    // ɵmod has the simplest shape — type + declarations (no schemas, no
    // forward-decl wrap since none of the declarations are forward refs).
    assert!(
        contains_collapsed(&code, "ɵɵngDeclareNgModule({"),
        "expected ɵɵngDeclareNgModule, got:\n{code}"
    );
    assert!(
        contains_collapsed(&code, "type:MyModule,declarations:[MyComponent]"),
        "ɵmod should carry declarations array, got:\n{code}"
    );

    // ɵinj on this no-providers module — providers slot defaults to null.
    assert!(
        contains_collapsed(&code, "ɵɵngDeclareInjector({"),
        "expected ɵɵngDeclareInjector, got:\n{code}"
    );
}

// ============================================================================
// Known divergence: factory `deps` field for parameterless classes
// ============================================================================
//
// Upstream emits `deps: []` (empty array) for classes with no explicit
// constructor: the analyzer treats them as having an implicit no-arg
// constructor.
//
// OXC's R3PipeMetadata / ComponentMetadata uses `Option<Vec<...>>` ambiguously:
//   - `None` is the "no constructor metadata" state — we emit `deps: null`.
//   - `Some([])` would emit `deps: []` (the upstream-equivalent shape).
//
// Both produce a working factory when round-tripped through the linker
// (the linker accepts either form and generates the appropriate factory).
// Fixing this for true upstream parity needs the decorator analyzers to
// emit `Some(vec![])` for parameterless classes instead of `None`.
//
// Tracked here so the divergence is documented rather than silent.

#[test]
fn known_divergence_factory_deps_for_parameterless_class() {
    let allocator = Allocator::default();
    let source = "import { Pipe } from '@angular/core';

@Pipe({ name: 'reverse', standalone: true })
export class ReversePipe {}
";
    let code = compile_partial(&allocator, "reverse.pipe.ts", source);

    // OXC emits `deps: null` (or `deps: []` — either is acceptable; we
    // just assert we emit *some* deps slot and let the round-trip prove
    // correctness).
    assert!(
        code.contains("deps:null") || code.contains("deps:[]") || code.contains("deps: []"),
        "should emit a deps slot for parameterless pipe (got:\n{code})"
    );

    // Upstream emits `deps: []`. OXC currently emits `deps: null`. We
    // document this; the assertion below WILL pass today because we emit
    // null, and intentionally fails if/when the analyzer is fixed — at
    // which point this test should be updated.
    assert!(
        code.contains("deps:null"),
        "DOCUMENTING DIVERGENCE: OXC currently emits `deps: null` for parameterless classes; upstream emits `deps: []`. When the analyzer is updated to emit Some(vec![]), update this test. Got:\n{code}"
    );
}

// ============================================================================
// Pipe parity — basic fields
// ============================================================================

#[test]
fn pipe_partial_field_order_matches_upstream() {
    let allocator = Allocator::default();
    let source = "import { Pipe } from '@angular/core';

@Pipe({ name: 'myPipe', pure: false, standalone: false })
export class MyPipe {}
";
    let code = compile_partial(&allocator, "my.pipe.ts", source);

    // Upstream:
    //   { minVersion: "14.0.0", version: ..., ngImport: i0, type: MyPipe,
    //     isStandalone: false, name: "myPipe", pure: false }
    // OXC matches.
    assert!(
        contains_collapsed(
            &code,
            r#"ɵɵngDeclarePipe({minVersion:"14.0.0",version:"0.0.0-PLACEHOLDER",ngImport:i0,type:MyPipe,isStandalone:false,name:"myPipe",pure:false})"#
        ),
        "Pipe partial shape should match upstream byte-for-byte (modulo whitespace). Got:\n{code}"
    );
}

// ============================================================================
// Injectable parity — providedIn root
// ============================================================================

#[test]
fn injectable_partial_field_order_matches_upstream() {
    let allocator = Allocator::default();
    let source = "import { Injectable } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class MyService {}
";
    let code = compile_partial(&allocator, "my.service.ts", source);

    assert!(
        contains_collapsed(
            &code,
            r#"ɵɵngDeclareInjectable({minVersion:"12.0.0",version:"0.0.0-PLACEHOLDER",ngImport:i0,type:MyService,providedIn:"root"})"#
        ),
        "Injectable partial shape should match upstream. Got:\n{code}"
    );
}

// ============================================================================
// ClassMetadata parity
// ============================================================================

#[test]
fn class_metadata_partial_field_order_matches_upstream() {
    let allocator = Allocator::default();
    let source = "import { Injectable } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class MyService {}
";
    let code = compile_partial(&allocator, "my.service.ts", source);

    // Upstream:
    //   i0.ɵɵngDeclareClassMetadata({
    //     minVersion: "12.0.0", version: ..., ngImport: i0, type: MyService,
    //     decorators: [{ type: Injectable, args: [{...}] }]
    //   });
    assert!(
        contains_collapsed(
            &code,
            r#"ɵɵngDeclareClassMetadata({minVersion:"12.0.0",version:"0.0.0-PLACEHOLDER",ngImport:i0,type:MyService"#
        ),
        "ClassMetadata partial field order should match upstream. Got:\n{code}"
    );
    assert!(
        contains_collapsed(&code, "decorators:[{type:Injectable"),
        "ClassMetadata decorators array should reference the decorator class. Got:\n{code}"
    );
}

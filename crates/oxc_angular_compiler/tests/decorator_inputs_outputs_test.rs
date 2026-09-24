//! `inputs:` declared in `@Directive` metadata were silently dropped from the
//! compiled definition. This is the original report; exhaustive parity with
//! ngtsc for `inputs:` / `outputs:` / `queries:` metadata is checked by
//! `decorator_metadata_ngtsc_test.rs`.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

/// The whitespace-free `ɵɵdefineDirective` call for `class_name`.
fn define_call(code: &str, class_name: &str) -> String {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let start = compact.find(&format!("({{type:{class_name},")).expect("define call");
    let end = compact[start..].find("});").map_or(compact.len(), |i| start + i);
    compact[start..end].to_string()
}

#[test]
fn inputs_declared_in_directive_metadata_are_compiled() {
    // The report used `const bool = (v) => ...`; ngtsc (and oxc) reject a
    // function expression assigned to a variable as a transform, so this uses a
    // function declaration.
    let source = "import { Directive, Input } from '@angular/core';

export function bool(v: unknown): boolean { return v === '' || v === true; }

@Directive({
  selector: 'a',
  inputs: ['x', { name: 'y', alias: 'why', transform: bool }],
})
class A {
  x?: string;
  y?: boolean;
}

@Directive()
abstract class B {
  @Input() p?: string;
  @Input({ alias: 'q-q', transform: bool }) q?: boolean;
}

@Directive({ selector: 'c' })
class C extends B {}
";
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "test.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    // ngtsc: `inputs: { x: "x", y: [2, "why", "y", bool] }`
    assert!(
        define_call(&result.code, "A").contains(r#"inputs:{x:"x",y:[2,"why","y",bool]}"#),
        "{}",
        define_call(&result.code, "A")
    );
    assert!(define_call(&result.code, "B").contains(r#"inputs:{p:"p",q:[2,"q-q","q",bool]}"#));
    assert!(define_call(&result.code, "C").contains("features:[i0.ɵɵInheritDefinitionFeature]"));
}

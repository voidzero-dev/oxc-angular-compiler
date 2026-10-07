//! Spread arguments in decorator metadata must be preserved (issue #511):
//! `f(...P)` is not `f(P)`, and `new Box(...P)` is not `new Box(P)`.
//! Plus the propDecorators shape rule from the same issue: a member whose
//! decorators are all non-Angular still gets an entry (`x: []`).

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

fn compile(source: &str, mode: CompilationMode) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { compilation_mode: mode, ..Default::default() };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    // Whitespace-insensitive comparisons.
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

const SPREAD_SOURCE: &str = "import { Component } from '@angular/core';
const P: any[] = [];
function f(...a: any[]) { return a; }
class Box { constructor(...a: any[]) {} }
@Component({ selector: 'c', template: '', providers: f(...P) })
export class C {}
@Component({
  selector: 'c2',
  template: '',
  providers: [{ provide: 'T', useValue: new Box(...P) }, ...P],
})
export class C2 {}
";

#[test]
fn full_mode_call_spread_preserved() {
    let code = compile(SPREAD_SOURCE, CompilationMode::Full);
    assert!(
        code.contains("ProvidersFeature(f(...P))"),
        "providers call spread must survive, got:\n{code}"
    );
    assert!(
        code.contains(r#"providers:f(...P)"#),
        "setClassMetadata call spread must survive, got:\n{code}"
    );
}

#[test]
fn full_mode_new_spread_preserved() {
    let code = compile(SPREAD_SOURCE, CompilationMode::Full);
    assert!(
        code.contains("useValue:newBox(...P)"),
        "new-expression spread must survive, got:\n{code}"
    );
    assert!(code.contains(",...P]"), "array spread in providers must survive, got:\n{code}");
}

#[test]
fn partial_mode_call_and_new_spread_preserved() {
    let code = compile(SPREAD_SOURCE, CompilationMode::Partial);
    assert!(
        code.contains(r#"providers:f(...P)"#),
        "ngDeclareComponent providers call spread must survive, got:\n{code}"
    );
    assert!(
        code.contains("useValue:newBox(...P)},...P]"),
        "ngDeclareComponent new-expression + array spread must survive, got:\n{code}"
    );
}

/// ngtsc lists a member in `propDecorators` whenever it carries decorators,
/// even when none of them are Angular's — `@Foo() x` emits `x: []`
/// (metadata.ts:107 + decoratedClassMemberToMetadata).
#[test]
fn member_with_only_non_angular_decorators_gets_empty_entry() {
    let source = "import { Component, Input } from '@angular/core';
function Foo(): PropertyDecorator { return () => {}; }
@Component({ selector: 'c', template: '' })
export class C {
  @Foo() x: any;
  @Input() y: any;
}
";
    let code = compile(source, CompilationMode::Full);
    assert!(
        code.contains("x:[]") || code.contains("\"x\":[]"),
        "non-Angular-decorated member must emit an empty propDecorators entry, got:\n{code}"
    );
    assert!(
        code.contains("y:[{type:Input}]") || code.contains("\"y\":[{type:Input}]"),
        "Angular-decorated member must keep its entry, got:\n{code}"
    );
}

/// A member with a non-Angular decorator AND a signal initializer still
/// emits `x: []` — upstream runs the decorated-member branch, not the
/// undecorated-metadata extractor.
#[test]
fn decorated_signal_member_emits_empty_entry() {
    let source = "import { Component, Input, input } from '@angular/core';
function Foo(): PropertyDecorator { return () => {}; }
@Component({ selector: 'c', template: '' })
export class C {
  @Foo() x = input<string>('');
  @Input() y: any;
}
";
    let code = compile(source, CompilationMode::Full);
    assert!(
        code.contains("x:[]") || code.contains("\"x\":[]"),
        "decorated signal member must emit `x: []`, got:\n{code}"
    );
}

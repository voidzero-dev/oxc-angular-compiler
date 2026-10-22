//! In `setClassMetadata`, a signal query's predicate (`contentChildren(Pane)`) is
//! wrapped in `forwardRef(() => Pane)` unless it is a string or a call, as
//! ngtsc's `memberMetadataFromSignalQuery` does: `setClassMetadata` runs as soon
//! as the class is defined, so a class declared later in the file would be read
//! before it is initialized (a TDZ error in development builds). The expected
//! metadata is `@angular/compiler-cli` 21.0.1's for the same classes.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

fn compile(source: &str) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "test.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    result
        .code
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '\'' { '"' } else { c })
        .collect()
}

fn assert_contains(code: &str, expected: &str) {
    assert!(code.contains(expected), "Expected `{expected}` in:\n{code}");
}

#[test]
fn query_of_a_class_declared_later_is_a_forward_ref() {
    let code = compile(
        r"
import { Component, contentChildren, viewChild } from '@angular/core';
@Component({ selector: 'd-parent', template: '<ng-content />' })
export class DParent {
  panes = contentChildren(DPane);
  first = viewChild(DPane);
}
@Component({ selector: 'd-pane', template: '' })
export class DPane {}
",
    );
    assert_contains(
        &code,
        "panes:[{type:i0.ContentChildren,args:[i0.forwardRef(()=>DPane),{isSignal:true}]}]",
    );
    assert_contains(
        &code,
        "first:[{type:i0.ViewChild,args:[i0.forwardRef(()=>DPane),{isSignal:true}]}]",
    );
}

#[test]
fn query_of_a_class_declared_earlier_is_a_forward_ref_too() {
    let code = compile(
        r"
import { Component, ContentChildren, ElementRef, QueryList, ViewChildren, contentChild, viewChildren } from '@angular/core';
@Component({ selector: 'f-pane', template: '' })
export class FPane {}
@Component({ selector: 'f-parent', template: '<ng-content />' })
export class FParent {
  one = contentChild(FPane, { read: ElementRef });
  views = viewChildren('ref');
  @ContentChildren(FPane) legacy!: QueryList<FPane>;
  @ViewChildren('a, b') strings!: QueryList<unknown>;
}
",
    );
    assert_contains(
        &code,
        "one:[{type:i0.ContentChild,args:[i0.forwardRef(()=>FPane),{...{read:ElementRef},isSignal:true}]}]",
    );
    // a string predicate, and decorator queries, stay as written
    assert_contains(&code, r#"views:[{type:i0.ViewChildren,args:["ref",{isSignal:true}]}]"#);
    assert_contains(&code, "legacy:[{type:ContentChildren,args:[FPane]}]");
    assert_contains(&code, r#"strings:[{type:ViewChildren,args:["a,b"]}]"#);
}

#[test]
fn call_predicates_stay_as_written() {
    // ngtsc leaves a call as written, an optional call (`a?.b()`) included, since
    // `ts.isCallExpression` is true for it.
    let code = compile(
        r"
import { Component, viewChild } from '@angular/core';
const tokens: any = { get: () => 'ref' };
@Component({ selector: 'g-cmp', template: '' })
export class GCmp {
  plain = viewChild(tokens.get());
  optional = viewChild(tokens?.get());
  optionalCall = viewChild(tokens.get?.());
}
",
    );
    assert_contains(&code, "plain:[{type:i0.ViewChild,args:[tokens.get(),{isSignal:true}]}]");
    assert_contains(&code, "optional:[{type:i0.ViewChild,args:[tokens?.get(),{isSignal:true}]}]");
    assert_contains(
        &code,
        "optionalCall:[{type:i0.ViewChild,args:[tokens.get?.(),{isSignal:true}]}]",
    );
}

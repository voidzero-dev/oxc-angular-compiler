//! A `${...}` interpolation in an inline template can be a number, a literal or
//! a same-file `const` (`const WIDTH = 150`), as well as a string: ngtsc's
//! partial evaluator folds it as JavaScript converts the number to a string.
//! Before, the component was left uncompiled. The expected `consts` are
//! `@angular/compiler-cli` 21.0.1's for the same components.

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
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn number_consts_in_a_template_literal() {
    let code = compile(
        r#"
import { Component } from '@angular/core';
const WIDTH = 150;
const HEIGHT = 80;
const CLS = 'box';
@Component({ selector: 'e-cmp', template: `<canvas class="${CLS}" style="width: ${WIDTH}px; height: ${HEIGHT}px;"></canvas>` })
export class ECmp {}
"#,
    );
    assert!(
        code.contains(r#"consts:[[1,"box",2,"width","150px","height","80px"]]"#),
        "Got:\n{code}"
    );
}

#[test]
fn chained_and_negative_number_consts_and_number_literals() {
    let code = compile(
        r#"
import { Component } from '@angular/core';
const WIDTH = 150;
const SIZE = `${WIDTH}px`;
const OFFSET = -2;
@Component({ selector: 'e3-cmp', template: `<div style="width: ${SIZE}; margin: ${OFFSET}px; opacity: ${0.5};"></div>` })
export class E3Cmp {}
"#,
    );
    assert!(
        code.contains(r#"consts:[[2,"width","150px","margin","-2px","opacity","0.5"]]"#),
        "Got:\n{code}"
    );
}

#[test]
fn evaluator_only_template_literals_keep_their_fields() {
    // `${W * 2}` (with `W` an alias of another const) folds only through the
    // partial evaluator. ngtsc keeps the fields as written in setClassMetadata
    // and uses the folded selector.
    let code = compile(
        r#"
import { Component } from '@angular/core';
const BASE = 150;
const W = BASE;
@Component({ selector: `x-${W * 2}`, exportAs: `e${+W}`, template: `<i style="width: ${W * 2}px"></i>` })
export class C {}
"#,
    );
    assert!(code.contains(r#"selectors:[["x-300"]]"#), "Got:\n{code}");
    assert!(code.contains(r#"exportAs:["e150"]"#), "Got:\n{code}");
    assert!(code.contains("selector:`x-${"), "Got:\n{code}");
    assert!(code.contains("template:`<istyle=\"width:${"), "Got:\n{code}");
}

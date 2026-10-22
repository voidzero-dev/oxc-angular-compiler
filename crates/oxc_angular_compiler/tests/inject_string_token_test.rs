//! `@Inject('TOKEN')` with a string token injects the string, as ngtsc compiles
//! it: `ɵɵdirectiveInject('TOKEN')` in a component's, directive's or pipe's
//! factory and `ɵɵinject('TOKEN')` in an injectable's, with the parameter's other
//! flags. The expected calls are `@angular/compiler-cli` 21.0.1's output for the
//! same classes.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

/// The factory code for `source`, without whitespace and with `'` as `"`.
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
fn component_factory_injects_the_string() {
    let code = compile(
        r"
import { Component, Inject, Optional, Self } from '@angular/core';
@Component({ selector: 'a-cmp', template: '' })
export class ACmp {
  constructor(@Inject('API_URL') url: string, @Optional() @Self() @Inject('FLAGS') flags: string[]) {}
}
",
    );
    assert_contains(&code, r#"ɵɵdirectiveInject("API_URL"),i0.ɵɵdirectiveInject("FLAGS",10))"#);
    assert!(!code.contains("ɵɵinvalidFactoryDep"), "Got:\n{code}");
}

#[test]
fn injectable_factory_injects_the_string() {
    let code = compile(
        r"
import { Inject, Injectable } from '@angular/core';
@Injectable({ providedIn: 'root' })
export class AService {
  constructor(@Inject('API_URL') url: string) {}
}
",
    );
    assert_contains(&code, r#"(i0.ɵɵinject("API_URL"))"#);
}

#[test]
fn directive_factory_injects_the_string() {
    let code = compile(
        r"
import { Directive, Inject } from '@angular/core';
@Directive({ selector: '[aDir]' })
export class ADir {
  constructor(@Inject('API_URL') url: string) {}
}
",
    );
    assert_contains(&code, r#"(i0.ɵɵdirectiveInject("API_URL"))"#);
}

#[test]
fn pipe_factory_injects_the_string() {
    let code = compile(
        r"
import { Inject, Pipe } from '@angular/core';
@Pipe({ name: 'aPipe' })
export class APipe {
  constructor(@Inject('API_URL') url: string) {}
  transform(v: unknown) { return v; }
}
",
    );
    assert_contains(&code, r#"(i0.ɵɵdirectiveInject("API_URL",16))"#);
}

#[test]
fn component_string_token_forms() {
    // A template literal with no substitutions, parentheses and TypeScript casts are
    // still string tokens.
    let code = compile(
        r"
import { Component, Inject } from '@angular/core';
@Component({ selector: 'a-cmp', template: '' })
export class ACmp {
  constructor(@Inject(`TPL`) a: string, @Inject(('PAREN')) b: string, @Inject('CAST' as any) c: string) {}
}
",
    );
    assert_contains(
        &code,
        r#"(i0.ɵɵdirectiveInject("TPL"),i0.ɵɵdirectiveInject("PAREN"),i0.ɵɵdirectiveInject("CAST"))"#,
    );
}

#[test]
fn component_last_inject_wins() {
    // ngtsc uses the last `@Inject`, so a class token after a string one is not a string.
    let code = compile(
        r"
import { Component, Inject } from '@angular/core';
class Svc {}
@Component({ selector: 'a-cmp', template: '' })
export class ACmp {
  constructor(@Inject('dup') @Inject(Svc) a: any) {}
}
",
    );
    assert_contains(&code, r"(i0.ɵɵdirectiveInject(Svc))");
}

#[test]
fn partial_component_factory_declares_the_string() {
    let allocator = Allocator::default();
    let options = TransformOptions {
        compilation_mode: oxc_angular_compiler::CompilationMode::Partial,
        ..TransformOptions::default()
    };
    let result = transform_angular_file(
        &allocator,
        "test.ts",
        r#"
import { Component, Inject, Optional } from '@angular/core';
@Component({ selector: 'a-cmp', template: '' })
export class ACmp {
  constructor(@Inject('API_URL') url: string, @Optional() @Inject("it's") b: string) {}
}
"#,
        Some(&options),
        None,
    );
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert_contains(&code, r#"deps:[{token:"API_URL"},{token:"it's",optional:true}]"#);
}

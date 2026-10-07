//! Arrow functions copied into decorator metadata must keep parameter
//! defaults, `async`, and rest parameters (issue #512).
//!
//! `convert_arrow_function_expression` can't express these in the output
//! AST, so it emits the source verbatim (types stripped) — the same
//! fallback as destructured params and non-arrow function expressions.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

fn compile(source: &str, mode: CompilationMode) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { compilation_mode: mode, ..Default::default() };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    // Whitespace-insensitive comparisons — the emitters place it differently.
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

const DIRECTIVE: &str = "import { Directive, Input } from '@angular/core';
@Directive({ selector: '[d]' })
export class D {
  @Input({ transform: (v: string = 'a') => v.length }) a = 0;
  @Input({ transform: async (v: any) => 1 }) b: any;
}
";

// `useFactory` arrows can legally use rest params (input transforms cannot —
// both oxc and ngtsc reject a spread first parameter).
const COMPONENT: &str = "import { Component } from '@angular/core';
@Component({
  selector: 'c',
  template: '',
  providers: [
    { provide: 'T', useFactory: async (x: number = 1) => x },
    { provide: 'R', useFactory: (...args: number[]) => args.length },
  ],
})
export class C {}
";

#[test]
fn full_mode_input_transform_keeps_default_value() {
    let code = compile(DIRECTIVE, CompilationMode::Full);
    assert!(
        code.contains(r#""a",(v="a")=>v.length"#),
        "default param must survive in inputs map, got:\n{code}"
    );
    assert!(
        code.contains(r#"{transform:(v="a")=>v.length}"#),
        "default param must survive in setClassMetadata, got:\n{code}"
    );
}

#[test]
fn full_mode_input_transform_keeps_async() {
    let code = compile(DIRECTIVE, CompilationMode::Full);
    assert!(code.contains(r#""b",async(v)=>1"#), "async must survive in inputs map, got:\n{code}");
    assert!(
        code.contains(r#"{transform:async(v)=>1}"#),
        "async must survive in setClassMetadata, got:\n{code}"
    );
}

#[test]
fn full_mode_use_factory_keeps_async_default_and_rest() {
    let code = compile(COMPONENT, CompilationMode::Full);
    assert!(
        code.contains("useFactory:async(x=1)=>x"),
        "useFactory must keep async + default, got:\n{code}"
    );
    assert!(
        code.contains("useFactory:(...args)=>args.length"),
        "rest param must survive, got:\n{code}"
    );
}

#[test]
fn partial_mode_input_transform_keeps_default_and_async() {
    let code = compile(DIRECTIVE, CompilationMode::Partial);
    assert!(
        code.contains(r#""a",(v="a")=>v.length"#),
        "default param must survive in ngDeclareDirective inputs, got:\n{code}"
    );
    assert!(
        code.contains(r#"{transform:async(v)=>1}"#),
        "async must survive in ngDeclareClassMetadata, got:\n{code}"
    );
}

#[test]
fn partial_mode_use_factory_keeps_async_and_default() {
    let code = compile(COMPONENT, CompilationMode::Partial);
    assert!(
        code.contains("useFactory:async(x=1)=>x"),
        "useFactory must keep async + default in ngDeclareComponent, got:\n{code}"
    );
}

/// A defaulted arrow inside `@Inject` on a pipe constructor must survive:
/// the raw-source fallback needs the file's source text, and the emitted
/// factory must carry the token rather than `ɵɵinvalidFactoryDep`.
const PIPE: &str = "import { Inject, Pipe, forwardRef } from '@angular/core';
export class Token {}

@Pipe({ name: 'p' })
export class P {
  constructor(@Inject(forwardRef((x = Token) => x)) t: unknown) {}
}
";

#[test]
fn pipe_inject_arrow_token_full_mode() {
    let code = compile(PIPE, CompilationMode::Full);
    assert!(
        !code.contains("invalidFactoryDep"),
        "factory must not degrade to invalidFactoryDep, got:\n{code}"
    );
    assert!(
        code.contains("(x=Token)=>x"),
        "@Inject arrow token must survive verbatim, got:\n{code}"
    );
}

#[test]
fn pipe_inject_arrow_token_partial_mode() {
    let code = compile(PIPE, CompilationMode::Partial);
    assert!(
        !code.contains("invalidFactoryDep"),
        "factory deps must not degrade to invalidFactoryDep, got:\n{code}"
    );
    assert!(
        code.contains("(x=Token)=>x"),
        "@Inject arrow token must survive verbatim, got:\n{code}"
    );
}

/// Plain arrows keep the structured emit — the fallback only kicks in when
/// the AST can't represent the arrow.
#[test]
fn plain_arrow_still_uses_structured_emit() {
    let allocator = Allocator::default();
    let source = "import { Directive, Input } from '@angular/core';
@Directive({ selector: '[d]' })
export class D {
  @Input({ transform: (v: string) => v.length }) a = 0;
}
";
    let options = TransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors());
    let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        code.contains("(v)=>v.length") && !code.contains("v="),
        "plain arrow should emit structurally, got:\n{}",
        result.code
    );
}

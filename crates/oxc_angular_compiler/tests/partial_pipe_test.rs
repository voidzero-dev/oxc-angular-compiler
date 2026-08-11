//! Tests for the partial-declaration Pipe emitter (`ɵɵngDeclarePipe`).

use oxc_allocator::{Allocator, Box};
use oxc_angular_compiler::{
    CompilationMode, TransformOptions, compile_declare_pipe_from_metadata, link,
    output::ast::{OutputExpression, ReadVarExpr},
    output::emitter::JsEmitter,
    pipe::R3PipeMetadataBuilder,
    transform_angular_file,
};
use oxc_str::Ident;

fn read_var<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        &allocator,
    ))
}

fn emit(expr: &OutputExpression<'_>) -> String {
    JsEmitter::new().emit_expression(expr)
}

#[test]
fn standalone_pure_named_pipe_omits_optional_fields() {
    // Pure (default) + standalone (default true post-v19) — neither field
    // should appear, matching upstream's omit-when-default behavior.
    let allocator = Allocator::default();
    let meta =
        R3PipeMetadataBuilder::new(Ident::from("UpperPipe"), read_var(&allocator, "UpperPipe"))
            .pipe_name(Ident::from("upper"))
            .pure(true)
            .is_standalone(true)
            .build();
    let expr = compile_declare_pipe_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"name:"upper""#), "expected name:\"upper\", got: {js}");
    assert!(!js.contains("isStandalone"), "isStandalone should be omitted when true, got: {js}");
    assert!(!js.contains("pure"), "pure should be omitted when true, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn impure_pipe_emits_pure_false() {
    let allocator = Allocator::default();
    let meta =
        R3PipeMetadataBuilder::new(Ident::from("AsyncPipe"), read_var(&allocator, "AsyncPipe"))
            .pipe_name(Ident::from("async"))
            .pure(false)
            .is_standalone(true)
            .build();
    let expr = compile_declare_pipe_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("pure:false"), "expected pure:false, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn non_standalone_pipe_emits_isstandalone_false() {
    let allocator = Allocator::default();
    let meta =
        R3PipeMetadataBuilder::new(Ident::from("LegacyPipe"), read_var(&allocator, "LegacyPipe"))
            .pipe_name(Ident::from("legacy"))
            .pure(true)
            .is_standalone(false)
            .build();
    let expr = compile_declare_pipe_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("isStandalone:false"), "expected isStandalone:false, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn pipe_uses_pipe_name_over_class_name() {
    // When pipe_name is set, it wins. Mirrors upstream pipe.ts:57:
    // `meta.pipeName ?? meta.name`.
    let allocator = Allocator::default();
    let meta = R3PipeMetadataBuilder::new(
        Ident::from("CurrencyPipe"),
        read_var(&allocator, "CurrencyPipe"),
    )
    .pipe_name(Ident::from("currency"))
    .pure(true)
    .is_standalone(true)
    .build();
    let expr = compile_declare_pipe_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"name:"currency""#), "expected name:\"currency\", got: {js}");
    assert!(!js.contains(r#"name:"CurrencyPipe""#), "should not use class name, got: {js}");
}

#[test]
fn pipe_falls_back_to_class_name_when_pipe_name_missing() {
    let allocator = Allocator::default();
    let meta = R3PipeMetadataBuilder::new(Ident::from("MyPipe"), read_var(&allocator, "MyPipe"))
        .pure(true)
        .is_standalone(true)
        .build();
    let expr = compile_declare_pipe_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"name:"MyPipe""#), "expected name:\"MyPipe\", got: {js}");
}

#[test]
fn field_order_matches_upstream() {
    let allocator = Allocator::default();
    let meta = R3PipeMetadataBuilder::new(Ident::from("Foo"), read_var(&allocator, "Foo"))
        .pipe_name(Ident::from("foo"))
        .pure(false)
        .is_standalone(false)
        .build();
    let expr = compile_declare_pipe_from_metadata(&allocator, &meta);
    let js = emit(&expr);

    let min_idx = js.find("minVersion").unwrap();
    let ver_idx = js.find("version:").unwrap();
    let ng_idx = js.find("ngImport").unwrap();
    let type_idx = js.find("type:").unwrap();
    let is_idx = js.find("isStandalone").unwrap();
    let name_idx = js.find("name:").unwrap();
    let pure_idx = js.find("pure:").unwrap();
    assert!(
        min_idx < ver_idx
            && ver_idx < ng_idx
            && ng_idx < type_idx
            && type_idx < is_idx
            && is_idx < name_idx
            && name_idx < pure_idx,
        "field order should match upstream partial/pipe.ts (minVersion, version, ngImport, type, isStandalone, name, pure), got: {js}"
    );
}

#[test]
fn partial_pipe_e2e_via_transform_angular_file() {
    // Keep the source TS-syntax-free in the class body (no implements, no
    // typed methods) — transform_angular_file does Angular decoration but
    // doesn't strip TS, and we want the output to be linker-parseable.
    let allocator = Allocator::default();
    let source = "import { Pipe } from '@angular/core';

@Pipe({ name: 'reverse', pure: true, standalone: true })
export class ReversePipe {}
";

    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "reverse.pipe.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let code = &result.code;

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclarePipe"),
        "expected ɵɵngDeclarePipe in partial output, got:\n{code}"
    );
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareFactory"),
        "expected ɵɵngDeclareFactory in partial output, got:\n{code}"
    );
    assert!(
        !code.contains("\u{0275}\u{0275}definePipe"),
        "ɵɵdefinePipe should not appear in partial mode, got:\n{code}"
    );
    assert!(code.contains(r#"name:"reverse""#), "expected name:\"reverse\", got:\n{code}");

    // Round-trip through linker — partial pipe + factory should expand back
    // into a working full ɵɵdefinePipe form.
    let linked = link(&allocator, code, "reverse.pipe.mjs");
    assert!(linked.linked, "linker should accept the partial pipe output. emitted code:\n{code}");
    assert!(
        linked.code.contains("\u{0275}\u{0275}definePipe"),
        "linked output should contain ɵɵdefinePipe, got:\n{}",
        linked.code
    );
    assert!(
        !linked.code.contains("\u{0275}\u{0275}ngDeclare"),
        "linked output should have no ngDeclare* calls left, got:\n{}",
        linked.code
    );
}

#[test]
fn full_mode_pipe_unchanged() {
    // Sanity: default mode (Full) still emits ɵɵdefinePipe.
    let allocator = Allocator::default();
    let source = "import { Pipe } from '@angular/core';

@Pipe({ name: 'reverse', standalone: true })
export class ReversePipe {}
";

    let result = transform_angular_file(&allocator, "reverse.pipe.ts", source, None, None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);

    assert!(
        result.code.contains("\u{0275}\u{0275}definePipe"),
        "default-mode (Full) should still emit ɵɵdefinePipe, got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("\u{0275}\u{0275}ngDeclarePipe"),
        "Full mode must not leak ngDeclare calls, got:\n{}",
        result.code
    );
}

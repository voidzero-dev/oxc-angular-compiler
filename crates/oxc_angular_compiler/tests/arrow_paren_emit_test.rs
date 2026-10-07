//! Arrow bodies whose leftmost token is `{` must keep their parentheses
//! (issue #510): `(v) => ({value: v}).value` must not emit as
//! `(v) => {value:v}.value` — invalid JS. ngtsc keeps the source text via
//! `WrappedNodeExpr`; our converter rebuilds the AST, so the emitter walks
//! the body's leftmost token (`emits_leading_brace`).

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    CompilationMode, TransformOptions,
    output::emitter::JsEmitter,
    parser::html::HtmlParser,
    pipeline::{emit::compile_template, ingest::ingest_component},
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions as HtmlTransformOptions},
    transform_angular_file,
};
use oxc_str::Ident;

fn compile(source: &str) -> String {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Full, ..Default::default() };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Compile a template through the IR pipeline (ingest → phases → emit) so the
/// strip_nonrequired_parentheses phase and the emitter are both exercised.
fn compile_tpl(template: &str) -> String {
    let allocator = Allocator::default();
    let parser = HtmlParser::with_expansion_forms(&allocator, template, "test.html");
    let html_result = parser.parse();
    assert!(html_result.errors.is_empty());
    let transformer = HtmlToR3Transform::new(
        &allocator,
        template,
        HtmlTransformOptions { angular_version: None, ..HtmlTransformOptions::default() },
    );
    let r3_result = transformer.transform(&html_result.nodes);
    assert!(r3_result.errors.is_empty());
    let mut job = ingest_component(&allocator, Ident::from("TestComponent"), r3_result.nodes);
    let result = compile_template(&mut job);
    let emitter = JsEmitter::new();
    let mut output = String::new();
    for decl in &result.declarations {
        output.push_str(&emitter.emit_statement(decl));
    }
    output.push_str(&emitter.emit_statements(&result.template_fn.statements));
    output.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The issue's `@Input transform` repro: member access on a parenthesized
/// object literal inside an arrow body.
#[test]
fn input_transform_paren_object_member() {
    let code = compile(
        "import {Directive, Input} from '@angular/core';
@Directive({selector: '[d]'})
export class D {
  @Input({transform: (v: any) => ({value: v}).value}) a: any;
}
",
    );
    assert!(
        code.contains("transform:(v)=>({value:v}).value"),
        "parens around the object must be kept, got:\n{code}"
    );
    assert!(
        !code.contains("=>({value:v}") || code.contains("=>({value:v}).value"),
        "inner object must stay parenthesized, got:\n{code}"
    );
}

/// The issue's provider-factory repro.
#[test]
fn provider_factory_paren_object_member() {
    let code = compile(
        "import {Component} from '@angular/core';
@Component({selector: 'c', template: '', providers: [{provide: 'T', useFactory: () => ({a: 1}).a}]})
export class C {}
",
    );
    assert!(
        code.contains("useFactory:()=>({a:1}).a"),
        "parens around the object must be kept, got:\n{code}"
    );
}

/// `() => ({...})` (issue #43) still emits the object parens exactly once.
#[test]
fn plain_object_arrow_body_still_parenthesized() {
    let code = compile(
        "import {Component} from '@angular/core';
@Component({selector: 'c', template: '', providers: [{provide: 'T', useFactory: () => ({a: 1})}]})
export class C {}
",
    );
    assert!(
        code.contains("useFactory:()=>({a:1})"),
        "object-literal arrow body must stay parenthesized, got:\n{code}"
    );
    assert!(!code.contains("=>({a:1}))"), "no double parens, got:\n{code}");
    assert!(!code.contains("=>(({a:1})"), "no double parens, got:\n{code}");
}

/// Deeper chains: call on a parenthesized object, nested member access, and
/// source parens on a plain identifier (kept, as ngtsc emits them verbatim).
#[test]
fn chained_and_unneeded_parens() {
    let code = compile(
        "import {Component} from '@angular/core';
@Component({selector: 'c', template: '', providers: [
  {provide: 'A', useFactory: () => ({f: () => 1}).f()},
  {provide: 'B', useFactory: () => ({o: {x: 2}}).o.x},
  {provide: 'C', useFactory: (v: any) => (v)},
]})
export class C {}
",
    );
    assert!(code.contains("useFactory:()=>({f:()=>1}).f()"), "call on object, got:\n{code}");
    assert!(
        code.contains("useFactory:()=>({o:{x:2}}).o.x"),
        "nested member on object, got:\n{code}"
    );
    assert!(code.contains("useFactory:(v)=>(v)"), "source parens kept verbatim, got:\n{code}");
}

/// Source parens that JavaScript requires for reasons beyond a leading `{`
/// must survive too (Codex review): unary in an exponentiation base, a
/// number-literal member access, an optional-chain callee, and an arrow
/// invoked immediately.
#[test]
fn other_required_source_parens_survive() {
    let code = compile(
        "import {Component} from '@angular/core';
@Component({selector: 'c', template: '', providers: [
  {provide: 'A', useFactory: () => (-1) ** 2},
  {provide: 'B', useFactory: () => (1).toString()},
  {provide: 'C', useFactory: (o: any) => (o?.m)()},
]})
export class C {}
",
    );
    // BinaryOperator wraps itself in parens in our emit style.
    assert!(code.contains("()=>((-1)**2)"), "unary base of ** needs parens, got:\n{code}");
    assert!(
        code.contains("()=>(1).toString()"),
        "number-literal member access needs parens, got:\n{code}"
    );
    assert!(code.contains("(o)=>(o?.m)()"), "optional-chain call needs parens, got:\n{code}");
}

/// Partial mode emits the same arrow in `setClassMetadata`/`ngDeclareComponent`.
#[test]
fn partial_mode_paren_object_member() {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result = transform_angular_file(
        &allocator,
        "test.ts",
        "import {Directive, Input} from '@angular/core';
@Directive({selector: '[d]'})
export class D {
  @Input({transform: (v: any) => ({value: v}).value}) a: any;
}
",
        Some(&options),
        None,
    );
    assert!(!result.has_errors(), "got: {:?}", result.diagnostics);
    let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        code.contains("transform:(v)=>({value:v}).value"),
        "partial-mode metadata must keep parens, got:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Template pipeline: strip_nonrequired_parentheses must drop source parens
// except the ones JavaScript grammar requires — the emitter now prints any
// Parenthesized that survives, so a missed strip or a missed required mark
// both show in the output.
// ---------------------------------------------------------------------------

/// Every unary-precedence operator is a forbidden `**` base — `!x ** 2`,
/// `typeof x ** 2`, `void x ** 2` are SyntaxErrors, like `-x ** 2`. Upstream's
/// strip phase only keeps `UnaryOperatorExpr` parens because its emit goes
/// through the TS printer; we emit JS directly, so all four must survive.
#[test]
fn template_unary_bases_of_exponentiation_keep_parens() {
    for (tpl, needle) in [
        (r"<div>{{ (-1) ** 2 }}</div>", "((-1)**2)"),
        (r"<div>{{ (!a) ** 2 }}</div>", "((!ctx.a)**2)"),
        (r"<div>{{ (typeof v) ** 2 }}</div>", "((typeofctx.v)**2)"),
        (r"<div>{{ (void 0) ** 2 }}</div>", "((void0)**2)"),
        // Nested parens: the inner wrapper is required and survives the strip.
        (r"<div>{{ ((-1)) ** 2 }}</div>", "((-1)**2)"),
    ] {
        let code = compile_tpl(tpl);
        assert!(code.contains(needle), "{tpl} emitted:\n{code}");
    }
}

/// Non-required parens in templates strip — `(x)`, `(pipe) || b`, and a
/// parenthesized safe-read receiver emit without them (upstream parity).
#[test]
fn template_nonrequired_parens_strip() {
    let code = compile_tpl(r"<div>{{ (x) }}</div>");
    assert!(code.contains("Interpolate(ctx.x)"), "got:\n{code}");

    let code = compile_tpl(r"<div>{{ ((data$ | async) || fb)?.name }}</div>");
    assert!(
        code.contains("(i0.ɵɵpipeBind1(2,1,ctx.data$)||ctx.fb)"),
        "pipe parens must strip, got:\n{code}"
    );
    assert!(
        !code.contains("(i0.ɵɵpipeBind1(2,1,ctx.data$))||"),
        "no extra parens around the pipe, got:\n{code}"
    );
}

/// `??`/`&&`/`||`/`?:` mixes keep their required parens (TS-compat hazards).
#[test]
fn template_nullish_and_logical_parens_survive() {
    let code = compile_tpl(r"<div>{{ (a ?? b) && c }}</div>");
    assert!(code.contains("((ctx.a??ctx.b)&&ctx.c)"), "got:\n{code}");

    let code = compile_tpl(r"<div>{{ (a ? b : c) ?? d }}</div>");
    assert!(code.contains("??ctx.d"), "got:\n{code}");
    assert!(code.contains("((ctx.a?ctx.b:ctx.c)??ctx.d)"), "ternary ?? parens, got:\n{code}");
}

//! A name read inside a template arrow function resolves to the arrow's own
//! parameter (or an enclosing arrow's) before template variables and the
//! component context: `open.update((o) => !o)` must not emit `(o) => !ctx.o`.
//!
//! Arrows in listeners stay in place; everywhere else they are hoisted to a
//! shared factory and instantiated through `ɵɵarrowFunction`.
//!
//! Ported from Angular's `updateParameterReferences` in
//! `template/pipeline/src/ingest.ts` and its `generateArrowFunctions` phase.
//! The expected strings are ngtsc's output for the same templates, in this
//! emitter's spelling (parenthesized params and binary operands).

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

/// Compile a template through the IR pipeline, with whitespace removed.
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

#[track_caller]
fn assert_emits(template: &str, expected: &str) {
    let code = compile_tpl(template);
    assert!(code.contains(expected), "expected `{expected}` for `{template}`, got:\n{code}");
}

#[test]
fn listener_arrow_reads_its_parameter() {
    assert_emits(
        r#"<pressable (press)="open.update((o) => !o)"></pressable>"#,
        "returnctx.open.update((o)=>!o);",
    );
}

#[test]
fn listener_arrow_without_parameters_reads_the_component() {
    assert_emits(
        r#"<pressable (press)="open.update(() => !open())"></pressable>"#,
        "returnctx.open.update(()=>!ctx.open());",
    );
}

#[test]
fn listener_arrow_reads_several_parameters() {
    assert_emits(
        r#"<button (click)="run((a, b) => a + b)"></button>"#,
        "returnctx.run((a,b)=>(a+b));",
    );
}

#[test]
fn parameter_wins_over_a_component_property_of_the_same_name() {
    assert_emits(
        r#"<button (click)="open.update((open) => !open)"></button>"#,
        "returnctx.open.update((open)=>!open);",
    );
}

#[test]
fn listener_arrow_reads_a_parameter_and_a_component_property() {
    assert_emits(
        r#"<button (click)="open.update((o) => o || fallback())"></button>"#,
        "returnctx.open.update((o)=>(o||ctx.fallback()));",
    );
}

#[test]
fn nested_arrow_reads_the_outer_parameter() {
    assert_emits(
        r#"<button (click)="run((a) => (b) => a + b + other)"></button>"#,
        "returnctx.run((a)=>(b)=>((a+b)+ctx.other));",
    );
}

#[test]
fn parameter_shadows_a_let_declaration() {
    assert_emits(
        r#"@let v = 1; <button (click)="run((v) => v + other)"></button>{{ v }}"#,
        "returnctx.run((v)=>(v+ctx.other));",
    );
}

#[test]
fn parameter_shadows_a_for_item() {
    assert_emits(
        r#"@for (item of items; track item) { <button (click)="run((item) => item + other)"></button> }"#,
        "i0.ɵɵrestoreView(_r1);constctx_r1=i0.ɵɵnextContext();\
         returni0.ɵɵresetView(ctx_r1.run((item)=>(item+ctx_r1.other)));",
    );
}

#[test]
fn parameter_shadows_a_template_reference() {
    assert_emits(
        r#"<input #el /><button (click)="run((el) => el + other)"></button>"#,
        "returnctx.run((el)=>(el+ctx.other));",
    );
}

#[test]
fn listener_arrow_reads_template_variables_that_are_not_parameters() {
    assert_emits(
        r#"<input #el />@for (item of items; track item) { <button (click)="run((x) => x + item + el.value + other)"></button> }"#,
        "constitem_r2=i0.ɵɵrestoreView(_r1).$implicit;constctx_r2=i0.ɵɵnextContext();\
         constel_r4=i0.ɵɵreference(1);\
         returni0.ɵɵresetView(ctx_r2.run((x)=>(((x+item_r2)+el_r4.value)+ctx_r2.other)));",
    );
}

#[test]
fn listener_arrow_reads_dollar_event() {
    assert_emits(
        r#"<button (click)="run((x) => x + $event.type)"></button>"#,
        "listener($event){returnctx.run((x)=>(x+$event.type));}",
    );
}

/// ngtsc adds a nested arrow's parameters to the enclosing arrow's set once it has
/// visited that nested arrow, so a later read of the same name is left as a plain
/// variable while an earlier one still reads the component.
#[test]
fn nested_arrow_parameters_leak_to_later_reads_like_ngtsc() {
    assert_emits(
        r#"<button (click)="run((a) => f((b) => b) + b)"></button>"#,
        "returnctx.run((a)=>(ctx.f((b)=>b)+b));",
    );
    assert_emits(
        r#"<button (click)="run((a) => b + f((b) => b))"></button>"#,
        "returnctx.run((a)=>(ctx.b+ctx.f((b)=>b)));",
    );
}

// Outside listeners the arrow is hoisted to a shared factory, `(ctx, view) => arrow`,
// and instantiated with `ɵɵarrowFunction(varOffset, factory, ctx)`.

#[track_caller]
fn assert_hoists(template: &str, factory: &str, usage: &str) {
    let code = compile_tpl(template);
    assert!(code.contains(factory), "expected `{factory}` for `{template}`, got:\n{code}");
    assert!(code.contains(usage), "expected `{usage}` for `{template}`, got:\n{code}");
}

#[test]
fn property_binding_arrow_reads_its_parameter() {
    assert_hoists(
        r#"<div [title]="run((o) => !o)"></div>"#,
        "constarrowFn0=(ctx,view)=>(o)=>!o;",
        r#"i0.ɵɵproperty("title",ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
    assert_hoists(
        r#"<div [title]="run((open) => !open)"></div>"#,
        "constarrowFn0=(ctx,view)=>(open)=>!open;",
        r#"i0.ɵɵproperty("title",ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
}

#[test]
fn property_binding_arrow_without_parameters_reads_the_component() {
    assert_hoists(
        r#"<div [title]="run(() => !open())"></div>"#,
        "constarrowFn0=(ctx,view)=>()=>!ctx.open();",
        r#"i0.ɵɵproperty("title",ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
}

/// Only the outermost arrow is hoisted; the arrow it returns stays in place.
#[test]
fn nested_arrow_stays_inside_the_hoisted_arrow() {
    assert_hoists(
        r#"<div [title]="run((a) => (b) => a + b + other)"></div>"#,
        "constarrowFn0=(ctx,view)=>(a)=>(b)=>((a+b)+ctx.other);",
        r#"i0.ɵɵproperty("title",ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
}

#[test]
fn property_binding_parameter_shadows_template_variables() {
    assert_hoists(
        r#"@let v = 1; <div [title]="run((v) => v + other)"></div>{{ v }}"#,
        "constarrowFn0=(ctx,view)=>(v)=>(v+ctx.other);",
        r#"i0.ɵɵproperty("title",ctx.run(i0.ɵɵarrowFunction(2,arrowFn0,ctx)));"#,
    );
    assert_hoists(
        r#"<input #el /><div [title]="run((el) => el + other)"></div>"#,
        "constarrowFn0=(ctx,view)=>(el)=>(el+ctx.other);",
        r#"i0.ɵɵproperty("title",ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
}

/// In an embedded view the hoisted arrow restores the view it was created in, then
/// walks to the component context itself.
#[test]
fn hoisted_arrow_in_an_embedded_view_restores_the_view() {
    assert_hoists(
        r#"@for (item of items; track item) {<div [title]="run((item) => item + other)"></div>}"#,
        "constarrowFn0=(ctx,view)=>(item)=>{i0.ɵɵrestoreView(view);\
         constctx_r0=i0.ɵɵnextContext();returni0.ɵɵresetView((item+ctx_r0.other));};",
        r#"constctx_r0=i0.ɵɵnextContext();i0.ɵɵproperty("title",ctx_r0.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
}

/// Angular's `reify` pools a `@for`'s `_forTrack` with its create operation, before the
/// arrow functions of the update operations, so ngtsc declares `_forTrack0` first.
#[test]
fn track_function_is_pooled_before_hoisted_arrows() {
    let code = compile_tpl(
        r#"<div [title]="run((a) => a + n)"></div>@for (item of items; track item.id + n) {<i [title]="run((b) => b + item.id)"></i>}"#,
    );
    let track = code.find("function_forTrack0").expect(&code);
    let arrow = code.find("constarrowFn0=").expect(&code);
    assert!(track < arrow, "expected `_forTrack0` before `arrowFn0`, got:\n{code}");
    assert!(code.find("constarrowFn1=").is_some_and(|i| arrow < i), "got:\n{code}");
}

#[test]
fn hoisted_arrow_reads_template_variables_that_are_not_parameters() {
    assert_hoists(
        r#"<input #el />@for (item of items; track item) {<div [title]="run((x) => x + item + el.value + other)"></div>}"#,
        "constarrowFn0=(ctx,view)=>(x)=>{constitem_r1=i0.ɵɵrestoreView(view).$implicit;\
         constctx_r1=i0.ɵɵnextContext();constel_r3=i0.ɵɵreference(1);\
         returni0.ɵɵresetView((((x+item_r1)+el_r3.value)+ctx_r1.other));};",
        r#"constctx_r1=i0.ɵɵnextContext();i0.ɵɵproperty("title",ctx_r1.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));"#,
    );
}

#[test]
fn interpolation_arrow_reads_its_parameters() {
    assert_hoists(
        "{{ run((a, b) => a + b) }}",
        "constarrowFn0=(ctx,view)=>(a,b)=>(a+b);",
        "i0.ɵɵtextInterpolate(ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));",
    );
    assert_hoists(
        "{{ run((o) => o || fallback()) }}",
        "constarrowFn0=(ctx,view)=>(o)=>(o||ctx.fallback());",
        "i0.ɵɵtextInterpolate(ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));",
    );
    assert_hoists(
        "{{ run(() => !open()) }}",
        "constarrowFn0=(ctx,view)=>()=>!ctx.open();",
        "i0.ɵɵtextInterpolate(ctx.run(i0.ɵɵarrowFunction(1,arrowFn0,ctx)));",
    );
}

/// Equivalent arrows share one factory but each gets its own var slot.
#[test]
fn identical_arrows_share_a_factory() {
    let code = compile_tpl("{{ run((o) => !o) }}{{ run((o) => !o) }}");
    assert_eq!(code.matches("constarrowFn").count(), 1, "got:\n{code}");
    assert!(code.contains("i0.ɵɵarrowFunction(2,arrowFn0,ctx)"), "got:\n{code}");
    assert!(code.contains("i0.ɵɵarrowFunction(3,arrowFn0,ctx)"), "got:\n{code}");
}

/// ngtsc hoists an arrow in a `track` expression into the track function, where `ctx`
/// is not in scope, so that call throws. The arrow is kept in place instead.
#[test]
fn track_expression_arrow_stays_in_place() {
    let code = compile_tpl("@for (item of items; track run((a) => a)) {<i></i>}");
    assert!(code.contains("returnthis.run((a)=>a);"), "got:\n{code}");
    assert!(!code.contains("arrowFn"), "got:\n{code}");
}

/// An `@let` that the optimizer turns into a plain statement still has its arrow
/// function named, and the arrow keeps its `restoreView` statement.
#[test]
fn let_arrows_reading_other_lets() {
    let code = compile_tpl("@let a = other; @let f = () => a; @let g = () => f() + a; {{ g() }}");
    assert!(
        code.contains(
            "constarrowFn0=(ctx,view)=>()=>{i0.ɵɵrestoreView(view);\
             consta_r1=i0.ɵɵreadContextLet(0);returni0.ɵɵresetView(a_r1);};"
        ),
        "got:\n{code}"
    );
    assert!(
        code.contains(
            "constarrowFn1=(ctx,view)=>()=>{i0.ɵɵrestoreView(view);\
             consta_r2=i0.ɵɵreadContextLet(0);constf_r3=i0.ɵɵreadContextLet(2);\
             returni0.ɵɵresetView((f_r3()+a_r2));};"
        ),
        "got:\n{code}"
    );
}

#[test]
fn let_declaration_arrow_is_hoisted() {
    assert_hoists(
        "@let fn = (a) => a + other; {{ fn(1) }}",
        "constarrowFn0=(ctx,view)=>(a)=>(a+ctx.other);",
        "constfn_r1=i0.ɵɵarrowFunction(1,arrowFn0,ctx);",
    );
}

#[test]
fn host_listener_arrow_reads_its_parameter() {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Full, ..Default::default() };
    let source = "import {Directive, signal} from '@angular/core';
@Directive({
  selector: '[d]',
  host: {
    '(click)': 'someSignal.update(prev => prev + 1)',
    '(mousedown)': 'someSignal.update(() => componentProp + 1)',
  },
})
export class D {
  someSignal = signal(0);
  componentProp = 1;
}
";
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(code.contains("returnctx.someSignal.update((prev)=>(prev+1));"), "got:\n{code}");
    assert!(
        code.contains("returnctx.someSignal.update(()=>(ctx.componentProp+1));"),
        "got:\n{code}"
    );
}

#[test]
fn directive_host_binding_arrow_is_hoisted() {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Full, ..Default::default() };
    let source = "import {Directive} from '@angular/core';
@Directive({
  selector: '[d]',
  host: {'[attr.with-context]': '((a, b) => a / b + componentProp)(6, 12)'},
})
export class D {
  componentProp = 1;
}
";
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        code.contains("constarrowFn0=(ctx,view)=>(a,b)=>((a/b)+ctx.componentProp);"),
        "got:\n{code}"
    );
    assert!(
        code.contains(
            r#"i0.ɵɵattribute("with-context",i0.ɵɵarrowFunction(1,arrowFn0,ctx)(6,12));"#
        ),
        "got:\n{code}"
    );
    assert!(code.contains("hostVars:2"), "got:\n{code}");
}

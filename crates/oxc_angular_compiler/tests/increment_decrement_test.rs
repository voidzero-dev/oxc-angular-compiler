//! `++` / `--` in event listeners (Angular 22.3, angular/angular#700 upstream
//! port). Every expected string is the output of `@angular/compiler-cli`
//! 22.3.0-next.4 (`ngc`), compared without whitespace.
//!
//! Our emitter wraps every binary expression in parentheses, which ngc's
//! TypeScript printer does not; the few cases below that contain a binary
//! operator are written in that form and say so.

use oxc_allocator::Allocator;
use oxc_angular_compiler::linker::{LinkOptions, link_with_options};
use oxc_angular_compiler::{
    AngularVersion, CompilationMode, TransformOptions, transform_angular_file,
};

fn version() -> AngularVersion {
    AngularVersion::new(22, 3, 0)
}

fn strip(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile_mode(source: &str, mode: CompilationMode) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions {
        compilation_mode: mode,
        angular_version: Some(version()),
        ..Default::default()
    };
    let result = transform_angular_file(&allocator, "app.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    result.code
}

fn compile(source: &str) -> String {
    strip(&compile_mode(source, CompilationMode::Full))
}

fn link(code: &str) -> String {
    let allocator = Allocator::default();
    let options = LinkOptions { angular_version: Some(version()), ..Default::default() };
    let result = link_with_options(&allocator, code, "app.mjs", &options);
    assert!(result.linked, "Expected the linker to link:\n{code}");
    strip(&result.code)
}

/// The diagnostics of compiling `source`.
fn diagnostics(source: &str) -> Vec<String> {
    let allocator = Allocator::default();
    let options = TransformOptions { angular_version: Some(version()), ..Default::default() };
    let result = transform_angular_file(&allocator, "app.ts", source, Some(&options), None);
    result.diagnostics.iter().map(ToString::to_string).collect()
}

fn assert_contains(code: &str, expected: &str) {
    let expected = strip(expected);
    assert!(code.contains(&expected), "Expected `{expected}` in:\n{code}");
}

fn component(template: &str) -> String {
    format!(
        "import {{Component}} from '@angular/core';
@Component({{selector: 'app', template: `{template}`}})
export class App {{ count = 0; other = {{n: 1}}; items = [1]; maybe: any = {{n: 1}}; x = 0; handler: any; fn(a: any, b: any) {{}} }}"
    )
}

/// The body of the listener of the only button in `template`.
fn listener(template: &str, body: &str) {
    let code = compile(&component(template));
    assert_contains(&code, &format!("_listener(){{{body}}});"));
}

#[test]
fn sanity_listener_statements() {
    let code = compile(&component(r#"<button (click)="count++; --other.n; items[0]++"></button>"#));
    assert_contains(
        &code,
        r#"i0.ɵɵdomListener("click", function App_Template_button_click_0_listener() { ctx.count++; --ctx.other.n; return ctx.items[0]++; });"#,
    );
}

/// `r3_view_compiler/operators.ts` from the compliance tests.
#[test]
fn compliance_operators() {
    let source = r#"import { Component, Pipe } from '@angular/core';

@Pipe({name: 'identity'})
export class IdentityPipe {
  transform(value: any) {
    return value;
  }
}

export class Bar {}

@Component({
  template: `
    {{ 1 + 2 }}
    {{ (1 % 2) + 3 / 4 * 5 ** 6 }}
    {{ +1 }}
    {{ typeof {} === 'object' }}
    {{ !(typeof {} === 'object') }}
    {{ typeof foo?.bar === 'string' }}
    {{ typeof foo?.bar | identity }}
    {{ void 'test' }}
    {{ (-1) ** 3 }}
    {{ 'bar' in foo }}
    {{ bar instanceof Bar }}
    <button (click)="number += 1"></button>
    <button (click)="number -= 1"></button>
    <button (click)="number *= 1"></button>
    <button (click)="number /= 1"></button>
    <button (click)="number %= 1"></button>
    <button (click)="number **= 1"></button>
    <button (click)="number &&= 1"></button>
    <button (click)="number ||= 1"></button>
    <button (click)="number ??= 1"></button>
    <button (click)="number++"></button>
    <button (click)="number--"></button>
    <button (click)="++number"></button>
    <button (click)="--number"></button>
  `,
  imports: [IdentityPipe],
})
export class MyApp {
  foo: {bar?: string} = {bar: 'baz'};
  number = 1;
  bar = new Bar();
  Bar = Bar;
}
"#;
    let code = compile(source);
    assert_contains(&code, r#"decls: 15, vars: 15, consts: [[3, "click"]]"#);
    for (slot, body) in
        [(11, "ctx.number++"), (12, "ctx.number--"), (13, "++ctx.number"), (14, "--ctx.number")]
    {
        assert_contains(
            &code,
            &format!("function MyApp_Template_button_click_{slot}_listener() {{ return {body}; }}"),
        );
    }
}

#[test]
fn this_non_null_and_parenthesized_targets() {
    listener(r#"<button (click)="this.count++"></button>"#, "return ctx.count++;");
    listener(r#"<button (click)="++this.count"></button>"#, "return ++ctx.count;");
    listener(r#"<button (click)="maybe!.n++"></button>"#, "return ctx.maybe.n++;");
    listener(r#"<button (click)="items[0]!++"></button>"#, "return ctx.items[0]++;");
    listener(r#"<button (click)="(count)++"></button>"#, "return ctx.count++;");
    listener(r#"<button (click)="(count!)--"></button>"#, "return ctx.count--;");
    listener(r#"<button (click)="--(count)"></button>"#, "return --ctx.count;");
    listener(r#"<button (click)="(count++)"></button>"#, "return ctx.count++;");
}

#[test]
fn precedence() {
    listener(r#"<button (click)="-count++"></button>"#, "return -ctx.count++;");
    listener(r#"<button (click)="-(count++)"></button>"#, "return -ctx.count++;");
    listener(r#"<button (click)="!count++"></button>"#, "return !ctx.count++;");
    listener(r#"<button (click)="typeof count++"></button>"#, "return typeof ctx.count++;");
    listener(r#"<button (click)="void count--"></button>"#, "return void ctx.count--;");
    // Adjacent operators keep a space, like TypeScript's printer.
    listener(r#"<button (click)="- --count"></button>"#, "return - --ctx.count;");
    listener(r#"<button (click)="+ ++count"></button>"#, "return + ++ctx.count;");
    listener(r#"<button (click)="-(-count)"></button>"#, "return - -ctx.count;");
    listener(r#"<button (click)="- -count"></button>"#, "return - -ctx.count;");
    // A unary operand of a member access keeps its parentheses.
    listener(
        r#"<button (click)="(count++).toString()"></button>"#,
        "return (ctx.count++).toString();",
    );
    listener(
        r#"<button (click)="(++count).toFixed()"></button>"#,
        "return (++ctx.count).toFixed();",
    );
    listener(r#"<button (click)="(-count).toFixed()"></button>"#, "return (-ctx.count).toFixed();");
}

#[test]
fn update_expressions_as_operands() {
    listener(r#"<button (click)="items[count++]++"></button>"#, "return ctx.items[ctx.count++]++;");
    listener(
        r#"<button (click)="fn(count++, --other.n)"></button>"#,
        "return ctx.fn(ctx.count++, --ctx.other.n);",
    );
    let code = compile(&component(r#"<button (click)="$event.detail++"></button>"#));
    assert_contains(&code, "_listener($event) { return $event.detail++; });");
    // ngc: `return ctx.count++ + ctx.other.n;` (see the module docs on binary parentheses).
    listener(
        r#"<button (click)="count+++other.n"></button>"#,
        "return (ctx.count++ + ctx.other.n);",
    );
    // ngc: `return ctx.x = ctx.count++;`
    listener(r#"<button (click)="x = count++"></button>"#, "return (ctx.x = ctx.count++);");
    // ngc: `return (ctx.count++) ** 2;`
    listener(r#"<button (click)="(count++) ** 2"></button>"#, "return ((ctx.count++) ** 2);");
    // ngc: `return ctx.handler = () => ctx.count++;`
    listener(
        r#"<button (click)="handler = () => count++"></button>"#,
        "return (ctx.handler = () => ctx.count++);",
    );
}

#[test]
fn embedded_view_targets() {
    let code = compile(&component(
        r#"<ng-template let-item><button (click)="item.n++; count++"></button></ng-template>
@for (row of items; track $index) {<button (click)="row.n++; count--"></button>}
@if (other; as o) {<button (click)="o.n++"></button>}"#,
    ));
    assert_contains(
        &code,
        "function App_ng_template_0_Template_button_click_0_listener() { const item_r2 = i0.ɵɵrestoreView(_r1).$implicit; const ctx_r2 = i0.ɵɵnextContext(); item_r2.n++; return i0.ɵɵresetView(ctx_r2.count++); }",
    );
    assert_contains(
        &code,
        "function App_For_2_Template_button_click_0_listener() { const row_r5 = i0.ɵɵrestoreView(_r4).$implicit; const ctx_r2 = i0.ɵɵnextContext(); row_r5.n++; return i0.ɵɵresetView(ctx_r2.count--); }",
    );
    assert_contains(
        &code,
        "function App_Conditional_3_Template_button_click_0_listener() { const o_r7 = i0.ɵɵrestoreView(_r6); return i0.ɵɵresetView(o_r7.n++); }",
    );
}

#[test]
fn host_listeners() {
    let code = compile(
        "import {Component} from '@angular/core';
@Component({selector: 'app', template: '', host: {'(click)': 'n++', '(keydown)': '--n; other.n++'}})
export class App { n = 0; other = {n: 1}; }",
    );
    assert_contains(
        &code,
        r#"i0.ɵɵlistener("click", function App_click_HostBindingHandler() { return ctx.n++; })("keydown", function App_keydown_HostBindingHandler() { --ctx.n; return ctx.other.n++; });"#,
    );
}

#[test]
fn arrow_functions_in_bindings() {
    let code = compile(
        "import {Component, Directive, Input} from '@angular/core';
@Directive({selector: '[takesFn]'})
export class TakesFn { @Input() takesFn: any; }
@Component({selector: 'app', imports: [TakesFn], template: `<div [takesFn]=\"() => a++\"></div>{{ (() => a++)() }}`})
export class App { a = 0; }",
    );
    assert_contains(&code, "const arrowFn0 = (ctx, view) => () => ctx.a++;");
    assert_contains(&code, r#"i0.ɵɵproperty("takesFn", i0.ɵɵarrowFunction(2, arrowFn0, ctx));"#);
    assert_contains(&code, "i0.ɵɵtextInterpolate(i0.ɵɵarrowFunction(3, arrowFn0, ctx)());");
}

#[test]
fn optional_chain_receiver() {
    // ngc emits these as written (they are not valid JavaScript targets either).
    listener(r#"<button (click)="maybe?.n.x++"></button>"#, "return ctx.maybe?.n.x++;");
}

#[test]
fn binding_errors() {
    // ngc: `Parser Error: Bindings cannot contain assignments at the end of the expression [a++]`
    for template in [r#"<div [title]="count++"></div>"#, "{{ count++ }}", "{{ --count }}"] {
        let errors = diagnostics(&component(template));
        assert!(
            errors
                .iter()
                .any(|e| e
                    .contains("Bindings cannot contain assignments at the end of the expression")),
            "{template}: {errors:?}"
        );
    }
}

#[test]
fn invalid_target_errors() {
    for (template, message) in [
        (
            r#"<button (click)="maybe?.n++"></button>"#,
            "The '?.' operator cannot be used in the assignment",
        ),
        (
            r#"<button (click)="++maybe?.[0]"></button>"#,
            "The '?.' operator cannot be used in the assignment",
        ),
        (r#"<button (click)="5++"></button>"#, "The expression cannot be used in the assignment"),
        (
            r#"<button (click)="fn()++"></button>"#,
            "The expression cannot be used in the assignment",
        ),
        (
            r#"<button (click)="(count + x)++"></button>"#,
            "The expression cannot be used in the assignment",
        ),
        (
            r#"<button (click)="count++.toString()"></button>"#,
            "Unexpected token '.' at column 8 in [count++.toString()]",
        ),
    ] {
        let errors = diagnostics(&component(template));
        assert!(errors.iter().any(|e| e.contains(message)), "{template}: {errors:?}");
    }
}

#[test]
fn tagged_template_tag() {
    // ngc: `return (ctx.count++) `x`;`, its printer keeps the parentheses.
    listener(r#"<button (click)="(count++)\`x\`"></button>"#, "return (ctx.count++)`x`;");
    listener(r#"<button (click)="(--count)\`x\`"></button>"#, "return (--ctx.count)`x`;");
}

/// The diagnostics of compiling `template`, which must hold exactly `expected`
/// (each followed by Angular's location text).
fn assert_errors(template: &str, expected: &[&str]) {
    let errors = diagnostics(&component(template));
    assert_eq!(errors.len(), expected.len(), "{template}: {errors:?}");
    for (error, message) in errors.iter().zip(expected) {
        assert!(error.contains(message), "{template}: expected `{message}` in {errors:?}");
    }
}

#[test]
fn missing_operand_errors() {
    // ngc 22.3.0-next.4 messages (`Parser Error: ... in [...]`). For `++;` and
    // `++,a` it also reports `Empty expressions are not allowed`, which this
    // compiler does not report for any empty action.
    for (template, expected) in [
        (r#"<button (click)="++;"></button>"#, &["Unexpected token ; at column 3 in [++;]"][..]),
        (r#"<button (click)="fn(++)"></button>"#, &["Unexpected token ) at column 6 in [fn(++)]"]),
        (r#"<button (click)="[++]"></button>"#, &["Unexpected token ] at column 4 in [[++]]"]),
        (r#"<button (click)="(++)"></button>"#, &["Unexpected token ) at column 4 in [(++)]"]),
        (r#"<button (click)="++,x"></button>"#, &["Unexpected token , at column 3 in [++,x]"]),
        (r#"<button (click)="++ ;x"></button>"#, &["Unexpected token ; at column 4 in [++ ;x]"]),
        (
            r#"<button (click)="--;++;"></button>"#,
            &[
                "Unexpected token ; at column 3 in [--;++;]",
                "Unexpected token ; at column 6 in [--;++;]",
            ],
        ),
    ] {
        assert_errors(template, expected);
    }
}

#[test]
fn pipe_binding_errors() {
    // ngc reports the assignment only, not a second error for the pipe.
    let source = |template: &str| {
        format!(
            "import {{Component, Pipe}} from '@angular/core';
@Pipe({{name: 'foo'}}) export class FooPipe {{ transform(...a: any[]) {{ return a; }} }}
@Component({{selector: 'app', imports: [FooPipe], template: `{template}`}})
export class App {{ count = 0; x = 0; }}"
        )
    };
    for (template, expected) in [
        (
            r#"<div [title]="count++ | foo"></div>"#,
            "Bindings cannot contain assignments at column 9",
        ),
        (
            r#"<div [title]="x | foo:count++"></div>"#,
            "Bindings cannot contain assignments at the end of the expression",
        ),
        (
            r#"<div [title]="x | foo:count++:x"></div>"#,
            "Bindings cannot contain assignments at column 16",
        ),
    ] {
        let errors = diagnostics(&source(template));
        assert_eq!(errors.len(), 1, "{template}: {errors:?}");
        assert!(errors[0].contains(expected), "{template}: {errors:?}");
    }
}

const LINK_SOURCE: &str = "import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `<button (click)=\"count++; --other.n; items[0]++\"></button>@if (other; as o) {<button (click)=\"o.n--\"></button>}`,
  host: {'(click)': 'count++'},
})
export class App { count = 0; other = {n: 1}; items = [1]; }";

/// The parts of ngc's full output for [`LINK_SOURCE`] that hold the updates.
fn assert_link_source_output(code: &str) {
    assert_contains(
        code,
        "function App_Conditional_1_Template_button_click_0_listener() { const o_r2 = i0.ɵɵrestoreView(_r1); return i0.ɵɵresetView(o_r2.n--); }",
    );
    assert_contains(
        code,
        r#"i0.ɵɵlistener("click", function App_click_HostBindingHandler() { return ctx.count++; });"#,
    );
    assert_contains(
        code,
        r#"i0.ɵɵdomListener("click", function App_Template_button_click_0_listener() { ctx.count++; --ctx.other.n; return ctx.items[0]++; });"#,
    );
}

#[test]
fn full_compilation_of_link_source() {
    assert_link_source_output(&compile(LINK_SOURCE));
}

/// ngc's partial output keeps the template as written; linking it compiles the updates.
#[test]
fn link_ngc_partial_output() {
    let partial = r#"import { Component } from '@angular/core';
import * as i0 from "@angular/core";
export class App {
    count = 0;
    other = { n: 1 };
    items = [1];
    static ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "22.3.0-next.4", ngImport: i0, type: App, deps: [], target: i0.ɵɵFactoryTarget.Component });
    static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "22.3.0-next.4", type: App, isStandalone: true, selector: "app", host: { listeners: { "click": "count++" } }, ngImport: i0, template: `<button (click)="count++; --other.n; items[0]++"></button>@if (other; as o) {<button (click)="o.n--"></button>}`, isInline: true });
}
"#;
    assert_link_source_output(&link(partial));
}

/// Our own partial output round-trips through the linker.
#[test]
fn link_own_partial_output() {
    let partial = compile_mode(LINK_SOURCE, CompilationMode::Partial);
    assert_contains(&strip(&partial), r#"host:{listeners:{"click":"count++"}}"#);
    assert_link_source_output(&link(&partial));
}

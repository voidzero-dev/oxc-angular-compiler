//! ngtsc compiles all the classes of a file with one constant pool: `_c` constants
//! (`freshName`) and `uniqueName` names (`_forTrack`, `arrowFn`) are counted across the file
//! apart from each other, an equal pure function or shared function is declared once by the
//! first class and reused by the later ones, and a component pools its host bindings'
//! constants (`baseDirectiveFields`) before its template's.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    CompilationMode, ComponentHmrUpdateInput, HostMetadataInput, TransformOptions,
    compile_component_hmr_update, compile_template_to_js_with_options, link,
    transform_angular_file,
};

fn transform(source: &str) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, None, None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result.code
}

/// `code` without whitespace, so the emitter's line wrapping doesn't matter.
fn squash(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The value `squashed` declares `name` with, up to its `;`.
fn value<'c>(squashed: &'c str, name: &str) -> &'c str {
    let start = squashed.find(&format!("const{name}=")).expect(squashed) + name.len() + 6;
    let end = squashed[start..].find(';').expect(squashed);
    &squashed[start..start + end]
}

/// The names of the top-level `const`s of `code`, in order.
fn constants(code: &str) -> Vec<&str> {
    code.lines()
        .filter_map(|line| line.strip_prefix("const "))
        .filter_map(|rest| rest.split([' ', '=']).next())
        .collect()
}

#[test]
fn track_functions_are_numbered_apart_from_constants() {
    // ngtsc: `_c0`, `_c1`, `_forTrack0` for A, then `_forTrack1` for B, not
    // a name numbered from B's first `_c` index.
    let code = transform(
        r#"
import { Component } from '@angular/core';

@Component({
  selector: 'cmp-a',
  template: '<p [title]="[k, 1]" [class]="{a: k}"></p>@for (it of items; track it.id) {<b></b>}',
})
export class A { k = 1; items: any[] = []; }

@Component({
  selector: 'cmp-b',
  template: '<p [title]="[k, 2]"></p>@for (it of items; track it.name) {<b></b>}',
})
export class B { k = 1; items: any[] = []; }
"#,
    );
    assert_eq!(constants(&code), ["_c0", "_c1", "_forTrack0", "_c2", "_forTrack1"], "{code}");
    assert!(
        squash(&code).contains("ɵɵrepeaterCreate(1,B_For_2_Template,1,0,\"b\",null,_forTrack1)"),
        "{code}"
    );
}

#[test]
fn equal_track_functions_are_declared_once_per_file() {
    let code = transform(
        r"
import { Component } from '@angular/core';

@Component({ selector: 'cmp-a', template: '@for (it of items; track it.id) {<b></b>}' })
export class A { items: any[] = []; }

@Component({ selector: 'cmp-b', template: '<p></p>@for (it of items; track it.id) {<i></i>}' })
export class B { items: any[] = []; }
",
    );
    assert_eq!(constants(&code), ["_forTrack0"], "{code}");
    let squashed = squash(&code);
    assert!(squashed.contains("A_For_1_Template,1,0,\"b\",null,_forTrack0)"), "{code}");
    assert!(squashed.contains("B_For_2_Template,1,0,\"i\",null,_forTrack0)"), "{code}");
}

#[test]
fn equal_arrow_functions_are_declared_once_per_file() {
    let code = transform(
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'cmp-a', template: '<p [title]="map((y) => y * 2)"></p>' })
export class A { map(f: any) { return f; } }

@Component({
  selector: 'cmp-b',
  template: '<p [title]="map((y) => y * 2)"></p><p [title]="map((q) => q * 3)"></p>',
})
export class B { map(f: any) { return f; } }
"#,
    );
    assert_eq!(constants(&code), ["arrowFn0", "arrowFn1"], "{code}");
    assert_eq!(squash(&code).matches(",arrowFn0,ctx)").count(), 2, "{code}");
}

#[test]
fn equal_pure_functions_of_a_directive_and_a_component_are_one_constant() {
    // ngtsc: `const _c0 = (a0, a1) => ({ height: a0, width: a1 });`, read by
    // the directive's host bindings and the component's template.
    let code = transform(
        r#"
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[dir]', host: { '[style]': '{height: h, width: w}' } })
export class Dir { h = 1; w = 2; }

@Component({ selector: 'cmp', template: '<div [style]="{height: h, width: w}"></div><p [title]="[h, 5]"></p>' })
export class Cmp { h = 1; w = 2; }
"#,
    );
    assert_eq!(constants(&code), ["_c0", "_c1"], "{code}");
    let squashed = squash(&code);
    assert_eq!(squashed.matches("ɵɵstyleMap(i0.ɵɵpureFunction2(").count(), 2, "{code}");
    assert_eq!(squashed.matches(",_c0,ctx.h,ctx.w)").count(), 2, "{code}");
    assert_eq!(value(&squashed, "_c1"), "(a0)=>[a0,5]", "{code}");
}

#[test]
fn host_binding_constants_come_before_the_template_ones() {
    // ngtsc: the query predicate, then the host bindings' pure function, then
    // the template's, each declared in that order.
    let code = transform(
        r#"
import { Component, ViewChild } from '@angular/core';

@Component({
  selector: 'cmp',
  template: '<div [class]="{a: k}"></div>',
  host: { '[class]': '{hostA: k}' },
})
export class Cmp { @ViewChild('x') x: any; k = true; }
"#,
    );
    assert_eq!(constants(&code), ["_c0", "_c1", "_c2"], "{code}");
    let squashed = squash(&code);
    assert_eq!(value(&squashed, "_c0"), "[\"x\"]", "{code}");
    assert!(value(&squashed, "_c1").contains("hostA:a0"), "{code}");
    assert!(value(&squashed, "_c2").contains("a:a0"), "{code}");
}

#[test]
fn template_reuses_a_host_binding_pure_function() {
    let code = transform(
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'cmp', template: '<div [class]="{a: k}"></div>', host: { '[class]': '{a: k}' } })
export class Cmp { k = true; }
"#,
    );
    assert_eq!(constants(&code), ["_c0"], "{code}");
    assert_eq!(squash(&code).matches("i0.ɵɵpureFunction1(").count(), 2, "{code}");
}

#[test]
fn compile_template_pools_host_constants_first() {
    // As a full compile of the component: the host bindings' `{q: x}` and
    // `{k: x}` first, the template's `{q: x}` reusing `_c0`, then `{z: x}`.
    let allocator = Allocator::default();
    let options = TransformOptions {
        host: Some(HostMetadataInput {
            properties: vec![
                ("[style]".to_string(), "{q: x}".to_string()),
                ("[class]".to_string(), "{k: x}".to_string()),
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let template = r#"<i [title]="{q: x}" [id]="{z: x}"></i>"#;
    let output = compile_template_to_js_with_options(&allocator, template, "A", "a.ts", &options)
        .expect("compiles");
    let code = output.code;
    assert_eq!(constants(&code), ["_c0", "_c1", "_c2"], "{code}");
    let squashed = squash(&code);
    assert!(value(&squashed, "_c0").contains("q:a0"), "{code}");
    assert!(value(&squashed, "_c1").contains("k:a0"), "{code}");
    assert!(value(&squashed, "_c2").contains("z:a0"), "{code}");
    assert!(squashed.contains(r#"ɵɵproperty("title",i0.ɵɵpureFunction1(2,_c0,ctx.x))"#), "{code}");
}

#[test]
fn hmr_update_declares_the_host_constants_its_template_reads() {
    let source = r#"
import { Component } from '@angular/core';

@Component({ selector: 'cmp', template: '<div [class]="{a: k}"></div>', host: { '[class]': '{a: k}' } })
export class Cmp { k = true; }
"#;
    let allocator = Allocator::default();
    let input = ComponentHmrUpdateInput { class_name: "Cmp", ..Default::default() };
    let options = TransformOptions { hmr: true, ..Default::default() };
    let update =
        compile_component_hmr_update(&allocator, "test.ts", source, &input, Some(&options), None);
    assert!(update.diagnostics.is_empty(), "{:?}", update.diagnostics);
    let module = update.module.expect("update module");
    let squashed = squash(&module);
    assert!(value(&squashed, "_c0").contains("a:a0"), "{module}");
    assert!(squashed.contains("_c0,ctx.k"), "{module}");
}

#[test]
fn linked_component_keeps_template_and_host_arrow_functions_apart() {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let source = r#"
import { Component } from '@angular/core';

@Component({
  selector: 'cmp',
  template: '<p [title]="map((y) => y * 2)"></p>',
  host: { '[title]': 'map((q) => q * 3)' },
})
export class Cmp { map(f: any) { return f; } }
"#;
    let result = transform_angular_file(&allocator, "case.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    let linked = link(&allocator, &result.code, "case.ts");
    assert!(linked.linked, "{}", result.code);
    let squashed = squash(&linked.code);
    assert_eq!(squashed.matches("constarrowFn0=").count(), 1, "{}", linked.code);
    assert_eq!(squashed.matches("constarrowFn1=").count(), 1, "{}", linked.code);
    // ngtsc's linker compiles the host bindings first.
    assert_eq!(value(&squashed, "arrowFn0"), "(ctx,view)=>(q)=>(q*3)", "{}", linked.code);
    assert_eq!(value(&squashed, "arrowFn1"), "(ctx,view)=>(y)=>(y*2)", "{}", linked.code);
}

#[test]
fn linked_component_pools_host_constants_first_and_shares_them_with_the_template() {
    // ngtsc's linker (`compileComponentFromMetadata`, one pool): the host
    // bindings' pure and arrow functions come first, and the template reuses
    // the equal ones.
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let source = r#"
import { Component } from '@angular/core';

@Component({
  selector: 'cmp',
  template: '<i [title]="f((a) => a + 1)" [style]="{width: w, height: h}" [class]="[w, 1]"></i>',
  host: { '[title]': 'f((a) => a + 1)', '[style]': '{width: w, height: h}', '[class]': '{x: w}' },
})
export class Cmp { w = 1; h = 2; f(x: any) { return ''; } }
"#;
    let result = transform_angular_file(&allocator, "case.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    let linked = link(&allocator, &result.code, "case.ts");
    assert!(linked.linked, "{}", result.code);
    assert_eq!(constants(&linked.code), ["_c0", "_c1", "arrowFn0", "_c2"], "{}", linked.code);
    let squashed = squash(&linked.code);
    assert!(value(&squashed, "_c0").contains("width:a0,height:a1"), "{}", linked.code);
    assert!(value(&squashed, "_c1").contains("x:a0"), "{}", linked.code);
    assert_eq!(value(&squashed, "_c2"), "(a0)=>[a0,1]", "{}", linked.code);
    assert_eq!(squashed.matches(",_c0,ctx.w,ctx.h)").count(), 2, "{}", linked.code);
    assert_eq!(squashed.matches(",arrowFn0,ctx)").count(), 2, "{}", linked.code);
}

#[test]
fn pure_functions_whose_strings_join_alike_stay_apart() {
    // `'a","b'` must not read as the two entries `'a', 'b'`. Angular's
    // `GenericKeyFn` doesn't escape strings and gives both bindings one
    // function, so `B` would get a two-entry array; Oxc keeps them apart, in
    // one class as across classes.
    let code = transform(
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'a-a', template: '<i [title]="[k, &apos;a&quot;,&quot;b&apos;]"></i>' })
export class A { k = 1; }

@Component({
  selector: 'b-b',
  template: '<i [title]="[k, &apos;a&apos;, &apos;b&apos;]" [id]="[k, &apos;a&quot;,&quot;b&apos;]"></i>',
})
export class B { k = 1; }
"#,
    );
    assert_eq!(constants(&code), ["_c0", "_c1"], "{code}");
    let squashed = squash(&code);
    assert_eq!(value(&squashed, "_c0"), r#"(a0)=>[a0,"a\",\"b"]"#, "{code}");
    assert_eq!(value(&squashed, "_c1"), r#"(a0)=>[a0,"a","b"]"#, "{code}");
    assert!(
        squashed.contains(r#"ɵɵdomProperty("title",i0.ɵɵpureFunction1(2,_c1,ctx.k))("id""#),
        "{code}"
    );
    assert_eq!(squashed.matches(",_c0,ctx.k)").count(), 2, "{code}");
}

#[test]
fn i18n_message_names_are_counted_across_the_file() {
    // ngtsc names messages with `pool.uniqueName`: `B` goes on from `A`'s
    // `i18n_1` and `..._1`, with external ids as without.
    let source = r#"
import { Component } from '@angular/core';

@Component({ selector: 'a-a', template: '<p i18n="@@foo">Hello {{w}}</p><b i18n-title="@@bar" title="T"></b>' })
export class A { w = 1; }

@Component({ selector: 'b-b', template: '<p i18n="@@baz">Bye</p>' })
export class B {}
"#;
    let names = |use_external_ids: bool| {
        let allocator = Allocator::default();
        let options =
            TransformOptions { i18n_use_external_ids: use_external_ids, ..Default::default() };
        let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
        assert!(!result.has_errors(), "{:?}", result.diagnostics);
        // `(i18n_0=MSG_...)`, the Closure branch's assignments.
        squash(&result.code)
            .split("(i18n_")
            .skip(1)
            .filter_map(|part| part.split(')').next())
            .filter(|assignment| assignment.contains("=MSG_"))
            .map(|assignment| format!("i18n_{assignment}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(true),
        [
            "i18n_0=MSG_EXTERNAL_bar$$TEST_TS_0",
            "i18n_1=MSG_EXTERNAL_foo$$TEST_TS_1",
            "i18n_2=MSG_EXTERNAL_baz$$TEST_TS_2",
        ]
    );
    assert_eq!(
        names(false),
        ["i18n_0=MSG_TEST_TS_0", "i18n_1=MSG_TEST_TS_1", "i18n_2=MSG_TEST_TS_2"]
    );
}

#[test]
fn every_class_declares_its_pool_after_the_imports() {
    // ngtsc declares the whole constant pool, child view functions included,
    // after the imports and in pool order, not each class's part before it.
    let code = transform(
        r#"
import { Component } from '@angular/core';
const helper = 1;
@Component({ selector: 'cmp-a', template: '<p [title]="[k, 1]"></p>@if (k) {<b></b>}' })
export class A { k = helper; }
export function f() { return helper; }
@Component({ selector: 'cmp-b', template: '<p [title]="[k, 2]"></p>@if (k) {<i></i>}' })
export class B { k = 1; }
"#,
    );
    let at = |needle: &str| code.find(needle).expect(&code);
    let order = [
        "import * as i0",
        "const _c0",
        "function A_Conditional_1_Template",
        "const _c1",
        "function B_Conditional_1_Template",
        "const helper",
        "export class A",
        "export function f",
        "export class B",
    ];
    assert!(order.windows(2).all(|pair| at(pair[0]) < at(pair[1])), "{code}");
}

#[test]
fn pool_goes_before_a_class_that_an_import_follows() {
    // ngtsc moves the imports up and declares the pool before the class. The
    // class's static fields read `_c1`, so it can't come after the class.
    let code = transform(
        r#"
import { Component } from '@angular/core';
@Component({ selector: 'cmp-a', template: '<ng-content select="x"></ng-content><ng-content></ng-content>' })
export class A {}
import { helper } from './other';
export const h = helper;
"#,
    );
    let at = |needle: &str| code.find(needle).expect(&code);
    assert!(at("const _c1") < at("export class A"), "{code}");
}

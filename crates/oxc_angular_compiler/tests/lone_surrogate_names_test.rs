//! Input and output names holding a lone surrogate (`'x-\uD800'`, #505).
//!
//! ngtsc reads them like any other string and prints each lone code unit as
//! a `\uXXXX` escape. The expected fragments below are ngtsc 22.2's output
//! with the whitespace removed.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    CompilationMode, TransformOptions, TransformResult, transform_angular_file,
};

fn compile(source: &str, options: TransformOptions) -> TransformResult {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "t.ts", source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The `.d.ts` members of `class_name`, whitespace removed.
fn dts(result: &TransformResult, class_name: &str) -> String {
    let decl = result.dts_declarations.iter().find(|d| d.class_name == class_name).unwrap();
    compact(&decl.members)
}

fn assert_has(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(haystack.contains(needle), "missing `{needle}` in\n{haystack}");
    }
}

const DECORATORS: &str = r"import { Directive, EventEmitter, Input, Output } from '@angular/core';
@Directive({ selector: '[d]' })
export class D {
  @Input() 'x-\uD800': any;
  @Input({ transform: (v: string) => 1 }) 'y-\uD800': number = 0;
  @Input('z-\uDC00') z: any;
  @Input({ alias: 'w-\uD800' }) w: any;
  @Output() 'o-\uD800' = new EventEmitter();
  @Output('p-\uD800') p = new EventEmitter();
}
";

#[test]
fn decorator_inputs_and_outputs() {
    let result = compile(DECORATORS, TransformOptions::default());
    assert_has(
        &compact(&result.code),
        &[
            r#"inputs:{"x-\uD800":"x-\uD800","y-\uD800":[2,"y-\uD800","y-\uD800",(v)=>1],z:[0,"z-\uDC00","z"],w:[0,"w-\uD800","w"]}"#,
            r#"outputs:{"o-\uD800":"o-\uD800",p:"p-\uD800"}"#,
            // `setClassMetadata` keys; the decorator arguments are the source text.
            r#"{"x-\uD800":[{type:Input}],"y-\uD800":[{type:Input,args:[{transform:(v)=>1}]}],z:[{type:Input,args:['z-\uDC00']}]"#,
            r#""o-\uD800":[{type:Output}],p:[{type:Output,args:['p-\uD800']}]"#,
        ],
    );
    assert_has(
        &dts(&result, "D"),
        &[
            r#"{"x-\uD800":{"alias":"x-\uD800";"required":false;};"y-\uD800":{"alias":"y-\uD800";"required":false;};"z":{"alias":"z-\uDC00";"required":false;};"w":{"alias":"w-\uD800";"required":false;};},{"o-\uD800":"o-\uD800";"p":"p-\uD800";}"#,
            r#"static"ngAcceptInputType_y-\uD800":string;"#,
        ],
    );
}

#[test]
fn partial_declaration() {
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result = compile(DECORATORS, options);
    assert_has(
        &compact(&result.code),
        &[
            r#"inputs:{"x-\uD800":"x-\uD800","y-\uD800":["y-\uD800","y-\uD800",(v)=>1],z:["z-\uDC00","z"],w:["w-\uD800","w"]}"#,
            r#"outputs:{"o-\uD800":"o-\uD800",p:"p-\uD800"}"#,
        ],
    );
}

#[test]
fn signal_apis() {
    let result = compile(
        r"import { Directive, input, model, output } from '@angular/core';
@Directive({ selector: '[s]' })
export class S {
  'a-\uD800' = input<string>();
  b = input('', { alias: 'b-\uD800' });
  'm-\uD800' = model(0);
  c = output({ alias: 'c-\uD800' });
}
",
        TransformOptions::default(),
    );
    assert_has(
        &compact(&result.code),
        &[
            r#"inputs:{"a-\uD800":[1,"a-\uD800"],b:[1,"b-\uD800","b"],"m-\uD800":[1,"m-\uD800"]}"#,
            r#"outputs:{"m-\uD800":"m-\uD800Change",c:"c-\uD800"}"#,
        ],
    );
    assert_has(
        &dts(&result, "S"),
        &[r#""a-\uD800":{"alias":"a-\uD800";"required":false;"isSignal":true;}"#],
    );
}

#[test]
fn metadata_arrays() {
    let result = compile(
        r"import { Directive, EventEmitter } from '@angular/core';
@Directive({
  selector: '[e]',
  inputs: ['q-\uD800', 'r: r-\uD800', { name: 's', alias: 's-\uD800' }],
  outputs: ['t-\uD800'],
})
export class E { 'q-\uD800': any; r: any; s: any; 't-\uD800' = new EventEmitter(); }
",
        TransformOptions::default(),
    );
    assert_has(
        &compact(&result.code),
        &[
            r#"inputs:{"q-\uD800":"q-\uD800",r:[0,"r-\uD800","r"],s:[0,"s-\uD800","s"]}"#,
            r#"outputs:{"t-\uD800":"t-\uD800"}"#,
        ],
    );
}

#[test]
fn escape_case_and_position() {
    // A lowercase escape prints uppercase, as ngtsc's generated strings do. A
    // key starting or ending with a lone unit stays quoted (ngtsc writes it
    // unquoted, as U+FFFD, which is not valid JavaScript).
    let result = compile(
        r"import { Directive, Input } from '@angular/core';
@Directive({ selector: '[m]' })
export class M {
  @Input() 'l-\ud800': any;
  @Input() '\uD800start': any;
  @Input() 'end\uDC00': any;
  @Input('lc-\ud800') lc: any;
}
",
        TransformOptions::default(),
    );
    assert_has(
        &compact(&result.code),
        &[
            r#"inputs:{"l-\uD800":"l-\uD800","\uD800start":"\uD800start","end\uDC00":"end\uDC00",lc:[0,"lc-\uD800","lc"]}"#,
            r#"lc:[{type:Input,args:['lc-\ud800']}]"#,
        ],
    );
}

#[test]
fn jit_member_names() {
    let result = compile(
        r"import { Directive, Input, input } from '@angular/core';
@Directive({ selector: '[j]' })
export class J {
  @Input() 'x-\uD800': any;
  'a-\uD800' = input<string>();
}
",
        TransformOptions { jit: true, ..Default::default() },
    );
    // JIT output is reprinted by oxc's codegen, which writes lowercase hex.
    assert_has(
        &compact(&result.code),
        &[
            r#""x-\ud800":[{type:Input}]"#,
            r#""a-\ud800":[{type:i0.Input,args:[{isSignal:true,alias:"a-\ud800""#,
        ],
    );
}

#[test]
fn host_attribute_value() {
    // Metadata strings are read the same way: ngtsc emits
    // `hostAttrs: ["title", "v\uD800"]`.
    let result = compile(
        r"import { Directive } from '@angular/core';
@Directive({ selector: '[h]', host: { title: 'v\uD800' } })
export class H {}
",
        TransformOptions::default(),
    );
    assert_has(&compact(&result.code), &[r#"hostAttrs:["title","v\uD800"]"#]);
}

#[test]
fn selector_with_a_lone_surrogate_is_an_error() {
    // oxc's selector parser can't read it, so it is reported rather than
    // compiled to a directive that matches nothing.
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "t.ts",
        r"import { Directive } from '@angular/core';
@Directive({ selector: '[k\uD800]' })
export class K {}
",
        None,
        None,
    );
    let messages: Vec<String> = result.diagnostics.iter().map(|d| d.message.to_string()).collect();
    assert_eq!(messages, ["selector must be a string Value could not be determined statically."]);
}

#[test]
fn private_use_characters_are_printed_as_themselves() {
    // A real U+10F800 (private use) or U+FDD0 (noncharacter) is not a stored
    // lone surrogate, alone or next to each other the other way round.
    let source = "import { Component, Input } from '@angular/core';
@Component({ selector: 'p', template: '<i>\u{10F800}\u{FDD0}</i>' })
export class P {
  @Input() 'a\u{10F800}': any;
  @Input() 'b\u{FDD0}': any;
  @Input() 'c\u{10F800}\u{FDD0}': any;
}
";
    let result = compile(source, TransformOptions::default());
    assert_has(
        &compact(&result.code),
        &[
            "\"a\u{10F800}\":\"a\u{10F800}\"",
            "\"b\u{FDD0}\":\"b\u{FDD0}\"",
            "\"c\u{10F800}\u{FDD0}\":\"c\u{10F800}\u{FDD0}\"",
            "\"\u{10F800}\u{FDD0}\"",
        ],
    );
    assert!(!compact(&result.code).contains(r"\uD800"), "{}", result.code);
}

#[test]
fn accept_type_member_is_quoted() {
    // ngtsc prints `ngAcceptInputType_<U+FFFD>`, which is not valid TS.
    let result = compile(
        r"import { Directive, Input } from '@angular/core';
@Directive({ selector: '[a]' })
export class A { @Input({ transform: (v: string) => 1 }) '\uD800': number = 0; }
",
        TransformOptions::default(),
    );
    assert_has(&dts(&result, "A"), &[r#"static"ngAcceptInputType_\uD800":string;"#]);
}

#[test]
fn query_writes_use_a_key() {
    // ngtsc prints `ctx.q-<U+FFFD>`, which is not valid JS.
    let result = compile(
        r"import { Component, ViewChild, viewChild } from '@angular/core';
@Component({ selector: 'q', template: '' })
export class Q { @ViewChild('r') 'q-\uD800': any; 'v\uD800' = viewChild('r'); }
",
        TransformOptions::default(),
    );
    assert_has(
        &compact(&result.code),
        &[r#"ɵɵviewQuerySignal(ctx["v\uD800"],"#, r#"(ctx["q-\uD800"]=_t.first)"#],
    );
}

#[test]
fn accept_type_member_clash_is_reported() {
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "t.ts",
        r"import { Directive, Input } from '@angular/core';
@Directive({ selector: '[c]' })
export class C {
  @Input({ transform: (v: string) => 1 }) 'q\uD800': number = 0;
  static 'ngAcceptInputType_q\uD800': string;
}
",
        None,
        None,
    );
    assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
    assert!(
        result.diagnostics[0].message.contains("Class cannot have both a transform function"),
        "{:?}",
        result.diagnostics
    );
}

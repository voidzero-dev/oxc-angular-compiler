//! Text outside ASCII in string literals (#707).
//!
//! ngtsc's output goes through the TypeScript printer. A string literal the
//! compiler creates is printed with text outside ASCII as `\uXXXX` escapes (one
//! per UTF-16 code unit), while a node copied from the source (a
//! `WrappedNodeExpr`: providers, `setClassMetadata` arguments, `@Inject` /
//! `@Attribute` tokens, the NgModule `id`, an inline template in partial output)
//! keeps its source text. The Angular linker builds Babel nodes and its Babel 8
//! generator prints raw UTF-8, escaping only Unicode spaces.
//!
//! Expected fragments are ngtsc 22.2.4 / Angular linker 22.2.4 output with line
//! breaks and indentation removed.

use std::collections::HashMap;

use oxc_allocator::Allocator;
use oxc_angular_compiler::linker::{LinkOptions, link_with_options};
use oxc_angular_compiler::{
    AngularVersion, CompilationMode, ResolvedResources, TransformOptions, TransformResult,
    transform_angular_file,
};

fn compile_with(
    source: &str,
    mode: CompilationMode,
    resources: Option<&ResolvedResources>,
) -> TransformResult {
    let allocator = Allocator::default();
    let options = TransformOptions { compilation_mode: mode, ..Default::default() };
    let result = transform_angular_file(&allocator, "c.ts", source, Some(&options), resources);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result
}

/// The output with line breaks and the indentation after them removed.
fn flat(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn full(source: &str) -> String {
    flat(&compile_with(source, CompilationMode::Full, None).code)
}

fn partial(source: &str) -> String {
    flat(&compile_with(source, CompilationMode::Partial, None).code)
}

fn dts(source: &str) -> String {
    let result = compile_with(source, CompilationMode::Full, None);
    result.dts_declarations.iter().map(|d| d.members.as_str()).collect::<Vec<_>>().join("\n")
}

fn link(code: &str, version: Option<AngularVersion>) -> String {
    let allocator = Allocator::default();
    let options = LinkOptions { angular_version: version, ..Default::default() };
    flat(&link_with_options(&allocator, code, "test.mjs", &options).code)
}

fn assert_has(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(haystack.contains(needle), "missing `{needle}` in\n{haystack}");
    }
}

fn assert_lacks(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(!haystack.contains(needle), "unexpected `{needle}` in\n{haystack}");
    }
}

const BASIC: &str = r#"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p title="é😀">ü</p>' }) export class C {}"#;

const HOST: &str = r#"import { Component, Input } from '@angular/core';
@Component({ selector: '[x=é]', template: '', host: { title: 'é', '[attr.y]': "'ü'" }, providers: [{ provide: 'pé', useValue: 'vü' }] }) export class C { @Input('ålias') x; }"#;

const STYLES: &str = r#"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p>x</p>', styles: ['p::after { content: "é😀"; }'] }) export class C {}"#;

// ---- full compile ----------------------------------------------------------

#[test]
fn template_text_and_attributes_are_escaped() {
    let code = full(BASIC);
    assert_has(
        &code,
        &[
            r#"consts:[["title","\u00E9\uD83D\uDE00"]]"#,
            r#"i0.ɵɵtext(1,"\u00FC")"#,
            // setClassMetadata keeps the decorator as written.
            r#"template:'<p title="é😀">ü</p>'"#,
        ],
    );
}

#[test]
fn interpolation_strings_are_escaped_and_template_literals_kept() {
    let code = full(
        r#"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p [title]="\'é\'" id="ä{{x}}ö">{{ \'ß\' }} ñ{{x}}ç</p>' }) export class C { x = 1; }"#,
    );
    assert_has(
        &code,
        &[
            r#"i0.ɵɵinterpolate1("\u00E4",ctx.x,"\u00F6"))("title","\u00E9")"#,
            r#"i0.ɵɵtextInterpolate2("","\u00DF"," \u00F1",ctx.x,"\u00E7")"#,
        ],
    );
    let code = full(
        r"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p>{{ `é${x}ü` }}</p>' }) export class C { x = 1; }",
    );
    assert_has(&code, &["i0.ɵɵtextInterpolate(`é${ctx.x}ü`)"]);
}

#[test]
fn styles_are_escaped() {
    let code = full(STYLES);
    assert_has(
        &code,
        &[
            r#"styles:["p[_ngcontent-%COMP%]::after { content: \"\u00E9\uD83D\uDE00\"; }"]"#,
            // ngtsc rebuilds the resources in setClassMetadata (`transformDecoratorResources`).
            r#"styles:["p::after { content: \"\u00E9\uD83D\uDE00\"; }"]"#,
        ],
    );
}

#[test]
fn i18n_strings_are_escaped_and_localize_text_kept() {
    let code = full(
        r#"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p i18n="désc|méaning@@ïd" title="tï" i18n-title>héllo {{x}}</p>' }) export class C { x = 1; }"#,
    );
    assert_has(
        &code,
        &[
            r#"goog.getMsg("t\u00EF")"#,
            r#"goog.getMsg("h\u00E9llo {$interpolation}""#,
            "tï`",
            r#"héllo ${"\uFFFD0\uFFFD"}:INTERPOLATION:`"#,
        ],
    );
}

#[test]
fn host_selector_and_inputs_are_escaped_and_providers_kept() {
    let code = full(HOST);
    assert_has(
        &code,
        &[
            r#"selectors:[["","x","\u00E9"]]"#,
            r#"hostAttrs:["title","\u00E9"]"#,
            r#"i0.ɵɵattribute("y","\u00FC")"#,
            r#"inputs:{x:[0,"\u00E5lias","x"]}"#,
            "i0.ɵɵProvidersFeature([{provide:'pé',useValue:'vü'}])",
            r#"selector:'[x=é]'"#,
        ],
    );
    let dts = dts(HOST);
    assert_has(&dts, &[r#""[x=\u00E9]""#, r#""alias": "\u00E5lias""#]);
}

#[test]
fn external_template_is_escaped_in_metadata() {
    let source = r"import { Component } from '@angular/core';
@Component({ selector: 'c', templateUrl: './c.html' }) export class C {}";
    let templates =
        HashMap::from([("./c.html".to_string(), r#"<b title="ñ">é😀</b>"#.to_string())]);
    let resources = ResolvedResources { templates, styles: HashMap::new() };
    let code = flat(&compile_with(source, CompilationMode::Full, Some(&resources)).code);
    assert_has(
        &code,
        &[
            r#"consts:[["title","\u00F1"]]"#,
            r#"i0.ɵɵtext(1,"\u00E9\uD83D\uDE00")"#,
            r#"template:"<b title=\"\u00F1\">\u00E9\uD83D\uDE00</b>""#,
        ],
    );
    let code = flat(&compile_with(source, CompilationMode::Partial, Some(&resources)).code);
    assert_has(&code, &[r#"template:"<b title=\"\u00F1\">\u00E9\uD83D\uDE00</b>""#]);
}

#[test]
fn signal_debug_name_is_escaped() {
    let code = full(
        r"import { Component, signal, input } from '@angular/core';
@Component({ selector: 'c', template: '' }) export class Café { naïve = signal(0); ünput = input<number>(); }",
    );
    assert_has(
        &code,
        &[
            r#"debugName: "na\u00EFve""#,
            r#"debugName: "\u00FCnput""#,
            r#"inputs:{ünput:[1,"\u00FCnput"]}"#,
        ],
    );
}

#[test]
fn attribute_and_inject_string_tokens_keep_source_text() {
    // Directive path.
    let source = r"import { Directive, Attribute, Inject } from '@angular/core';
@Directive({ selector: '[d]' }) export class D { constructor(@Attribute('é') a: string, @Inject('tök') t: any) {} }";
    assert_has(&full(source), &["i0.ɵɵinjectAttribute('é')", "i0.ɵɵdirectiveInject('tök')"]);
    assert_has(&dts(source), &[r#"attribute: "\u00E9""#]);

    // Component path.
    let source = r"import { Component, Attribute, Inject, Optional } from '@angular/core';
@Component({ selector: 'c', template: '' }) export class C { constructor(@Attribute('é') a: string, @Inject('tök') t: any, @Optional() @Inject('ö') o: any) {} }";
    assert_has(
        &full(source),
        &["i0.ɵɵinjectAttribute('é'),i0.ɵɵdirectiveInject('tök'),i0.ɵɵdirectiveInject('ö',8)"],
    );
    // ngtsc: `deps: [{ token: 'é', attribute: true }, { token: 'tök' }, { token: 'ö', optional: true }]`
    assert_has(
        &partial(source),
        &["deps:[{token:'é',attribute:true},{token:'tök'},{token:'ö',optional:true}]"],
    );
}

#[test]
fn ng_module_id_keeps_source_text() {
    let source = r"import { NgModule } from '@angular/core';
@NgModule({ id: 'mé' }) export class M {}";
    assert_has(&full(source), &["id:'mé'", "i0.ɵɵregisterNgModuleType(M,'mé')"]);
    assert_has(&partial(source), &["id:'mé'"]);
}

#[test]
fn quoted_keys_in_source_objects_keep_source_text() {
    let code = full(
        r#"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '', providers: [{ provide: 'x', useValue: { 'é-x': 'a', café: 'b', "q": 'é' } }] }) export class C {}"#,
    );
    // ngtsc: `useValue: { 'é-x': 'a', café: 'b', "q": 'é' }` (ASCII literals
    // are re-quoted, as before).
    assert_has(&code, &[r#"useValue:{'é-x':"a",café:"b","q":'é'}"#]);

    let code = full(
        r"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '', host: { '(é)': 'f()', 'class': 'é', '[class.é]': 'x' } }) export class C { x; f() {} }",
    );
    assert_has(&code, &[r#"host:{'(é)':"f()","class":'é','[class.é]':"x"}"#]);
}

#[test]
fn added_import_lines_escape_the_module_specifier() {
    let code = full(
        r"import { Component } from '@angular/core';
import { Svc } from './sé';
@Component({ selector: 'c', template: '' }) export class C { constructor(s: Svc) {} }",
    );
    // ngtsc: `import * as i1 from "./s\u00E9";` (the quote style differs, as before).
    assert_has(&code, &[r"import * as i1 from './s\u00E9';", "i0.ɵɵdirectiveInject(i1.Svc)"]);
}

#[test]
fn jit_resource_imports_escape_the_specifier() {
    let allocator = Allocator::default();
    let options = TransformOptions { jit: true, ..Default::default() };
    let source = r"import { Component } from '@angular/core';
@Component({ selector: 'd', templateUrl: './té.html', styleUrls: ['./sé.css'] }) export class Y {}";
    let result = transform_angular_file(&allocator, "c.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    // @angular/build 22.2.6 `createJitResourceTransformer` + TypeScript printer.
    assert_has(
        &result.code,
        &[
            r#"import __NG_CLI_RESOURCE__0 from "angular:jit:template:file;./t\u00E9.html";"#,
            r#"import __NG_CLI_RESOURCE__1 from "angular:jit:style:file;./s\u00E9.css";"#,
        ],
    );
}

// ---- partial compile -------------------------------------------------------

#[test]
fn partial_inline_template_keeps_source_text() {
    assert_has(&partial(BASIC), &[r#"template:'<p title="é😀">ü</p>',isInline:true"#]);
    let code = partial(
        r"import { Component } from '@angular/core';
@Component({ selector: 'c', template: `<p>é</p>` }) export class C {}",
    );
    assert_has(&code, &["template:`<p>é</p>`,isInline:true"]);
    let code = partial(
        r"import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p>é</p>' }) export class C {}",
    );
    assert_has(&code, &[r"template:'<p>é</p>',isInline:true"]);
    // A template read from a constant is a new literal.
    let code = partial(
        r"import { Component } from '@angular/core';
const T = '<p>é</p>';
@Component({ selector: 'c', template: T }) export class C {}",
    );
    assert_has(&code, &[r#"template:"<p>\u00E9</p>",isInline:true"#]);
}

#[test]
fn partial_declaration_strings_are_escaped() {
    let code = partial(HOST);
    assert_has(
        &code,
        &[
            r#"selector:"[x=\u00E9]""#,
            r#"inputs:{x:["\u00E5lias","x"]}"#,
            r#"host:{attributes:{"title":"\u00E9"},properties:{"attr.y":"'\u00FC'"}}"#,
            "providers:[{provide:'pé',useValue:'vü'}]",
        ],
    );
    let code = partial(STYLES);
    assert_has(&code, &[r#"styles:["p::after { content: \"\u00E9\uD83D\uDE00\"; }"]"#]);
}

// ---- linker ----------------------------------------------------------------

/// ngtsc 22.2.4's partial output for [`BASIC`].
const NGTSC_PARTIAL_BASIC: &str = r#"import * as i0 from "@angular/core";
export class C {
    static ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "22.2.2", ngImport: i0, type: C, deps: [], target: i0.ɵɵFactoryTarget.Component });
    static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: C, isStandalone: true, selector: "c", ngImport: i0, template: '<p title="é😀">ü</p>', isInline: true });
}"#;

#[test]
fn linker_prints_raw_text_like_babel() {
    // Angular linker: `consts: [["title", "é😀"]]`, `i0.ɵɵtext(1, "ü")`.
    let code = link(NGTSC_PARTIAL_BASIC, None);
    assert_has(&code, &[r#"consts: [["title","é😀"]]"#, r#"i0.ɵɵtext(1,"ü")"#]);
    assert_lacks(&code, &[r"\u00"]);
}

#[test]
fn linker_escapes_unicode_spaces_like_babel() {
    // Angular linker: `consts: [["title", "b\u2028c", "id", "a\xA0b"]]`.
    let partial = r#"import * as i0 from "@angular/core";
export class C {
    static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: C, isStandalone: true, selector: "c", ngImport: i0, template: '<p title="b\u2028c" id="a\u00A0b">x</p>', isInline: true });
}"#;
    let code = link(partial, None);
    assert_has(&code, &[r#"consts: [["title","b\u2028c","id","a\xA0b"]]"#]);
}

#[test]
fn linker_reads_declaration_strings_like_angular() {
    // Names are read with `getString()` and printed as new literals (raw);
    // host attribute values and providers are copied from the source.
    let partial = r#"import * as i0 from "@angular/core";
export class C {
    static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.2", type: C, isStandalone: true, selector: "[x=\u00E9]", inputs: { x: ["\u00E5lias", "x"] }, outputs: { öut: "\u00F6ut" }, host: { attributes: { "title": "\u00E9" }, properties: { "attr.y": "'\u00FC'" }, classAttribute: "\u00E9" }, providers: [{ provide: 'pé', useValue: 'vü' }], exportAs: ["\u00E9"], ngImport: i0, template: '', isInline: true });
}"#;
    let code = link(partial, None);
    // Angular linker 22.2.4 (Babel 8): `selectors: [["", "x", "é"]]`,
    // `hostAttrs: ["title", "\u00E9", 1, "é"]`, `i0.ɵɵattribute("y", "ü")`,
    // `inputs: { x: [0, "ålias", "x"] }`, `outputs: { öut: "öut" }`,
    // `exportAs: ["é"]`, `i0.ɵɵProvidersFeature([{ provide: 'pé', useValue: 'vü' }])`.
    assert_has(
        &code,
        &[
            r#"selectors: [["", "x", "é"]]"#,
            r#"hostAttrs: ["title", "\u00E9", 1, "é"]"#,
            r#"i0.ɵɵattribute("y","ü")"#,
            r#"inputs: { x: [0, "ålias", "x"] }"#,
            r#"outputs: { öut: "öut" }"#,
            r#"exportAs: ["é"]"#,
            "i0.ɵɵProvidersFeature([{ provide: 'pé', useValue: 'vü' }])",
        ],
    );
}

#[test]
fn linker_reads_host_directive_names_like_angular() {
    // ngtsc 22.2.4 partial output of
    // `hostDirectives: [{ directive: H, inputs: ['é: a'], outputs: ['ö: b'] }]`.
    let partial = r#"import * as i0 from "@angular/core";
export class X {
    static ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "22.2.4", type: X, isStandalone: true, selector: "c", hostDirectives: [{ directive: H, inputs: ["\u00E9", "a"], outputs: ["\u00F6", "b"] }], ngImport: i0, template: '', isInline: true });
}"#;
    let code = link(partial, None);
    // Angular linker 22.2.4 (Babel 8): the names are read with `getString()`
    // and printed as new literals, `inputs: ["é", "a"], outputs: ["ö", "b"]`.
    assert_has(
        &code,
        &[
            r#"i0.ɵɵHostDirectivesFeature([{ directive: H, inputs: ["é", "a"], outputs: ["ö", "b"] }])"#,
        ],
    );
    assert_lacks(&code, &[r"\u00"]);
}

// ---- linker changeDetection -------------------------------------------------

fn declare_component(version: &str, change_detection: &str) -> String {
    format!(
        r#"import * as i0 from "@angular/core";
export class C {{
    static ɵcmp = i0.ɵɵngDeclareComponent({{ minVersion: "14.0.0", version: "{version}", type: C, isStandalone: true, selector: "c", ngImport: i0, template: '', isInline: true{change_detection} }});
}}"#
    )
}

const ON_PUSH: &str = ", changeDetection: i0.ChangeDetectionStrategy.OnPush";
const DEFAULT: &str = ", changeDetection: i0.ChangeDetectionStrategy.Default";

/// The 19.2.15, 20.1.8, 20.2.0 and 21.2.0 linkers default a missing field to
/// `ChangeDetectionStrategy.Default`, and their compiler only writes a strategy
/// other than `Default`: no field for a missing or `Default` value, `0` for OnPush.
#[test]
fn pre_v22_linker_leaves_out_the_default_strategy() {
    for (major, minor, patch) in [(19, 2, 15), (20, 1, 8), (20, 2, 0), (21, 2, 0)] {
        let linker = Some(AngularVersion::new(major, minor, patch));
        let declared = format!("{major}.{minor}.{patch}");
        let code = link(&declare_component(&declared, ""), linker);
        assert_lacks(&code, &["changeDetection"]);
        let code = link(&declare_component(&declared, DEFAULT), linker);
        assert_lacks(&code, &["changeDetection"]);
        let code = link(&declare_component(&declared, ON_PUSH), linker);
        assert_has(&code, &["changeDetection: 0"]);
    }
}

/// The 22.2.4 linker reads a pre-v22 declaration with no field as Eager.
#[test]
fn v22_linker_keeps_eager_for_pre_v22_declarations() {
    let linker = Some(AngularVersion::new(22, 2, 4));
    assert_has(&link(&declare_component("20.1.8", ""), linker), &["changeDetection: 1"]);
    assert_has(&link(&declare_component("20.1.8", DEFAULT), linker), &["changeDetection: 1"]);
    assert_lacks(&link(&declare_component("22.2.4", ""), linker), &["changeDetection"]);
    // With no app version, the linker keeps the latest behaviour.
    assert_has(&link(&declare_component("20.1.8", ""), None), &["changeDetection: 1"]);
}

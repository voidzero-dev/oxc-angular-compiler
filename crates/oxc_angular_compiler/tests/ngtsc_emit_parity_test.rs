//! Small emit-shape differences from ngtsc (issue #623), each checked against
//! `@angular/compiler-cli` 22.2.3 output:
//!
//! - a constructor parameter without a usable token makes the whole factory
//!   `ɵɵinvalidFactory()` (full) / `deps: "invalid"` (partial), never
//!   `ɵɵinvalidFactoryDep(n)`
//! - `ɵmod` never lists `schemas` (the NgModule handler passes `schemas: []`)
//! - a directive's query predicates are pooled (`const _c0 = [...]`)
//! - a default-imported DI token stays `Foo`, not `i1.default`; its import is
//!   dropped when `@Inject` / `@Attribute` override it and no metadata is emitted
//! - an `@Inject(TOKEN)` token stays as written, not `i1.TOKEN`

// The expected output has object literals like `{token:Foo}`.
#![expect(clippy::literal_string_with_formatting_args)]

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

/// The compiled `source`, without whitespace.
fn compile_with(source: &str, options: &TransformOptions) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, Some(options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile(source: &str) -> String {
    compile_with(source, &TransformOptions::default())
}

fn compile_partial(source: &str) -> String {
    compile_with(
        source,
        &TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() },
    )
}

fn compile_without_metadata(source: &str, mode: CompilationMode) -> String {
    compile_with(
        source,
        &TransformOptions {
            compilation_mode: mode,
            emit_class_metadata: false,
            ..Default::default()
        },
    )
}

fn assert_contains(code: &str, expected: &str) {
    assert!(code.contains(expected), "Expected `{expected}` in:\n{code}");
}

fn assert_not_contains(code: &str, unexpected: &str) {
    assert!(!code.contains(unexpected), "Did not expect `{unexpected}` in:\n{code}");
}

const INVALID_DEP_CASES: &[(&str, &str)] = &[
    (
        "string param",
        "import { Injectable } from '@angular/core';
@Injectable()
export class S { constructor(s: string) {} }",
    ),
    (
        "untyped param",
        "import { Injectable } from '@angular/core';
@Injectable()
export class S { constructor(s) {} }",
    ),
    (
        "second param invalid",
        "import { Injectable } from '@angular/core';
class Ok {}
@Injectable()
export class S { constructor(o: Ok, s: string) {} }",
    ),
    (
        "directive without selector",
        "import { Directive } from '@angular/core';
@Directive()
export class D { constructor(s: string) {} }",
    ),
    (
        "component",
        "import { Component } from '@angular/core';
@Component({ selector: 'c', template: '' })
export class C { constructor(s: string) {} }",
    ),
];

#[test]
fn invalid_ctor_dep_makes_the_full_factory_invalid() {
    for (name, source) in INVALID_DEP_CASES {
        let code = compile(source);
        assert!(
            code.contains("_Factory(__ngFactoryType__){i0.ɵɵinvalidFactory();}"),
            "{name}:\n{code}"
        );
        assert!(!code.contains("ɵɵinvalidFactoryDep"), "{name}:\n{code}");
    }
}

#[test]
fn invalid_ctor_dep_makes_the_partial_deps_invalid() {
    for (name, source) in INVALID_DEP_CASES {
        let code = compile_partial(source);
        assert!(code.contains("deps:\"invalid\""), "{name}:\n{code}");
        assert!(!code.contains("token:null"), "{name}:\n{code}");
    }
}

const SCHEMAS_MODULE: &str = "
import { NgModule, NO_ERRORS_SCHEMA, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';
@NgModule({ schemas: [NO_ERRORS_SCHEMA, CUSTOM_ELEMENTS_SCHEMA] })
export class M {}
";

#[test]
fn ng_module_schemas_are_left_out_of_the_full_definition() {
    let code = compile(SCHEMAS_MODULE);
    assert_contains(&code, "i0.ɵɵdefineNgModule({type:M})");
    // The class metadata still lists them.
    assert_contains(&code, "args:[{schemas:[NO_ERRORS_SCHEMA,CUSTOM_ELEMENTS_SCHEMA]}]");
}

#[test]
fn ng_module_schemas_are_left_out_of_the_partial_declaration() {
    let code = compile_partial(SCHEMAS_MODULE);
    assert_contains(
        &code,
        "ɵɵngDeclareNgModule({minVersion:\"14.0.0\",version:\"22.2.3\",ngImport:i0,type:M})",
    );
    assert_contains(&code, "args:[{schemas:[NO_ERRORS_SCHEMA,CUSTOM_ELEMENTS_SCHEMA]}]");
}

#[test]
fn directive_query_predicates_are_pooled_before_host_binding_constants() {
    let code = compile(
        "
import { Directive, ViewChild, ContentChild } from '@angular/core';
@Directive({ selector: '[d]', host: { '[style]': '{color: c, width: w}', '[class]': '[c, w]' } })
export class D {
  c = 'red'; w = 1;
  @ViewChild('test1') a: any;
  @ContentChild('c1') c1: any;
}
",
    );
    // Content queries first, then view queries, then the host pure functions.
    assert_contains(
        &code,
        "const_c0=[\"c1\"];const_c1=[\"test1\"];const_c2=(a0,a1)=>({color:a0,width:a1});const_c3=(a0,a1)=>[a0,a1];",
    );
    assert_contains(&code, "i0.ɵɵcontentQuery(dirIndex,_c0,5)");
    assert_contains(&code, "i0.ɵɵviewQuery(_c1,5)");
    assert_contains(&code, "i0.ɵɵpureFunction2(4,_c2,ctx.c,ctx.w)");
    assert_contains(&code, "i0.ɵɵpureFunction2(7,_c3,ctx.c,ctx.w)");
}

#[test]
fn directive_query_predicates_continue_the_file_numbering() {
    let code = compile(
        "
import { Directive, ViewChild, ViewChildren, ContentChildren } from '@angular/core';
@Directive({ selector: '[d1]', host: { '[style]': '{color: c}' } })
export class D1 {
  c = 'red';
  @ViewChild('v1') a: any;
  @ContentChildren('k1,k2') b: any;
}
@Directive({ selector: '[d2]' })
export class D2 {
  @ViewChildren('v2') a: any;
}
",
    );
    assert_contains(
        &code,
        "const_c0=[\"k1\",\"k2\"];const_c1=[\"v1\"];const_c2=(a0)=>({color:a0});",
    );
    assert_contains(&code, "const_c3=[\"v2\"];");
    assert_contains(&code, "i0.ɵɵcontentQuery(dirIndex,_c0,4)");
    assert_contains(&code, "i0.ɵɵviewQuery(_c1,5)");
    assert_contains(&code, "i0.ɵɵviewQuery(_c3,5)");
}

const DEFAULT_IMPORT: &str = "
import { Injectable, Component, Inject } from '@angular/core';
import Foo from './foo';
@Injectable()
export class S { constructor(f: Foo, @Inject(Foo) g: Foo) {} }
@Component({ selector: 'c', template: '' })
export class C { constructor(f: Foo) {} }
";

#[test]
fn default_imported_token_keeps_its_local_name() {
    for mode in [CompilationMode::Full, CompilationMode::Partial] {
        for code in [
            compile_with(
                DEFAULT_IMPORT,
                &TransformOptions { compilation_mode: mode, ..Default::default() },
            ),
            compile_without_metadata(DEFAULT_IMPORT, mode),
        ] {
            assert_contains(&code, "importFoofrom'./foo';");
            assert_not_contains(&code, "i1.default");
            assert_not_contains(&code, "import*asi1");
            if mode == CompilationMode::Full {
                assert_contains(&code, "(i0.ɵɵinject(Foo),i0.ɵɵinject(Foo))");
                assert_contains(&code, "(i0.ɵɵdirectiveInject(Foo))");
            } else {
                assert_contains(&code, "type:S,deps:[{token:Foo},{token:Foo}]");
                assert_contains(&code, "type:C,deps:[{token:Foo}]");
            }
        }
    }
}

const DEFAULT_IMPORT_OVERRIDDEN: &str = "
import { Injectable, Directive, Component, Inject, Attribute, Optional } from '@angular/core';
import Foo from './foo';
import Bar from './bar';
import { TOKEN } from './tok';
@Injectable()
export class S { constructor(@Inject(TOKEN) f: Foo, @Optional() b: Bar) {} }
@Directive({ selector: '[x]' })
export class D { constructor(@Attribute('x') f: Foo) {} }
@Component({ selector: 'c', template: '' })
export class C { constructor(@Optional() @Inject(TOKEN) f: Foo) {} }
";

#[test]
fn default_import_overridden_by_inject_or_attribute_is_elided_without_metadata() {
    // `@Inject` / `@Attribute` replace the type as the factory's token, so
    // without class metadata nothing names `Foo` and ngtsc drops its import.
    // `@Optional()` keeps the type as the token.
    for mode in [CompilationMode::Full, CompilationMode::Partial] {
        let code = compile_without_metadata(DEFAULT_IMPORT_OVERRIDDEN, mode);
        assert_not_contains(&code, "importFoofrom");
        assert_contains(&code, "importBarfrom'./bar';");
        assert_contains(&code, "import{TOKEN}from'./tok';");
        if mode == CompilationMode::Full {
            assert_contains(&code, "(i0.ɵɵinject(TOKEN),i0.ɵɵinject(Bar,8))");
            assert_contains(&code, "(i0.ɵɵinjectAttribute(\"x\"))");
        } else {
            assert_contains(&code, "type:S,deps:[{token:TOKEN},{token:Bar,optional:true}]");
        }

        // The metadata's `ctorParameters` still names it (`type: Foo`).
        let code = compile_with(
            DEFAULT_IMPORT_OVERRIDDEN,
            &TransformOptions { compilation_mode: mode, ..Default::default() },
        );
        assert_contains(&code, "importFoofrom'./foo';");
    }
}

const INJECT_TOKENS: &str = "
import { Injectable, Component, Inject, Optional, DOCUMENT } from '@angular/core';
import { TOKEN, Svc } from './tok';
import { TOKEN as TK } from './tok';
import * as t from './tok';
@Injectable()
export class S {
  constructor(@Inject(TOKEN) a: string, @Inject(DOCUMENT) b: Document, c: Svc, @Optional() @Inject(TK) d: any, @Inject(t.TOKEN) e: any) {}
}
@Component({ selector: 'c', template: '' })
export class C { constructor(@Inject(TOKEN) a: string, @Inject(DOCUMENT) b: any) {} }
";

#[test]
fn inject_token_is_kept_as_written() {
    for mode in [CompilationMode::Full, CompilationMode::Partial] {
        for code in [
            compile_with(
                INJECT_TOKENS,
                &TransformOptions { compilation_mode: mode, ..Default::default() },
            ),
            compile_without_metadata(INJECT_TOKENS, mode),
        ] {
            // The imports the factory names stay; only the type token is namespaced.
            assert_contains(&code, "import{TOKEN}from");
            assert_contains(&code, "import{TOKENasTK}from'./tok';");
            assert_contains(&code, "import*astfrom'./tok';");
            assert_contains(&code, "DOCUMENT}from");
            assert_not_contains(&code, "i1.TOKEN");
            assert_not_contains(&code, "i0.DOCUMENT");
            if mode == CompilationMode::Full {
                assert_contains(
                    &code,
                    "(i0.ɵɵinject(TOKEN),i0.ɵɵinject(DOCUMENT),i0.ɵɵinject(i1.Svc),i0.ɵɵinject(TK,8),i0.ɵɵinject(t.TOKEN))",
                );
                assert_contains(
                    &code,
                    "(i0.ɵɵdirectiveInject(TOKEN),i0.ɵɵdirectiveInject(DOCUMENT))",
                );
            } else {
                assert_contains(
                    &code,
                    "deps:[{token:TOKEN},{token:DOCUMENT},{token:i1.Svc},{token:TK,optional:true},{token:t.TOKEN}]",
                );
                assert_contains(&code, "type:C,deps:[{token:TOKEN},{token:DOCUMENT}]");
            }
        }
    }
}

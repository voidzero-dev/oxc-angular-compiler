//! Without class metadata (`advanced_optimizations`), import elision drops an
//! import used only in decorators compilation removes. It must keep every
//! import the output still references: decorators the transform leaves in
//! place, and decorator arguments it copies into the factory or definition by
//! their local name.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

fn compile(source: &str) -> String {
    compile_checked(source, true)
}

fn compile_checked(source: &str, no_errors: bool) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions {
        advanced_optimizations: true,
        emit_class_metadata: false,
        ..TransformOptions::default()
    };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!no_errors || !result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

/// Whether a named import binds `name` in `code` (namespace imports aside).
fn imports(code: &str, name: &str) -> bool {
    code.lines().filter(|l| l.starts_with("import") && !l.contains("* as")).any(|l| {
        l.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).any(|w| w == name)
    })
}

/// Whether `code` names `name` outside its import lines (not as a member, like `i1.name`).
fn uses(code: &str, name: &str) -> bool {
    code.lines().filter(|l| !l.starts_with("import")).any(|line| {
        line.match_indices(name).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + name.len()..].chars().next();
            let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
            !before.is_some_and(|c| ident(c) || c == '.') && !after.is_some_and(ident)
        })
    })
}

/// Each name stays imported, and the output still names it: no import is kept for nothing.
fn assert_kept(source: &str, names: &[&str]) {
    let code = compile(source);
    for name in names {
        assert!(imports(&code, name), "`{name}` must stay imported:\n{code}");
        assert!(uses(&code, name), "`{name}` is kept but the output doesn't name it:\n{code}");
    }
}

#[test]
fn plain_inject_token_is_kept_and_decorators_are_elided() {
    // Like ngtsc, the factory names the token as written (`ɵɵinject(TOKEN, 8)`).
    let code = compile(
        r#"
import { Injectable, Inject, Optional } from "@angular/core";
import { TOKEN } from "./tokens";
@Injectable()
export class B { constructor(@Optional() @Inject(TOKEN) v: unknown) {} }
"#,
    );
    assert!(imports(&code, "TOKEN"), "{code}");
    assert!(!imports(&code, "Inject"), "{code}");
    assert!(!imports(&code, "Optional"), "{code}");
    assert!(code.contains("ɵɵinject(TOKEN,8)"), "{code}");
    assert!(!code.contains("i1.TOKEN"), "{code}");
}

#[test]
fn inject_forward_ref_keeps_token() {
    // The factory copies `forwardRef(() => TOKEN)` as written.
    assert_kept(
        r#"
import { Injectable, Inject, forwardRef } from "@angular/core";
import { TOKEN } from "./tokens";
@Injectable()
export class A { constructor(@Inject(forwardRef(() => TOKEN)) v: unknown) {} }
@Injectable()
export class B { constructor(@Inject(TOKEN) v: unknown) {} }
"#,
        &["TOKEN", "forwardRef"],
    );
}

#[test]
fn inject_member_expression_keeps_token() {
    // The factory copies `TOKEN.sub` as written, like ngtsc.
    for arg in ["TOKEN.sub", "TOKEN?.a.b"] {
        let code = compile(&format!(
            r#"
import {{ Injectable, Inject }} from "@angular/core";
import {{ TOKEN }} from "./tokens";
@Injectable() export class S {{ constructor(@Inject({arg}) private v: unknown) {{}} }}
"#
        ));
        assert!(imports(&code, "TOKEN"), "{code}");
        assert!(code.contains(&format!("ɵɵinject({arg})")), "{code}");
        assert!(!code.contains("i1."), "{code}");
    }
}

#[test]
fn inject_computed_member_keeps_object() {
    // The factory copies `TOKEN['sub']` as written, with the local name.
    assert_kept(
        r#"
import { Injectable, Inject } from "@angular/core";
import { TOKEN } from "./tokens";
@Injectable() export class S { constructor(@Inject(TOKEN['sub']) private v: unknown) {} }
"#,
        &["TOKEN"],
    );
}

#[test]
fn non_angular_decorator_on_declare_field_keeps_its_references() {
    assert_kept(
        r#"
import { Injectable, Inject } from "@angular/core";
import { TOKEN } from "./tokens";
@Injectable()
export class A { constructor(@Inject(TOKEN) v: unknown) {} }
export class O { @Dec(TOKEN) declare p: string; }
function Dec(..._a: any[]): any { return () => {}; }
"#,
        &["TOKEN"],
    );
    assert_kept(
        r#"
import { Injectable } from "@angular/core";
import { TOKEN } from "./tokens";
@Injectable() export class A { @Dec(TOKEN) declare p: string; }
function Dec(..._a: any[]): any { return () => {}; }
"#,
        &["TOKEN"],
    );
}

#[test]
fn param_decorators_of_a_class_that_is_not_compiled_stay() {
    for class in [
        "export class P { constructor(@Inject(TOKEN) private v: unknown) {} }",
        "@Foo() export class P { constructor(@Inject(TOKEN) private v: unknown) {} }",
    ] {
        assert_kept(
            &format!(
                r#"
import {{ Injectable, Inject }} from "@angular/core";
import {{ TOKEN }} from "./tokens";
declare function Foo(): any;
@Injectable() export class S {{ constructor(@Inject(TOKEN) private v: unknown) {{}} }}
{class}
"#
            ),
            &["Inject", "TOKEN"],
        );
    }
}

#[test]
fn param_decorators_of_a_component_whose_template_is_unresolved_stay() {
    // The template is not resolved, so the component is emitted as written, without its definition (an error
    // is reported too).
    let code = compile_checked(
        r#"
import { Component, Inject, LOCALE_ID } from "@angular/core";
@Component({ selector: "app-root", templateUrl: "./app.component.html" })
export class AppComponent { constructor(@Inject(LOCALE_ID) public locale: string) {} }
"#,
        false,
    );
    assert!(code.contains("@Inject(LOCALE_ID)"), "{code}");
    for name in ["Inject", "LOCALE_ID"] {
        assert!(imports(&code, name), "`{name}` must stay imported:\n{code}");
    }
}

#[test]
fn ctor_overload_implementation_param_decorators_are_elided() {
    // Angular compliance case r3_view_compiler_di/di/ctor_overload.ts: the factory reads the
    // implementation, so its `@Optional()` is compiled into the flags and stripped.
    let code = compile(
        r#"
import {Injectable, Optional} from '@angular/core';
class MyDependency {}
class MyOptionalDependency {}
@Injectable()
export class MyService {
  constructor(dep: MyDependency);
  constructor(dep: MyDependency, @Optional() optionalDep?: MyOptionalDependency) {}
}
"#,
    );
    assert!(!imports(&code, "Optional"), "{code}");
    assert!(!uses(&code, "Optional"), "{code}");
    let flat: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(flat.contains("ɵɵinject(MyOptionalDependency,8)"), "{code}");
}

#[test]
fn decorators_on_declare_fields_that_stay_keep_their_imports() {
    // A non-Angular decorator on a compiled class.
    assert_kept(
        r#"
import { Component } from "@angular/core";
import { TOKEN } from "./tokens";
declare function Foo(x: any): any;
@Component({selector:"a",template:""}) export class C { @Foo(TOKEN) declare x: string; }
"#,
        &["TOKEN"],
    );
    // An Angular decorator on a class that is not compiled.
    assert_kept(
        r#"
import { Inject } from "@angular/core";
import { TOKEN } from "./tokens";
export class Q { @Inject(TOKEN) declare z: string; }
"#,
        &["Inject", "TOKEN"],
    );
}

#[test]
fn declare_query_predicate_keeps_import() {
    // Compiled into `ɵɵviewQuery(Foo, 5)`.
    let source = r#"
import { Component, ViewChild } from "@angular/core";
import { Foo } from "./foo";
@Component({ selector: "x", template: "" })
export class C {
    @ViewChild(Foo, { static: true }) declare q: Foo;
}
"#;
    assert_kept(source, &["Foo"]);
    let code = compile(source);
    assert!(code.contains("viewQuery(Foo"), "{code}");
    // The decorator itself is gone.
    assert!(!imports(&code, "ViewChild"), "{code}");
}

#[test]
fn abstract_base_param_decorators_stay() {
    assert_kept(
        r#"
import { Injectable, Inject } from "@angular/core";
import { TOKEN } from "./tokens";
export abstract class Base {
    constructor(@Inject(TOKEN) protected value: unknown) {}
}
@Injectable()
export class TestService {
    constructor(@Inject(TOKEN) private value: unknown) {}
}
"#,
        &["Inject", "TOKEN"],
    );
}

#[test]
fn declare_input_transform_keeps_import() {
    // Compiled into `inputs: {disabled: [2, "disabled", "disabled", booleanAttribute]}`.
    assert_kept(
        r#"
import { Component, Input, booleanAttribute } from "@angular/core";
@Component({selector:'x', template:''})
export class A { @Input({ transform: booleanAttribute }) declare disabled: boolean; }
"#,
        &["booleanAttribute"],
    );
    assert_kept(
        r#"
import { Component, Input } from "@angular/core";
import { toNum } from "./t";
@Component({selector:'x', template:''})
export class A { @Input({ transform: toNum }) declare v: number; }
"#,
        &["toNum"],
    );
}

#[test]
fn declare_query_read_option_keeps_import() {
    // `read` is compiled into the query call by its local name, like the predicate.
    assert_kept(
        r#"
import { Component, ContentChildren, QueryList } from "@angular/core";
import { Foo, Bar } from "./foo";
@Component({ selector: "x", template: "" })
export class C { @ContentChildren(Foo, { read: Bar }) declare items: QueryList<Bar>; }
"#,
        &["Foo", "Bar"],
    );
    assert_kept(
        r#"
import { Component, ViewChild, ViewContainerRef } from "@angular/core";
@Component({ selector: "x", template: "<i #ref></i>" })
export class C { @ViewChild("ref", { read: ViewContainerRef }) declare ref: ViewContainerRef; }
"#,
        &["ViewContainerRef"],
    );
}

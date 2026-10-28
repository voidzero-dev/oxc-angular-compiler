//! A constructor parameter typed with a qualified name (`ns.Foo`, `ns.one.One`)
//! is injected through the value that name refers to, as ngtsc's `typeToValue`
//! resolves it: a namespace import's members read off the emitted namespace
//! import (`i1.Zero`, `i1.one.One`), a named import's off its export
//! (`i1.ns.Zero`), and a local namespace's as written. The cases port
//! `ngtsc_spec.ts`: "should generate correct imports for type references to
//! namespaced symbols using a namespace import" / "... using named imports".

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

fn assert_contains(code: &str, expected: &str) {
    assert!(code.contains(expected), "Expected `{expected}` in:\n{code}");
}

#[test]
fn namespace_import_members_are_injected_off_the_namespace() {
    let code = compile(
        r"
import { Injectable } from '@angular/core';
import * as ns from 'ns';
@Injectable()
export class MyService {
  constructor(zero: ns.Zero, one: ns.one.One, two: ns.one.two.Two) {}
}
",
    );
    assert_contains(&code, "import*asi1from'ns';");
    assert_contains(
        &code,
        "(i0.ɵɵinject(i1.Zero),i0.ɵɵinject(i1.one.One),i0.ɵɵinject(i1.one.two.Two))",
    );
    assert!(!code.contains("ɵɵinvalidFactoryDep"), "Got:\n{code}");
}

#[test]
fn named_import_members_are_injected_off_the_export() {
    let code = compile(
        r"
import { Injectable } from '@angular/core';
import { ns } from 'ns';
import { ns as alias } from 'ns';
@Injectable()
export class MyService {
  constructor(
    zero: ns.Zero,
    one: ns.one.One,
    two: ns.one.two.Two,
    aliasedZero: alias.Zero,
    aliasedOne: alias.one.One,
    aliasedTwo: alias.one.two.Two,
  ) {}
}
",
    );
    assert_contains(&code, "import*asi1from'ns';");
    assert_contains(
        &code,
        "i0.ɵɵinject(i1.ns.Zero),i0.ɵɵinject(i1.ns.one.One),i0.ɵɵinject(i1.ns.one.two.Two),\
         i0.ɵɵinject(i1.ns.Zero),i0.ɵɵinject(i1.ns.one.One),i0.ɵɵinject(i1.ns.one.two.Two)",
    );
}

#[test]
fn local_namespace_members_are_injected_as_written() {
    let code = compile(
        r"
import { Injectable } from '@angular/core';
namespace local { export class L {} }
@Injectable()
export class MyService {
  constructor(l: local.L) {}
}
",
    );
    assert_contains(&code, "(i0.ɵɵinject(local.L))");
}

/// Every decorator compiles its factory deps the same way.
#[test]
fn every_decorator_injects_qualified_types() {
    for (decorator, inject) in [
        ("Component({ selector: 'a-cmp', template: '' })", "ɵɵdirectiveInject"),
        ("Directive({ selector: '[aDir]' })", "ɵɵdirectiveInject"),
        ("NgModule({})", "ɵɵinject"),
    ] {
        let name = decorator.split('(').next().unwrap();
        let code = compile(&format!(
            r"
import {{ {name} }} from '@angular/core';
import * as ns from './ns';
import {{ mod }} from './mod';
@{decorator}
export class A {{
  constructor(zero: ns.Zero, one: ns.one.One, b: mod.B | null) {{}}
}}
"
        ));
        assert_contains(
            &code,
            &format!("(i0.{inject}(i1.Zero),i0.{inject}(i1.one.One),i0.{inject}(i2.mod.B))"),
        );
    }

    let code = compile(
        r"
import { Pipe } from '@angular/core';
import * as ns from './ns';
@Pipe({ name: 'a' })
export class APipe {
  constructor(one: ns.one.One) {}
}
",
    );
    assert_contains(&code, "(i0.ɵɵdirectiveInject(i1.one.One,16))");
}

/// Partial compilation declares the token off the emitted namespace import,
/// like ngtsc: import elision drops the user's `import * as ns` / `{ ns }`.
#[test]
fn partial_component_declares_the_qualified_token() {
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    for (import, token) in [("* as ns", "{token:i1.one.One}"), ("{ ns }", "{token:i1.ns.one.One}")]
    {
        let code = compile_with(
            &format!(
                r"
import {{ Component }} from '@angular/core';
import {import} from './ns';
@Component({{ selector: 'a-cmp', template: '' }})
export class ACmp {{
  constructor(one: ns.one.One) {{}}
}}
"
            ),
            &options,
        );
        assert_contains(&code, "import*asi1from'./ns';");
        assert_contains(&code, &format!("deps:[{token}]"));
    }
}

/// A component's `@Inject(TOKEN.sub)` is injected as written, not the parameter type.
#[test]
fn component_inject_member_chain_wins_over_the_type() {
    let code = compile(
        r"
import { Component, Inject } from '@angular/core';
import * as ns from './ns';
import { TOKEN } from './tokens';
@Component({ selector: 'a-cmp', template: '' })
export class ACmp {
  constructor(@Inject(TOKEN.a.b) a: ns.Zero, @Inject(ns.TOKEN) b: unknown) {}
}
",
    );
    assert_contains(&code, "(i0.ɵɵdirectiveInject(TOKEN.a.b),i0.ɵɵdirectiveInject(ns.TOKEN))");
}

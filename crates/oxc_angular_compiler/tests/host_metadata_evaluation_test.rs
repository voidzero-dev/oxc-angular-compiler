//! `host` in `@Component` / `@Directive` metadata is evaluated statically, as ngtsc's
//! `evaluateHostExpressionBindings` does: a spread of an object, or a host that is itself
//! a constant, compiles to the same bindings as the entries written out in a literal.
//! A host that cannot be used is an error on the host expression, never a silent drop.
//!
//! Every expectation is what `@angular/compiler-cli` 22.2.1 produces for the same source:
//! for the equivalence cases its definitions are identical for both forms, and the error
//! messages are its diagnostics (with the chained line joined by a space, as elsewhere).

use oxc_allocator::Allocator;
use oxc_angular_compiler::{AngularVersion, TransformOptions, transform_angular_file};
use oxc_diagnostics::OxcDiagnostic;

const MEMBERS: &str = "go() {} other() {} t = ''; on = true; w = '';";

fn source(decorator: &str, preamble: &str, host: &str) -> String {
    let rest =
        if decorator == "Component" { "selector: 'x', template: ''," } else { "selector: '[x]'," };
    format!(
        "import {{ {decorator} }} from '@angular/core';\n{preamble}\n\
         @{decorator}({{ {rest} host: {host} }})\nexport class X {{ {MEMBERS} }}\n"
    )
}

fn compile(source: &str) -> (String, Vec<OxcDiagnostic>) {
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "/x/a.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    (result.code, result.diagnostics)
}

/// The whitespace-free host part of the compiled definition: `hostAttrs`, `hostVars`
/// and `hostBindings`, or nothing when the class has no host metadata.
fn host_definition(decorator: &str, preamble: &str, host: &str) -> String {
    host_part(&source(decorator, preamble, host))
}

/// The whitespace-free host part of the definition compiled from `source`.
fn host_part(source: &str) -> String {
    let (code, diagnostics) = compile(source);
    assert!(diagnostics.is_empty(), "unexpected diagnostics for `{source}`: {diagnostics:?}");
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let start = compact.find("({type:X,").expect("definition");
    // The definition ends where the class metadata call, which repeats `host`, starts.
    let definition = &compact[start..];
    let definition = &definition[..definition.find("(()=>").unwrap_or(definition.len())];
    let Some(from) = definition.find(",host") else { return String::new() };
    let to = definition[from..].find("decls:").map_or(definition.len(), |i| from + i);
    definition[from..to].to_string()
}

/// `host` compiles to what `literal` compiles to.
#[track_caller]
fn assert_same_as_literal(decorator: &str, preamble: &str, host: &str, literal: &str) {
    let expected = host_definition(decorator, "", literal);
    assert_eq!(
        host_definition(decorator, preamble, host),
        expected,
        "`host: {host}` should compile like `host: {literal}`"
    );
}

/// Compiling `host` reports exactly `message`, on the host expression.
#[track_caller]
fn assert_error(preamble: &str, host: &str, message: &str) {
    for decorator in ["Component", "Directive"] {
        let source = source(decorator, preamble, host);
        let (_, diagnostics) = compile(&source);
        let [diagnostic] = diagnostics.as_slice() else {
            panic!("expected one diagnostic for `{host}`, got {diagnostics:?}");
        };
        assert_eq!(diagnostic.message, message, "for `host: {host}`");
        let labels: Vec<&str> = diagnostic
            .labels
            .iter()
            .map(|l| &source[l.offset() as usize..(l.offset() + l.len()) as usize])
            .collect();
        assert_eq!(labels, [host], "label for `host: {host}`");
    }
}

const NOT_AN_OBJECT: &str = "Decorator host metadata must be an object";
const UNPARSEABLE_VALUE: &str =
    "Decorator host metadata must be a string -> string object, but found unparseable value";

// The two reported inputs.

#[test]
fn spread_inside_the_host_object() {
    let (code, diagnostics) = compile(
        "import { Component } from '@angular/core';
const SHARED = { '(press)': 'go()' };
@Component({
  selector: 'x',
  template: '',
  host: { ...SHARED, accessibilityRole: 'button' },
})
export class X { go() {} }
",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains(r#"hostAttrs:["accessibilityRole","button"]"#), "{compact}");
    assert!(
        compact.contains(
            r#"i0.ɵɵlistener("press",functionX_press_HostBindingHandler(){returnctx.go();});"#
        ),
        "{compact}"
    );
}

#[test]
fn host_that_is_a_constant() {
    let (code, diagnostics) = compile(
        "import { Component } from '@angular/core';
const HOST = { '(press)': 'go()' };
@Component({ selector: 'x', template: '', host: HOST })
export class X { go() {} }
",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains(
            r#"i0.ɵɵlistener("press",functionX_press_HostBindingHandler(){returnctx.go();});"#
        ),
        "{compact}"
    );
}

// Spreads.

#[test]
fn spread_before_and_after_other_entries() {
    let shared = "const SHARED = { '(press)': 'go()' };";
    let literal = "{ '(press)': 'go()', accessibilityRole: 'button' }";
    for decorator in ["Component", "Directive"] {
        assert_same_as_literal(
            decorator,
            shared,
            "{ ...SHARED, accessibilityRole: 'button' }",
            literal,
        );
        assert_same_as_literal(
            decorator,
            shared,
            "{ accessibilityRole: 'button', ...SHARED }",
            "{ accessibilityRole: 'button', '(press)': 'go()' }",
        );
    }
}

/// The later entry wins, and keeps the position of the first one with that key.
#[test]
fn later_entry_overrides_an_earlier_one() {
    assert_same_as_literal(
        "Component",
        "const SHARED = { role: 'a', '(press)': 'go()' };",
        "{ ...SHARED, role: 'b', '(press)': 'other()' }",
        "{ role: 'b', '(press)': 'other()' }",
    );
    assert_same_as_literal(
        "Component",
        "const SHARED = { role: 'a' };",
        "{ role: 'b', ...SHARED }",
        "{ role: 'a' }",
    );
    assert_same_as_literal(
        "Component",
        "const SHARED = { role: 'x', id: 'i' };",
        "{ ...SHARED, title: 't', role: 'y' }",
        "{ role: 'y', id: 'i', title: 't' }",
    );
}

#[test]
fn two_spreads_and_a_spread_of_a_spread() {
    assert_same_as_literal(
        "Component",
        "const A = { '(press)': 'go()' }; const B = { '[title]': 't', role: 'x' };",
        "{ ...A, ...B }",
        "{ '(press)': 'go()', '[title]': 't', role: 'x' }",
    );
    assert_same_as_literal(
        "Component",
        "const A = { '(press)': 'go()' }; const B = { ...A, role: 'x' };",
        "{ ...B, id: 'y' }",
        "{ '(press)': 'go()', role: 'x', id: 'y' }",
    );
}

// A host that is a constant.

#[test]
fn constant_host_with_and_without_as_const() {
    for decorator in ["Component", "Directive"] {
        assert_same_as_literal(
            decorator,
            "const HOST = { '(press)': 'go()', role: 'x' };",
            "HOST",
            "{ '(press)': 'go()', role: 'x' }",
        );
        assert_same_as_literal(
            decorator,
            "const HOST = { '(press)': 'go()', role: 'x' } as const;",
            "HOST",
            "{ '(press)': 'go()', role: 'x' }",
        );
    }
}

/// ngtsc evaluates these too; none of them is an error.
#[test]
fn other_statically_resolvable_hosts() {
    let literal = "{ '(press)': 'go()' }";
    for (preamble, host) in [
        ("let HOST = { '(press)': 'go()' };", "HOST"),
        ("var HOST = { '(press)': 'go()' };", "HOST"),
        ("const HOST: Record<string, string> = { '(press)': 'go()' };", "HOST"),
        ("const HOST = { '(press)': 'go()' };", "HOST as any"),
        ("const HOST = { '(press)': 'go()' };", "(HOST)"),
        ("const HOST = { '(press)': 'go()' };", "HOST!"),
        ("const C = { h: { '(press)': 'go()' } };", "C.h"),
        ("class C { static HOST = { '(press)': 'go()' }; }", "C.HOST"),
        ("function make() { return { '(press)': 'go()' }; }", "make()"),
        ("function make() { return { '(press)': 'go()' }; }", "{ ...make() }"),
        ("let SHARED = { '(press)': 'go()' };", "{ ...SHARED }"),
        ("const DEV = true; const A = { '(press)': 'go()' };", "{ ...(DEV ? A : {}) }"),
    ] {
        assert_same_as_literal("Component", preamble, host, literal);
    }
}

// Values.

#[test]
fn entry_values_that_are_string_constants() {
    assert_same_as_literal(
        "Component",
        "const HANDLER = 'go()'; const ROLE = 'button';",
        "{ '(press)': HANDLER, role: ROLE }",
        "{ '(press)': 'go()', role: 'button' }",
    );
    assert_same_as_literal(
        "Component",
        "const KEY = '(press)';",
        "{ [KEY]: 'go()' }",
        "{ '(press)': 'go()' }",
    );
    assert_same_as_literal(
        "Component",
        "const N = 'x'; enum E { A = 'a' } const DEV = true; const role = 'r';",
        "{ id: `a-${N}`, title: E.A, lang: DEV ? 'a' : 'b', role }",
        "{ id: 'a-x', title: 'a', lang: 'a', role: 'r' }",
    );
}

// Every kind of key, through a spread and through a constant.

#[test]
fn every_kind_of_key() {
    let literal = "{ '(click)': 'go()', '[title]': 't', '[class.x]': 'on', \
                   '[style.width]': 'w', '[attr.aria-label]': 't', role: 'button', \
                   class: 'a b', style: 'color: red' }";
    let preamble = format!("const ALL = {literal};");
    for decorator in ["Component", "Directive"] {
        let expected = host_definition(decorator, "", literal);
        for needle in [
            r#"hostAttrs:["role","button",1,"a","b",2,"color","red"]"#,
            "hostVars:6",
            r#"i0.ɵɵlistener("click""#,
            r#"i0.ɵɵdomProperty("title",ctx.t)"#,
            r#"i0.ɵɵattribute("aria-label",ctx.t)"#,
            r#"i0.ɵɵstyleProp("width",ctx.w)"#,
            r#"i0.ɵɵclassProp("x",ctx.on)"#,
        ] {
            assert!(expected.contains(needle), "missing `{needle}` in:\n{expected}");
        }
        assert_same_as_literal(decorator, &preamble, "{ ...ALL }", literal);
        assert_same_as_literal(decorator, &preamble, "ALL", literal);
    }
}

/// A plain literal host compiles exactly as it did before hosts were evaluated.
#[test]
fn literal_host_is_unchanged() {
    assert_eq!(
        host_definition("Component", "", "{ '(press)': 'go()', accessibilityRole: 'button' }"),
        r#",hostAttrs:["accessibilityRole","button"],hostBindings:functionX_HostBindings(rf,ctx){if((rf&1)){i0.ɵɵlistener("press",functionX_press_HostBindingHandler(){returnctx.go();});}},"#
    );
    assert_eq!(host_definition("Component", "", "{}"), "");
    assert_eq!(host_definition("Component", "const HOST = {};", "HOST"), "");
    assert_same_as_literal(
        "Component",
        "",
        "{ role: 'a', id: 'i', role: 'b' }",
        "{ role: 'b', id: 'i' }",
    );
}

// Errors: what ngtsc reports, on the host expression.

#[test]
fn host_that_is_not_an_object() {
    for (preamble, host, suffix) in [
        ("", "['a']", "Value is of type '[string]'."),
        ("", "'a'", "Value is of type 'string'."),
        ("", "null", "Value is of type 'null'."),
        ("", "undefined", "Value is of type 'undefined'."),
        ("declare function make(): any;", "make()", "Value could not be determined statically."),
        (
            "const HOST = Object.freeze({ '(press)': 'go()' });",
            "HOST",
            "Value could not be determined statically.",
        ),
        (
            "const HOST = { '(press)': 'go()' } satisfies Record<string, string>;",
            "HOST",
            "Value could not be determined statically.",
        ),
    ] {
        assert_error(preamble, host, &format!("{NOT_AN_OBJECT} {suffix}"));
    }
}

/// A spread or member ngtsc cannot evaluate makes the whole object unresolvable.
#[test]
fn host_object_that_cannot_be_evaluated() {
    for (preamble, host) in [
        ("declare function make(): any;", "{ ...make(), role: 'x' }"),
        ("", "{ ...NOPE, role: 'x' }"),
        ("const S = 'abc';", "{ ...S }"),
        ("const S = ['a'];", "{ ...S }"),
        ("const K = 1;", "{ [K]: 'a' }"),
        ("", "{ role() { return 'a' } }"),
        ("", "{ get role() { return 'a' } }"),
    ] {
        assert_error(
            preamble,
            host,
            &format!("{NOT_AN_OBJECT} Value could not be determined statically."),
        );
    }
}

#[test]
fn host_value_that_is_not_a_string() {
    for (preamble, host, suffix) in [
        ("", "{ tabindex: 0 }", "Value is of type 'number'."),
        ("", "{ '[tabindex]': 0 }", "Value is of type 'number'."),
        ("", "{ '(click)': 0 }", "Value is of type 'number'."),
        ("", "{ class: 1 }", "Value is of type 'number'."),
        ("const S = { hidden: true };", "{ ...S }", "Value is of type 'boolean'."),
        ("", "{ role: { a: 1 } }", "Value is of type '{ a: number }'."),
        ("", "{ role: null }", "Value is of type 'null'."),
        ("const U = undefined;", "{ role: U }", "Value is of type 'undefined'."),
        ("declare const s: string;", "{ role: s }", "Value is a reference to 's'."),
        ("declare const s: string;", "{ '[title]': s }", "Value is a reference to 's'."),
        // The first unusable value is the one reported.
        ("", "{ a: 1, b: true }", "Value is of type 'number'."),
    ] {
        assert_error(preamble, host, &format!("{UNPARSEABLE_VALUE} {suffix}"));
    }
}

/// A value ngtsc cannot evaluate at all is an error for everything but a plain attribute.
#[test]
fn dynamic_value_of_a_binding() {
    let make = "declare function make(): string;";
    assert_error(make, "{ '[title]': make() }", "Property binding must be string");
    assert_error(make, "{ '(click)': make() }", "Event binding must be string");
    assert_error(make, "{ class: make() }", "Class binding must be string");
    assert_error(make, "{ style: make() }", "Style binding must be string");
}

/// ngtsc emits a plain attribute's dynamic value as it is written, in `hostAttrs`
/// (`["role", make()]`). That includes a value computed from an import whose file has no
/// readable value (ngtsc_spec.ts: "should accept dynamic host attribute bindings"), but not
/// the import itself, see [`host_that_depends_on_an_import`].
#[test]
fn dynamic_value_of_a_plain_attribute() {
    let make = "declare function make(): string;";
    for decorator in ["Component", "Directive"] {
        for (preamble, host, attrs) in [
            (make, "{ role: make() }", r#"hostAttrs:["role",make()]"#),
            ("", "{ role: NOPE }", r#"hostAttrs:["role",NOPE]"#),
            (
                &format!("{make} const S = {{ title: make() }};"),
                "{ ...S, id: 'a' }",
                r#"hostAttrs:["title",make(),"id","a"]"#,
            ),
            (
                "import { SHARED } from './other';",
                "{ role: SHARED + '!' }",
                r#"hostAttrs:["role",(SHARED+"!")]"#,
            ),
            (
                "import * as ns from './other';",
                "{ role: ns.foo.bar }",
                r#"hostAttrs:["role",ns.foo.bar]"#,
            ),
            (
                "import { foo } from './other';\nconst test = foo.bar();",
                "{ 'test': test }",
                r#"hostAttrs:["test",test]"#,
            ),
        ] {
            let definition = host_definition(decorator, preamble, host);
            assert!(definition.contains(attrs), "for `host: {host}`: {definition}");
        }
    }
}

/// A partial declaration carries the expression in `host.attributes`.
#[test]
fn dynamic_value_of_a_plain_attribute_partial() {
    for decorator in ["Component", "Directive"] {
        let source = source(decorator, "declare function make(): string;", "{ role: make() }");
        let allocator = Allocator::default();
        let options = TransformOptions {
            compilation_mode: oxc_angular_compiler::CompilationMode::Partial,
            ..Default::default()
        };
        let result = transform_angular_file(&allocator, "/x/a.ts", &source, Some(&options), None);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let compact: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("host:{attributes:{\"role\":make()}}"), "{compact}");
    }
}

/// A dynamic attribute value with no expression to emit where the metadata is
/// compiled: one that uses the parameters of a function the metadata calls.
#[test]
fn dynamic_value_of_a_plain_attribute_out_of_scope() {
    assert_error(
        "declare function make(): string;\nfunction h(v: string) { return { role: v + make() }; }",
        "h('a')",
        "Decorator host metadata must be a string -> string object, but the value of 'role' \
         could not be determined statically, and OXC can't emit the expression it is written \
         as where the metadata is compiled.",
    );
}

/// ngtsc reads another file to evaluate an import. One file at a time, that is an error
/// that says so. A plain attribute set to the import itself (`SHARED`, `ns.SHARED`) is one
/// too: with no readable value in the other file, ngtsc rejects it in full and local
/// compilation mode alike ("found unparseable value"), unlike a value computed from it
/// (see above).
#[test]
fn host_that_depends_on_an_import() {
    let imported = |subject: &str| {
        format!(
            "{subject} depends on 'SHARED', which is imported from another module. \
             OXC compiles one file at a time and cannot evaluate values from other files."
        )
    };
    let preamble =
        "import { SHARED } from './other';\nimport * as ns from './other';\nconst R = SHARED;";
    for (decorator, subject) in [("Component", "@Component.host"), ("Directive", "@Directive.host")]
    {
        for host in [
            "SHARED",
            "{ ...SHARED, role: 'x' }",
            "{ role: SHARED }",
            "{ role: (SHARED as string) }",
            "{ role: R }",
            "{ role: ns.SHARED }",
            "{ '(click)': SHARED }",
        ] {
            let source = source(decorator, preamble, host);
            let (_, diagnostics) = compile(&source);
            let [diagnostic] = diagnostics.as_slice() else {
                panic!("expected one diagnostic for `{host}`, got {diagnostics:?}");
            };
            assert_eq!(diagnostic.message, imported(subject), "for `host: {host}`");
        }
    }
}

/// ngtsc builds `hostMetadata` by assigning each entry to a plain object, so
/// `__proto__` invokes the setter and never becomes a key — in a literal, in
/// a constant, or coming in through a spread.
#[test]
fn proto_key_is_dropped() {
    for decorator in ["Component", "Directive"] {
        for (preamble, host) in [
            ("", "{ __proto__: 'x', role: 'r' }"),
            ("", "{ '__proto__': 'x', role: 'r' }"),
            ("const P = { __proto__: 'x' };", "{ ...P, role: 'r' }"),
            ("const P = { __proto__: 'x' };", "{ ...P }"),
        ] {
            let (code, diagnostics) = compile(&source(decorator, preamble, host));
            assert!(diagnostics.is_empty(), "diagnostics for `{host}`: {diagnostics:?}");
            let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
            // Only the compiled host counts — `ɵsetClassMetadata` echoes the
            // decorator arguments verbatim, like ngtsc does.
            assert!(
                !compact.contains(r#"hostAttrs:["__proto__""#)
                    && !compact.contains(r#"hostAttrs:[1,"__proto__""#),
                "`__proto__` reached hostAttrs for `{host}`:\n{compact}"
            );
        }
        // The rest of the object is still read.
        let (code, _) = compile(&source(decorator, "", "{ __proto__: 'x', role: 'r' }"));
        let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains(r#""role","r""#), "{compact}");
    }
}

/// A class with `@HostBinding` / `@HostListener` members and, optionally, `host`.
fn member_source(decorator: &str, host: &str, members: &str) -> String {
    let rest =
        if decorator == "Component" { "selector: 'x', template: ''," } else { "selector: '[x]'," };
    let host = if host.is_empty() { String::new() } else { format!("host: {host}") };
    format!(
        "import {{ {decorator}, HostBinding, HostListener }} from '@angular/core';\n\
         @{decorator}({{ {rest} {host} }})\nexport class X {{ {MEMBERS} {members} }}\n"
    )
}

/// A listener or property named exactly `__proto__` is dropped too: ngtsc's
/// `parseHostBindings` writes `listeners['__proto__'] = …` into a plain
/// object, where the prototype setter ignores a string (#687).
#[test]
fn proto_binding_is_dropped() {
    for decorator in ["Component", "Directive"] {
        let host = host_definition(
            decorator,
            "",
            "{ '(__proto__)': 'other()', '(click)': 'go()', '[__proto__]': 'w', '[title]': 't' }",
        );
        assert!(!host.contains("ctx.other("), "{host}");
        assert!(!host.contains(r#"ɵɵdomProperty("__proto__""#), "{host}");
        assert!(host.contains("hostVars:1,"), "{host}");
        assert!(host.contains(r#"ɵɵlistener("click""#), "{host}");
        assert!(host.contains(r#"ɵɵdomProperty("title",ctx.t)"#), "{host}");
    }
}

/// `@HostBinding('__proto__')`, `@HostBinding()` on a member named `__proto__`
/// and `@HostListener('__proto__')` are dropped, like a `host:` entry: ngtsc's
/// `extractHostBindings` assigns them into the same plain object.
#[test]
fn proto_member_bindings_are_dropped() {
    for decorator in ["Component", "Directive"] {
        let host = host_part(&member_source(
            decorator,
            "",
            "@HostBinding('__proto__') a = 1; \
             @HostListener('__proto__', ['$event']) onP(e: any) {} \
             @HostBinding('title') t2 = '';",
        ));
        assert!(!host.contains("ctx.onP("), "{host}");
        assert!(!host.contains(r#"ɵɵdomProperty("__proto__""#), "{host}");
        assert!(host.contains("hostVars:1,"), "{host}");
        assert!(host.contains(r#"ɵɵdomProperty("title",ctx.t2)"#), "{host}");

        for (host, members) in [
            ("", "@HostBinding() __proto__ = 1 as any;"),
            // A `host:` entry and a member with the same name.
            ("{ '(__proto__)': 'go()' }", "@HostListener('__proto__') b() {}"),
            // Only `__proto__` members: no host bindings at all.
            ("", "@HostListener('__proto__') b() {} @HostBinding('__proto__') c = 1;"),
        ] {
            let definition = host_part(&member_source(decorator, host, members));
            assert_eq!(definition, "", "for `{members}`");
        }
    }
}

/// Only the exact name is dropped: ngtsc keeps every other key that contains
/// `__proto__`, each one bound once.
#[test]
fn proto_lookalikes_are_kept() {
    for decorator in ["Component", "Directive"] {
        for (host, members, expected) in [
            ("{ '(window:__proto__)': 'go()' }", "", r#"ɵɵlistener("__proto__",function"#),
            ("{ '(__proto__.x)': 'go()' }", "", r#"ɵɵlistener("__proto__.x",function"#),
            ("{ '[attr.__proto__]': 't' }", "", r#"ɵɵattribute("__proto__",ctx.t)"#),
            ("{ '[class.__proto__]': 't' }", "", r#"ɵɵclassProp("__proto__",ctx.t)"#),
            ("{ '[style.__proto__]': 't' }", "", r#"ɵɵstyleProp("__proto__",ctx.t)"#),
            // `"__proto__ "`, compacted.
            ("{ '[__proto__ ]': 't' }", "", r#"ɵɵdomProperty("__proto__",ctx.t)"#),
            ("", "@HostBinding('[__proto__]') k = 1;", r#"ɵɵdomProperty("[__proto__]",ctx.k)"#),
            ("", "@HostBinding() __proto__x = 2;", r#"ɵɵdomProperty("__proto__x",ctx.__proto__x)"#),
            (
                "",
                "@HostBinding('title') get __proto__() { return ''; }",
                r#"ɵɵdomProperty("title",ctx.__proto__)"#,
            ),
        ] {
            let definition = host_part(&member_source(decorator, host, members));
            assert!(definition.contains(expected), "for `{host}` `{members}`: {definition}");
        }
        let window = host_definition(decorator, "", "{ '(window:__proto__)': 'go()' }");
        assert!(window.contains("ɵɵresolveWindow"), "{window}");
    }
}

/// A dropped key is still checked first: ngtsc rejects a value that is not a
/// string before it assigns the binding.
#[test]
fn proto_binding_with_a_dynamic_value_is_an_error() {
    let make = "declare function make(): string;";
    assert_error(make, "{ '(__proto__)': make() }", "Event binding must be string");
    assert_error(make, "{ '[__proto__]': make() }", "Property binding must be string");
}

/// A `@HostBinding` member is read through `this`, like ngtsc's
/// `getSafePropertyAccessString('this', member.name)`: a name that isn't an
/// identifier, or is a keyword, still reads the member (#690). The expected
/// calls are what ngtsc 22.2.2 emits.
#[test]
fn host_binding_member_is_read_through_this() {
    for decorator in ["Component", "Directive"] {
        let host = host_part(&member_source(
            decorator,
            "",
            "@HostBinding('id') 'my-prop' = 1; @HostBinding('title') m = 2; \
             @HostBinding('attr.t') true = 4; @HostBinding('attr.u') this = 5; \
             @HostBinding('attr.v') \"it's\" = 6; @HostBinding('attr.w') 'a.b' = 7; \
             @HostBinding('attr.x') 'a{{b}}' = 8; @HostBinding('attr.y') $ok = 9;",
        ));
        assert!(host.contains(r#"ɵɵdomProperty("id",ctx["my-prop"])("title",ctx.m)"#), "{host}");
        assert!(
            host.contains(
                r#"ɵɵattribute("t",ctx.true)("u",ctx.this)("v",ctx["it's"])("w",ctx["a.b"])("x",ctx["a{{b}}"])("y",ctx.$ok)"#
            ),
            "{host}"
        );
    }
}

/// A computed `host` key isn't metadata: ngtsc's `reflectObjectLiteral`
/// returns null for a ComputedPropertyName, and the error check doesn't see
/// the property either — applying it here would silently drop an invalid
/// host, or apply a valid one ngtsc ignores.
#[test]
fn computed_host_key_is_not_metadata() {
    for decorator in ["Component", "Directive"] {
        for (preamble, property) in [
            // A valid host ngtsc never reads — and an invalid one that must
            // not error, since the property is ignored like upstream.
            ("", "['host']: { role: 'r' }"),
            ("", "['host']: { role: 1 }"),
            ("const HOST = 'host';", "[HOST]: { role: 'r' }"),
        ] {
            let rest = if decorator == "Component" {
                "selector: 'x', template: '',"
            } else {
                "selector: '[x]',"
            };
            let src = format!(
                "import {{ {decorator} }} from '@angular/core';\n{preamble}\n\
                 @{decorator}({{ {rest} {property} }})\nexport class X {{ {MEMBERS} }}\n"
            );
            let (code, diagnostics) = compile(&src);
            assert!(
                diagnostics.is_empty(),
                "unexpected diagnostics for `{property}`: {diagnostics:?}"
            );
            assert!(!code.contains("hostAttrs"), "no host output for `{property}`");
        }
    }
}

/// `verifyHostBindings` forbids binding a host property or attribute to an
/// event; the errors are ngtsc's HOST_BINDING_PARSE_ERROR messages
/// (compiler.ts's `validateNoEventBindings`).
#[test]
fn binding_to_an_event_is_disallowed() {
    for (host, message) in [
        (
            "{ '[onclick]': 't' }",
            "Binding to event property 'onclick' is disallowed for security reasons, \
             please use (click)=...\nIf 'onclick' is a directive input, make sure the \
             directive is imported by the current module.",
        ),
        (
            "{ '[attr.onload]': 't' }",
            "Binding to event attribute 'onload' is disallowed for security reasons, \
             please use (load)=...",
        ),
        (
            "{ '[onClick]': 't' }",
            "Binding to event property 'onClick' is disallowed for security reasons, \
             please use (Click)=...\nIf 'onClick' is a directive input, make sure the \
             directive is imported by the current module.",
        ),
    ] {
        assert_error("", host, message);
    }
    // Only property keys are checked: '(onx)' is a listener, 'onx' an attribute.
    let literal = "{ '(onx)': 'go()', onx: 'a' }";
    assert_same_as_literal("Component", "const H = { '(onx)': 'go()', onx: 'a' };", "H", literal);
}

/// #704: `verifyHostBindings` reports every bad key, and ngtsc 22.2.4 joins
/// the messages with "\n" in one NG5001.
#[test]
fn every_event_binding_is_reported() {
    assert_error(
        "",
        "{ '[onclick]': 't', '[attr.onload]': 't' }",
        "Binding to event property 'onclick' is disallowed for security reasons, please use \
         (click)=...\nIf 'onclick' is a directive input, make sure the directive is imported by \
         the current module.\nBinding to event attribute 'onload' is disallowed for security \
         reasons, please use (load)=...",
    );
}

/// #704: a `()` or `[]` key has no name. ngtsc 22.2.4 reports the listeners
/// first, then the properties, then the event bindings, whatever the order
/// of the keys.
#[test]
fn empty_binding_name_is_an_error() {
    const EVENT: &str = "Event name is missing in binding";
    const PROPERTY: &str = "Property name is missing in binding";
    const ONCLICK: &str = "Binding to event property 'onclick' is disallowed for security \
                           reasons, please use (click)=...\nIf 'onclick' is a directive input, \
                           make sure the directive is imported by the current module.";
    assert_error("", "{ '()': 'go()' }", EVENT);
    assert_error("", "{ '[]': 't' }", PROPERTY);
    assert_error("", "{ '[]': 't', '()': 'go()' }", &format!("{EVENT}\n{PROPERTY}"));
    assert_error("", "{ '[]': 't', '[onclick]': 't' }", &format!("{PROPERTY}\n{ONCLICK}"));
    assert_error("", "{ '[onclick]': 't', '[]': 't' }", &format!("{PROPERTY}\n{ONCLICK}"));
    // An evaluated host: a constant, a spread, a key given twice.
    assert_error("const H = { '[]': 't' };", "H", PROPERTY);
    assert_error(
        "const S = { '()': 'go()' };",
        "{ ...S, '[]': 't' }",
        &format!("{EVENT}\n{PROPERTY}"),
    );
    assert_error(
        "const S = { '[]': 't' };",
        "{ '[]': 'w', ...S, '()': 'go()' }",
        &format!("{EVENT}\n{PROPERTY}"),
    );
}

/// The `[]` / `()` checks came with 21.2.8 and 22.0.0-next.7. Before them
/// ngtsc's `HOST_REG_EXP` needs a name, `[]` and `()` are plain host
/// attributes and nothing is reported (ngtsc 19.2.15, 20.1.8, 20.2.0;
/// `parseHostBindings` + `verifyHostBindings` of 21.2.7 and 22.0.0-next.6).
/// The event-binding check runs on every version.
#[test]
fn empty_binding_name_is_reported_from_21_2_8_and_22_0_0_next_7() {
    use oxc_angular_compiler::Prerelease;
    let next = |n| AngularVersion::new(22, 0, 0).with_prerelease(Prerelease::Next(n));
    let source = source("Directive", "", "{ '[]': 't', '()': 'go()' }");
    let errors = |version: AngularVersion| -> Vec<String> {
        let allocator = Allocator::default();
        let options =
            TransformOptions { angular_version: Some(version), ..TransformOptions::default() };
        let result = transform_angular_file(&allocator, "/x/a.ts", &source, Some(&options), None);
        result.diagnostics.into_iter().map(|d| d.message.to_string()).collect()
    };
    let both = vec!["Event name is missing in binding\nProperty name is missing in binding"];
    for version in [
        AngularVersion::new(19, 2, 15),
        AngularVersion::new(20, 3, 35),
        AngularVersion::new(21, 0, 9),
        AngularVersion::new(21, 2, 7),
        next(6),
    ] {
        assert!(errors(version).is_empty(), "{version:?}");
    }
    for version in [AngularVersion::new(21, 2, 8), next(7), AngularVersion::new(22, 0, 0)] {
        assert_eq!(errors(version), both, "{version:?}");
    }

    let source = source.replace("'()': 'go()'", "'[onclick]': 't'");
    let allocator = Allocator::default();
    let options = TransformOptions {
        angular_version: Some(AngularVersion::new(21, 2, 7)),
        ..TransformOptions::default()
    };
    let result = transform_angular_file(&allocator, "/x/a.ts", &source, Some(&options), None);
    let messages: Vec<String> = result.diagnostics.iter().map(|d| d.message.to_string()).collect();
    assert_eq!(
        messages,
        vec![
            "Binding to event property 'onclick' is disallowed for security reasons, please use \
             (click)=...\nIf 'onclick' is a directive input, make sure the directive is imported \
             by the current module."
        ]
    );
}

/// Only the name inside the brackets is checked, so `[ ]` and `( )` compile
/// (ngtsc 22.2.4 with `typeCheckHostBindings: false`; its host type-check
/// reports NG8002 for `' '` otherwise, which oxc doesn't do). The decorators
/// are not checked by NG5001 either: `@HostListener('')` compiles, and
/// `@HostBinding('')` only fails ngtsc's type-check.
#[test]
fn blank_or_decorator_binding_name_is_not_an_error() {
    let source = source("Directive", "", "{ '[ ]': 't', '( )': 'go()' }");
    let (code, diagnostics) = compile(&source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(code.contains(r#"i0.ɵɵlistener(" ",function X___HostBindingHandler()"#), "{code}");
    assert!(code.contains(r#"i0.ɵɵdomProperty(" ",ctx.t)"#), "{code}");

    let code = host_part(
        "import { Directive, HostBinding, HostListener } from '@angular/core';
@Directive({ selector: '[x]' })
export class X { @HostBinding('') a = 1; @HostListener('') b() {} }",
    );
    assert!(
        code.contains(r#"i0.ɵɵlistener("",functionX__HostBindingHandler(){returnctx.b();})"#),
        "{code}"
    );
    assert!(code.contains(r#"i0.ɵɵdomProperty("",ctx.a)"#), "{code}");
}

/// A method or accessor called `host` in the decorator object is a class
/// member, not metadata — ngtsc's `reflectObjectLiteral` skips it, so it must
/// not produce the "must be an object" error (regression: `config_property`
/// matched it by name).
#[test]
fn host_that_is_a_method_or_getter_is_not_metadata() {
    for decorator in ["Component", "Directive"] {
        for host in ["host() { return {}; }", "get host() { return {}; }", "set host(v) {}"] {
            let rest = if decorator == "Component" {
                "selector: 'x', template: '',"
            } else {
                "selector: '[x]',"
            };
            let src = format!(
                "import {{ {decorator} }} from '@angular/core';\n\
                 @{decorator}({{ {rest} {host} }})\nexport class X {{ {MEMBERS} }}\n"
            );
            let (code, diagnostics) = compile(&src);
            assert!(diagnostics.is_empty(), "unexpected diagnostics for `{host}`: {diagnostics:?}");
            assert!(!code.contains("hostAttrs"), "no host output for `{host}`");
        }
    }
}

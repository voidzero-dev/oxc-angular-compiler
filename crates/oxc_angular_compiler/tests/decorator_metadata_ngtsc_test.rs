//! Parity with ngtsc for `inputs:` / `outputs:` / `queries:` decorator metadata
//! and input transforms.
//!
//! `fixtures/decorator_metadata_ngtsc.json` is a snapshot of ngtsc's output
//! (`@angular/compiler-cli` 22.1.7) for each case. Each case's `origin` says
//! where it comes from: an Angular spec (sources taken verbatim from
//! angular/angular) or a probe of ngtsc's partial evaluator and `.d.ts`
//! printer. Cases ngtsc and oxc can't agree on by design carry a `skip`
//! reason (mostly: they need declarations from another file). For every case this
//! compares the diagnostics word for word and, when ngtsc accepts the file,
//! each class's `inputs`/`outputs` maps and query functions (ignoring
//! whitespace and parentheses, which the two emitters place differently) and
//! its `.d.ts` members (ignoring whitespace, whose layout TypeScript's printer
//! derives from source positions).
//!
//! One deliberate difference: when ngtsc types an `ngAcceptInputType_*` with a
//! type from another module (`i1.Foo`, plus an `import * as i1` it adds to the
//! `.d.ts`), oxc writes `unknown`. Aliases numbered per source file can't be
//! merged safely into bundled declaration files; `i0` (`@angular/core`) is
//! always imported, so those types are kept.

use std::collections::{BTreeMap, HashMap};

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, TransformResult, transform_angular_file};
use serde_json::Value;

const FIXTURES: &str = include_str!("fixtures/decorator_metadata_ngtsc.json");

/// `s` without the characters `drop` matches, except inside string literals:
/// `"a b"` stays `"a b"`, so a stray space in an emitted name is a mismatch.
fn strip_outside_strings(s: &str, drop: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if matches!(c, '"' | '\'' | '`') {
            out.push(c);
            while let Some(inner) = chars.next() {
                out.push(inner);
                if inner == '\\' {
                    out.extend(chars.next());
                } else if inner == c {
                    break;
                }
            }
        } else if !drop(c) {
            out.push(c);
        }
    }
    out
}

fn strip(s: &str) -> String {
    strip_outside_strings(s, char::is_whitespace)
}

fn strip_parens(s: &str) -> String {
    strip_outside_strings(s, |c| c == '(' || c == ')')
}

/// A UTF-16 offset (what ngtsc reports) as a byte offset into `source`.
fn byte_offset(source: &str, utf16: u64) -> usize {
    let mut units = 0;
    for (at, c) in source.char_indices() {
        if units >= utf16 {
            return at;
        }
        units += c.len_utf16() as u64;
    }
    source.len()
}

/// The balanced `{...}` / `(...)` / `[...]` starting at byte `open`.
fn balanced(text: &str, open: usize) -> &str {
    let bytes = text.as_bytes();
    let mut stack = Vec::new();
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' => stack.push(b'}'),
            b'(' => stack.push(b')'),
            b'[' => stack.push(b']'),
            c if stack.last() == Some(&c) => {
                stack.pop();
                if stack.is_empty() {
                    return &text[open..=i];
                }
            }
            _ => {}
        }
        i += 1;
    }
    &text[open..]
}

/// ngtsc adds a docs link to a few diagnostics when it formats them; the link
/// depends on the Angular release, so it isn't part of the comparison.
fn without_docs_link(message: &str) -> &str {
    let Some(at) = message.find(" Find more at https://") else { return message };
    message[..at].strip_suffix('.').unwrap_or(&message[..at])
}

/// Per class, what the fixture records: `inputs`/`outputs`/`contentQueries`/
/// `viewQuery` from the JS (whitespace removed, `_cN` constants inlined) and
/// `dts:<member>` from the `.d.ts`.
fn oxc_classes(
    js: &str,
    dts: &[oxc_angular_compiler::dts::DtsDeclaration],
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut consts = BTreeMap::new();
    for (at, _) in js.match_indices("const _c") {
        let rest = &js[at + 6..];
        let Some(eq) = rest.find(" = ") else { continue };
        let name = rest[..eq].to_string();
        consts.insert(name, strip(balanced(rest, eq + 3)));
    }
    let inline = |s: String| {
        let mut out = s;
        // Longest names first so `_c10` isn't replaced as `_c1` + `0`.
        let mut names: Vec<&String> = consts.keys().collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        for name in names {
            out = out.replace(name.as_str(), &consts[name]);
        }
        out
    };

    let mut classes: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for marker in ["ɵɵdefineDirective(", "ɵɵdefineComponent("] {
        for (at, _) in js.match_indices(marker) {
            let def = balanced(js, at + marker.len() - 1);
            let compact = strip(def);
            let Some(name) = compact
                .strip_prefix("({type:")
                .map(|r| r.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap())
            else {
                continue;
            };
            let entry = classes.entry(name.to_string()).or_default();
            for key in ["inputs", "outputs"] {
                if let Some(pos) = compact.find(&format!(",{key}:{{")) {
                    let open = pos + key.len() + 2;
                    entry.insert(key.into(), balanced(&compact, open).to_string());
                }
            }
            for key in ["contentQueries", "viewQuery"] {
                if let Some(pos) = def
                    .find(&format!("{key}: function"))
                    .or_else(|| def.find(&format!("{key}:function")))
                {
                    let params_end = pos + def[pos..].find(')').unwrap();
                    let open = params_end + def[params_end..].find('{').unwrap();
                    entry.insert(key.into(), inline(strip(balanced(def, open))));
                }
            }
        }
    }
    for decl in dts {
        let entry = classes.entry(decl.class_name.clone()).or_default();
        for line in decl.members.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("static ") else { continue };
            let member = rest.split(':').next().unwrap().trim_matches('"');
            if member == "ɵdir" || member == "ɵcmp" || member.starts_with("ngAcceptInputType_") {
                entry.insert(format!("dts:{member}"), strip(line));
            }
        }
    }
    classes
}

/// `staticngAcceptInputType_x:<type>;` becomes `...:unknown;` when `<type>`
/// references a module other than `@angular/core` (see the module docs).
fn accept_type_without_other_modules(key: &str, expected: &str) -> String {
    let other_module = expected.match_indices('i').any(|(at, _)| {
        let before = expected[..at].chars().last();
        let digits: String = expected[at + 1..].chars().take_while(char::is_ascii_digit).collect();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
            && !digits.is_empty()
            && digits != "0"
            && expected[at + 1 + digits.len()..].starts_with('.')
    });
    if key.starts_with("dts:ngAcceptInputType_") && other_module {
        let colon = expected.find(':').unwrap();
        format!("{}:unknown;", &expected[..colon])
    } else {
        expected.to_string()
    }
}

#[test]
fn decorator_metadata_matches_ngtsc() {
    let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
    let mut failures = Vec::new();
    let mut compared = 0;

    for fixture in fixtures["fixtures"].as_array().unwrap() {
        if fixture.get("skip").is_some() {
            continue;
        }
        compared += 1;
        let name = fixture["name"].as_str().unwrap();
        let source = fixture["files"]["test.ts"].as_str().unwrap();
        let allocator = Allocator::default();
        let result = transform_angular_file(
            &allocator,
            "test.ts",
            source,
            Some(&TransformOptions::default()),
            None,
        );

        let expected: Vec<&str> = fixture["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| without_docs_link(d.as_str().unwrap()))
            .collect();
        let actual: Vec<String> = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == oxc_diagnostics::Severity::Error)
            .map(|d| d.message.to_string())
            .collect();
        if actual != expected {
            failures.push(format!(
                "{name}\n  diagnostics\n    ngtsc: {expected:?}\n    oxc:   {actual:?}"
            ));
        }
        // Where each diagnostic points, when the fixture records it.
        if let Some(spans) = fixture.get("diagnosticSpans").and_then(Value::as_array) {
            let expected: Vec<Option<(usize, usize)>> = spans
                .iter()
                .map(|span| {
                    let span = span.as_array()?;
                    let offset = |i: usize| byte_offset(source, span[i].as_u64().unwrap());
                    Some((offset(0), offset(1)))
                })
                .collect();
            let actual: Vec<Option<(usize, usize)>> = result
                .diagnostics
                .iter()
                .filter(|d| d.severity == oxc_diagnostics::Severity::Error)
                .map(|d| {
                    let label = d.labels.first()?;
                    let (offset, len) = (label.offset() as usize, label.len() as usize);
                    Some((offset, offset + len))
                })
                .collect();
            if actual != expected {
                let text = |spans: &[Option<(usize, usize)>]| {
                    spans
                        .iter()
                        .map(|s| s.map(|(start, end)| &source[start..end]))
                        .collect::<Vec<_>>()
                };
                failures.push(format!(
                    "{name}\n  diagnostic spans\n    ngtsc: {:?}\n    oxc:   {:?}",
                    text(&expected),
                    text(&actual)
                ));
            }
        }

        // ngtsc emits nothing for a class it rejects.
        if !expected.is_empty() {
            continue;
        }
        let oxc = oxc_classes(&result.code, &result.dts_declarations);
        for (class, members) in fixture["classes"].as_object().unwrap() {
            for (key, value) in members.as_object().unwrap() {
                let expected = accept_type_without_other_modules(key, value.as_str().unwrap());
                let expected = expected.as_str();
                let actual =
                    oxc.get(class).and_then(|m| m.get(key)).map_or("<missing>", String::as_str);
                let same = if key.starts_with("dts:") {
                    actual == expected
                } else {
                    strip_parens(actual) == strip_parens(expected)
                };
                if !same {
                    failures.push(format!(
                        "{name}\n  {class}.{key}\n    ngtsc: {expected}\n    oxc:   {actual}"
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} mismatches with ngtsc:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert_eq!(compared, 575, "fixtures compared");
}

fn transform(source: &str) -> TransformResult {
    let allocator = Allocator::default();
    transform_angular_file(&allocator, "test.ts", source, Some(&TransformOptions::default()), None)
}

/// Each error's message and the source text it points at.
fn errors(result: &TransformResult, source: &str) -> Vec<(String, String)> {
    result
        .diagnostics
        .iter()
        .filter(|d| d.severity == oxc_diagnostics::Severity::Error)
        .map(|d| {
            let label = d
                .labels
                .first()
                .map_or("", |l| &source[l.offset() as usize..(l.offset() + l.len()) as usize]);
            (d.message.to_string(), label.to_string())
        })
        .collect()
}

/// ngtsc reads a value imported from another file and compiles these (checked
/// with @angular/compiler-cli 22.1.7 and the export in a second file). oxc sees
/// one file, so it can't, and says so rather than guessing: a dropped alias or
/// `required` flag would compile a different binding.
#[test]
fn values_imported_from_another_module_are_reported_as_unreadable() {
    let cases = [
        ("inputs: SHARED", "SHARED", "SHARED", "inputs"),
        ("inputs: [...SHARED, 'x']", "SHARED", "[...SHARED, 'x']", "inputs"),
        ("inputs: [NAME]", "NAME", "[NAME]", "inputs"),
        ("inputs: [ns.NAME]", "NAME", "[ns.NAME]", "inputs"),
        ("inputs: [{name: 'x', alias: ALIAS}]", "ALIAS", "[{name: 'x', alias: ALIAS}]", "inputs"),
        ("inputs: [{name: 'x', required: REQ}]", "REQ", "[{name: 'x', required: REQ}]", "inputs"),
        ("inputs: SHARED.concat(['x'])", "SHARED", "SHARED.concat(['x'])", "inputs"),
        ("inputs: [`${NAME}x`]", "NAME", "[`${NAME}x`]", "inputs"),
        ("inputs: C ? ['a'] : ['b']", "C", "C ? ['a'] : ['b']", "inputs"),
        // `&&` / `||` need an import only when it decides or is the result
        // (the snapshot's `shortCircuit-*` probes are the ones that don't).
        ("inputs: [NAME || 'x']", "NAME", "[NAME || 'x']", "inputs"),
        ("inputs: ['' || NAME]", "NAME", "['' || NAME]", "inputs"),
        ("inputs: ['x' && NAME]", "NAME", "['x' && NAME]", "inputs"),
        ("inputs: 1 ? SHARED : ['x']", "SHARED", "1 ? SHARED : ['x']", "inputs"),
        ("outputs: [0 || NAME]", "NAME", "[0 || NAME]", "outputs"),
        ("inputs: mk()", "mk", "mk()", "inputs"),
        ("outputs: [...OUTS]", "OUTS", "[...OUTS]", "outputs"),
    ];
    for (meta, name, span, field) in cases {
        let source = format!(
            "import {{Directive}} from '@angular/core';
import {{SHARED, NAME, ALIAS, REQ, OUTS, C, mk}} from './shared';
import * as ns from './shared';
@Directive({{selector: '[d]', {meta}}})
export class Dir {{}}
"
        );
        let result = transform(&source);
        let message = format!(
            "@Directive.{field} depends on '{name}', which is imported from another module. \
             OXC compiles one file at a time and cannot evaluate values from other files."
        );
        assert_eq!(errors(&result, &source), vec![(message, span.to_string())], "{meta}");
    }

    // A reference is all a transform needs: nothing to evaluate.
    let source = "import {Directive} from '@angular/core';
import {fn} from './shared';
@Directive({selector: '[d]', inputs: [{name: 'x', transform: fn}]})
export class Dir {}
";
    let result = transform(source);
    assert!(errors(&result, source).is_empty(), "{:?}", errors(&result, source));
    assert!(strip(&result.code).contains(r#"inputs:{x:[2,"x","x",fn]}"#), "{}", result.code);
}

/// The same for an `@Input(...)` argument: ngtsc 22.1.7 reads these from
/// `./shared` (with `export const OPTS = {alias: 'y'}`, it compiles the input
/// as `y`). oxc can't, so it reports them instead of compiling the input
/// without its alias, `required` flag or transform.
#[test]
fn input_decorator_options_imported_from_another_module_are_reported() {
    let cases = [
        ("@Input(OPTS) x: any;", "OPTS", "OPTS"),
        ("@Input((OPTS)) x: any;", "OPTS", "(OPTS)"),
        ("@Input(NAME) x: any;", "NAME", "NAME"),
        ("@Input(ns.OPTS) x: any;", "OPTS", "ns.OPTS"),
        ("@Input({...OPTS}) x: any;", "OPTS", "{...OPTS}"),
        ("@Input({alias: 'z', ...OPTS}) x: any;", "OPTS", "{alias: 'z', ...OPTS}"),
        ("@Input(LOCAL) x: any;", "OPTS", "LOCAL"),
        ("@Input({alias: NAME}) x: any;", "NAME", "{alias: NAME}"),
        ("@Input({required: REQ}) x: any;", "REQ", "{required: REQ}"),
        ("@Input(OPTS) set x(v: any) {}", "OPTS", "OPTS"),
    ];
    for (member, name, span) in cases {
        let source = format!(
            "import {{Directive, Input}} from '@angular/core';
import {{OPTS, NAME, REQ}} from './shared';
import * as ns from './shared';
const LOCAL = OPTS;
@Directive({{selector: '[d]'}})
export class Dir {{
  {member}
}}
"
        );
        let message = format!(
            "@Input depends on '{name}', which is imported from another module. \
             OXC compiles one file at a time and cannot evaluate values from other files."
        );
        assert_eq!(
            errors(&transform(&source), &source),
            vec![(message, span.to_string())],
            "{member}"
        );
    }

    // An imported transform is a reference: nothing to evaluate.
    let source = "import {Directive, Input} from '@angular/core';
import {fn} from './shared';
@Directive({selector: '[d]'})
export class Dir {
  @Input({alias: 'y', transform: fn}) x: any;
}
";
    let result = transform(source);
    assert!(errors(&result, source).is_empty(), "{:?}", errors(&result, source));
    assert!(strip(&result.code).contains(r#"inputs:{x:[2,"y","x",fn]}"#), "{}", result.code);
}

/// A transform returned by a function the metadata calls is emitted where the
/// directive is compiled, outside that function. A parameter becomes the
/// argument it was passed (the snapshot's `scope-*` probes); anything else that
/// uses the function's parameters would mean something else there, or nothing.
/// ngtsc 22.1.7 emits `(v) => v + name` as written (`name` is then
/// `window.name`) and `booleanAttribute` for `o.t` (the name the argument was
/// first given); oxc reports these instead. (`this` and `arguments` there
/// aren't analyzable, so ngtsc rejects those itself: `scope-this`,
/// `scope-arguments`.)
#[test]
fn transforms_using_the_parameters_of_a_called_function_are_reported() {
    let cases = [
        (
            "function make(name: string) { return [{ name, transform: (v: string) => v + name }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(o: any) { return [{ name: 'x', transform: o.t }]; }",
            "inputs: make({ t: booleanAttribute })",
            "@Directive.inputs",
            "make({ t: booleanAttribute })",
        ),
        (
            "function opts(n: string) { return { transform: (v: string) => v + n }; }",
            "",
            "@Input",
            "opts('a')",
        ),
        // Scoped like JavaScript: only a nested non-arrow function has its own
        // `arguments`, and a name declared in a block isn't in scope after it
        // (the snapshot's `scope-shadowed*` probes are the names that are).
        (
            "function make(name: string) { return [{ name, transform: (v: string) => arguments.length }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name, transform: (v: string) => { { const name = v; } return name; } }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name, transform: (v: string) => ({ [name]: v }) }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name, transform: (v: string) => { switch (name) { case 'x': let name = v; return name; } return v; } }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name, transform: function (v: string) { return (() => name)(); } }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        // Types are erased (the snapshot's `scope-typeOnly-*` probes), but the
        // runtime parts next to them aren't: the expression inside `x!`, `as`,
        // `satisfies` and `<T>x`, a parameter's default and a call's arguments.
        (
            "function make(name: string) { return [{ name: 'x', transform: (v: string) => name! }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name: 'x', transform: (v: string) => name as typeof name }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name: 'x', transform: (v: string) => name satisfies string }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name: 'x', transform: (v: string) => <typeof name>name }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function make(name: string) { return [{ name: 'x', transform: (v: string, d: typeof name = name) => v }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
        (
            "function id<T>(v: T) { return v; }
function make(name: string) { return [{ name: 'x', transform: (v: string) => id<typeof name>(name) }]; }",
            "inputs: make('x')",
            "@Directive.inputs",
            "make('x')",
        ),
    ];
    for (helper, meta, subject, span) in cases {
        let member = if meta.is_empty() { "@Input(opts('a')) x: any;" } else { "x: any;" };
        let source = format!(
            "import {{Directive, Input, booleanAttribute}} from '@angular/core';
{helper}
@Directive({{selector: '[d]', {meta}}})
export class Dir {{ {member} }}
"
        );
        let result = transform(&source);
        let message = format!(
            "{subject}: the transform of \"x\" uses a parameter of the function it's \
             written in. OXC can't emit it outside that function."
        );
        assert_eq!(errors(&result, &source), vec![(message, span.to_string())], "{helper}");
    }
}

/// A value ngtsc reads through `typeof NS.X` is written in the namespace, so a
/// transform in it that uses the namespace's declarations means nothing where
/// the directive is compiled. ngtsc 22.1.7 emits these as written (`fn`,
/// `(v) => P`), which throws a ReferenceError when the module loads; oxc
/// reports them. The snapshot's `r6-ns-*` probes cover the values it reads.
#[test]
fn transforms_using_a_namespace_s_declarations_are_reported() {
    let cases = [
        (
            "namespace NS { export function fn(v: string) { return 1; } export const INPUTS = [{ name: 'x', transform: fn }]; }",
            "typeof NS.INPUTS",
            "X",
            "NS",
        ),
        (
            "namespace NS { const P = 1; export const INPUTS = [{ name: 'x', transform: (v: string) => P }]; }",
            "typeof NS.INPUTS",
            "X",
            "NS",
        ),
        (
            "namespace NS { const P = 1; export namespace Inner { export const INPUTS = [{ name: 'x', transform: (v: string) => P }]; } }",
            "typeof NS.Inner.INPUTS",
            "X",
            "NS.Inner",
        ),
        (
            "namespace NS { const P = 1; export function make() { return [{ name: 'x', transform: (v: string) => P }]; } }",
            "typeof NS.make",
            "X()",
            "NS",
        ),
    ];
    for (namespace, ty, inputs, name) in cases {
        let source = format!(
            "import {{Directive}} from '@angular/core';
{namespace}
declare const X: {ty};
@Directive({{selector: '[d]', inputs: {inputs}}})
export class Dir {{ x: any; }}
"
        );
        let result = transform(&source);
        let message = format!(
            "@Directive.inputs: the transform of \"x\" uses a declaration of namespace {name}, \
             which isn't in scope outside it. OXC can't emit it there."
        );
        assert_eq!(errors(&result, &source), vec![(message, inputs.to_string())], "{namespace}");
    }

    // Names that resolve at the top level are emitted: the transform's own
    // parameters, a top-level function and an import.
    let source = "import {Directive, booleanAttribute} from '@angular/core';
function top(v: string) { return 1; }
namespace NS {
  export const INPUTS = [
    { name: 'a', transform: (v: string) => v.length },
    { name: 'b', transform: top },
    { name: 'c', transform: booleanAttribute },
  ];
}
declare const X: typeof NS.INPUTS;
@Directive({selector: '[d]', inputs: X})
export class Dir { a: any; b: any; c: any; }
";
    let result = transform(source);
    assert!(errors(&result, source).is_empty(), "{:?}", errors(&result, source));
    assert!(
        strip(&result.code).contains(
            r#"inputs:{a:[2,"a","a",(v)=>v.length],b:[2,"b","b",top],c:[2,"c","c",booleanAttribute]}"#
        ),
        "{}",
        result.code
    );
}

/// ngtsc reports a signal input or output also listed in `inputs:` /
/// `outputs:` only for Angular's own `input()`, `model()`, `output()` and
/// `outputFromObservable()`: imported from their module by name (under any
/// alias) or through a namespace import. ngtsc 22.1.7 compiles these without
/// an error, since `input` here isn't Angular's. The snapshot's
/// `initializerApi-*` probes cover the ones ngtsc rejects, and the outputs.
///
/// oxc still compiles such a member as a signal input (it recognises signal
/// members by name), where ngtsc makes it a plain one: that's why these compare
/// only the diagnostics.
#[test]
fn only_angular_initializer_apis_collide_with_metadata() {
    let cases = [
        ("import * as local from './other';", "local.input()"),
        ("import {input} from './other';", "input()"),
        ("import {input} from './other';", "input.required()"),
        ("import {model} from './other';", "model()"),
    ];
    for (import, initializer) in cases {
        let source = format!(
            "import {{Directive}} from '@angular/core';
{import}
@Directive({{selector: '[d]', inputs: ['value']}})
export class Dir {{ value = {initializer}; }}
"
        );
        let result = transform(&source);
        assert_eq!(errors(&result, &source), vec![], "{import} {initializer}");
    }
}

/// Inputs ngtsc can't compile either (it overflows its stack on the recursive
/// ones and on a chain of 3000 consts) must not overflow oxc's stack or hang.
#[test]
fn evaluation_is_bounded() {
    let header = "import {Directive} from '@angular/core';\n";
    let class = |inputs: &str| {
        format!("@Directive({{selector: '[d]', inputs: {inputs}}})\nexport class Dir {{}}\n")
    };
    let chain = |n: usize, link: &dyn Fn(usize) -> String| {
        let mut source = format!("{header}const X0 = ['a'];\n");
        for i in 1..n {
            source += &format!("const X{i} = {};\n", link(i - 1));
        }
        source + &class(&format!("X{}", n - 1))
    };
    let unreadable =
        "Failed to resolve @Directive.inputs to an array Value could not be determined statically.";
    let cases = [
        // Long chains are evaluated one link at a time.
        (chain(3000, &|prev| format!("X{prev}")), None),
        (chain(3000, &|prev| format!("[...X{prev}]")), None),
        (
            format!(
                "{header}{}const X3000 = ['a'];\n{}",
                (0..3000).map(|i| format!("const X{i} = [...X{}];\n", i + 1)).collect::<String>(),
                class("X0")
            ),
            None,
        ),
        // ... also in a namespace, read through `typeof`.
        (
            format!(
                "{header}namespace NS {{\nconst X0 = ['a'];\n{}export const Y = X2999;\n}}\ndeclare const Z: typeof NS.Y;\n{}",
                (1..3000).map(|i| format!("const X{i} = [...X{}];\n", i - 1)).collect::<String>(),
                class("Z")
            ),
            None,
        ),
        // Cycles and recursion.
        (format!("{header}const A: any = B;\nconst B: any = A;\n{}", class("A")), Some(unreadable)),
        (
            format!("{header}function f(): any {{ return f(); }}\n{}", class("f()")),
            Some(unreadable),
        ),
        (
            format!(
                "{header}function f(x: any): any {{ return x ? f(x) : f(x); }}\n{}",
                class("f(1)")
            ),
            Some(unreadable),
        ),
        (format!("{header}class U {{ static a: any = U.a; }}\n{}", class("U.a")), Some(unreadable)),
        (format!("{header}enum E {{ A = E.A }}\n{}", class("[`${E.A}`]")), None),
        // ... also where a spread argument's elements were written.
        (
            format!(
                "{header}const A: any[] = [...A];\nfunction f(...a: any[]) {{ return a; }}\n{}",
                class("f(...A)")
            ),
            None,
        ),
        // Exponential growth.
        (
            format!(
                "{header}const A0 = ['a'];\n{}{}",
                (1..64)
                    .map(|i| format!("const A{i} = [...A{0}, ...A{0}];\n", i - 1))
                    .collect::<String>(),
                class("A63")
            ),
            Some(unreadable),
        ),
        (
            format!(
                "{header}const A0 = ['a'];\n{}function f(...a: any[]) {{ return a; }}\n{}",
                (1..64)
                    .map(|i| format!("const A{i} = [...A{0}, ...A{0}];\n", i - 1))
                    .collect::<String>(),
                class("f(...A63)")
            ),
            None,
        ),
        (
            format!(
                "{header}function f(n: number): any {{ return n > 0 ? [...f(n - 1), ...f(n - 1)] : ['a']; }}\n{}",
                class("f(64)")
            ),
            None,
        ),
    ];
    for (source, expected) in cases {
        let result = transform(&source);
        let messages: Vec<String> = errors(&result, &source).into_iter().map(|e| e.0).collect();
        match expected {
            Some(expected) => assert_eq!(messages, vec![expected.to_string()]),
            // Anything but a crash (a dynamic element is reported, not compiled).
            None => assert!(messages.len() <= 1, "{messages:?}"),
        }
    }
}

/// ngtsc emits the method's bare name for `transform: Utils.coerce` (a static
/// method), which here would silently bind the unrelated top-level `coerce`.
/// oxc keeps the expression as written.
#[test]
fn static_method_transform_is_not_confused_with_a_same_named_function() {
    let source = "import {Directive, Input} from '@angular/core';
export function coerce(v: string) { return 1; }
class Utils { static coerce(v: boolean) { return 2; } }
@Directive({selector: '[d]'})
export class Dir { @Input({transform: Utils.coerce}) x: any; }";
    let code = strip(&transform(source).code);
    assert!(code.contains(r#"inputs:{x:[2,"x","x",Utils.coerce]}"#), "{code}");
}

/// A transform read through a namespace import (`core.booleanAttribute`):
/// ngtsc 22.1.7 reports that it can't reference it, at the declaration in
/// @angular/core's `.d.ts`, which the snapshot can't record (see
/// `probe: eval-nsImportMember`). oxc reports it on the expression.
#[test]
fn namespace_imported_transform_cannot_be_referenced() {
    let source = "import {Directive, Input} from '@angular/core';
import * as core from '@angular/core';
@Directive({selector: '[d]'})
export class Dir { @Input({transform: core.booleanAttribute}) v: any; }
";
    let message = "Input transform function could not be referenced \
                   Value is a reference to 'booleanAttribute'.";
    assert_eq!(
        errors(&transform(source), source),
        vec![(message.to_string(), "core.booleanAttribute".to_string())]
    );
}

/// A transform declared in another file (an import, a namespace member or a
/// global from TypeScript's lib, like `Intl` or the DOM's `atob`): ngtsc
/// 22.1.7 reports these at that declaration, which the snapshot can't record
/// (the fixtures below are skipped for that reason, with the diagnostics ngtsc
/// reported). oxc reports the same message on the expression.
#[test]
fn transform_declared_in_another_file_is_reported_on_the_expression() {
    let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
    let cases = [
        ("probe: transform-clashImported", "booleanAttribute"),
        ("probe: transform-clashImportedMeta", "booleanAttribute"),
        ("probe: transform-clashGlobal", "parseInt"),
        ("probe: transform-globalNumber", "Number"),
        ("probe: transform-globalNumberClash", "Number"),
        ("probe: transform-globalString", "String"),
        ("probe: transform-nsClash", "u.toNum"),
        ("probe: transform-global-Intl", "Intl"),
        ("probe: transform-global-Reflect", "Reflect"),
        ("probe: transform-global-clashAtob", "atob"),
    ];
    for (name, expression) in cases {
        let fixture = fixtures["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap_or_else(|| panic!("{name}"));
        let source = fixture["files"]["test.ts"].as_str().unwrap();
        let expected: Vec<(String, String)> = fixture["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d.as_str().unwrap().to_string(), expression.to_string()))
            .collect();
        assert_eq!(errors(&transform(source), source), expected, "{name}");
    }
}

/// A name the file declares as a namespace or an `import x = ...` alias isn't
/// a global declared elsewhere, so it isn't assumed to be a function the way
/// `atob` is: it's resolved like ngtsc 22.1.7 does. `transform: U` is a
/// reference to the namespace (at its declaration), and `transform: f` is the
/// function the alias names, emitted by the alias's name. The snapshot's
/// `r6-nsval-*` and `r6-alias-*` probes cover more forms.
#[test]
fn file_namespaces_and_aliases_are_not_assumed_to_be_global_functions() {
    let namespace = "namespace U { export function f(v: string) { return 1; } }";
    let source = |pre: &str, expr: &str| {
        format!(
            "import {{Directive, Input}} from '@angular/core';\n{pre}\n\
             @Directive({{selector: '[d]'}})\n\
             export class Dir {{\n  @Input({{transform: {expr}}}) x: any;\n}}\n"
        )
    };

    let with_namespace = source(namespace, "U");
    assert_eq!(
        errors(&transform(&with_namespace), &with_namespace),
        vec![(
            "Input transform must be a function Value is a reference to 'U'.".to_string(),
            namespace.to_string()
        )]
    );

    let with_alias = source(&format!("{namespace}\nimport f = U.f;"), "f");
    let result = transform(&with_alias);
    assert_eq!(errors(&result, &with_alias), vec![]);
    assert!(strip(&result.code).contains(r#"inputs:{x:[2,"x","x",f]}"#), "{}", result.code);
}

/// A shorthand `{ transform }` naming a global is looked up through
/// TypeScript's shorthand symbol, which ngtsc 22.1.7 never accepts as a
/// transform: "could not be determined statically" when nothing declares
/// it, and an error about the declaration when a lib or `.d.ts` does (at
/// that declaration). So unlike `transform: atob`, it isn't assumed to be a
/// function.
#[test]
fn shorthand_transform_naming_a_global_is_rejected() {
    let source = "import {Directive} from '@angular/core';
@Directive({selector: '[d]', inputs: [{name: 'x', transform}]})
export class Dir {
  x!: any;
}
";
    assert_eq!(
        errors(&transform(source), source),
        vec![(
            "Input transform must be a function Value could not be determined statically."
                .to_string(),
            "transform".to_string()
        )]
    );
}

/// ngtsc checks an overloaded static method's first declaration, not its
/// implementation (checked with @angular/compiler-cli 22.1.7, which compiles
/// this; the snapshot can't hold it because ngtsc emits the method's bare name,
/// see `static_method_transform_is_not_confused_with_a_same_named_function`).
#[test]
fn overloaded_static_method_transform_is_checked_at_its_first_declaration() {
    let source = "import {Directive, Input} from '@angular/core';
interface Foo {}
class U { static c(v: string): number; static c(v: string | Foo) { return 1; } }
@Directive({selector: '[d]'})
export class Dir { @Input({transform: U.c}) x!: number; }
";
    let result = transform(source);
    assert_eq!(errors(&result, source), vec![]);
    let code = strip(&result.code);
    assert!(code.contains(r#"inputs:{x:[2,"x","x",U.c]}"#), "{code}");
}

/// Member decorators through a namespace import are Angular's only when the
/// namespace imports `@angular/core` (ngtsc 22.1.7 compiles the query, host
/// binding and listener below for `core`, and none of them for `foreign`).
/// The snapshot compares the inputs and outputs of both
/// (`probe: transform-coreNamespaceMembers` and
/// `probe: transform-foreignNamespaceMembers`).
#[test]
fn namespaced_member_decorators_need_an_angular_core_namespace() {
    let members = "
export class Cmp {
  @NS.ViewChild('ref') ref: any;
  @NS.HostBinding('class.a') a = true;
  @NS.HostListener('click') onClick() {}
}
";
    let core = "import * as NS from '@angular/core';
@NS.Component({selector: 'c', template: '<div #ref></div>'})";
    let foreign = "import {Component} from '@angular/core';
import * as NS from 'foreign-decorators';
@Component({selector: 'c', template: '<div #ref></div>'})";
    for (header, compiled) in [(core, true), (foreign, false)] {
        let code = transform(&format!("{header}{members}")).code;
        for part in ["viewQuery", "ɵɵclassProp(\"a\"", "ɵɵlistener(\"click\""] {
            assert_eq!(code.contains(part), compiled, "{part} in\n{code}");
        }
    }
}

/// `resolved_imports` points an imported name at the file that declares it
/// (past a barrel). It doesn't change which module the import is from, so
/// `@core.Input()` through `import * as core from '@angular/core'` is still
/// Angular's when `core` is mapped.
#[test]
fn namespaced_member_decorators_ignore_resolved_import_paths() {
    let source = "import {Component} from '@angular/core';
import * as core from '@angular/core';
@Component({selector: 'c', template: ''})
export class Cmp {
  @core.Input() x: any;
  @core.Output() y: any;
}
";
    let options = TransformOptions {
        resolved_imports: Some(HashMap::from([(
            "core".to_string(),
            "../node_modules/@angular/core/fesm2022/core.mjs".to_string(),
        )])),
        ..TransformOptions::default()
    };
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    let code = strip(&result.code);
    assert!(code.contains(r#"inputs:{x:"x"}"#), "{}", result.code);
    assert!(code.contains(r#"outputs:{y:"y"}"#), "{}", result.code);
}

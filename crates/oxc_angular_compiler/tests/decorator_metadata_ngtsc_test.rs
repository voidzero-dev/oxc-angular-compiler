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

use std::collections::BTreeMap;

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};
use serde_json::Value;

const FIXTURES: &str = include_str!("fixtures/decorator_metadata_ngtsc.json");

fn strip(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn strip_parens(s: &str) -> String {
    s.chars().filter(|c| *c != '(' && *c != ')').collect()
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
    assert_eq!(compared, 98, "fixtures compared");
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
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "test.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    let code = strip(&result.code);
    assert!(code.contains(r#"inputs:{x:[2,"x","x",Utils.coerce]}"#), "{code}");
}

//! Like ngtsc, a constructor parameter decorator (`@Inject`, `@Optional`,
//! `@Self`, `@SkipSelf`, `@Host`, `@Attribute`) is Angular's only when it's
//! imported from `@angular/core`: by name under any alias
//! (`import {Inject as Inj}`), or through a namespace import (`@ng.Inject()`).
//! Angular's are compiled into the factory, `setClassMetadata` and JIT
//! `ctorParameters`, and removed. Another module's or a local one with the same
//! name is ignored by all three and left on the parameter (JIT lowers it as
//! `__param(index, decorator)` in the class's `__decorate`, as TypeScript does).
//!
//! `fixtures/ctor_param_decorators_ngtsc.json` is a snapshot of
//! `@angular/compiler-cli` 22.1.7 for class `A` in each probe's `test.ts` (the
//! other files are what it imports): its diagnostics, the arguments of its
//! factory, the `ctorParameters` of `setClassMetadata`, the parameter
//! decorators left (as the `__param(...)` TypeScript lowers them to), and the
//! same two for Angular's JIT transform (`constructorParametersDownlevelTransform`).
//! Text is compared without whitespace, with `'` as `"` and without trailing
//! commas. Cases oxc deliberately handles differently carry a `skip` reason.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, TransformResult, transform_angular_file};
use serde_json::Value;

const FIXTURES: &str = include_str!("fixtures/ctor_param_decorators_ngtsc.json");

fn transform(source: &str, options: &TransformOptions) -> TransformResult {
    let allocator = Allocator::default();
    transform_angular_file(&allocator, "test.ts", source, Some(options), None)
}

/// `text` without whitespace, with `'` as `"`, without trailing commas and
/// without ngtsc's `/* @ts-ignore */`.
fn normalize(text: &str) -> String {
    let text: String = text
        .replace("/* @ts-ignore */", "")
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '\'' { '"' } else { c })
        .collect();
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == ',' && chars.get(i + 1).is_some_and(|n| matches!(n, ']' | '}' | ')')) {
            continue;
        }
        out.push(c);
    }
    out
}

/// The bracketed text opening at byte `open` of `text`, skipping strings.
fn bracketed(text: &str, open: usize) -> &str {
    let bytes = text.as_bytes();
    let (mut depth, mut i) = (0, open);
    while i < bytes.len() {
        match bytes[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while bytes[i] != q {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open..=i];
                }
            }
            _ => {}
        }
        i += 1;
    }
    &text[open..]
}

/// The arguments `A`'s factory constructs it with. oxc reads an `@Inject`
/// token imported from another file through its namespace import (`i1.TOKEN`),
/// where ngtsc names the import (`TOKEN`); both are the same value.
fn factory(js: &str) -> Option<String> {
    const NEW: &str = "(__ngFactoryType__ || A)";
    let at = js.find(&format!("{NEW}("))? + NEW.len();
    Some(normalize(bracketed(js, at)).replace("i1.TOKEN", "TOKEN"))
}

/// The `ctorParameters` array of `setClassMetadata(A, ...)`.
fn ctor_parameters(js: &str) -> Option<String> {
    let at = js.find("setClassMetadata(A")?;
    let arrow = at + js[at..].find("() =>")?;
    let open = arrow + js[arrow..].find('[')?;
    Some(normalize(bracketed(js, open)))
}

/// The `ctorParameters` array of JIT output.
fn jit_ctor_parameters(js: &str) -> Option<String> {
    let at = js.find("ctorParameters = () =>")?;
    let open = at + js[at..].find('[')?;
    Some(normalize(bracketed(js, open)))
}

/// The `__param(index, decorator)` entries in `js`, sorted.
fn lowered_param_decorators(js: &str) -> Vec<String> {
    let mut found: Vec<String> = js
        .match_indices("__param(")
        .map(|(at, _)| normalize(&format!("__param{}", bracketed(js, at + "__param".len()))))
        .collect();
    found.sort();
    found
}

/// The decorators left on `A`'s constructor parameters in AOT output, as the
/// `__param(index, decorator)` TypeScript lowers them to, sorted.
fn param_decorators_left(js: &str) -> Vec<String> {
    let Some(at) = js.find("constructor(") else { return Vec::new() };
    let params = bracketed(js, at + "constructor".len());
    let params = &params[1..params.len() - 1];
    // Split at top-level commas.
    let (mut list, mut depth, mut start) = (Vec::new(), 0, 0);
    for (i, c) in params.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                list.push(&params[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    list.push(&params[start..]);

    let mut left = Vec::new();
    for (index, param) in list.iter().enumerate() {
        for (at, _) in param.match_indices('@') {
            let name_end = param[at + 1..]
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
                .map_or(param.len(), |end| at + 1 + end);
            let mut decorator = param[at + 1..name_end].to_string();
            if param[name_end..].starts_with('(') {
                decorator.push_str(bracketed(param, name_end));
            }
            left.push(normalize(&format!("__param({index}, {decorator})")));
        }
    }
    left.sort();
    left
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn optional(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

#[test]
fn ctor_param_decorators_match_ngtsc() {
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
        let mut check = |what: &str, ngtsc: String, oxc: String| {
            if ngtsc != oxc {
                failures.push(format!("{name}\n  {what}\n    ngtsc: {ngtsc}\n    oxc:   {oxc}"));
            }
        };

        let aot = transform(
            source,
            &TransformOptions { emit_class_metadata: true, ..TransformOptions::default() },
        );
        let errors: Vec<String> = aot
            .diagnostics
            .iter()
            .filter(|d| d.severity == oxc_diagnostics::Severity::Error)
            .map(|d| d.message.to_string())
            .collect();
        check(
            "diagnostics",
            format!("{:?}", strings(&fixture["diagnostics"])),
            format!("{errors:?}"),
        );
        check(
            "factory",
            format!("{:?}", optional(&fixture["factory"])),
            format!("{:?}", factory(&aot.code)),
        );
        check(
            "setClassMetadata ctorParameters",
            format!("{:?}", optional(&fixture["ctorParameters"])),
            format!("{:?}", ctor_parameters(&aot.code)),
        );
        check(
            "parameter decorators left",
            format!("{:?}", strings(&fixture["paramDecoratorsLeft"])),
            format!("{:?}", param_decorators_left(&aot.code)),
        );

        let jit = transform(source, &TransformOptions { jit: true, ..TransformOptions::default() });
        check(
            "JIT ctorParameters",
            format!("{:?}", optional(&fixture["jitCtorParameters"])),
            format!("{:?}", jit_ctor_parameters(&jit.code)),
        );
        check(
            "JIT __param",
            format!("{:?}", strings(&fixture["jitParamDecorators"])),
            format!("{:?}", lowered_param_decorators(&jit.code)),
        );
    }
    assert!(
        failures.is_empty(),
        "{} mismatches with ngtsc:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert_eq!(compared, 29, "fixtures compared");
}

/// A parameter decorator left in the output keeps its import: another module's
/// `@Inject` / `@Optional` (and the `@Inject` argument) aren't Angular's, so
/// they aren't removed, and neither is what they need at runtime.
#[test]
fn foreign_param_decorators_keep_their_imports() {
    let source = "import {Component} from '@angular/core';\nimport {Inject, Optional} from './foreign';\nimport {Dep, TOKEN} from './dep';\n@Component({selector: 'a-c', template: ''})\nexport class A {\n  constructor(@Inject(TOKEN) a: Dep, @Optional() b: Dep) {}\n}\n";
    let result = transform(source, &TransformOptions::default());
    assert!(result.code.contains("import {Inject, Optional} from './foreign';"), "{}", result.code);
    assert!(result.code.contains("TOKEN } from"), "{}", result.code);
    assert!(
        result.code.contains("constructor(@Inject(TOKEN) a: Dep, @Optional() b: Dep)"),
        "{}",
        result.code
    );
}

/// JIT output imports `__param` from tslib only when it lowers a parameter
/// decorator with it.
#[test]
fn jit_imports_param_helper_only_when_used() {
    let options = TransformOptions { jit: true, ..TransformOptions::default() };
    let angular = transform(
        "import {Component, Optional} from '@angular/core';\n@Component({selector: 'a-c', template: ''})\nexport class A {\n  constructor(@Optional() a: Dep) {}\n}\n",
        &options,
    );
    assert!(angular.code.contains("import { __decorate } from \"tslib\";"), "{}", angular.code);

    let foreign = transform(
        "import {Component} from '@angular/core';\nimport {Optional} from './foreign';\n@Component({selector: 'a-c', template: ''})\nexport class A {\n  constructor(@Optional() a: Dep) {}\n}\n",
        &options,
    );
    assert!(
        foreign.code.contains("import { __decorate, __param } from \"tslib\";"),
        "{}",
        foreign.code
    );
}

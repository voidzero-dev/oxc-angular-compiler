//! Like ngtsc, a class decorator (`@Component`, `@Directive`, `@Pipe`,
//! `@Injectable`, `@NgModule`) is Angular's only when it's imported from
//! `@angular/core`: by name under any alias (`import {Component as Cmp}`), or
//! through a namespace import (`@ng.Component()`). Another module's, a local or
//! an undeclared one is left on the class, which isn't compiled. One imported
//! through another module that re-exports Angular's (`import {Component} from
//! './shared'`) is left alone too: oxc doesn't follow re-exports.
//!
//! `fixtures/class_decorators_ngtsc.json` is a snapshot of ngtsc's output
//! (`@angular/compiler-cli` 22.1.7) for class `A` in each probe's `test.ts`
//! (the other files are what it imports): its diagnostics, the definitions it
//! gets (`ɵcmp`, `ɵfac`, ...), the decorators left on it and its members, and
//! the decorator types listed in `setClassMetadata`. Cases oxc deliberately
//! handles differently carry a `skip` reason.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, TransformResult, transform_angular_file};
use serde_json::Value;

const FIXTURES: &str = include_str!("fixtures/class_decorators_ngtsc.json");

fn transform(source: &str, options: &TransformOptions) -> TransformResult {
    let allocator = Allocator::default();
    transform_angular_file(&allocator, "test.ts", source, Some(options), None)
}

/// The `static ɵ...` definitions in `js`, sorted.
fn definitions(js: &str) -> Vec<String> {
    let mut defs: Vec<String> = js
        .match_indices("static ɵ")
        .map(|(at, _)| {
            let rest = &js[at + "static ".len()..];
            rest[..rest.find(|c: char| !c.is_alphanumeric() && c != 'ɵ').unwrap()].to_string()
        })
        .collect();
    defs.sort();
    defs.dedup();
    defs
}

/// The decorators (`@X` / `@ns.X`) left in `js`, outside strings and comments,
/// sorted.
fn decorators_left(js: &str) -> Vec<String> {
    let mut left = Vec::new();
    for (at, _) in js.match_indices('@') {
        let before = js[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || "_*/'\"`".contains(c)) {
            continue;
        }
        let name: String = js[at + 1..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            left.push(name);
        }
    }
    left.sort();
    left
}

/// The end of the bracketed text opening at `open`, skipping strings.
fn closing(text: &str, open: usize) -> usize {
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
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    text.len()
}

/// The decorator types `setClassMetadata(A, [...])` lists in `js`, if it's there.
fn class_metadata(js: &str) -> Option<Vec<String>> {
    let at = js.find("setClassMetadata(A,")?;
    let open = at + js[at..].find('[')?;
    let decorators = &js[open..=closing(js, open)];
    let (mut types, mut depth) = (Vec::new(), 0);
    let mut i = 0;
    while i < decorators.len() {
        let rest = &decorators[i..];
        match rest.as_bytes()[0] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while decorators.as_bytes()[i] != q {
                    i += if decorators.as_bytes()[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth -= 1,
            _ if depth == 2 && rest.starts_with("type:") => {
                let ty = rest["type:".len()..].trim_start();
                let end = ty.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.').unwrap();
                types.push(ty[..end].to_string());
            }
            _ => {}
        }
        i += 1;
    }
    Some(types)
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

#[test]
fn class_decorators_match_ngtsc() {
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
        let result = transform(source, &TransformOptions::default());
        let mut check = |what: &str, ngtsc: String, oxc: String| {
            if ngtsc != oxc {
                failures.push(format!("{name}\n  {what}\n    ngtsc: {ngtsc}\n    oxc:   {oxc}"));
            }
        };

        let expected = strings(&fixture["diagnostics"]);
        let actual: Vec<String> = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == oxc_diagnostics::Severity::Error)
            .map(|d| d.message.to_string())
            .collect();
        check("diagnostics", format!("{expected:?}"), format!("{actual:?}"));
        // ngtsc emits nothing for a class it rejects.
        if !expected.is_empty() {
            continue;
        }
        check(
            "definitions",
            format!("{:?}", strings(&fixture["definitions"])),
            format!("{:?}", definitions(&result.code)),
        );
        check(
            "decorators left",
            format!("{:?}", strings(&fixture["decoratorsLeft"])),
            format!("{:?}", decorators_left(&result.code)),
        );
        // oxc lists only the decorator it compiles the class for (ngtsc lists
        // all of Angular's: `[Injectable, Component]`), in the same form.
        let ngtsc = fixture["classMetadata"].as_array().map(|_| strings(&fixture["classMetadata"]));
        let oxc = class_metadata(&result.code);
        let listed = match (&ngtsc, &oxc) {
            (Some(ngtsc), Some(oxc)) => !oxc.is_empty() && oxc.iter().all(|t| ngtsc.contains(t)),
            (None, None) => true,
            _ => false,
        };
        if !listed {
            check("setClassMetadata", format!("{ngtsc:?}"), format!("{oxc:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches with ngtsc:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert_eq!(compared, 40, "fixtures compared");
}

/// ngtsc compiles an aliased `@Injectable` (`import {Injectable as Inj}`) but
/// gives it no factory: its `needsFactory` compares the decorator's written
/// name with `'Injectable'`, so `ɵprov` references a missing `A.ɵfac` (the
/// skipped `injectableAlias` / `aliasNotDecoratorName` probes). oxc gives it
/// the factory, as for `@Injectable`.
#[test]
fn aliased_injectable_gets_a_factory() {
    for source in [
        "import {Injectable as Inj} from '@angular/core';\n@Inj({providedIn: 'root'})\nexport class A {}\n",
        "import {Injectable as Component} from '@angular/core';\n@Component({providedIn: 'root'})\nexport class A {}\n",
    ] {
        let result = transform(source, &TransformOptions::default());
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(definitions(&result.code), ["ɵfac", "ɵprov"], "{}", result.code);
        assert!(decorators_left(&result.code).is_empty(), "{}", result.code);
    }
}

/// In JIT mode, too, only Angular's class decorators are rewritten for the
/// runtime compiler; another library's `@Component` is left as written.
#[test]
fn jit_rewrites_only_angular_class_decorators() {
    let options = TransformOptions { jit: true, ..TransformOptions::default() };
    let aliased = transform(
        "import {Component as Cmp} from '@angular/core';\n@Cmp({selector: 'a-c', templateUrl: './a.html'})\nexport class A {}\n",
        &options,
    );
    assert!(aliased.code.contains("__decorate([Cmp({"), "{}", aliased.code);
    assert!(!aliased.code.contains("templateUrl"), "{}", aliased.code);
    assert!(decorators_left(&aliased.code).is_empty(), "{}", aliased.code);

    let foreign = "import {Component} from './foreign';\n@Component({selector: 'a-c', templateUrl: './a.html'})\nexport class A {}\n";
    let result = transform(foreign, &options);
    assert_eq!(result.code, foreign);
}

/// A declaration an Angular decorator references after the class is hoisted
/// above it, so the definition doesn't read it in its TDZ. Another library's
/// `@Component` isn't compiled, so the file's statements keep their order.
#[test]
fn only_angular_class_decorators_hoist_their_references() {
    let source = |import: &str, decorator: &str| {
        format!(
            "{import}\n@{decorator}({{selector: 'a-c', template: '', providers: [{{provide: TOKEN, useValue: 1}}]}})\nexport class A {{}}\nconst TOKEN = 'tok';\n"
        )
    };
    let hoisted = |code: &str| code.find("const TOKEN").unwrap() < code.find("class A").unwrap();

    let aliased = transform(
        &source("import {Component as Cmp} from '@angular/core';", "Cmp"),
        &TransformOptions::default(),
    );
    assert!(hoisted(&aliased.code), "{}", aliased.code);

    let foreign = transform(
        &source("import {Component} from './foreign';", "Component"),
        &TransformOptions::default(),
    );
    assert!(!hoisted(&foreign.code), "{}", foreign.code);
}

/// `@Service` is Angular's under any alias too (`@NgService()` for
/// `import {Service as NgService}`), like ngtsc's `findAngularDecorator`: the
/// declaration its metadata references after the class is hoisted above it,
/// as for `@Service`, so the compiled class doesn't read it in its TDZ.
/// Another library's `Service` under the same alias isn't compiled, so the
/// file's statements keep their order.
#[test]
fn aliased_service_hoists_its_references() {
    let source = |import: &str| {
        format!(
            "{import}\n@NgService({{factory: FACTORY}})\nexport class A {{}}\nconst FACTORY = () => new A();\n"
        )
    };
    let hoisted = |code: &str| code.find("const FACTORY").unwrap() < code.find("class A").unwrap();

    let aliased = transform(
        &source("import {Service as NgService} from '@angular/core';"),
        &TransformOptions::default(),
    );
    assert!(aliased.code.contains("static ɵprov"), "{}", aliased.code);
    assert!(hoisted(&aliased.code), "{}", aliased.code);

    let foreign = transform(
        &source("import {Service as NgService} from './di';"),
        &TransformOptions::default(),
    );
    assert!(!foreign.code.contains("static ɵprov"), "{}", foreign.code);
    assert!(!hoisted(&foreign.code), "{}", foreign.code);
}

/// With HMR on, each compiled component's `@Component` is reported as written,
/// so a build tool can find the decorator the compiler took. Another library's
/// `@Component` isn't compiled, so it isn't reported.
#[test]
fn hmr_reports_component_decorators_as_written() {
    let source = "import * as ng from '@angular/core';\nimport {Component as Cmp} from '@angular/core';\nimport {Component} from './foreign';\n@Cmp({selector: 'a-c', template: ''})\nexport class A {}\n@ng . Component({selector: 'b-c', template: ''})\nexport class B {}\n@Component({selector: 'c-c', template: ''})\nexport class C {}\n";
    let result = transform(source, &TransformOptions { hmr: true, ..TransformOptions::default() });
    let mut reported: Vec<(&str, &str)> =
        result.component_decorators.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    reported.sort_unstable();
    assert_eq!(reported, [("test.ts@A", "Cmp"), ("test.ts@B", "ng . Component")]);
}

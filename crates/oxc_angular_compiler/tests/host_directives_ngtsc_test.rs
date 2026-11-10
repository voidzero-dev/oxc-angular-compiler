//! `hostDirectives` references and mappings, compared with ngtsc (#702).
//!
//! Expected text is what `@angular/compiler-cli` 22.2.4 writes for the same
//! sources (full and partial JS, and the `.d.ts`), compared with whitespace
//! removed. Two known differences:
//! - ngtsc writes `forwardRef(() => X)` in partial output, oxc writes
//!   `forwardRef(function () { return X; })`.
//! - For a string export name (`export { X as "s-dir" }`) ngtsc writes
//!   `i1.s-dir`, which does not parse; oxc writes `i1["s-dir"]`.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    CompilationMode, TransformOptions, TransformResult, transform_angular_file,
};

const MODES: [CompilationMode; 2] = [CompilationMode::Full, CompilationMode::Partial];

const HEADER: &str = "import { Component, Directive, forwardRef, Input, Output, EventEmitter } from '@angular/core';\n";

fn transform(source: &str, mode: CompilationMode) -> TransformResult {
    let allocator = Allocator::default();
    let source = format!("{HEADER}{source}");
    let options = TransformOptions { compilation_mode: mode, ..Default::default() };
    let result = transform_angular_file(&allocator, "src/test.ts", &source, Some(&options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[track_caller]
fn assert_has(code: &str, expected: &str) {
    assert!(squash(code).contains(&squash(expected)), "expected `{expected}` in:\n{code}");
}

/// The `iN` → module pairs the JS imports as a namespace, in order.
fn js_namespace_imports(code: &str) -> Vec<(String, String)> {
    code.lines()
        .filter_map(|line| line.strip_prefix("import * as i"))
        .filter_map(|rest| {
            let (number, module) = rest.split_once(" from ")?;
            if !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let alias = format!("i{number}");
            let module = module.trim_end_matches(';').trim_matches(['\'', '"']);
            Some((alias.to_string(), module.to_string()))
        })
        .collect()
}

#[track_caller]
fn assert_imports(code: &str, expected: &[(&str, &str)]) {
    let expected: Vec<(String, String)> =
        expected.iter().map(|(a, m)| ((*a).to_string(), (*m).to_string())).collect();
    assert_eq!(js_namespace_imports(code), expected, "{code}");
}

/// The `.d.ts` members of `class_name`.
fn dts<'r>(result: &'r TransformResult, class_name: &str) -> &'r str {
    &result
        .dts_declarations
        .iter()
        .find(|decl| decl.class_name == class_name)
        .unwrap_or_else(|| panic!("no .d.ts for {class_name}"))
        .members
}

/// The `.d.ts` namespace import of `alias` for `class_name`.
fn dts_import<'r>(result: &'r TransformResult, class_name: &str, alias: &str) -> &'r str {
    result
        .dts_declarations
        .iter()
        .find(|decl| decl.class_name == class_name)
        .and_then(|decl| decl.namespace_imports.get(alias))
        .map_or("", String::as_str)
}

// ---- input / output pairs ---------------------------------------------------

#[test]
fn partial_writes_field_then_alias() {
    let source = r"import { H } from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [{ directive: H, inputs: ['color: c'], outputs: ['changed: ch'] }] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [{ directive: H, inputs: ['color: c'], outputs: ['changed: ch'] }] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_has(
        &full.code,
        r#"i0.ɵɵHostDirectivesFeature([{ directive: i1.H, inputs: ["color", "c"], outputs: ["changed", "ch"] }])"#,
    );
    let partial = transform(source, CompilationMode::Partial);
    assert_eq!(
        squash(&partial.code)
            .matches(&squash(
                r#"hostDirectives: [{ directive: i1.H, inputs: ["color", "c"], outputs: ["changed", "ch"] }]"#
            ))
            .count(),
        2,
        "{}",
        partial.code
    );
}

#[test]
fn partial_mapping_duplicates_and_several_pairs() {
    let source = r"@Directive({ selector: '[l]' }) export class L { @Input() color: string; @Input() x: string; @Output() changed = new EventEmitter(); }
@Component({ selector: 'c', template: '', hostDirectives: [{ directive: L, inputs: ['color: c', 'x: y'], outputs: ['changed'] }] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [{ directive: L, inputs: ['color: c', 'color'], outputs: ['changed: ch'] }] }) export class Dir {}";
    let partial = transform(source, CompilationMode::Partial);
    assert_has(
        &partial.code,
        r#"hostDirectives: [{ directive: L, inputs: ["color", "c", "x", "y"], outputs: ["changed", "changed"] }]"#,
    );
    assert_has(
        &partial.code,
        r#"hostDirectives: [{ directive: L, inputs: ["color", "color"], outputs: ["changed", "ch"] }]"#,
    );
}

#[test]
fn dts_mapping_is_keyed_by_field() {
    let source = r"import { H } from './dep';
@Directive({ selector: '[l]' }) export class L {}
@Component({ selector: 'c', template: '', hostDirectives: [{ directive: H, inputs: ['color: c', 'x: y'], outputs: ['changed'] }] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [{ directive: L, inputs: ['color: c', 'color'], outputs: ['changed: ch'] }] }) export class Dir {}";
    for mode in MODES {
        let result = transform(source, mode);
        assert!(
            dts(&result, "Cmp").contains(
                r#"[{ directive: typeof i1.H; inputs: { "color": "c"; "x": "y"; }; outputs: { "changed": "changed"; }; }]"#
            ),
            "{mode:?}: {}",
            dts(&result, "Cmp")
        );
        assert!(
            dts(&result, "Dir").contains(
                r#"[{ directive: typeof L; inputs: { "color": "color"; }; outputs: { "changed": "ch"; }; }]"#
            ),
            "{mode:?}: {}",
            dts(&result, "Dir")
        );
    }
}

// ---- references by the module's export name ----------------------------------

#[test]
fn aliased_import_uses_export_name() {
    let source = r"import { H as P } from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [P, { directive: P, inputs: ['color: c'] }] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [P, { directive: P, inputs: ['color: c'] }] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_eq!(
        squash(&full.code)
            .matches(&squash(
                r#"ɵɵHostDirectivesFeature([i1.H, { directive: i1.H, inputs: ["color", "c"] }])"#
            ))
            .count(),
        2,
        "{}",
        full.code
    );
    let partial = transform(source, CompilationMode::Partial);
    assert_eq!(
        squash(&partial.code)
            .matches(&squash(r#"hostDirectives: [{ directive: i1.H }, { directive: i1.H, inputs: ["color", "c"] }]"#))
            .count(),
        2,
        "{}",
        partial.code
    );
    assert!(!squash(&full.code).contains("i1.P"), "{}", full.code);
}

#[test]
fn default_import_uses_default_export() {
    let source = r"import DD from './dep2';
import { default as DE } from './dep2';
@Component({ selector: 'c', template: '', hostDirectives: [DD, DE] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [DD, DE] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_eq!(
        squash(&full.code).matches("ɵɵHostDirectivesFeature([i1.default,i1.default])").count(),
        2,
        "{}",
        full.code
    );
    let partial = transform(source, CompilationMode::Partial);
    assert_eq!(
        squash(&partial.code)
            .matches("hostDirectives:[{directive:i1.default},{directive:i1.default}]")
            .count(),
        2,
        "{}",
        partial.code
    );
}

#[test]
fn forward_ref_to_aliased_and_namespace_member() {
    let source = r"import { H as P } from './dep';
import * as dep from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [forwardRef(() => P), { directive: forwardRef(() => dep.H), outputs: ['changed: ch'] }] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [forwardRef(() => P)] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_has(
        &full.code,
        r#"i0.ɵɵHostDirectivesFeature(function () { return [i1.H, { directive: i1.H, outputs: ["changed", "ch"] }]; })"#,
    );
    assert_has(&full.code, "i0.ɵɵHostDirectivesFeature(function () { return [i1.H]; })");
    let result = transform(source, CompilationMode::Partial);
    assert_has(
        &result.code,
        r#"hostDirectives: [{ directive: i0.forwardRef(function () { return i1.H; }) }, { directive: i0.forwardRef(function () { return i1.H; }), outputs: ["changed", "ch"] }]"#,
    );
    assert!(dts(&result, "Cmp").contains(
        r#"[{ directive: typeof i1.H; inputs: {}; outputs: {}; }, { directive: typeof i1.H; inputs: {}; outputs: { "changed": "ch"; }; }]"#
    ));
}

// ---- namespace members --------------------------------------------------------

#[test]
fn namespace_member() {
    let source = r"import * as dep from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [dep.H, { directive: dep.H, inputs: ['color: c'] }] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [dep.H, { directive: dep.H, inputs: ['color: c'] }] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_eq!(
        squash(&full.code)
            .matches(&squash(
                r#"ɵɵHostDirectivesFeature([i1.H, { directive: i1.H, inputs: ["color", "c"] }])"#
            ))
            .count(),
        2,
        "{}",
        full.code
    );
    assert_imports(&full.code, &[("i0", "@angular/core"), ("i1", "./dep")]);
    let partial = transform(source, CompilationMode::Partial);
    assert_eq!(
        squash(&partial.code)
            .matches(&squash(r#"hostDirectives: [{ directive: i1.H }, { directive: i1.H, inputs: ["color", "c"] }]"#))
            .count(),
        2,
        "{}",
        partial.code
    );
    for mode in MODES {
        let result = transform(source, mode);
        for class in ["Cmp", "Dir"] {
            assert!(
                dts(&result, class).contains(
                    r#"[{ directive: typeof i1.H; inputs: {}; outputs: {}; }, { directive: typeof i1.H; inputs: { "color": "c"; }; outputs: {}; }]"#
                ),
                "{mode:?} {class}: {}",
                dts(&result, class)
            );
            assert_eq!(dts_import(&result, class, "i1"), "./dep");
        }
    }
}

#[test]
fn namespace_member_in_constant_list() {
    let source = r"import * as dep from './dep';
import { H as P } from './dep';
const HD = [dep.H, { directive: P, inputs: ['color: c'] }];
@Component({ selector: 'c', template: '', hostDirectives: HD }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [...HD] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_eq!(
        squash(&full.code)
            .matches(&squash(
                r#"ɵɵHostDirectivesFeature([i1.H, { directive: i1.H, inputs: ["color", "c"] }])"#
            ))
            .count(),
        2,
        "{}",
        full.code
    );
    for class in ["Cmp", "Dir"] {
        assert!(dts(&full, class).contains(
            r#"[{ directive: typeof i1.H; inputs: {}; outputs: {}; }, { directive: typeof i1.H; inputs: { "color": "c"; }; outputs: {}; }]"#
        ));
    }
}

/// The export name of `dep.H` is never looked up as a local name: here `H`
/// is also a named import from another module.
#[test]
fn namespace_member_named_like_another_import() {
    let source = r"import * as dep from './dep';
import { H } from './dep3';
@Component({ selector: 'c', template: '', hostDirectives: [dep.H, H] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [dep.H, H] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_eq!(
        squash(&full.code).matches("ɵɵHostDirectivesFeature([i1.H,i2.H])").count(),
        2,
        "{}",
        full.code
    );
    assert_imports(&full.code, &[("i0", "@angular/core"), ("i1", "./dep"), ("i2", "./dep3")]);
    let partial = transform(source, CompilationMode::Partial);
    assert_eq!(
        squash(&partial.code).matches("hostDirectives:[{directive:i1.H},{directive:i2.H}]").count(),
        2,
        "{}",
        partial.code
    );
    for mode in MODES {
        let result = transform(source, mode);
        for class in ["Cmp", "Dir"] {
            assert!(
                dts(&result, class).contains(
                    "[{ directive: typeof i1.H; inputs: {}; outputs: {}; }, { directive: typeof i2.H; inputs: {}; outputs: {}; }]"
                ),
                "{mode:?} {class}: {}",
                dts(&result, class)
            );
            assert_eq!(dts_import(&result, class, "i1"), "./dep");
            assert_eq!(dts_import(&result, class, "i2"), "./dep3");
        }
    }
}

#[test]
fn namespace_member_named_like_a_local_class() {
    let source = r"import * as dep from './dep';
@Directive({ selector: '[hl]' }) export class H {}
@Component({ selector: 'c', template: '', hostDirectives: [dep.H, H] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [dep.H, H] }) export class Dir {}";
    let full = transform(source, CompilationMode::Full);
    assert_eq!(
        squash(&full.code).matches("ɵɵHostDirectivesFeature([i1.H,H])").count(),
        2,
        "{}",
        full.code
    );
    for class in ["Cmp", "Dir"] {
        assert!(dts(&full, class).contains(
            "[{ directive: typeof i1.H; inputs: {}; outputs: {}; }, { directive: typeof H; inputs: {}; outputs: {}; }]"
        ));
    }
}

// ---- string export names ------------------------------------------------------

#[test]
fn string_export_name() {
    for source in [
        r#"import { "s-dir" as SDir } from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [SDir] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [SDir] }) export class Dir {}"#,
        r#"import * as dep from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [dep["s-dir"]] }) export class Cmp {}
@Directive({ selector: '[d]', hostDirectives: [dep["s-dir"]] }) export class Dir {}"#,
    ] {
        let full = transform(source, CompilationMode::Full);
        assert_eq!(
            squash(&full.code).matches(r#"ɵɵHostDirectivesFeature([i1["s-dir"]])"#).count(),
            2,
            "{}",
            full.code
        );
        let partial = transform(source, CompilationMode::Partial);
        assert_eq!(
            squash(&partial.code).matches(r#"hostDirectives:[{directive:i1["s-dir"]}]"#).count(),
            2,
            "{}",
            partial.code
        );
        for class in ["Cmp", "Dir"] {
            assert!(
                dts(&full, class)
                    .contains(r#"[{ directive: typeof i1["s-dir"]; inputs: {}; outputs: {}; }]"#),
                "{class}: {}",
                dts(&full, class)
            );
        }
    }
}

// ---- namespace numbering --------------------------------------------------------

/// ngtsc emits `ɵfac` before `ɵcmp`, so a constructor dependency's module
/// takes `i1` before the host directive's.
#[test]
fn partial_numbers_factory_deps_first() {
    for source in [
        r"import { H as P } from './dep';
import { Svc } from './svc';
@Component({ selector: 'c', template: '', hostDirectives: [P] }) export class Cmp { constructor(s: Svc) {} }",
        r"import { Svc } from './svc';
import * as dep from './dep';
@Component({ selector: 'c', template: '', hostDirectives: [dep.H] }) export class Cmp { constructor(s: Svc) {} }",
    ] {
        for mode in MODES {
            let result = transform(source, mode);
            assert_imports(
                &result.code,
                &[("i0", "@angular/core"), ("i1", "./svc"), ("i2", "./dep")],
            );
        }
        let partial = transform(source, CompilationMode::Partial);
        assert_has(&partial.code, "deps: [{ token: i1.Svc }]");
        assert_has(&partial.code, "hostDirectives: [{ directive: i2.H }]");
    }
}

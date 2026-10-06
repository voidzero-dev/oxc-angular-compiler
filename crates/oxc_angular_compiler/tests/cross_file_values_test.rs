//! End-to-end tests for `TransformOptions::resolve_imported_values`
//! (#518): decorator metadata that references values exported from other
//! files (`inputs: INPUTS`, `@Input(OPTS)`) must evaluate like ngtsc's
//! program-wide checker instead of erroring with "imported from another
//! module". Unresolvable imports keep that diagnostic.

#![cfg(feature = "cross_file_elision")]

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, TransformResult, transform_angular_file};
use tempfile::TempDir;

fn create_test_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

/// The whitespace-free `ɵɵdefineDirective` call for `class_name`.
fn define_call(code: &str, class_name: &str) -> String {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let start = compact.find(&format!("({{type:{class_name},")).expect("define call");
    let end = compact[start..].find("});").map_or(compact.len(), |i| start + i);
    compact[start..end].to_string()
}

fn transform(dir: &TempDir, source: &str, options: &TransformOptions) -> TransformResult {
    let path = create_test_file(dir.path(), "app/test.ts", source);
    let allocator = Allocator::default();
    transform_angular_file(&allocator, path.to_str().unwrap(), source, Some(options), None)
}

fn resolve_options() -> TransformOptions {
    TransformOptions { resolve_imported_values: true, ..Default::default() }
}

fn error_messages(result: &TransformResult) -> String {
    result.diagnostics.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")
}

#[test]
fn imported_inputs_array_evaluates() {
    let dir = TempDir::new().unwrap();
    create_test_file(dir.path(), "app/meta.ts", "export const INPUTS = ['x', 'y: why'];");
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { INPUTS } from './meta';
@Directive({ selector: 'a', inputs: INPUTS })
export class A {}
"#,
        &resolve_options(),
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert!(
        define_call(&result.code, "A").contains(r#"inputs:{x:"x",y:[0,"why","y"]}"#),
        "{}",
        define_call(&result.code, "A")
    );
}

#[test]
fn input_member_options_object_evaluates() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/meta.ts",
        "export const OPTS = { alias: 'r', required: true };",
    );
    let result = transform(
        &dir,
        r#"import { Directive, Input } from '@angular/core';
import { OPTS } from './meta';
@Directive({ selector: 'a' })
export class A { @Input(OPTS) prop?: string; }
"#,
        &resolve_options(),
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert!(
        define_call(&result.code, "A").contains(r#"inputs:{prop:[0,"r","prop"]}"#),
        "{}",
        define_call(&result.code, "A")
    );
}

#[test]
fn namespace_import_member_evaluates() {
    let dir = TempDir::new().unwrap();
    create_test_file(dir.path(), "app/meta.ts", "export const X = { alias: 'x-x' };");
    let result = transform(
        &dir,
        r#"import { Directive, Input } from '@angular/core';
import * as meta from './meta';
@Directive({ selector: 'a' })
export class A { @Input(meta.X) prop?: string; }
"#,
        &resolve_options(),
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert!(
        define_call(&result.code, "A").contains(r#"inputs:{prop:[0,"x-x","prop"]}"#),
        "{}",
        define_call(&result.code, "A")
    );
}

#[test]
fn imported_values_follow_re_export_chain() {
    let dir = TempDir::new().unwrap();
    create_test_file(dir.path(), "app/impl.ts", "export const INPUTS = ['x'];");
    create_test_file(dir.path(), "app/inner.ts", "export { INPUTS as X } from './impl';");
    create_test_file(dir.path(), "app/outer.ts", "export * from './inner';");
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { X } from './outer';
@Directive({ selector: 'a', inputs: X })
export class A {}
"#,
        &resolve_options(),
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert!(
        define_call(&result.code, "A").contains(r#"inputs:{x:"x"}"#),
        "{}",
        define_call(&result.code, "A")
    );
}

#[test]
fn read_files_are_reported_as_dependencies() {
    let dir = TempDir::new().unwrap();
    let meta = create_test_file(dir.path(), "app/meta.ts", "export const INPUTS = ['x'];");
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { INPUTS } from './meta';
@Directive({ selector: 'a', inputs: INPUTS })
export class A {}
"#,
        &resolve_options(),
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    // Compare canonicalized paths: the resolver's `full_path()` can differ
    // from `tempdir`'s form (macOS /var -> /private/var, Windows `\\?\`
    // verbatim prefixes and 8.3 short names like RUNNER~1).
    let meta = std::fs::canonicalize(&meta).unwrap();
    assert!(
        result.dependencies.iter().any(|d| std::fs::canonicalize(d).is_ok_and(|c| c == meta)),
        "dependencies {:?} should contain {}",
        result.dependencies,
        meta.display()
    );
}

#[test]
fn unresolvable_import_keeps_diagnostic() {
    let dir = TempDir::new().unwrap();
    // `require()` isn't statically evaluable; the import stays opaque.
    create_test_file(dir.path(), "app/meta.ts", "export const INPUTS = require('./other');");
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { INPUTS } from './meta';
@Directive({ selector: 'a', inputs: INPUTS })
export class A {}
"#,
        &resolve_options(),
    );
    assert!(result.has_errors());
    let errors = error_messages(&result);
    assert!(errors.contains("'INPUTS', which is imported from another module"), "{errors}");
}

#[test]
fn missing_export_keeps_diagnostic() {
    let dir = TempDir::new().unwrap();
    create_test_file(dir.path(), "app/meta.ts", "export const OTHER = ['x'];");
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { INPUTS } from './meta';
@Directive({ selector: 'a', inputs: INPUTS })
export class A {}
"#,
        &resolve_options(),
    );
    assert!(result.has_errors());
    assert!(
        error_messages(&result).contains("'INPUTS', which is imported from another module"),
        "{}",
        error_messages(&result)
    );
}

#[test]
fn feature_off_keeps_diagnostic() {
    let dir = TempDir::new().unwrap();
    create_test_file(dir.path(), "app/meta.ts", "export const INPUTS = ['x'];");
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { INPUTS } from './meta';
@Directive({ selector: 'a', inputs: INPUTS })
export class A {}
"#,
        &TransformOptions::default(),
    );
    assert!(result.has_errors());
    assert!(
        error_messages(&result).contains("'INPUTS', which is imported from another module"),
        "{}",
        error_messages(&result)
    );
}

#[test]
fn package_import_is_not_resolved() {
    let dir = TempDir::new().unwrap();
    let result = transform(
        &dir,
        r#"import { Directive } from '@angular/core';
import { INPUTS } from 'some-pkg';
@Directive({ selector: 'a', inputs: INPUTS })
export class A {}
"#,
        &resolve_options(),
    );
    assert!(result.has_errors());
    assert!(
        error_messages(&result).contains("'INPUTS', which is imported from another module"),
        "{}",
        error_messages(&result)
    );
}

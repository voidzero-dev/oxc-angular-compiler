//! Version-gated `changeDetection` emit.
//!
//! Angular reworked `ChangeDetectionStrategy` in v22: `OnPush = 0` became the
//! default and `Eager = 1` was added (`Default = 1` is a deprecated alias). The
//! full definition emits the numeric value only when it differs from the
//! value the runtime assumes for an omitted field, and that default flipped:
//!
//! - v22+: default is `OnPush` (0); emit `1` for `Eager`, omit `OnPush`.
//!   Runtime: `onPush = changeDetection !== Eager`.
//! - < v22: default is `Default`/`Eager` (1); emit `0` for `OnPush`, omit the
//!   default. Runtime: `onPush = changeDetection === OnPush`.
//!
//! So the emit must be gated on the target Angular version, or one regime
//! silently breaks the other.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{AngularVersion, TransformOptions, transform_angular_file};

fn compile(source: &str, version: Option<AngularVersion>) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { angular_version: version, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

/// The `ɵɵdefineComponent` config region, excluding the trailing
/// `ɵsetClassMetadata` (which keeps the symbolic strategy regardless).
fn define_region(code: &str) -> String {
    let start = code.find("\u{0275}\u{0275}defineComponent").expect("defineComponent");
    let end = code.find("setClassMetadata").unwrap_or(code.len());
    code[start..end].to_string()
}

fn component(field: &str) -> String {
    format!(
        "import {{ Component, ChangeDetectionStrategy }} from '@angular/core';\n\
         @Component({{ selector: 'app-x', template: ''{field} }})\n\
         export class X {{}}\n"
    )
}

fn has_change_detection(region: &str, value: u32) -> bool {
    region.contains(&format!("changeDetection:{value}"))
        || region.contains(&format!("changeDetection: {value}"))
}

const V22: Option<AngularVersion> = Some(AngularVersion { major: 22, minor: 0, patch: 0 });
const V21: Option<AngularVersion> = Some(AngularVersion { major: 21, minor: 0, patch: 0 });

// --- v22+ (and unknown version, which assumes latest) -----------------------

#[test]
fn v22_eager_emits_one() {
    let region = define_region(&compile(
        &component(", changeDetection: ChangeDetectionStrategy.Eager"),
        V22,
    ));
    assert!(has_change_detection(&region, 1), "v22 Eager should emit 1: {region}");
}

#[test]
fn v22_default_alias_emits_one() {
    let region = define_region(&compile(
        &component(", changeDetection: ChangeDetectionStrategy.Default"),
        V22,
    ));
    assert!(has_change_detection(&region, 1), "v22 Default (==Eager) should emit 1: {region}");
}

#[test]
fn v22_on_push_is_omitted() {
    let region = define_region(&compile(
        &component(", changeDetection: ChangeDetectionStrategy.OnPush"),
        V22,
    ));
    assert!(
        !region.contains("changeDetection"),
        "v22 OnPush (default) should be omitted: {region}"
    );
}

#[test]
fn unknown_version_defaults_to_v22_behavior() {
    let region = define_region(&compile(
        &component(", changeDetection: ChangeDetectionStrategy.Eager"),
        None,
    ));
    assert!(
        has_change_detection(&region, 1),
        "unknown version should assume latest (emit 1): {region}"
    );
}

// --- pre-v22 (Angular 21) — the backward-compatibility cases ----------------

#[test]
fn v21_on_push_emits_zero() {
    // The regression guard: a pre-v22 explicit OnPush must emit `0`, otherwise
    // the 17-21 runtime (`onPush = cd === OnPush`) treats the omitted field as
    // eager and the component silently loses OnPush.
    let region = define_region(&compile(
        &component(", changeDetection: ChangeDetectionStrategy.OnPush"),
        V21,
    ));
    assert!(has_change_detection(&region, 0), "pre-v22 OnPush should emit 0: {region}");
}

#[test]
fn v21_default_is_omitted() {
    let region = define_region(&compile(
        &component(", changeDetection: ChangeDetectionStrategy.Default"),
        V21,
    ));
    assert!(
        !region.contains("changeDetection"),
        "pre-v22 Default (default) should be omitted: {region}"
    );
}

#[test]
fn unspecified_is_omitted_on_both_versions() {
    for version in [V22, V21, None] {
        let region = define_region(&compile(&component(""), version));
        assert!(
            !region.contains("changeDetection"),
            "an unspecified strategy must be omitted (version {version:?}): {region}"
        );
    }
}

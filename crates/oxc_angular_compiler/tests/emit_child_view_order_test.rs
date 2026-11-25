//! Embedded view functions are declared in Angular's emit order: a view's
//! children come right before it, depth first, and siblings keep their
//! template order (`emitChildViews` in Angular's `emit.ts`).
//!
//! The expected lists are the `pool.statements` names that
//! `@angular/compiler` 22.3.0 gives for the same templates.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

/// Names of the embedded view functions (`function X_..._Template(`), in
/// output order. The root `X_Template` is left out.
fn view_fn_names(template: &str) -> Vec<String> {
    let source = format!(
        "import {{ Component }} from '@angular/core';\n\
         @Component({{ selector: 'x', template: `{template}` }})\n\
         export class X {{}}\n"
    );
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "/x/a.ts",
        &source,
        Some(&TransformOptions::default()),
        None,
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result
        .code
        .split("function ")
        .skip(1)
        .filter_map(|rest| {
            let name = rest.split('(').next()?;
            (name != "X_Template" && name.starts_with("X_") && name.ends_with("_Template"))
                .then(|| name.to_string())
        })
        .collect()
}

#[test]
fn mixed_blocks_match_angular_order() {
    let template = "@if (a) {@if (b) {x} @for (i of c; track i) {y} @empty {e}} \
                    @switch (a) { @case (1) {p @if (q) {r}} @default {d} } \
                    <ng-template><b>@if (k) {m}</b></ng-template> \
                    @if (z) {u} @else if (y) {t} @else {s}";
    assert_eq!(
        view_fn_names(template),
        [
            "X_Conditional_0_Conditional_0_Template",
            "X_Conditional_0_For_2_Template",
            "X_Conditional_0_ForEmpty_3_Template",
            "X_Conditional_0_Template",
            "X_Case_1_Conditional_1_Template",
            "X_Case_1_Template",
            "X_Case_2_Template",
            "X_ng_template_3_Conditional_1_Template",
            "X_ng_template_3_Template",
            "X_Conditional_4_Template",
            "X_Conditional_5_Template",
            "X_Conditional_6_Template",
        ]
    );
}

#[test]
fn many_siblings_keep_order_with_children_first() {
    let n = 300;
    let template = "@if (a) {@if (b) {x}} ".repeat(n);
    let expected: Vec<String> = (0..n)
        .flat_map(|k| {
            [
                format!("X_Conditional_{k}_Conditional_0_Template"),
                format!("X_Conditional_{k}_Template"),
            ]
        })
        .collect();
    assert_eq!(view_fn_names(&template), expected);
}

#[test]
fn deep_chain_emits_innermost_first() {
    let depth = 40;
    let template = format!("{}x{}", "@if (a) {".repeat(depth), "}".repeat(depth));
    let expected: Vec<String> =
        (1..=depth).rev().map(|d| format!("X{}_Template", "_Conditional_0".repeat(d))).collect();
    assert_eq!(view_fn_names(&template), expected);
}

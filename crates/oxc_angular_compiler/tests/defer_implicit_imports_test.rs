//! Implicit `@defer` dependencies (#313): a standalone component or directive
//! listed in `@Component.imports` and matched only inside `@defer` blocks is
//! lazy-loaded through a dynamic `import()`, the way ngc's full compilation
//! does, when cross-file analysis can read its selector
//! (`TransformOptions::resolve_imported_values`).

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

fn options() -> TransformOptions {
    TransformOptions { resolve_imported_values: true, ..Default::default() }
}

fn transform_with(dir: &TempDir, source: &str, options: &TransformOptions) -> TransformResult {
    let path = create_test_file(dir.path(), "app/parent.component.ts", source);
    let allocator = Allocator::default();
    let result =
        transform_angular_file(&allocator, path.to_str().unwrap(), source, Some(options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    result
}

fn transform(dir: &TempDir, source: &str) -> TransformResult {
    transform_with(dir, source, &options())
}

fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn lazy_cmp(dir: &TempDir) {
    create_test_file(
        dir.path(),
        "app/lazy.ts",
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-lazy', template: 'lazy' })
export class LazyCmp {}
"#,
    );
}

fn assert_lazy(code: &str, module: &str, member: &str) {
    let compact = compact(code);
    assert!(
        compact.contains(&format!("import(\"{module}\").then((m)=>m.{member})"))
            || compact.contains(&format!("import(\"{module}\").then(m=>m.{member})")),
        "Expected `import('{module}').then(m => m.{member})`. Output:\n{code}"
    );
}

fn assert_eager(code: &str, module: &str) {
    assert!(
        !code.contains(&format!("import(\"{module}\")")),
        "`{module}` must stay an eager dependency. Output:\n{code}"
    );
}

#[test]
fn component_used_only_inside_defer_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    let compact = compact(code);

    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(
        compact.contains("constParent_Defer_1_DepsFn=()=>[import(\"./lazy\")")
            && compact.contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"),
        "The block's resolver must be wired into `ɵɵdefer`. Output:\n{code}"
    );
    // Like ngc: nothing is used eagerly, so there is no `dependencies`.
    assert!(
        !compact.contains("dependencies:"),
        "The lazy component must leave the eager dependencies. Output:\n{code}"
    );
    assert!(
        !code.contains("import { LazyCmp }"),
        "The static import of the lazy component must be removed. Output:\n{code}"
    );
    assert!(code.contains("setClassMetadataAsync"), "Output:\n{code}");
}

#[test]
fn component_also_used_outside_defer_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '<app-lazy/> @defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert_eager(code, "./lazy");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[LazyCmp])"), "{code}");
    assert!(code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn component_used_in_a_placeholder_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> } @placeholder { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_eager(&result.code, "./lazy");
}

#[test]
fn component_used_in_nested_control_flow_inside_defer_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { @switch (mode) { @case (1) { <div><app-lazy/></div> } } }',
})
export class Parent { mode = 1; }
"#,
    );
    assert_lazy(&result.code, "./lazy", "LazyCmp");
}

#[test]
fn aliased_import_reads_the_exported_name() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp as Lazy } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [Lazy],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_lazy(&result.code, "./lazy", "LazyCmp");
}

#[test]
fn default_import_reads_default() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/lazy.ts",
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-lazy', template: 'lazy' })
export default class LazyCmp {}
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import LazyCmp from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_lazy(&result.code, "./lazy", "default");
}

/// Port of ngtsc's `defer_spec.ts`: "should lazy-load dependency referenced
/// with a fowrardRef".
#[test]
fn forward_ref_import_used_only_inside_defer_is_lazy_loaded() {
    for entry in [
        "forwardRef(() => LazyCmp)",
        "forwardRef(() => { return LazyCmp; })",
        "forwardRef(function () { return LazyCmp; })",
    ] {
        let dir = TempDir::new().unwrap();
        lazy_cmp(&dir);
        let result = transform(
            &dir,
            &format!(
                r#"
import {{ Component, forwardRef }} from '@angular/core';
import {{ LazyCmp }} from './lazy';

@Component({{
    selector: 'app-parent',
    imports: [{entry}],
    template: '@defer {{ <app-lazy/> }}',
}})
export class Parent {{}}
"#
            ),
        );
        let code = &result.code;
        assert_lazy(code, "./lazy", "LazyCmp");
        assert!(compact(code).contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"), "{code}");
        assert!(!compact(code).contains("dependencies:"), "{code}");
        assert!(!code.contains("import { LazyCmp }"), "{code}");
    }
}

#[test]
fn forward_ref_entry_next_to_a_cast_entry_is_lazy_loaded_alone() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    create_test_file(
        dir.path(),
        "app/eager.ts",
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-eager', template: 'eager' })
export class EagerCmp {}
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component, forwardRef } from '@angular/core';
import { LazyCmp } from './lazy';
import { EagerCmp } from './eager';

@Component({
    selector: 'app-parent',
    imports: [forwardRef(() => LazyCmp), EagerCmp as any],
    template: '@defer { <app-lazy/> <app-eager/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[EagerCmp])"), "{code}");
    assert!(code.contains("import { EagerCmp }"), "{code}");
}

fn shared_other_cmp(dir: &TempDir) {
    other_cmp(dir);
    create_test_file(
        dir.path(),
        "app/shared.ts",
        r"
import { OtherCmp } from './other';

export const SHARED = [OtherCmp];
",
    );
}

#[test]
fn entry_next_to_a_spread_entry_is_lazy_loaded() {
    // ngc's `registerDeferrableCandidate` skips the spread, and the classes it
    // brings in stay eager.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    shared_other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { SHARED } from './shared';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, ...SHARED],
    template: '@defer { <app-lazy/> } <app-other/>',
})
export class Parent {}
",
    );
    let code = &result.code;
    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(compact(code).contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"), "{code}");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[...SHARED])"), "{code}");
    assert!(!code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn entry_deferred_by_two_components_one_with_a_spread_entry_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    shared_other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { SHARED } from './shared';

@Component({ selector: 'app-a', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
export class ParentA {}

@Component({
    selector: 'app-b',
    imports: [LazyCmp, ...SHARED],
    template: '@defer { <app-lazy/> } <app-other/>',
})
export class ParentB {}
",
    );
    let code = &result.code;
    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(
        compact(code).contains("setClassMetadataAsync(ParentA,()=>[import(\"./lazy\")")
            && compact(code).contains("setClassMetadataAsync(ParentB,()=>[import(\"./lazy\")"),
        "{code}"
    );
    assert!(!code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn entry_next_to_a_nested_array_entry_is_lazy_loaded() {
    // Like a spread, ngc skips the nested array, and its classes stay eager.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let source = r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, [OtherCmp]],
    template: '@defer { <app-lazy/> } <app-other/>',
})
export class Parent {}
";
    let code = &transform(&dir, source).code;
    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(compact(code).contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"), "{code}");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[[OtherCmp]])"), "{code}");
    assert!(!code.contains("import { LazyCmp }"), "{code}");

    let code = &transform_with(&dir, source, &partial_options()).code;
    assert!(
        compact(code)
            .contains("deferBlockDependencies:[()=>[import(\"./lazy\").then((m)=>m.LazyCmp)]]"),
        "{code}"
    );
    assert!(!code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn entry_next_to_a_namespace_member_entry_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let code = &transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import * as other from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, other.OtherCmp],
    template: '@defer { <app-lazy/> } <app-other/>',
})
export class Parent {}
",
    )
    .code;
    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[other.OtherCmp])"), "{code}");
    assert!(!code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn deps_fns_list_classes_of_entries_that_are_not_identifiers() {
    // ngc's scope flattens nested arrays and spreads and resolves `ns.Cmp`, so
    // a block's function lists what they bring in, eagerly, once per class:
    // `other.LazyCmp` is `LazyCmp`, which its identifier entry lazy-loads.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    create_test_file(
        dir.path(),
        "app/third.ts",
        r"
import { Component } from '@angular/core';

@Component({ selector: 'app-third', template: 'third' })
export class ThirdCmp {}

export { LazyCmp } from './lazy';
",
    );
    let source = r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';
import * as third from './third';

const SHARED = [third.ThirdCmp];

@Component({
    selector: 'app-parent',
    imports: [third.LazyCmp, LazyCmp, [OtherCmp], ...SHARED],
    template: '@defer { <app-third/><app-other/><app-lazy/> }',
})
export class Parent {}
";
    let code = &transform(&dir, source).code;
    assert!(
        compact(code).contains(
            "constParent_Defer_1_DepsFn=()=>[import(\"./lazy\").then((m)=>m.LazyCmp),OtherCmp,third.ThirdCmp];"
        ),
        "{code}"
    );
    assert!(!code.contains("import { LazyCmp }"), "{code}");

    let code = &transform_with(&dir, source, &partial_options()).code;
    assert!(
        compact(code).contains(
            "deferBlockDependencies:[()=>[import(\"./lazy\").then((m)=>m.LazyCmp),OtherCmp,third.ThirdCmp]]"
        ),
        "{code}"
    );
}

#[test]
fn partial_dependencies_list_a_class_two_entries_name_once() {
    // `more.LazyCmp` is `LazyCmp`: ngc's scope lists it once, and a component
    // matched twice on one element is an error at runtime.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    create_test_file(
        dir.path(),
        "app/more.ts",
        "export { LazyCmp } from './lazy';\nexport { OtherCmp } from './other';\n",
    );
    let source = r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import * as more from './more';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, more.LazyCmp, more.OtherCmp],
    template: '<app-lazy/><app-other/>',
})
export class Parent {}
";
    let code = &transform_with(&dir, source, &partial_options()).code;
    assert!(
        compact(code).contains(
            "dependencies:[{kind:\"directive\",type:LazyCmp,selector:\"*\"},{kind:\"directive\",type:more.OtherCmp,selector:\"*\"}]"
        ),
        "{code}"
    );
}

#[test]
fn partial_dependencies_leave_out_a_lazy_loaded_class_another_entry_names() {
    // `@defer` lazy-loads `LazyCmp`, which `more.LazyCmp` names too: ngc's
    // `dependencies` leaves it out, in whichever order the entries come.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    create_test_file(
        dir.path(),
        "app/more.ts",
        "export { LazyCmp } from './lazy';\nexport { OtherCmp } from './other';\n",
    );
    for imports in
        ["[LazyCmp, more.LazyCmp, more.OtherCmp]", "[more.LazyCmp, LazyCmp, more.OtherCmp]"]
    {
        let source = format!(
            r"
import {{ Component }} from '@angular/core';
import {{ LazyCmp }} from './lazy';
import * as more from './more';

@Component({{
    selector: 'app-parent',
    imports: {imports},
    template: '<app-other/> @defer {{ <app-lazy/> }}',
}})
export class Parent {{}}
"
        );
        let code = &transform_with(&dir, &source, &partial_options()).code;
        assert!(
            compact(code)
                .contains("dependencies:[{kind:\"directive\",type:more.OtherCmp,selector:\"*\"}]"),
            "{code}"
        );
        assert_lazy(code, "./lazy", "LazyCmp");
    }
}

#[test]
fn forward_ref_not_from_angular_core_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { forwardRef } from './my-forward-ref';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [forwardRef(() => LazyCmp)],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_eager(&result.code, "./lazy");
}

#[test]
fn import_referenced_elsewhere_in_the_file_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

export const EXPOSED = LazyCmp;

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_eager(&result.code, "./lazy");
}

#[test]
fn attribute_directive_used_only_inside_defer_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/tooltip.ts",
        r#"
import { Directive } from '@angular/core';

@Directive({ selector: '[appTooltip]' })
export class Tooltip {}
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { Tooltip } from './tooltip';

@Component({
    selector: 'app-parent',
    imports: [Tooltip],
    template: '<button>eager</button> @defer { <button appTooltip>lazy</button> }',
})
export class Parent {}
"#,
    );
    assert_lazy(&result.code, "./tooltip", "Tooltip");
}

#[test]
fn bound_input_matches_an_attribute_selector() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/tooltip.ts",
        r#"
import { Directive, input } from '@angular/core';

@Directive({ selector: '[appTooltip]' })
export class Tooltip { appTooltip = input(''); }
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { Tooltip } from './tooltip';

@Component({
    selector: 'app-parent',
    imports: [Tooltip],
    template: '<button [appTooltip]="text">eager</button> @defer { <span appTooltip></span> }',
})
export class Parent { text = ''; }
"#,
    );
    assert_eager(&result.code, "./tooltip");
}

#[test]
fn not_standalone_component_stays_eager() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/lazy.ts",
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-lazy', template: 'lazy', standalone: false })
export class LazyCmp {}
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_eager(&result.code, "./lazy");
}

#[test]
fn pipe_used_only_inside_defer_is_lazy_loaded() {
    // ngc matches a standalone pipe by its name: used only inside `@defer`,
    // it is lazy-loaded like a component, and listed after the directives.
    // A `when` condition is part of the enclosing template.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    create_test_file(
        dir.path(),
        "app/shout.ts",
        r"
import { Pipe } from '@angular/core';

@Pipe({ name: 'shout' })
export class ShoutPipe { transform(v: string) { return v; } }
",
    );
    create_test_file(
        dir.path(),
        "app/whisper.ts",
        r"
import { Pipe } from '@angular/core';

@Pipe({ name: 'whisper' })
export class WhisperPipe { transform(v: string) { return v; } }
",
    );
    let source = r"
import { Component } from '@angular/core';
import { ShoutPipe } from './shout';
import { WhisperPipe } from './whisper';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [ShoutPipe, LazyCmp, WhisperPipe],
    template: '@defer { {{ name | shout }} <app-lazy/> } @defer (when name | whisper) { {{ name | whisper }} }',
})
export class Parent { name = ''; }
";
    let code = &transform(&dir, source).code;
    assert!(
        compact(code).contains(
            "constParent_Defer_1_DepsFn=()=>[import(\"./lazy\").then((m)=>m.LazyCmp),import(\"./shout\").then((m)=>m.ShoutPipe)];"
        ),
        "{code}"
    );
    assert!(compact(code).contains("constParent_Defer_4_DepsFn=()=>[WhisperPipe];"), "{code}");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[WhisperPipe])"), "{code}");
    assert!(code.contains("import { WhisperPipe } from './whisper'"), "{code}");
    assert!(!code.contains("import { ShoutPipe }"), "{code}");

    let code = &transform_with(&dir, source, &partial_options()).code;
    assert!(
        compact(code).contains(
            "deferBlockDependencies:[()=>[import(\"./lazy\").then((m)=>m.LazyCmp),import(\"./shout\").then((m)=>m.ShoutPipe)],()=>[WhisperPipe]]"
        ),
        "{code}"
    );
}

#[test]
fn implicit_and_explicit_deferred_imports_share_the_resolver() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { Other } from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    deferredImports: [Other],
    template: '@defer { <app-lazy/> <app-other/> }',
})
export class Parent {}
"#,
    );
    assert_lazy(&result.code, "./lazy", "LazyCmp");
    assert_lazy(&result.code, "./other", "Other");
}

#[test]
fn hmr_keeps_imports_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform_with(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
        &TransformOptions { hmr: true, ..options() },
    );
    assert_eager(&result.code, "./lazy");
}

// ---------------------------------------------------------------------------
// Selector matching (`pipeline/selector.rs`, `component/defer_resolver.rs`):
// elements are matched the way ngc's `createCssSelectorFromNode` and
// `SelectorMatcher` match them.
// ---------------------------------------------------------------------------

/// Writes `app/dep.ts`, a standalone directive `Dep` with `selector`.
fn selector_matching_dep(dir: &TempDir, selector: &str) {
    create_test_file(
        dir.path(),
        "app/dep.ts",
        &format!(
            r#"
import {{ Directive }} from '@angular/core';

@Directive({{ selector: '{selector}' }})
export class Dep {{}}
"#
        ),
    );
}

/// Compiles a parent component that imports `Dep` and uses `template`.
fn selector_matching_transform(dir: &TempDir, template: &str) -> TransformResult {
    transform(
        dir,
        &format!(
            r#"
import {{ Component }} from '@angular/core';
import {{ Dep }} from './dep';

@Component({{
    selector: 'app-parent',
    imports: [Dep],
    template: `{template}`,
}})
export class Parent {{ x = ''; }}
"#
        ),
    )
}

#[test]
fn selector_matching_svg_element_used_eagerly_stays_eager() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "svg[appIcon], i[appIcon]");
    let result =
        selector_matching_transform(&dir, "<svg appIcon></svg> @defer { <i appIcon></i> }");
    assert_eager(&result.code, "./dep");
}

#[test]
fn selector_matching_element_inside_svg_used_eagerly_stays_eager() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "a[appLink]");
    let result =
        selector_matching_transform(&dir, "<svg><a appLink></a></svg> @defer { <a appLink></a> }");
    assert_eager(&result.code, "./dep");
}

#[test]
fn selector_matching_element_inside_svg_used_only_inside_defer_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "g[appDep]");
    let result = selector_matching_transform(&dir, "@defer { <svg><g appDep></g></svg> }");
    assert_lazy(&result.code, "./dep", "Dep");
}

#[test]
fn selector_matching_namespaced_attribute_used_eagerly_stays_eager() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "[href]");
    let result = selector_matching_transform(
        &dir,
        r#"<svg><a xlink:href="x"></a></svg> @defer { <a href="x"></a> }"#,
    );
    assert_eager(&result.code, "./dep");
}

#[test]
fn selector_matching_only_i18n_attributes_are_skipped() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "[i18nTip]");
    let result =
        selector_matching_transform(&dir, r#"<b i18nTip></b> @defer { <b [i18nTip]="x"></b> }"#);
    assert_eager(&result.code, "./dep");
}

#[test]
fn selector_matching_class_attribute_name_is_case_insensitive() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, ".foo");
    let result =
        selector_matching_transform(&dir, r#"<b CLASS="foo"></b> @defer { <b class="foo"></b> }"#);
    assert_eager(&result.code, "./dep");
}

#[test]
fn selector_matching_not_only_selector_matches_any_element() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, ":not(.plain)");
    let result = selector_matching_transform(&dir, "@defer { <span></span> }");
    assert_lazy(&result.code, "./dep", "Dep");

    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "[appDep], :not([appDep]):not(span)");
    let result = selector_matching_transform(&dir, "<div></div> @defer { <span appDep></span> }");
    assert_eager(&result.code, "./dep");
}

#[test]
fn selector_matching_empty_selector_in_a_list_matches_nothing() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "[appDep],");
    let result = selector_matching_transform(&dir, "<div></div> @defer { <span appDep></span> }");
    assert_lazy(&result.code, "./dep", "Dep");
}

#[test]
fn selector_matching_binding_overrides_static_attribute_value() {
    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, "[type=a]");
    let result = selector_matching_transform(
        &dir,
        r#"<input type="a" [type]="x"/> @defer { <input type="a"/> }"#,
    );
    assert_lazy(&result.code, "./dep", "Dep");

    let dir = TempDir::new().unwrap();
    selector_matching_dep(&dir, ".foo");
    let result = selector_matching_transform(
        &dir,
        r#"<div class="foo" [class]="x"></div> @defer { <div class="foo"></div> }"#,
    );
    assert_lazy(&result.code, "./dep", "Dep");
}

#[test]
fn selector_matching_placeholder_of_a_nested_defer_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { @defer { <p>x</p> } @placeholder { <app-lazy/> } }',
})
export class Parent {}
"#,
    );
    assert_lazy(&result.code, "./lazy", "LazyCmp");
}

// --- Import removal: names other emitted code needs, and ngc's
// per-declaration `canDefer` rule ---

/// `app/cmps.ts` with two standalone components, `app-lazy` and `app-eager`,
/// a `TOKEN` value and a `CmpOptions` interface.
fn two_cmps(dir: &TempDir) {
    create_test_file(
        dir.path(),
        "app/cmps.ts",
        r#"
import { Component, InjectionToken } from '@angular/core';

@Component({ selector: 'app-lazy', template: 'lazy' })
export class LazyCmp {}

@Component({ selector: 'app-eager', template: 'eager' })
export class EagerCmp {}

export const TOKEN = new InjectionToken<string>('token');

export interface CmpOptions {}
"#,
    );
}

/// A parent deferring `LazyCmp` next to a `jit: true` class that injects it.
fn jit_sibling_source(jit_first: bool) -> String {
    let jit = r#"
@Component({ selector: 'app-jit', template: '', jit: true })
export class JitCmp {
    constructor(lazy: LazyCmp) {}
}
"#;
    let parent = r#"
@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#;
    let (first, second) = if jit_first { (jit, parent) } else { (parent, jit) };
    format!(
        "import {{ Component }} from '@angular/core';\nimport {{ LazyCmp }} from './lazy';\n{first}{second}"
    )
}

#[test]
fn jit_sibling_ctor_parameter_keeps_import_eager_when_declared_first() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(&dir, &jit_sibling_source(true));
    let code = &result.code;
    assert!(compact(code).contains("{type:LazyCmp}"), "{code}");
    assert!(code.contains("import { LazyCmp }"), "{code}");
    assert_eager(code, "./lazy");
}

#[test]
fn jit_sibling_ctor_parameter_keeps_import_eager_when_declared_last() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(&dir, &jit_sibling_source(false));
    let code = &result.code;
    assert!(compact(code).contains("{type:LazyCmp}"), "{code}");
    assert!(code.contains("import { LazyCmp }"), "{code}");
    assert_eager(code, "./lazy");
}

#[test]
fn default_import_in_sibling_ctor_metadata_stays_eager() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/lazy.ts",
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-lazy', template: 'lazy' })
export default class LazyCmp {}
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component, Injectable } from '@angular/core';
import LazyCmp from './lazy';

@Injectable()
export class Svc {
    constructor(lazy: LazyCmp) {}
}

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert!(compact(code).contains("{type:LazyCmp}"), "{code}");
    assert!(code.contains("import LazyCmp from"), "{code}");
    assert_eager(code, "./lazy");
}

#[test]
fn deferred_component_injected_by_a_sibling_uses_a_namespace_import() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component, Injectable } from '@angular/core';
import { LazyCmp } from './lazy';

@Injectable()
export class Svc {
    constructor(lazy: LazyCmp) {}
}

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    // Like ngc: the token goes through a namespace import, so the removed
    // named import leaves no dangling reference.
    assert_lazy(code, "./lazy", "LazyCmp");
    assert!(!code.contains("import { LazyCmp }"), "{code}");
    assert!(code.contains("import * as i1 from './lazy'"), "{code}");
    assert!(compact(code).contains("ɵɵinject(i1.LazyCmp)"), "{code}");
}

#[test]
fn sibling_specifier_used_eagerly_keeps_the_declaration_eager() {
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp, EagerCmp } from './cmps';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, EagerCmp],
    template: '<app-eager/> @defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert_eager(code, "./cmps");
    assert!(code.contains("import { LazyCmp, EagerCmp }"), "{code}");
    // Like ngc: `LazyCmp` stays eager but is used only by the block, whose
    // function provides it.
    assert!(compact(code).contains("constParent_Defer_2_DepsFn=()=>[LazyCmp];"), "{code}");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[EagerCmp])"), "{code}");
}

#[test]
fn sibling_value_specifier_keeps_the_declaration_eager() {
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component, inject } from '@angular/core';
import { LazyCmp, TOKEN } from './cmps';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {
    token = inject(TOKEN);
}
"#,
    );
    let code = &result.code;
    assert_eager(code, "./cmps");
    assert!(code.contains("import { LazyCmp, TOKEN }"), "{code}");
}

#[test]
fn inline_type_specifier_does_not_block_deferral() {
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { type EagerCmp, LazyCmp } from './cmps';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {
    other?: EagerCmp;
}
"#,
    );
    let code = &result.code;
    assert_lazy(code, "./cmps", "LazyCmp");
    assert!(!code.contains("from \"./cmps\""), "{code}");
}

#[test]
fn all_specifiers_deferred_removes_the_declaration() {
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp, EagerCmp } from './cmps';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, EagerCmp],
    template: '@defer { <app-lazy/> <app-eager/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert_lazy(code, "./cmps", "LazyCmp");
    assert_lazy(code, "./cmps", "EagerCmp");
    assert!(!compact(code).contains("dependencies:"), "{code}");
    assert!(!code.contains("from \"./cmps\""), "{code}");
}

#[test]
fn sibling_specifier_in_deferred_imports_keeps_the_declaration_eager() {
    // The explicit entry's static import stays, so the declaration does too.
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp, EagerCmp } from './cmps';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    deferredImports: [EagerCmp],
    template: '@defer { <app-lazy/> <app-eager/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert!(code.contains("import { LazyCmp, EagerCmp }"), "{code}");
    assert!(compact(code).contains("ɵɵgetComponentDepsFactory(Parent,[LazyCmp])"), "{code}");
    assert!(!code.contains("m.LazyCmp"), "{code}");
}

#[test]
fn sibling_interface_without_type_keeps_the_declaration_eager() {
    // Like ngc, which can't tell an interface from a value here (`AssumeEager`).
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp, CmpOptions } from './cmps';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {
    options?: CmpOptions;
}
"#,
    );
    let code = &result.code;
    assert_eager(code, "./cmps");
    assert!(compact(code).contains("constParent_Defer_1_DepsFn=()=>[LazyCmp];"), "{code}");
    assert!(!compact(code).contains("dependencies:"), "{code}");
}

#[test]
fn computed_type_key_keeps_import_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {
    keyed?: { [LazyCmp]: string };
}
"#,
    );
    let code = &result.code;
    assert!(code.contains("import { LazyCmp }"), "{code}");
    assert_eager(code, "./lazy");
}

// ---------------------------------------------------------------------------
// Cross-file selector lookup (`cross_file_elision.rs`)
// ---------------------------------------------------------------------------

#[test]
fn namespace_member_does_not_select_an_unexported_class() {
    // `NS.LazyCmp` is recorded as the barrel's `LazyCmp` export, but the
    // top-level `LazyCmp` class is not exported as `LazyCmp` (at most under
    // another name). The `LazyCmp` the barrel exports comes from `./real`
    // (`app-real`), which is used eagerly.
    for export_list in ["", "export { LazyCmp as Alias };"] {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "app/real.ts",
            r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-real', template: 'real' })
export class LazyCmp {}
"#,
        );
        create_test_file(
            dir.path(),
            "app/barrel.ts",
            &format!(
                r#"
import {{ Component }} from '@angular/core';

export * from './real';

namespace NS {{
    export class LazyCmp {{}}
}}

@Component({{ selector: 'app-lazy', template: 'lazy' }})
class LazyCmp {{}}

{export_list}
"#
            ),
        );
        let result = transform(
            &dir,
            r#"
import { Component } from '@angular/core';
import { LazyCmp } from './barrel';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '<app-real/> @defer { <app-lazy/> }',
})
export class Parent {}
"#,
        );
        assert_eager(&result.code, "./barrel");
    }
}

#[test]
fn class_exported_by_a_local_export_list_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/lazy.ts",
        r#"
import { Component } from '@angular/core';

@Component({ selector: 'app-lazy', template: 'lazy' })
class Impl {}

export { Impl as LazyCmp };
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    assert_lazy(&result.code, "./lazy", "LazyCmp");
}

#[test]
fn computed_selector_key_stays_eager() {
    // The computed `[S]` key overrides `selector` with `app-b`, which is
    // used eagerly.
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/lazy.ts",
        r#"
import { Component } from '@angular/core';

const S = 'selector';

@Component({ selector: 'app-a', [S]: 'app-b', template: 'lazy' })
export class LazyCmp {}
"#,
    );
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '<app-b/> @defer { <app-a/> }',
})
export class Parent {}
"#,
    );
    assert_eager(&result.code, "./lazy");
}

// --- Per-block dependency functions: ngc's full compilation gives each
// `@defer` block its own `<Cmp>_Defer_<slot>_DepsFn` (`resolveDeferBlocks`,
// `compileDeferBlocks` and `compileDeferResolverFunction`) ---

/// `app/other.ts`, a standalone component `OtherCmp` with `app-other`.
fn other_cmp(dir: &TempDir) {
    create_test_file(
        dir.path(),
        "app/other.ts",
        r"
import { Component } from '@angular/core';

@Component({ selector: 'app-other', template: 'other' })
export class OtherCmp {}
",
    );
}

#[test]
fn each_defer_block_loads_only_its_own_dependencies() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, OtherCmp],
    template: '@defer { <app-lazy/> } @defer (on viewport) { <app-other/> } @placeholder { <p>x</p> }',
})
export class Parent {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(
        compact.contains(
            "constParent_Defer_1_DepsFn=()=>[import(\"./lazy\").then((m)=>m.LazyCmp)];\
             constParent_Defer_5_DepsFn=()=>[import(\"./other\").then((m)=>m.OtherCmp)];"
        ),
        "Each block must get its own resolver. Output:\n{code}"
    );
    assert!(compact.contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"), "{code}");
    assert!(compact.contains("ɵɵdefer(5,3,Parent_Defer_5_DepsFn,null,4)"), "{code}");
    assert!(!compact.contains("dependencies:"), "{code}");
    assert!(
        compact.contains(
            "setClassMetadataAsync(Parent,()=>[import(\"./lazy\").then((m)=>m.LazyCmp),\
             import(\"./other\").then((m)=>m.OtherCmp)],(LazyCmp,OtherCmp)=>"
        ),
        "{code}"
    );
}

#[test]
fn eager_dependency_used_in_a_defer_block_is_listed_by_reference() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '<app-lazy/> @defer { <app-lazy/> }',
})
export class Parent {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert_eager(code, "./lazy");
    assert!(compact.contains("constParent_Defer_2_DepsFn=()=>[LazyCmp];"), "{code}");
    assert!(compact.contains("ɵɵdefer(2,1,Parent_Defer_2_DepsFn)"), "{code}");
    assert!(compact.contains("ɵɵgetComponentDepsFactory(Parent,[LazyCmp])"), "{code}");
}

#[test]
fn import_referenced_elsewhere_is_listed_by_reference_next_to_a_lazy_one() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component, ViewChild } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, OtherCmp],
    template: '@defer { <app-lazy/><app-other/> }',
})
export class Parent { @ViewChild(LazyCmp) lazy: any; }
",
    );
    let code = &result.code;
    assert!(
        compact(code).contains(
            "constParent_Defer_1_DepsFn=()=>[LazyCmp,import(\"./other\").then((m)=>m.OtherCmp)];"
        ),
        "{code}"
    );
}

#[test]
fn same_file_component_used_in_a_defer_block_is_listed_by_reference() {
    let dir = TempDir::new().unwrap();
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';

@Component({ selector: 'app-local', template: 'local' })
class LocalCmp {}

@Component({
    selector: 'app-parent',
    imports: [LocalCmp],
    template: '@defer { <app-local/> }',
})
export class Parent {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(compact.contains("constParent_Defer_1_DepsFn=()=>[LocalCmp];"), "{code}");
    assert!(compact.contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"), "{code}");
    // Like ngc: the block's function provides `LocalCmp`, so it leaves
    // `dependencies`.
    assert!(!compact.contains("dependencies:"), "{code}");
    assert!(!code.contains("setClassMetadataAsync"), "{code}");
}

#[test]
fn defer_block_mixes_a_lazy_import_and_a_same_file_component() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({ selector: 'app-local', template: 'local' })
export class LocalCmp {}

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, LocalCmp],
    template: '@defer { <app-lazy/><app-local/> }',
})
export class Parent {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(
        compact.contains(
            "constParent_Defer_1_DepsFn=()=>[import(\"./lazy\").then((m)=>m.LazyCmp),LocalCmp];"
        ),
        "{code}"
    );
    assert!(!compact.contains("dependencies:"), "{code}");
}

#[test]
fn nested_defer_block_placeholder_belongs_to_the_outer_block() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, OtherCmp],
    template: '@defer { @defer (on viewport) { <app-other/> } @placeholder { <app-lazy/> } }',
})
export class Parent {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(
        compact
            .contains("constParent_Defer_1_DepsFn=()=>[import(\"./lazy\").then((m)=>m.LazyCmp)];"),
        "{code}"
    );
    assert!(
        compact.contains(
            "constParent_Defer_0_Defer_2_DepsFn=()=>[import(\"./other\").then((m)=>m.OtherCmp)];"
        ),
        "{code}"
    );
}

#[test]
fn class_metadata_lists_lazy_dependencies_block_by_block() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({
    selector: 'app-parent',
    imports: [OtherCmp, LazyCmp],
    template: '@defer { <app-lazy/> } @defer (on viewport) { <app-other/> } @placeholder { <p>x</p> }',
})
export class Parent {}
",
    );
    let code = &result.code;
    assert!(
        compact(code).contains(
            "setClassMetadataAsync(Parent,()=>[import(\"./lazy\").then((m)=>m.LazyCmp),\
             import(\"./other\").then((m)=>m.OtherCmp)],(LazyCmp,OtherCmp)=>"
        ),
        "{code}"
    );
}

#[test]
fn defer_block_without_dependencies_has_no_resolver() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <p>x</p> } <app-lazy/>',
})
export class Parent {}
",
    );
    let code = &result.code;
    assert!(compact(code).contains("ɵɵdefer(1,0);"), "{code}");
    assert!(!code.contains("DepsFn"), "{code}");
}

#[test]
fn deps_fn_name_drops_only_the_first_template_suffix() {
    // ngc names the function after `unit.fnName?.replace('_Template', '')`,
    // which replaces the first match only.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({ selector: 'app-a', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
export class Foo {}

@Component({ selector: 'app-b', imports: [OtherCmp], template: '@defer { <app-other/> }' })
export class Foo_Template {}

@Component({
    selector: 'app-c',
    imports: [LazyCmp],
    template: '@if (x) { @defer { <app-lazy/> } }',
})
export class My_TemplateCmp {
    x = true;
}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(compact.contains("constFoo_Defer_1_DepsFn=()=>[import(\"./lazy\")"), "{code}");
    assert!(compact.contains("ɵɵdefer(1,0,Foo_Defer_1_DepsFn)"), "{code}");
    assert!(
        compact.contains("constFoo_Template_Defer_1_DepsFn=()=>[import(\"./other\")"),
        "{code}"
    );
    assert!(compact.contains("ɵɵdefer(1,0,Foo_Template_Defer_1_DepsFn)"), "{code}");
    // `My_TemplateCmp`'s block loads the same thing as `Foo`'s, and ngc's
    // file-wide pool hands it `Foo_Defer_1_DepsFn`.
    assert_eq!(compact.matches("ɵɵdefer(1,0,Foo_Defer_1_DepsFn)").count(), 2, "{code}");
    assert!(!compact.contains("MyCmp_Conditional_0_Template_Defer_1_DepsFn"), "{code}");
    assert_eq!(code.matches("const Foo_Defer_1_DepsFn").count(), 1, "{code}");
}

#[test]
fn deps_fns_do_not_move_the_constant_names_of_later_components() {
    // ngc pools the function under its own name (`getSharedFunctionReference`
    // with `useUniqueName` false), which takes no counter value.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let source = |imports: &str| {
        format!(
            r#"
import {{ Component }} from '@angular/core';
import {{ LazyCmp }} from './lazy';

@Component({{ selector: 'app-local', template: 'local' }})
export class LocalCmp {{}}

@Component({{
    selector: 'app-a',
    imports: [{imports}],
    template: '<p [title]="[1, x]"></p> @defer {{ <app-lazy [title]="[x]"/> }} @defer {{ <app-local/> }}',
}})
export class A1 {{
    x = 1;
}}

@Component({{
    selector: 'app-b',
    imports: [LocalCmp],
    template: '<p [title]="[2, x]"></p> @defer {{ <app-local [title]="{{a: x}}"/> }}',
}})
export class A2 {{
    x = 1;
}}
"#
        )
    };
    // The name of A2's first pure function.
    let a2_constant = |code: &str| {
        let compact = compact(code);
        let end = compact.find("=(a0)=>[2,a0]").expect(code);
        let start = compact[..end].rfind("const").expect(code) + "const".len();
        compact[start..end].to_string()
    };
    let with_deps_fns = transform(&dir, &source("LazyCmp, LocalCmp")).code;
    assert!(with_deps_fns.contains("const A1_Defer_2_DepsFn"), "{with_deps_fns}");
    assert!(with_deps_fns.contains("const A1_Defer_5_DepsFn"), "{with_deps_fns}");
    let without = transform(&dir, &source("")).code;
    assert!(!without.contains("const A1_Defer"), "{without}");
    assert_eq!(a2_constant(&with_deps_fns), a2_constant(&without), "{with_deps_fns}");
}

#[test]
fn block_only_dependencies_leave_dependencies_of_each_component() {
    // Like ngc, a same-file component used only inside `@defer` is provided
    // by the block's function, not by `dependencies`. The second component's
    // function is the first one's, which ngc's single pool finds equivalent
    // (`getSharedFunctionReference`).
    let dir = TempDir::new().unwrap();
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';

@Component({ selector: 'app-local', template: 'local' })
export class LocalCmp {}

@Component({ selector: 'app-a', imports: [LocalCmp], template: '@defer { <app-local/> }' })
export class App1 {}

@Component({
    selector: 'app-b',
    imports: [LocalCmp],
    template: '<app-local/> @defer { <app-local/> }',
})
export class App2 {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(compact.contains("constApp1_Defer_1_DepsFn=()=>[LocalCmp];"), "{code}");
    assert!(compact.contains("ɵɵdefer(2,1,App1_Defer_1_DepsFn);"), "{code}");
    assert!(!compact.contains("App2_Defer_2_DepsFn"), "{code}");
    assert!(!compact.contains("ɵɵgetComponentDepsFactory(App1"), "{code}");
    assert!(compact.contains("ɵɵgetComponentDepsFactory(App2,[LocalCmp])"), "{code}");
}

#[test]
fn deps_fns_reading_an_imported_class_are_one_per_component() {
    // ngc reads an imported class through the identifier in each component's
    // `imports` (a `WrappedNodeExpr`), and `isEquivalent` compares nodes: the
    // blocks of one component share a function, two components don't.
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/deps.ts",
        r"
import { Component } from '@angular/core';

@Component({ selector: 'app-dep', template: 'dep' })
export class Dep {}

@Component({ selector: 'app-dep2', template: 'dep2' })
export class Dep2 {}
",
    );
    let result = transform(
        &dir,
        r"
import { Component } from '@angular/core';
import { Dep, Dep2 } from './deps';

@Component({
    selector: 'app-b',
    imports: [Dep, Dep2],
    template: '<app-dep2/> @defer { <app-dep/> } @defer { <app-dep/> }',
})
export class B {}

@Component({ selector: 'app-c', imports: [Dep, Dep2], template: '<app-dep2/> @defer { <app-dep/> }' })
export class C {}
",
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(compact.contains("constB_Defer_2_DepsFn=()=>[Dep];"), "{code}");
    assert!(compact.contains("ɵɵdefer(2,1,B_Defer_2_DepsFn);"), "{code}");
    assert!(compact.contains("ɵɵdefer(5,4,B_Defer_2_DepsFn);"), "{code}");
    assert!(compact.contains("constC_Defer_2_DepsFn=()=>[Dep];"), "{code}");
    assert!(compact.contains("ɵɵdefer(2,1,C_Defer_2_DepsFn);"), "{code}");
}

#[test]
fn deps_fns_reading_a_const_imports_array_are_shared_by_components() {
    // ngc resolves `imports: IMPORTS` through the `const`, and reads each class
    // through the identifier in its array, which both components share: their
    // equivalent functions are one. Classes outside an `imports` array literal
    // are never lazy-loaded.
    let dir = TempDir::new().unwrap();
    create_test_file(
        dir.path(),
        "app/deps.ts",
        r"
import { Component } from '@angular/core';

@Component({ selector: 'app-dep', template: 'dep' })
export class Dep {}

@Component({ selector: 'app-dep2', template: 'dep2' })
export class Dep2 {}
",
    );
    let source = r"
import { Component } from '@angular/core';
import { Dep, Dep2 } from './deps';

const IMPORTS = [Dep, Dep2];

@Component({ selector: 'app-a', imports: IMPORTS, template: '<app-dep/> @defer { <app-dep2/> }' })
export class A {}

@Component({ selector: 'app-b', imports: IMPORTS, template: '@defer { <app-dep/><app-dep2/> }' })
export class B {}

@Component({ selector: 'app-c', imports: IMPORTS, template: '@defer { <app-dep/><app-dep2/> }' })
export class C {}
";
    let code = &transform(&dir, source).code;
    let compact = compact(code);
    assert!(compact.contains("constA_Defer_2_DepsFn=()=>[Dep2];"), "{code}");
    assert!(compact.contains("ɵɵdefer(2,1,A_Defer_2_DepsFn);"), "{code}");
    assert!(compact.contains("constB_Defer_1_DepsFn=()=>[Dep,Dep2];"), "{code}");
    assert!(!compact.contains("C_Defer_1_DepsFn"), "{code}");
    assert_eq!(compact.matches("ɵɵdefer(1,0,B_Defer_1_DepsFn);").count(), 2, "{code}");
    assert!(code.contains("import { Dep, Dep2 }"), "{code}");
    assert_eager(code, "./deps");

    let code = &transform_with(&dir, source, &partial_options()).code;
    let partial = self::compact(code);
    assert!(partial.contains("deferBlockDependencies:[()=>[Dep2]]"), "{code}");
    assert_eq!(partial.matches("deferBlockDependencies:[()=>[Dep,Dep2]]").count(), 2, "{code}");
}

#[test]
fn deps_fns_read_an_imports_array_through_let_an_alias_and_as_const() {
    // ngc's partial evaluator reads any top-level variable, through `as const`
    // and through a variable naming another one.
    let dir = TempDir::new().unwrap();
    for declaration in [
        "let IMPORTS = [Dep, Dep2];",
        "const BASE = [Dep, Dep2];\nconst IMPORTS = BASE;",
        "const IMPORTS = [Dep, Dep2] as const;",
    ] {
        let source = format!(
            r"
import {{ Component }} from '@angular/core';

@Component({{ selector: 'app-dep', template: 'dep' }})
export class Dep {{}}

@Component({{ selector: 'app-dep2', template: 'dep2' }})
export class Dep2 {{}}

{declaration}

@Component({{ selector: 'app-a', imports: IMPORTS, template: '<app-dep/> @defer {{ <app-dep2/> }}' }})
export class A {{}}
"
        );
        let code = &transform(&dir, &source).code;
        let compact = compact(code);
        assert!(compact.contains("constA_Defer_2_DepsFn=()=>[Dep2];"), "{code}");
        assert!(compact.contains("ɵɵdefer(2,1,A_Defer_2_DepsFn);"), "{code}");

        let code = &transform_with(&dir, &source, &partial_options()).code;
        let partial = self::compact(code);
        assert!(partial.contains("deferBlockDependencies:[()=>[Dep2]]"), "{code}");
        assert!(!partial.contains("type:IMPORTS"), "{code}");
    }
}

// --- ngc's `DeferredSymbolTracker` over the whole file: the `imports`
// entries of every component that defers a class don't keep its import ---

/// Two components listing `LazyCmp` in `imports`, the second after `app-b`'s
/// template.
fn two_parents(lazy_template_b: &str, rest: &str) -> String {
    format!(
        r#"
import {{ Component }} from '@angular/core';
import {{ LazyCmp }} from './lazy';

@Component({{ selector: 'app-a', imports: [LazyCmp], template: '@defer {{ <app-lazy/> }}' }})
export class ParentA {{}}

@Component({{ selector: 'app-b', imports: [LazyCmp], template: '{lazy_template_b}' }})
export class ParentB {{}}
{rest}"#
    )
}

#[test]
fn import_deferred_by_two_components_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result =
        transform(&dir, &two_parents("@defer (on idle) { <app-lazy/> } @placeholder { p }", ""));
    let code = &result.code;
    let compact = compact(code);
    assert!(
        compact.contains("constParentA_Defer_1_DepsFn=()=>[import(\"./lazy\")")
            && compact.contains("ɵɵdefer(2,0,ParentA_Defer_1_DepsFn,null,1);"),
        "Both components must load `LazyCmp` lazily, through one shared function. Output:\n{code}"
    );
    assert!(!code.contains("import { LazyCmp }"), "{code}");
    assert!(
        compact.contains("setClassMetadataAsync(ParentA,()=>[import(\"./lazy\")")
            && compact.contains("setClassMetadataAsync(ParentB,()=>[import(\"./lazy\")"),
        "{code}"
    );
}

#[test]
fn import_used_eagerly_by_another_component_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(&dir, &two_parents("<app-lazy/>", ""));
    let code = &result.code;
    assert_eager(code, "./lazy");
    assert!(code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn import_listed_by_another_component_that_does_not_use_it_stays_eager() {
    // ngc marks an `imports` entry only for a component whose `@defer`
    // blocks use the class.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(&dir, &two_parents("@defer { nothing }", ""));
    let code = &result.code;
    assert_eager(code, "./lazy");
    assert!(code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn import_deferred_by_two_components_and_referenced_elsewhere_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result =
        transform(&dir, &two_parents("@defer { <app-lazy/> }", "export const LAZY = LazyCmp;\n"));
    let code = &result.code;
    assert_eager(code, "./lazy");
    assert!(code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn import_deferred_by_two_components_and_listed_by_a_module_stays_eager() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        &two_parents(
            "@defer { <app-lazy/> }",
            "import { NgModule } from '@angular/core';\n\
             @NgModule({ imports: [LazyCmp] })\nexport class LazyModule {}\n",
        ),
    );
    let code = &result.code;
    assert_eager(code, "./lazy");
    assert!(code.contains("import { LazyCmp }"), "{code}");
}

#[test]
fn specifiers_deferred_by_different_components_remove_the_declaration() {
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp, EagerCmp } from './cmps';

@Component({ selector: 'app-a', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
export class ParentA {}

@Component({ selector: 'app-b', imports: [EagerCmp], template: '@defer { <app-eager/> }' })
export class ParentB {}
"#,
    );
    let code = &result.code;
    assert_lazy(code, "./cmps", "LazyCmp");
    assert_lazy(code, "./cmps", "EagerCmp");
    // A declaration kept whole is printed with its own quotes.
    assert!(!code.contains("from './cmps'") && !code.contains("from \"./cmps\""), "{code}");
}

#[test]
fn class_two_imports_entries_name_is_loaded_once() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
"#,
    );
    let code = &result.code;
    assert!(!code.contains("import { LazyCmp }"), "{code}");
    assert!(
        compact(code).contains(
            "setClassMetadataAsync(Parent,()=>[import(\"./lazy\").then((m)=>m.LazyCmp)],(LazyCmp)=>"
        ),
        "{code}"
    );
}

#[test]
fn deps_fns_are_declared_in_pool_order_with_the_constants() {
    // ngtsc's pool keeps one list of statements: the DepsFn is pooled before
    // the pure function and the track function, and declared first.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let source = r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-a',
    imports: [LazyCmp],
    template: '@defer {<app-lazy/>}<i [title]="[k, 1]"></i>@for (x of xs; track x.id) {}',
})
export class A { k = 1; xs: any[] = []; }
"#;
    let code = transform(&dir, source).code;
    let declared: Vec<&str> = code
        .lines()
        .filter_map(|line| line.strip_prefix("const ").or_else(|| line.strip_prefix("function ")))
        .filter_map(|rest| rest.split([' ', '=', '(']).next())
        .collect();
    assert_eq!(
        declared,
        ["A_Defer_1_DepsFn", "_c0", "_forTrack0", "A_Defer_0_Template", "A_For_5_Template"],
        "{code}"
    );
}

fn partial_options() -> TransformOptions {
    TransformOptions {
        compilation_mode: oxc_angular_compiler::CompilationMode::Partial,
        ..options()
    }
}

#[test]
fn partial_declaration_lazy_loads_a_component_used_only_inside_defer() {
    // Like ngc's `compilePartial`: the blocks are resolved as in a full
    // compilation, and the declaration carries their dependency functions.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform_with(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
})
export class Parent {}
",
        &partial_options(),
    );
    let code = &result.code;
    let compact = compact(code);

    assert!(
        compact.contains("deferBlockDependencies:[()=>[import(\"./lazy\").then((m)=>m.LazyCmp)]]"),
        "The block's resolver must be in the declaration. Output:\n{code}"
    );
    assert!(!compact.contains("dependencies:[{"), "Output:\n{code}");
    assert!(!code.contains("import { LazyCmp }"), "Output:\n{code}");
    assert!(
        compact.contains(
            "ɵɵngDeclareClassMetadataAsync({minVersion:\"18.0.0\",version:\"22.2.3\",ngImport:i0,type:Parent,resolveDeferredDeps:()=>[import(\"./lazy\").then((m)=>m.LazyCmp)],resolveMetadata:(LazyCmp)=>"
        ),
        "Output:\n{code}"
    );

    // The linker wires the resolver into `ɵɵdefer`, as Angular's does.
    let allocator = Allocator::default();
    let linked = oxc_angular_compiler::linker::link(&allocator, code, "parent.component.js");
    let linked = linked.code.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    assert!(
        linked.contains("constParent_Defer_1_DepsFn=()=>[import(\"./lazy\")")
            && linked.contains("ɵɵdefer(1,0,Parent_Defer_1_DepsFn)"),
        "Linked:\n{linked}"
    );
}

#[test]
fn partial_declaration_lists_a_null_resolver_for_a_block_without_dependencies() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    other_cmp(&dir);
    let result = transform_with(
        &dir,
        r"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';
import { OtherCmp } from './other';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp, OtherCmp],
    template: '<app-other/> @defer { nothing } @defer { <app-lazy/> <app-other/> }',
})
export class Parent {}
",
        &partial_options(),
    );
    let code = &result.code;
    let compact = compact(code);

    assert!(
        compact.contains(
            "deferBlockDependencies:[null,()=>[import(\"./lazy\").then((m)=>m.LazyCmp),OtherCmp]]"
        ),
        "Output:\n{code}"
    );
    assert!(compact.contains("dependencies:[{kind:\"directive\",type:OtherCmp"), "Output:\n{code}");
}

// --- Classes nested in a function count with the file's other components ---

#[test]
fn import_deferred_by_a_nested_and_a_top_level_component_is_lazy_in_both() {
    // ngc's `DeferredSymbolTracker` sees every component of the file, nested
    // ones too: both lazy-load `LazyCmp`, and its static import goes.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

export function make() {
  @Component({ selector: 'app-inner', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
  class Inner {}
  return Inner;
}

@Component({ selector: 'app-top', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
export class Top {}
"#,
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(!code.contains("import { LazyCmp }"), "{code}");
    assert!(compact.contains("setClassMetadataAsync(Inner,()=>[import(\"./lazy\")"), "{code}");
    assert!(compact.contains("setClassMetadataAsync(Top,()=>[import(\"./lazy\")"), "{code}");
    assert!(!compact.contains("=()=>[LazyCmp]"), "{code}");
}

#[test]
fn import_deferred_by_two_nested_components_is_lazy_loaded() {
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

export function one() {
  @Component({ selector: 'app-one', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
  class One {}
  return One;
}

export function two() {
  @Component({ selector: 'app-two', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
  class Two {}
  return Two;
}
"#,
    );
    let code = &result.code;
    let compact = compact(code);
    assert!(!code.contains("import { LazyCmp }"), "{code}");
    assert!(compact.contains("setClassMetadataAsync(One,()=>[import(\"./lazy\")"), "{code}");
    assert!(compact.contains("setClassMetadataAsync(Two,()=>[import(\"./lazy\")"), "{code}");
}

#[test]
fn specifiers_deferred_by_a_top_level_and_a_nested_component_remove_the_declaration() {
    let dir = TempDir::new().unwrap();
    two_cmps(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp, EagerCmp } from './cmps';

@Component({ selector: 'app-top', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
export class Top {}

export function make() {
  @Component({ selector: 'app-inner', imports: [EagerCmp], template: '@defer { <app-eager/> }' })
  class Inner {}
  return Inner;
}
"#,
    );
    let code = &result.code;
    assert_lazy(code, "./cmps", "LazyCmp");
    assert_lazy(code, "./cmps", "EagerCmp");
    assert!(!code.contains("from './cmps'") && !code.contains("from \"./cmps\""), "{code}");
}

#[test]
fn nested_component_keeping_its_decorators_keeps_the_import() {
    // `Inner` reads a binding of `make`, so it keeps its decorators and JIT
    // compiles: its `imports` entry stays a reference to `LazyCmp`.
    let dir = TempDir::new().unwrap();
    lazy_cmp(&dir);
    let result = transform(
        &dir,
        r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

export function make(selector: string) {
  @Component({ selector, imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
  class Inner {}
  return Inner;
}

@Component({ selector: 'app-top', imports: [LazyCmp], template: '@defer { <app-lazy/> }' })
export class Top {}
"#,
    );
    let code = &result.code;
    assert_eager(code, "./lazy");
    assert!(code.contains("import { LazyCmp }"), "{code}");
}

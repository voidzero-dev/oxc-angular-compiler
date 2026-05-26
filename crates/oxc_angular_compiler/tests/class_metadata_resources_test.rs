//! Tests for inlining resolved external resources (`templateUrl` / `styleUrls` /
//! `styleUrl`) into the `setClassMetadata` arguments.
//!
//! Reference behavior is `transformDecoratorResources` in Angular's
//! `compiler-cli/src/ngtsc/annotations/component/src/resources.ts`. See the
//! compliance test at
//! `compiler-cli/test/compliance/test_cases/r3_compiler_compliance/class_metadata/`
//! for the canonical output shape (e.g. `templateUrl: 'test_cmp_template.html'`
//! becomes `template: "<span>Test template</span>\n"`).

use oxc_allocator::Allocator;
use oxc_angular_compiler::{ResolvedResources, TransformOptions, transform_angular_file};
use std::collections::HashMap;

/// Extract the `args` payload of the first `ɵsetClassMetadata` call.
///
/// `setClassMetadata` is emitted as
/// `setClassMetadata(ClassName, [{ type: ..., args: [{...}] }], null, null)`.
/// We slice from the call site through to the close of the decorators array
/// (which always ends with `}]` — close-decorator-object, close-array). Tests
/// then assert against that segment.
fn extract_metadata_args(code: &str) -> String {
    let start = code
        .find("ɵsetClassMetadata")
        .unwrap_or_else(|| panic!("setClassMetadata not present in output:\n{code}"));
    let tail = &code[start..];
    // Track paren depth from the opening `(` of `setClassMetadata(`. The end
    // of the second positional argument (the decorators array) is the comma
    // at paren-depth 1.
    let open = tail.find('(').expect("missing `(` after setClassMetadata in output");
    let bytes = tail.as_bytes();
    let mut depth: i32 = 0;
    let mut comma_count = 0;
    let mut end = tail.len();
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 1 => {
                comma_count += 1;
                if comma_count == 2 {
                    // First comma separates ClassName from decorators; second
                    // is the end of the decorators array.
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    tail[..end].to_string()
}

fn run_with_resources(source: &str, resources: ResolvedResources) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { emit_class_metadata: true, ..TransformOptions::default() };
    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        Some(&options),
        Some(&resources),
    );
    assert!(
        !result.has_errors(),
        "Compilation should succeed. Diagnostics: {:?}",
        result.diagnostics
    );
    result.code
}

/// `templateUrl` should be replaced by `template` with the resolved content.
/// Reference: compliance test `r3_compiler_compliance/class_metadata/class_decorators`.
#[test]
fn templateurl_is_replaced_with_inlined_template_content() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    templateUrl: 'test_cmp_template.html',
})
export class ComponentWithExternalResource {}
"#;

    let mut templates = HashMap::new();
    templates.insert("test_cmp_template.html".to_string(), "<span>Test template</span>\n".to_string());
    let resources = ResolvedResources { templates, styles: HashMap::new() };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        !metadata.contains("templateUrl"),
        "templateUrl should be removed. Got:\n{metadata}"
    );
    assert!(
        metadata.contains(r#"template:"<span>Test template</span>\n""#)
            || metadata.contains("template: \"<span>Test template</span>\\n\""),
        "Inlined template content should be present. Got:\n{metadata}"
    );
}

/// When the source has no resource fields at all, the metadata should be left
/// alone — even when `resolved_resources` is supplied. Angular's
/// `transformDecoratorResources` returns the original decorator in this case.
#[test]
fn no_resource_fields_leaves_metadata_untouched() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div>inline</div>',
    standalone: true,
})
export class InlineComponent {}
"#;

    let resources = ResolvedResources { templates: HashMap::new(), styles: HashMap::new() };
    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        metadata.contains(r#"template:"<div>inline</div>""#)
            || metadata.contains("template: \"<div>inline</div>\""),
        "Inline template should be preserved verbatim. Got:\n{metadata}"
    );
    assert!(
        metadata.contains(r#"selector:"test-cmp""#)
            || metadata.contains("selector: \"test-cmp\""),
        "Selector should be preserved. Got:\n{metadata}"
    );
}

/// `styleUrls` should be replaced by `styles` carrying the resolved content
/// strings. The `styleUrls` key itself must not appear in the output, since
/// Angular's `componentNeedsResolution` check uses `styleUrls?.length`.
#[test]
fn styleurls_is_replaced_with_inlined_styles_array() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styleUrls: ['./a.css', './b.css'],
})
export class StyledComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./a.css".to_string(), vec!["div { color: red; }".to_string()]);
    styles.insert("./b.css".to_string(), vec!["span { color: blue; }".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        !metadata.contains("styleUrls"),
        "styleUrls should be removed. Got:\n{metadata}"
    );
    assert!(
        metadata.contains(r#""div { color: red; }""#)
            || metadata.contains("'div { color: red; }'"),
        "Resolved style content for a.css should be inlined. Got:\n{metadata}"
    );
    assert!(
        metadata.contains(r#""span { color: blue; }""#)
            || metadata.contains("'span { color: blue; }'"),
        "Resolved style content for b.css should be inlined. Got:\n{metadata}"
    );
}

/// `styleUrl` (singular, Angular 17+) should also be replaced by `styles`.
/// Angular's `componentNeedsResolution` checks `component.styleUrl` too.
#[test]
fn styleurl_singular_is_replaced_with_inlined_styles_array() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styleUrl: './a.css',
})
export class StyledComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./a.css".to_string(), vec!["div { color: red; }".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        !metadata.contains("styleUrl:") && !metadata.contains("styleUrl "),
        "styleUrl should be removed. Got:\n{metadata}"
    );
    assert!(
        metadata.contains(r#""div { color: red; }""#)
            || metadata.contains("'div { color: red; }'"),
        "Resolved style content should be inlined. Got:\n{metadata}"
    );
}

/// Whitespace-only style strings should be dropped, matching Angular's
/// `style.trim().length > 0` filter in `transformDecoratorResources`. If every
/// resolved style is empty, the `styles` key should not appear at all.
#[test]
fn empty_styles_are_filtered_out() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styleUrls: ['./empty.css', './also-empty.css'],
})
export class EmptyStylesComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./empty.css".to_string(), vec![String::new()]);
    styles.insert("./also-empty.css".to_string(), vec!["    \n  ".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        !metadata.contains("styleUrls"),
        "styleUrls should be removed. Got:\n{metadata}"
    );
    assert!(
        !metadata.contains("styles:"),
        "When all resolved styles are empty/whitespace, the styles key should be omitted. Got:\n{metadata}"
    );
}

/// `componentNeedsResolution` from Angular's runtime fails if any of these
/// three keys remain in the metadata (alongside other conditions). This is the
/// blocker for TestBed integration when the AOT-compiled component is later
/// re-validated against its preserved metadata. Verify they're all gone.
#[test]
fn no_resource_keys_remain_after_inlining() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    templateUrl: './tmpl.html',
    styleUrls: ['./a.css'],
})
export class FullComponent {}
"#;

    let mut templates = HashMap::new();
    templates.insert("./tmpl.html".to_string(), "<p></p>".to_string());
    let mut styles = HashMap::new();
    styles.insert("./a.css".to_string(), vec!["p {}".to_string()]);
    let resources = ResolvedResources { templates, styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    for forbidden in ["templateUrl", "styleUrls", "styleUrl:"] {
        assert!(
            !metadata.contains(forbidden),
            "After inlining, `{forbidden}` should not appear in setClassMetadata. Got:\n{metadata}"
        );
    }
}

/// Resolved styles must be ADDED to any inline `styles` array already present
/// in the decorator, not replace it. The Angular reference impl reads existing
/// `styles` from the source decorator and includes them alongside the resolved
/// styleUrl content (with the same non-empty filter).
#[test]
fn inline_styles_array_is_merged_with_resolved_styles() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styles: ['.inline { color: green; }'],
    styleUrls: ['./external.css'],
})
export class MergedStylesComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./external.css".to_string(), vec![".external { color: red; }".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        metadata.contains(".inline { color: green; }"),
        "Pre-existing inline style should be preserved in merged output. Got:\n{metadata}"
    );
    assert!(
        metadata.contains(".external { color: red; }"),
        "Resolved external style should be inlined in merged output. Got:\n{metadata}"
    );
    assert!(
        !metadata.contains("styleUrls"),
        "styleUrls should be removed even when merging. Got:\n{metadata}"
    );
}

/// Bail-out: when emit_class_metadata is on, but the component has only inline
/// resources and no resource URLs, the metadata should pass through unchanged.
/// This is the "fast path" in Angular's reference impl (returns the original
/// decorator).
#[test]
fn purely_inline_component_metadata_passes_through() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div>hello</div>',
    styles: ['div { color: red; }'],
})
export class PureInlineComponent {}
"#;

    let resources = ResolvedResources { templates: HashMap::new(), styles: HashMap::new() };
    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        metadata.contains(r#"template:"<div>hello</div>""#)
            || metadata.contains("template: \"<div>hello</div>\""),
        "Inline template should be passed through. Got:\n{metadata}"
    );
    assert!(
        metadata.contains("div { color: red; }"),
        "Inline styles array should be preserved. Got:\n{metadata}"
    );
}

/// Inline `styles:[...]` source content must not be duplicated when the caller
/// also passes those styles in `inlined_styles`. ng-packagr / vite-side resolution
/// produces a merged styles list that already includes the inline source styles,
/// so the AST-side `styles:[...]` entry from the decorator should NOT be re-merged.
/// Without this guarantee, a component with `styles: ['.foo {}']` would end up
/// with `styles: ['.foo {}', '.foo {}']` in the metadata.
#[test]
fn inline_styles_are_not_duplicated_when_also_in_inlined_styles() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div>hello</div>',
    styles: ['.foo { color: red; }'],
})
export class InlineStylesComponent {}
"#;

    let resources = ResolvedResources { templates: HashMap::new(), styles: HashMap::new() };
    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    let occurrences = metadata.matches(".foo { color: red; }").count();
    assert_eq!(
        occurrences, 1,
        "Inline style should appear exactly once in setClassMetadata, not duplicated. Got {occurrences} occurrences in:\n{metadata}"
    );
}

// =================================================================
// Bug 1 (duplicate styles) — additional coverage variants
// =================================================================

/// Multiple distinct inline styles must each appear exactly once, in source
/// order. Catches a regression where a clever-but-broken set-based dedup might
/// accidentally drop legitimate duplicates the user wrote intentionally.
#[test]
fn multiple_distinct_inline_styles_each_appear_once_in_order() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styles: [
        '.a { color: red; }',
        '.b { color: green; }',
        '.c { color: blue; }',
    ],
})
export class MultiStylesComponent {}
"#;

    let resources = ResolvedResources { templates: HashMap::new(), styles: HashMap::new() };
    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    for needle in [".a { color: red; }", ".b { color: green; }", ".c { color: blue; }"] {
        assert_eq!(
            metadata.matches(needle).count(),
            1,
            "`{needle}` should appear exactly once. Metadata:\n{metadata}"
        );
    }
    // Order: a < b < c in the output text.
    let pa = metadata.find(".a {").expect("missing .a");
    let pb = metadata.find(".b {").expect("missing .b");
    let pc = metadata.find(".c {").expect("missing .c");
    assert!(pa < pb && pb < pc, "Styles should appear in source order. Metadata:\n{metadata}");
}

/// When source has BOTH `styles:[...]` AND `styleUrls:[...]`, neither the
/// inline nor the resolved-external content should be duplicated, and each
/// must be represented exactly once.
#[test]
fn no_duplicates_when_inline_styles_and_styleurls_coexist() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styles: ['.inline-1 {}', '.inline-2 {}'],
    styleUrls: ['./external.css'],
})
export class MixedStylesComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./external.css".to_string(), vec![".external {}".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    for needle in [".inline-1 {}", ".inline-2 {}", ".external {}"] {
        assert_eq!(
            metadata.matches(needle).count(),
            1,
            "`{needle}` should appear exactly once. Metadata:\n{metadata}"
        );
    }
}

/// When source has ONLY `styleUrls` (no inline styles), each resolved style
/// should appear once and only once.
#[test]
fn no_duplicates_in_pure_styleurls_resolution() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styleUrls: ['./a.css', './b.css'],
})
export class PureStyleUrlsComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./a.css".to_string(), vec![".a {}".to_string()]);
    styles.insert("./b.css".to_string(), vec![".b {}".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    for needle in [".a {}", ".b {}"] {
        assert_eq!(
            metadata.matches(needle).count(),
            1,
            "`{needle}` should appear exactly once. Metadata:\n{metadata}"
        );
    }
}

// =================================================================
// Bug 2 (empty-style filter) — additional coverage variants
// =================================================================

/// Non-empty styles should survive while empty/whitespace-only ones are
/// dropped. Mirrors Angular's `style.trim().length > 0` filter.
#[test]
fn mixed_empty_and_nonempty_styles_keeps_only_nonempty() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styleUrls: ['./real.css', './empty.css', './whitespace.css'],
})
export class MixedStyles {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./real.css".to_string(), vec![".real { color: red; }".to_string()]);
    styles.insert("./empty.css".to_string(), vec![String::new()]);
    styles.insert("./whitespace.css".to_string(), vec!["    \t\n  ".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        metadata.contains(".real { color: red; }"),
        "Non-empty style should survive. Got:\n{metadata}"
    );
    // The empty styles should be dropped — no empty quoted string literal.
    assert!(
        !metadata.contains(r#""""#),
        "Empty string literals should not appear in the styles array. Got:\n{metadata}"
    );
    // Catch both escaped and raw forms of the whitespace-only string.
    assert!(
        !metadata.contains("    \t\n  ") && !metadata.contains(r#""    \t\n  ""#),
        "Whitespace-only style should be filtered. Got:\n{metadata}"
    );
}

/// Inline source `styles:['']` (empty string inside the styles array of the
/// decorator) should also be filtered. Without this, the `styles: [""]`
/// literal survives into the metadata and TestBed receives a junk empty
/// stylesheet on recompilation.
#[test]
fn empty_inline_style_string_is_filtered_out() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styles: [''],
})
export class EmptyInlineComponent {}
"#;

    let resources = ResolvedResources { templates: HashMap::new(), styles: HashMap::new() };
    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        !metadata.contains("styles:"),
        "When the only style is empty, the styles key should be omitted entirely. Got:\n{metadata}"
    );
}

/// `styleUrl` (singular) with empty/whitespace content should produce no
/// `styles` key — same filter applies regardless of the source key spelling.
#[test]
fn empty_styleurl_singular_drops_styles_key() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
    styleUrl: './empty.css',
})
export class EmptyStyleUrlComponent {}
"#;

    let mut styles = HashMap::new();
    styles.insert("./empty.css".to_string(), vec!["   ".to_string()]);
    let resources = ResolvedResources { templates: HashMap::new(), styles };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        !metadata.contains("styles:") && !metadata.contains("styleUrl"),
        "Empty styleUrl should produce no styles key and no leftover styleUrl. Got:\n{metadata}"
    );
}

/// Property ordering after `templateUrl` → `template` should match Angular's
/// reference `transformDecoratorResources`. Angular uses `Map.delete('templateUrl')`
/// followed by `Map.set('template', …)`, which appends `template` at the end of
/// the Map's insertion order when no existing `template` key is present.
///
/// Concretely, source `{ selector, templateUrl, encapsulation }` must emit
/// `{ selector, encapsulation, template }` — not `{ selector, template, encapsulation }`.
/// This is the form `e2e/compare` checks against ngc's output via string equality.
#[test]
fn template_replacement_lands_at_end_matching_angular_map_semantics() {
    let source = r#"
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
    selector: 'test-cmp',
    templateUrl: './tmpl.html',
    encapsulation: ViewEncapsulation.None,
})
export class OrderedComponent {}
"#;

    let mut templates = HashMap::new();
    templates.insert("./tmpl.html".to_string(), "<p></p>".to_string());
    let resources = ResolvedResources { templates, styles: HashMap::new() };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    let normalized = metadata.replace([' ', '\n', '\t'], "");
    let selector_pos = normalized.find("selector:").expect("selector should be present");
    let encapsulation_pos =
        normalized.find("encapsulation:").expect("encapsulation should be present");
    let template_pos = normalized.find("template:").expect("template should be present");

    assert!(
        selector_pos < encapsulation_pos,
        "Source ordering of non-resource keys should be preserved. Got:\n{metadata}"
    );
    assert!(
        encapsulation_pos < template_pos,
        "Resolved template should appear AFTER all surviving source keys (Angular's \
         Map.delete + Map.set appends template to end of insertion order). Got:\n{metadata}"
    );
    assert!(
        !normalized.contains("templateUrl"),
        "templateUrl literal should not appear. Got:\n{metadata}"
    );
}

/// If the source decorator illegally contains BOTH inline `template` and
/// `templateUrl`, Angular's `Map.delete('templateUrl')` + `Map.set('template', …)`
/// semantics produce a single `template` key (Map.set on an existing key
/// overwrites in place; the original position is preserved). OXC must not emit
/// duplicate `template:` literals — that's invalid JS object syntax in strict mode
/// and a divergence from `ngc`'s output that would fail the e2e string-equality
/// comparison.
#[test]
fn source_with_both_template_and_template_url_emits_single_template_key() {
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<inline></inline>',
    templateUrl: './tmpl.html',
})
export class DoubleTemplateComponent {}
"#;

    let mut templates = HashMap::new();
    templates.insert("./tmpl.html".to_string(), "<from-url></from-url>".to_string());
    let resources = ResolvedResources { templates, styles: HashMap::new() };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    let template_count = metadata.matches("template:").count();
    assert_eq!(
        template_count, 1,
        "Expected exactly ONE `template:` key in setClassMetadata (Angular's \
         Map.set semantics overwrite, not duplicate). Got {template_count} in:\n{metadata}"
    );
    assert!(
        !metadata.contains("templateUrl"),
        "templateUrl literal should not appear after inlining. Got:\n{metadata}"
    );

    // ngc AOT prefers `templateUrl` content over inline `template` when both
    // are present — see `parseTemplateDeclaration` in
    // `compiler-cli/src/ngtsc/annotations/component/src/resources.ts` which
    // checks `component.has('templateUrl')` first and returns immediately.
    // OXC is an AOT-equivalent single-file compiler so it must match that
    // precedence (ngc JIT diverges, preferring inline — irrelevant here).
    assert!(
        metadata.contains("<from-url></from-url>"),
        "templateUrl content should win in AOT mode (ngc parity). Got:\n{metadata}"
    );
    assert!(
        !metadata.contains("<inline></inline>"),
        "Inline template should be discarded when templateUrl is also present \
         (matches ngc AOT `parseTemplateDeclaration`). Got:\n{metadata}"
    );
}

/// Resource-key inlining must apply ONLY to `@Component` decorators. Angular's
/// reference impl gates on `if (dec.name !== 'Component') return dec;` at the
/// top of `transformDecoratorResources`. If we don't check the decorator name,
/// other decorators that happen to use resource-shaped keys (legal TypeScript,
/// just nonsensical) get their literals corrupted.
///
/// This test exercises that via a constructor parameter decorator — `@Inject`
/// metadata goes through `build_decorator_metadata_array` with `decorator_idx == 0`
/// and would hit the resource-rewriting path without a name check.
#[test]
fn non_component_decorator_with_resource_shaped_keys_passes_through_untouched() {
    let source = r#"
import { Component, Inject } from '@angular/core';

@Component({
    selector: 'test-cmp',
    template: '<div></div>',
})
export class CtorComponent {
    constructor(@Inject({ templateUrl: './bogus.html', styleUrls: ['./bogus.css'] }) service: any) {}
}
"#;

    let resources = ResolvedResources { templates: HashMap::new(), styles: HashMap::new() };
    let code = run_with_resources(source, resources);

    // The `@Inject(...)` literal lands inside the ctorParameters → decorators
    // → args array of setClassMetadata. Its `templateUrl` / `styleUrls` keys
    // must survive verbatim — they're an opaque DI token, not a component config.
    assert!(
        code.contains("templateUrl:\"./bogus.html\"") || code.contains("templateUrl: './bogus.html'"),
        "@Inject's templateUrl literal must survive verbatim in ctorParameters. Got:\n{code}"
    );
    assert!(
        code.contains("styleUrls:[\"./bogus.css\"]") || code.contains("styleUrls:['./bogus.css']"),
        "@Inject's styleUrls literal must survive verbatim in ctorParameters. Got:\n{code}"
    );
}

/// Spread elements in the source decorator literal (`@Component({ ...config, … })`)
/// pass through verbatim to `setClassMetadata`. This is a known limitation: OXC
/// doesn't statically evaluate the spread argument, so resource fields living
/// inside the spread can leak past `componentNeedsResolution` at runtime.
///
/// Angular's reference impl operates on a `Map<string, ts.Expression>` already
/// produced by the annotation handler, which has resolved spreads upstream.
/// Until OXC has equivalent pre-extraction spread resolution, the safe behavior
/// is "preserve unchanged" — anything else would risk losing genuine user data.
/// This test locks that in so a future change can't silently regress it.
#[test]
fn spread_elements_in_component_decorator_pass_through_unchanged() {
    let source = r#"
import { Component } from '@angular/core';

const baseConfig = { changeDetection: 0 };

@Component({
    ...baseConfig,
    selector: 'test-cmp',
    templateUrl: './tmpl.html',
})
export class SpreadComponent {}
"#;

    let mut templates = HashMap::new();
    templates.insert("./tmpl.html".to_string(), "<p></p>".to_string());
    let resources = ResolvedResources { templates, styles: HashMap::new() };

    let code = run_with_resources(source, resources);
    let metadata = extract_metadata_args(&code);

    assert!(
        metadata.contains("...baseConfig"),
        "Spread element must be preserved verbatim. Got:\n{metadata}"
    );
    assert!(
        !metadata.contains("templateUrl"),
        "templateUrl outside the spread must still be inlined. Got:\n{metadata}"
    );
    assert!(
        metadata.contains("template:\"<p></p>\""),
        "Resolved template content must be present. Got:\n{metadata}"
    );
}

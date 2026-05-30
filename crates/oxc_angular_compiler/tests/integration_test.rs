//! Integration tests for the Angular template compiler.
//!
//! These tests verify the complete compilation pipeline from
//! HTML template string to JavaScript output.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    AngularVersion, R3Node, ResolvedResources, TransformOptions as ComponentTransformOptions,
    output::ast::FunctionExpr,
    output::emitter::JsEmitter,
    parser::html::HtmlParser,
    pipeline::{emit::compile_template, ingest::ingest_component},
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions},
    transform_angular_file,
};
use oxc_str::Ident;

/// Compiles an Angular template to JavaScript.
fn compile_template_to_js(template: &str, component_name: &str) -> String {
    compile_template_to_js_with_version(template, component_name, None)
}

/// Compiles an Angular template to JavaScript targeting a specific Angular version.
fn compile_template_to_js_with_version(
    template: &str,
    component_name: &str,
    angular_version: Option<AngularVersion>,
) -> String {
    use oxc_angular_compiler::pipeline::ingest::{IngestOptions, ingest_component_with_options};

    let allocator = Allocator::default();

    // Stage 1: Parse HTML (with expansion forms enabled for ICU/plural support)
    let parser = HtmlParser::with_expansion_forms(&allocator, template, "test.html");
    let html_result = parser.parse();

    // Check for parse errors
    if !html_result.errors.is_empty() {
        let errors: Vec<_> = html_result.errors.iter().map(|e| e.msg.clone()).collect();
        panic!("HTML parse errors: {errors:?}");
    }

    // Stage 2: Transform HTML AST to R3 AST
    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&html_result.nodes);

    // Check for transform errors
    if !r3_result.errors.is_empty() {
        let errors: Vec<_> = r3_result.errors.iter().map(|e| e.msg.clone()).collect();
        panic!("Transform errors: {errors:?}");
    }

    // Stage 3: Ingest R3 AST into IR
    let mut job = if let Some(version) = angular_version {
        let options = IngestOptions { angular_version: Some(version), ..Default::default() };
        ingest_component_with_options(
            &allocator,
            Ident::from(component_name),
            r3_result.nodes,
            options,
        )
    } else {
        ingest_component(&allocator, Ident::from(component_name), r3_result.nodes)
    };

    // Stage 4-5: Transform and emit
    let result = compile_template(&mut job);

    // Stage 6: Generate JavaScript
    let emitter = JsEmitter::new();

    // Emit declarations first (embedded view functions, constants)
    let mut output = String::new();
    for decl in &result.declarations {
        output.push_str(&emitter.emit_statement(decl));
        output.push('\n');
    }

    // Emit the main template function
    output.push_str(&emit_function(&emitter, &result.template_fn));
    output
}

/// Emit a FunctionExpr to JavaScript.
///
/// We emit statements individually since the emitter can work with references.
fn emit_function(emitter: &JsEmitter, func: &FunctionExpr<'_>) -> String {
    // Build the function header
    let mut result = String::new();
    result.push_str("function ");
    if let Some(ref name) = func.name {
        result.push_str(name.as_str());
    }
    result.push('(');
    for (i, param) in func.params.iter().enumerate() {
        if i > 0 {
            result.push(',');
        }
        result.push_str(param.name.as_str());
    }
    result.push_str(") {\n");

    // Emit each statement
    for stmt in &func.statements {
        let stmt_str = emitter.emit_statement(stmt);
        for line in stmt_str.lines() {
            result.push_str("  ");
            result.push_str(line);
            result.push('\n');
        }
    }

    result.push('}');
    result
}

// ============================================================================
// Basic Element Tests
// ============================================================================

#[test]
fn test_empty_template() {
    let js = compile_template_to_js("", "TestComponent");
    insta::assert_snapshot!("empty_template", js);
}

#[test]
fn test_simple_element() {
    let js = compile_template_to_js("<div></div>", "TestComponent");
    insta::assert_snapshot!("simple_element", js);
}

#[test]
fn test_element_with_text() {
    let js = compile_template_to_js("<div>Hello World</div>", "TestComponent");
    insta::assert_snapshot!("element_with_text", js);
}

#[test]
fn test_nested_elements() {
    let js = compile_template_to_js("<div><span>Inner</span></div>", "TestComponent");
    insta::assert_snapshot!("nested_elements", js);
}

#[test]
fn test_void_element() {
    let js = compile_template_to_js("<input>", "TestComponent");
    insta::assert_snapshot!("void_element", js);
}

// ============================================================================
// Text Interpolation Tests
// ============================================================================

#[test]
fn test_text_interpolation() {
    let js = compile_template_to_js("<div>{{name}}</div>", "TestComponent");
    insta::assert_snapshot!("text_interpolation", js);
}

#[test]
fn test_multiple_interpolations() {
    let js = compile_template_to_js("<div>{{first}} and {{second}}</div>", "TestComponent");
    insta::assert_snapshot!("multiple_interpolations", js);
}

#[test]
fn test_html_entity_between_interpolations() {
    // HTML entity &times; between two interpolations should produce raw × in the output
    let js = compile_template_to_js("<div>{{ a }}&times;{{ b }}</div>", "TestComponent");
    // Should produce: textInterpolate2("", ctx.a, "×", ctx.b)
    // Note: × (multiplication sign) = U+00D7, emitted as raw UTF-8
    assert!(
        js.contains("textInterpolate2(\"\",ctx.a,\"\u{00D7}\",ctx.b)"),
        "Expected textInterpolate2 with raw times character. Got:\n{js}"
    );
}

#[test]
fn test_html_entity_at_start_of_interpolation() {
    // Entity at start: &times;{{ a }}
    let js = compile_template_to_js("<div>&times;{{ a }}</div>", "TestComponent");
    // Should produce: textInterpolate1("×", ctx.a)
    // Note: × (multiplication sign) = U+00D7, emitted as raw UTF-8
    assert!(
        js.contains("textInterpolate1(\"\u{00D7}\",ctx.a)")
            || js.contains("textInterpolate(\"\u{00D7}\",ctx.a)"),
        "Expected textInterpolate with raw times character at start. Got:\n{js}"
    );
}

#[test]
fn test_multiple_html_entities_between_interpolations() {
    // Multiple entities: {{ a }}&nbsp;&times;&nbsp;{{ b }}
    let js =
        compile_template_to_js("<div>{{ a }}&nbsp;&times;&nbsp;{{ b }}</div>", "TestComponent");
    // Should produce: textInterpolate2("", ctx.a, "\u{00A0}×\u{00A0}", ctx.b)
    // Note: &nbsp; = U+00A0, &times; = U+00D7, both emitted as raw UTF-8
    assert!(
        js.contains("textInterpolate2(\"\",ctx.a,\"\u{00A0}\u{00D7}\u{00A0}\",ctx.b)"),
        "Expected textInterpolate2 with raw Unicode entities. Got:\n{js}"
    );
}

#[test]
fn test_interpolation_with_expression() {
    let js = compile_template_to_js("<div>{{user.name}}</div>", "TestComponent");
    insta::assert_snapshot!("interpolation_with_expression", js);
}

// ============================================================================
// Property Binding Tests
// ============================================================================

#[test]
fn test_property_binding() {
    let js = compile_template_to_js(r#"<div [title]="myTitle"></div>"#, "TestComponent");
    insta::assert_snapshot!("property_binding", js);
}

#[test]
fn test_attribute_binding() {
    let js = compile_template_to_js(r#"<div [attr.aria-label]="label"></div>"#, "TestComponent");
    insta::assert_snapshot!("attribute_binding", js);
}

#[test]
fn test_attribute_binding_with_interpolation() {
    // Test attr.* with interpolation (e.g., attr.viewBox="0 0 {{ size }} {{ size }}")
    // This should use ɵɵattribute with the prefix stripped
    let js = compile_template_to_js(
        r#"<svg attr.viewBox="0 0 {{ svgSize }} {{ svgSize }}"></svg>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("attribute_binding_with_interpolation", js);
}

#[test]
fn test_class_binding() {
    let js = compile_template_to_js(r#"<div [class.active]="isActive"></div>"#, "TestComponent");
    insta::assert_snapshot!("class_binding", js);
}

#[test]
fn test_style_binding() {
    let js = compile_template_to_js(r#"<div [style.color]="textColor"></div>"#, "TestComponent");
    insta::assert_snapshot!("style_binding", js);
}

#[test]
fn test_style_binding_camel_case() {
    // Test that camelCase style properties are converted to kebab-case
    let js = compile_template_to_js(
        r#"<div [style.backgroundColor]="bgColor" [style.fontSize]="size"></div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("style_binding_camel_case", js);
}

// ============================================================================
// Event Binding Tests
// ============================================================================

#[test]
fn test_event_binding() {
    let js =
        compile_template_to_js(r#"<button (click)="handleClick()"></button>"#, "TestComponent");
    insta::assert_snapshot!("event_binding", js);
}

#[test]
fn test_event_with_event_object() {
    let js = compile_template_to_js(r#"<input (input)="handleInput($event)">"#, "TestComponent");
    insta::assert_snapshot!("event_with_event_object", js);
}

#[test]
fn test_sequence_expression_in_event_handler() {
    // Test that sequence expressions (multiple statements separated by `;`)
    // in event handlers are properly preserved.
    // For example: `(blur)="isInputFocused.set(false); onTouch()"`
    // Should produce:
    //   ctx.isInputFocused.set(false);  // First statement
    //   return ctx.onTouch();           // Last statement wrapped in return
    let js = compile_template_to_js(
        r#"<input (blur)="isInputFocused.set(false); onTouch()">"#,
        "TestComponent",
    );
    insta::assert_snapshot!("sequence_expression_in_event_handler", js);
}

#[test]
fn test_sequence_expression_with_template_ref() {
    // Test that sequence expressions in event handlers work correctly
    // when there's a template reference on the element.
    // This reproduces an issue where the first statement would use `_unnamed_X`
    // instead of `ctx`.
    let js = compile_template_to_js(
        r#"<form>
  <input #input (blur)="isInputFocused.set(false); onTouch()"/>
  <button *ngIf="showBtn">test</button>
</form>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("sequence_expression_with_template_ref", js);
}

// ============================================================================
// Two-way Binding Tests
// ============================================================================

#[test]
fn test_two_way_binding() {
    let js = compile_template_to_js(r#"<input [(ngModel)]="name">"#, "TestComponent");
    insta::assert_snapshot!("two_way_binding", js);
}

// ============================================================================
// Template Reference Tests
// ============================================================================

#[test]
fn test_template_reference() {
    let js = compile_template_to_js(
        r#"<input #myInput><button (click)="myInput.focus()"></button>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("template_reference", js);
}

#[test]
fn test_unused_template_reference() {
    // The #unusedRef reference is declared but not used in the click handler
    // The optimizer should remove the reference variable from the listener
    let js = compile_template_to_js(
        r#"<input #unusedRef><button (click)="doSomething()"></button>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("unused_template_reference", js);
}

#[test]
fn test_multiple_refs_partial_use() {
    // Multiple references declared, but only one is used in the click handler
    // The optimizer should keep only the used reference (usedRef) and remove the unused ones
    let js = compile_template_to_js(
        r#"<input #usedRef><input #unusedRef1><input #unusedRef2><button (click)="usedRef.focus()"></button>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("multiple_refs_partial_use", js);
}

// ============================================================================
// Control Flow Tests
// ============================================================================

#[test]
fn test_if_block() {
    let js = compile_template_to_js(r"@if (condition) { <div>Visible</div> }", "TestComponent");
    insta::assert_snapshot!("if_block", js);
}

#[test]
fn test_if_else_block() {
    let js = compile_template_to_js(
        r"@if (condition) { <div>True</div> } @else { <div>False</div> }",
        "TestComponent",
    );
    insta::assert_snapshot!("if_else_block", js);
}

#[test]
fn test_if_else_block_with_different_classes() {
    let js = compile_template_to_js(
        r#"@if (condition) { <div class="class-a">True</div> } @else { <div class="class-b">False</div> }"#,
        "TestComponent",
    );
    insta::assert_snapshot!("if_else_block_with_different_classes", js);
}

#[test]
fn test_conditional_alias_with_listener() {
    // Test @if with alias (as) where the alias is used in a listener handler
    // This tests that:
    // 1. restoreView() result is assigned directly to the alias variable
    // 2. No intermediate ctx_r5 variable followed by const parent_r7 = ctx_r5
    // 3. In update blocks, the alias should have const parent_r6 = ctx
    let js = compile_template_to_js(
        r#"@if (getParent(renderedOptions); as parent) {
  <button (click)="viewOption(parent, $event)">Click me</button>
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("conditional_alias_with_listener", js);
}

#[test]
fn test_conditional_alias_with_binding_and_listener() {
    // Test @if with alias where the alias is used both in bindings and listener
    // This tests that:
    // 1. restoreView() result is assigned directly to the alias variable in listener
    // 2. In update blocks, the alias should have const parent = ctx
    let js = compile_template_to_js(
        r#"@if (getParent(renderedOptions); as parent) {
  <button [class.active]="parent.active" (click)="viewOption(parent, $event)">{{parent.label}}</button>
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("conditional_alias_with_binding_and_listener", js);
}

#[test]
fn test_bitwarden_if_else_template() {
    // Simplified bitwarden template that should produce different const indices
    let js = compile_template_to_js(
        r#"@if (showWarning | async) {
  <div class="tw-h-screen tw-flex tw-justify-center tw-items-center tw-p-4">
    Warning content
  </div>
} @else {
  <div class="tw-h-screen tw-w-screen">
    Main content
  </div>
}"#,
        "AppComponent",
    );
    insta::assert_snapshot!("bitwarden_if_else_template", js);
}

#[test]
fn test_for_block() {
    let js = compile_template_to_js(
        r"@for (item of items; track item.id) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    insta::assert_snapshot!("for_block", js);
}

#[test]
fn test_for_with_empty() {
    let js = compile_template_to_js(
        r"@for (item of items; track item.id) { <div>{{item.name}}</div> } @empty { <div>No items</div> }",
        "TestComponent",
    );
    insta::assert_snapshot!("for_with_empty", js);
}

#[test]
fn test_switch_block() {
    let js = compile_template_to_js(
        r"@switch (value) { @case (1) { <div>One</div> } @case (2) { <div>Two</div> } @default { <div>Other</div> } }",
        "TestComponent",
    );
    insta::assert_snapshot!("switch_block", js);
}

#[test]
fn test_switch_block_default_first() {
    // Test @switch with @default appearing first - Angular reorders @default last
    // Angular's ingestSwitchBlock iterates in source order, but generateConditionalExpressions
    // splices @default out as the ternary fallback. We reorder at ingest to match the final output.
    let js = compile_template_to_js(
        r"@switch (value) { @default { <div>Other</div> } @case (1) { <div>One</div> } @case (2) { <div>Two</div> } }",
        "TestComponent",
    );
    insta::assert_snapshot!("switch_block_default_first", js);
}

#[test]
fn test_if_alias_with_switch() {
    // Test @if with alias (as data) followed by @switch (data.xxx)
    // This is the pattern from Fido2Component that was generating unnecessary
    // const data_r13 = ctx; instead of using ctx.xxx directly
    let js = compile_template_to_js(
        r#"@if (data$ | async; as data) {
  @switch (data.message.type) {
    @case ("ConfirmNewCredential") { <div>Confirm</div> }
    @case ("PickCredential") { <div>Pick</div> }
    @default { <div>Other</div> }
  }
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("if_alias_with_switch", js);
}

// ============================================================================
// Defer Block Tests
// ============================================================================

#[test]
fn test_defer_block() {
    let js = compile_template_to_js(r"@defer { <heavy-component /> }", "TestComponent");
    insta::assert_snapshot!("defer_block", js);
}

/// Tests that @defer blocks inside i18n contexts get wrapped with i18nStart/i18nEnd.
/// Angular propagates i18n context into defer view templates so that the deferred
/// content is part of the i18n message. Each defer sub-block (main, loading,
/// placeholder, error) gets its own sub-template index.
///
/// Ported from Angular compliance test:
/// `r3_view_compiler_i18n/blocks/defer.ts`
#[test]
fn test_defer_inside_i18n() {
    let js = compile_template_to_js(
        r"<div i18n>
  Content:
  @defer (when isLoaded) {
    before<span>middle</span>after
  } @placeholder {
    before<div>placeholder</div>after
  } @loading {
    before<button>loading</button>after
  } @error {
    before<h1>error</h1>after
  }
</div>",
        "MyApp",
    );

    // Each deferred template function should be wrapped with i18nStart/i18nEnd
    // with increasing sub-template indices (1, 2, 3, 4)
    assert!(
        js.contains("i18nStart(0,0,1)"),
        "Main defer template should have i18nStart with sub-template index 1. Output:\n{js}"
    );
    assert!(
        js.contains("i18nStart(0,0,2)"),
        "Loading defer template should have i18nStart with sub-template index 2. Output:\n{js}"
    );
    assert!(
        js.contains("i18nStart(0,0,3)"),
        "Placeholder defer template should have i18nStart with sub-template index 3. Output:\n{js}"
    );
    assert!(
        js.contains("i18nStart(0,0,4)"),
        "Error defer template should have i18nStart with sub-template index 4. Output:\n{js}"
    );

    // The deferred templates should have 2 decls (i18nStart + element), not 1
    // domTemplate(N, fn, 2, 0) - 2 declarations for each deferred view
    assert!(
        js.contains("MyApp_Defer_2_Template,2,0)"),
        "Main defer domTemplate should have 2 decls. Output:\n{js}"
    );

    insta::assert_snapshot!("defer_inside_i18n", js);
}

/// When @defer is nested inside a structural directive (*ngIf template) that's inside
/// an i18n context, the i18n wrapping must propagate through the template boundary
/// to the defer view. This matches the unlock-view-confirm ClickUp pattern.
#[test]
fn test_defer_inside_structural_directive_in_i18n() {
    let js = compile_template_to_js(
        r#"<div i18n>
  text
  <span *ngIf="show">
    @defer (on idle) {
      <span>deferred</span>
    }
  </span>
</div>"#,
        "MyApp",
    );

    // The defer template should have i18nStart wrapping since it's
    // transitively inside an i18n context (through the *ngIf template)
    assert!(
        js.contains("i18nStart(0,"),
        "Defer template inside structural directive in i18n should have i18nStart. Output:\n{js}"
    );

    insta::assert_snapshot!("defer_inside_structural_directive_in_i18n", js);
}

#[test]
fn test_defer_with_loading() {
    let js = compile_template_to_js(
        r"@defer { <heavy-component /> } @loading { <div>Loading...</div> }",
        "TestComponent",
    );
    insta::assert_snapshot!("defer_with_loading", js);
}

#[test]
fn test_defer_on_viewport() {
    let js =
        compile_template_to_js(r"@defer (on viewport) { <heavy-component /> }", "TestComponent");
    insta::assert_snapshot!("defer_on_viewport", js);
}

/// Reproduces voidzero-dev/oxc-angular-compiler#289 (aligned with Angular's
/// local-compilation behavior): a `@defer` block on a component whose lazy
/// dependency is declared in the `@Component.deferredImports` array must emit
/// a deferrable-dependencies resolver — a no-arg arrow returning an array of
/// dynamic `import()` calls — wired in as the third argument of
/// `ɵɵdefer(...)`. Previously the resolver argument was omitted entirely and
/// no `import('./lazy')` appeared anywhere in the output.
#[test]
fn test_defer_emits_dependency_resolver_from_deferred_imports() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    deferredImports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
    standalone: true,
})
export class Parent {}
"#;

    let result = transform_angular_file(&allocator, "parent.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // The output must contain a dynamic import of the lazy module.
    assert!(
        code.contains("import(\"./lazy\")") || code.contains("import('./lazy')"),
        "Expected a dynamic `import('./lazy')` for the deferred dependency. Output:\n{code}"
    );

    // The dynamic import should be followed by `.then(...)` with a callback
    // that reads `m.LazyCmp` (the original / local export name). Exact
    // whitespace and parenthesization of the arrow params is up to the
    // emitter.
    let collapsed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        collapsed.contains(".then((m)=>m.LazyCmp)") || collapsed.contains(".then(m=>m.LazyCmp)"),
        "Expected `.then(m => m.LazyCmp)` chain after dynamic import. Output:\n{code}"
    );

    // The resolver expression must be wired into `ɵɵdefer(...)` as a non-null
    // third argument.
    assert!(
        code.contains("ɵɵdefer(1,0,() =>[import(\"./lazy\")")
            || code.contains("ɵɵdefer(1,0,()=>[import(\"./lazy\")"),
        "ɵɵdefer's 3rd argument (resolverFn) must be the deferrable-deps arrow function. Output:\n{code}"
    );

    // The deferred symbol must NOT appear in the eager dependencies factory.
    // (When the component has only `deferredImports` and no `imports`, the
    // `dependencies` field is omitted entirely.)
    assert!(
        !code.contains("ɵɵgetComponentDepsFactory(Parent,[LazyCmp]"),
        "Deferred symbol must not appear in the eager `ɵɵgetComponentDepsFactory` array. Output:\n{code}"
    );

    // `setClassMetadataAsync` (not the sync `setClassMetadata`) should be
    // emitted so the TestBed-facing metadata is built lazily — the deferred
    // symbol is reached through the async callback's parameter, not a
    // static import reference.
    assert!(
        code.contains("setClassMetadataAsync"),
        "Components with `deferredImports` should emit `setClassMetadataAsync`. Output:\n{code}"
    );
}

/// Aliased imports must use the original exported name in `m.X`, not the
/// local alias.
#[test]
fn test_defer_dependency_resolver_uses_original_export_name_for_aliases() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { HeavyWidget as Heavy } from './widget';

@Component({
    selector: 'app-parent',
    deferredImports: [Heavy],
    template: '@defer { <app-heavy/> }',
    standalone: true,
})
export class Parent {}
"#;

    let result = transform_angular_file(&allocator, "parent.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    assert!(
        code.contains("import(\"./widget\")") || code.contains("import('./widget')"),
        "Expected dynamic `import('./widget')`. Output:\n{code}"
    );

    // The chain must resolve `m.HeavyWidget` (the original export name), not
    // `m.Heavy` (the local alias).
    let collapsed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        collapsed.contains(".then((m)=>m.HeavyWidget)")
            || collapsed.contains(".then(m=>m.HeavyWidget)"),
        "Expected `.then(m => m.HeavyWidget)` using the original export name. Output:\n{code}"
    );
    assert!(
        !collapsed.contains("m.Heavy)") && !collapsed.contains("m.Heavy,"),
        "Aliased import must not resolve to the local alias `Heavy`. Output:\n{code}"
    );

    // The `setClassMetadataAsync` callback parameter must use the *local*
    // binding so the decorator metadata body's `Heavy` reference shadows the
    // outer static import — letting bundlers drop the eager declaration.
    // Angular emits `(HeavyWidget) =>` here and leaves the static import
    // pinned; we diverge to enable tree-shaking.
    assert!(
        collapsed.contains("(Heavy)=>"),
        "Expected `(Heavy) =>` callback parameter (local binding shadows static import). Output:\n{code}"
    );
    assert!(
        !collapsed.contains("(HeavyWidget)=>"),
        "Callback parameter must NOT be the original export name `HeavyWidget` (would leave the static import pinned). Output:\n{code}"
    );
}

/// Default imports must use `m.default` in the resolver chain.
#[test]
fn test_defer_dependency_resolver_uses_default_for_default_imports() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import LazyCmp from './lazy-default';

@Component({
    selector: 'app-parent',
    deferredImports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
    standalone: true,
})
export class Parent {}
"#;

    let result = transform_angular_file(&allocator, "parent.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    assert!(
        code.contains("import(\"./lazy-default\")") || code.contains("import('./lazy-default')"),
        "Expected dynamic `import('./lazy-default')`. Output:\n{code}"
    );

    let collapsed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        collapsed.contains(".then((m)=>m.default)") || collapsed.contains(".then(m=>m.default)"),
        "Default import must resolve to `m.default`. Output:\n{code}"
    );

    // The `setClassMetadataAsync` callback parameter name must be a legal JS
    // identifier — not the reserved word `default`. Use the local binding.
    assert!(
        !collapsed.contains("(default)=>") && !collapsed.contains("(default)=>"),
        "Default-import callback parameter must not be the reserved word `default`. Output:\n{code}"
    );
    assert!(
        collapsed.contains("(LazyCmp)=>"),
        "Default-import callback parameter must use the local binding name `LazyCmp`. Output:\n{code}"
    );
}

/// Components without `@defer` blocks must not generate a resolver function
/// (regression guard so we don't accidentally produce dead lazy-loading code).
#[test]
fn test_no_defer_block_skips_dependency_resolver() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { ChildCmp } from './child';

@Component({
    selector: 'app-parent',
    deferredImports: [ChildCmp],
    template: '<app-child/>',
    standalone: true,
})
export class Parent {}
"#;

    let result = transform_angular_file(&allocator, "parent.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    assert!(
        !code.contains("import(\"./child\")") && !code.contains("import('./child')"),
        "Components without @defer must not emit dynamic imports. Output:\n{code}"
    );
}

/// When the user lists a symbol only in `@Component.imports`, OXC must NOT
/// auto-detect it as deferrable (that's the full-compilation behavior, which
/// needs cross-file selector info OXC doesn't have). Issue #289's original
/// repro form (`imports: [LazyCmp]` with `@defer`) therefore emits no
/// resolver — the user must move the symbol to `deferredImports`.
#[test]
fn test_defer_with_only_imports_does_not_auto_detect() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { LazyCmp } from './lazy';

@Component({
    selector: 'app-parent',
    imports: [LazyCmp],
    template: '@defer { <app-lazy/> }',
    standalone: true,
})
export class Parent {}
"#;

    let result = transform_angular_file(&allocator, "parent.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    // `imports: [LazyCmp]` alone must remain eager — no dynamic import emitted.
    assert!(
        !code.contains("import(\"./lazy\")") && !code.contains("import('./lazy')"),
        "Symbols declared only in `imports` (not `deferredImports`) must not be lazy-loaded in local compilation. Output:\n{code}"
    );
}

/// Matches Angular's `validateNoImportOverlap` (handler.ts:2558+): a symbol
/// listed in both `imports` and `deferredImports` is an error — each
/// dependency must have a single, unambiguous role.
#[test]
fn test_defer_overlap_between_imports_and_deferred_imports_is_diagnostic() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { Mixed } from './mixed';

@Component({
    selector: 'app-parent',
    imports: [Mixed],
    deferredImports: [Mixed],
    template: '@defer { <app-mixed/> }',
    standalone: true,
})
export class Parent {}
"#;

    let result = transform_angular_file(&allocator, "parent.component.ts", source, None, None);

    assert!(
        result.has_errors(),
        "Symbol used in both `imports` and `deferredImports` must yield a diagnostic. Output:\n{}",
        result.code
    );
    let any_overlap_msg = result.diagnostics.iter().any(|d| {
        let s = format!("{d}");
        s.contains("`@Component.imports`") && s.contains("`@Component.deferredImports`")
    });
    assert!(
        any_overlap_msg,
        "Diagnostic should mention both `@Component.imports` and `@Component.deferredImports`. Got: {:?}",
        result.diagnostics
    );
}

#[test]
#[should_panic(
    expected = "Cannot specify additional `hydrate` triggers if `hydrate never` is present"
)]
fn test_hydrate_never_mutual_exclusivity() {
    // This should fail because "hydrate never" cannot be combined with other hydrate triggers
    compile_template_to_js(
        r"@defer (hydrate never; hydrate on idle) { <heavy-component /> }",
        "TestComponent",
    );
}

// ============================================================================
// Let Declaration Tests
// ============================================================================

#[test]
fn test_let_declaration() {
    let js = compile_template_to_js(
        r"@let greeting = 'Hello'; <div>{{greeting}}</div>",
        "TestComponent",
    );
    insta::assert_snapshot!("let_declaration", js);
}

#[test]
fn test_let_in_child_view() {
    // @let used inside @if (child view) should have declareLet and storeLet
    let js = compile_template_to_js(r"@let value = 123; @if (true) { {{value}} }", "TestComponent");
    insta::assert_snapshot!("let_in_child_view", js);
}

#[test]
fn test_let_in_property_binding_same_view() {
    // @let used only in property bindings in same view - should NOT have declareLet
    // This mimics the bitwarden layout.component.html case
    let js = compile_template_to_js(
        r#"@let id = "test-id"; <div [id]="id" [attr.data-id]="id"></div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("let_in_property_binding_same_view", js);
}

#[test]
fn test_let_used_in_ngif_child_view() {
    // @let used inside *ngIf (which creates a child view) SHOULD have declareLet and storeLet
    // This mimics the bitwarden anon-layout.component.html case:
    // @let iconInput = icon(); <div *ngIf="iconInput !== null"><bit-icon [icon]="iconInput"></bit-icon></div>
    let js = compile_template_to_js(
        r#"@let val = getData(); <div *ngIf="val !== null"><span [title]="val">{{val}}</span></div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("let_used_in_ngif_child_view", js);
}

#[test]
fn test_let_inside_for_if_with_component_method_call() {
    // Reproduces the _unnamed_N bug from ClickUp's stylesheet-viewer.component.html.
    //
    // Pattern: @for → @if → TWO @let declarations calling component methods
    //
    // When there are two @let expressions that both need the component context,
    // the context variable can't be inlined (used twice) and must be extracted
    // as a named variable like `const ctx_rN = i0.ɵɵnextContext()`.
    //
    // Expected (Angular):
    //   const item_rN = i0.ɵɵnextContext().$implicit;
    //   const ctx_rN = i0.ɵɵnextContext();
    //   i0.ɵɵstoreLet(ctx_rN.computeA(item_rN.id));
    //   ...
    //   const b_rN = i0.ɵɵstoreLet(ctx_rN.computeB(item_rN.text));
    //
    // Actual (Oxc bug):
    //   const item_rN = i0.ɵɵnextContext().$implicit;
    //   i0.ɵɵstoreLet(_unnamed_N.computeA(item_rN.id));
    //   ...
    //   const b_rN = i0.ɵɵstoreLet(i0.ɵɵnextContext().computeB(item_rN.text));
    let js = compile_template_to_js(
        r"@for (item of items; track item) { @if (showDetail()) { @let a = computeA(item.id); @let b = computeB(item.text); @if (a > 0) { <div>{{b}}</div> } } }",
        "TestComponent",
    );
    // The output must NOT contain _unnamed_ - all variables should be properly named
    assert!(
        !js.contains("_unnamed_"),
        "Generated JS contains _unnamed_ references, indicating a naming bug.\nGenerated JS:\n{js}"
    );
    insta::assert_snapshot!("let_inside_for_if_with_component_method_call", js);
}

// ============================================================================
// @let with pipe in child view Tests
// ============================================================================

#[test]
fn test_let_with_pipe_used_in_child_view() {
    // @let with pipe used in a child view (@if block) should keep BOTH declareLet and storeLet.
    //
    // When a @let wraps a pipe and is referenced from a child view:
    // - declareLet is needed because pipes use DI which requires the TNode
    // - storeLet is needed because the @let is accessed from another view via readContextLet
    //
    // Without storeLet, the pipe's varOffset would be wrong because storeLet contributes
    // 1 var to the var counting, and removing it shifts all subsequent varOffsets.
    //
    // Expected Angular output:
    //   i0.ɵɵstoreLet(i0.ɵɵpipeBind1(1, varOffset, ctx.name));
    //
    // Bug output (missing storeLet):
    //   i0.ɵɵpipeBind1(1, varOffset, ctx.name);
    let js = compile_template_to_js(
        r"@let value = name | uppercase; @if (true) { {{value}} }",
        "TestComponent",
    );
    // storeLet must wrap pipeBind because @let is used externally (in child @if view)
    assert!(
        js.contains("ɵɵstoreLet(i0.ɵɵpipeBind1("),
        "storeLet should wrap pipeBind1 when @let with pipe is used in child view. Output:\n{js}"
    );
    // declareLet must be present (pipes need TNode for DI)
    assert!(
        js.contains("ɵɵdeclareLet("),
        "declareLet should be present when @let contains a pipe. Output:\n{js}"
    );
    // readContextLet must be present in the child view
    assert!(
        js.contains("ɵɵreadContextLet("),
        "readContextLet should be present in child view. Output:\n{js}"
    );
    insta::assert_snapshot!("let_with_pipe_used_in_child_view", js);
}

#[test]
fn test_let_with_pipe_used_in_listener() {
    // @let with pipe used in an event listener in the same view should keep storeLet.
    //
    // Event listeners are callbacks (isCallback=true), so @let declarations
    // in the same view generate ContextLetReferenceExpr in the listener's handler ops.
    // This means the @let is "used externally" and storeLet must be preserved.
    let js = compile_template_to_js(
        r#"@let value = name | uppercase; <button (click)="onClick(value)">Click</button>"#,
        "TestComponent",
    );
    // storeLet must wrap pipeBind because @let is used externally (in listener callback)
    assert!(
        js.contains("ɵɵstoreLet(i0.ɵɵpipeBind1("),
        "storeLet should wrap pipeBind1 when @let with pipe is used in listener. Output:\n{js}"
    );
    insta::assert_snapshot!("let_with_pipe_used_in_listener", js);
}

#[test]
fn test_let_with_pipe_multiple_in_child_view_varoffset() {
    // Multiple @let declarations with pipes used in a child view.
    // Each storeLet contributes 1 var, so removing them would cause cumulative varOffset drift.
    //
    // This reproduces the ClickUp AdvancedTabComponent pattern where multiple @let
    // declarations with pipes have their storeLet wrappers incorrectly removed,
    // causing the second pipe's varOffset to drift by +1 for each missing storeLet.
    let js = compile_template_to_js(
        r"@let a = x | uppercase; @let b = y | lowercase; @if (true) { {{a}} {{b}} }",
        "TestComponent",
    );
    // Both @let values should have storeLet wrappers
    let store_let_count = js.matches("ɵɵstoreLet(").count();
    assert!(
        store_let_count >= 2,
        "Expected at least 2 storeLet calls for 2 @let declarations with pipes used in child view, got {store_let_count}. Output:\n{js}"
    );
    insta::assert_snapshot!("let_with_pipe_multiple_in_child_view_varoffset", js);
}

// ============================================================================
// Template literal Tests
// ============================================================================

#[test]
fn test_template_literal_with_pipe() {
    // {{ `${num | percent}` }} - template literal containing a pipe call on a @let variable.
    // TemplateLiteral was not handled in convert_ast_to_ir and fell through to
    // store_and_ref_expr, so the inner BindingPipe was never registered with
    // pipe_creation and the @let variable was resolved against ctx instead of the
    // local scope.
    let js = compile_template_to_js(r"@let num = 0.75; {{ `${num | percent}` }}", "TestComponent");
    assert!(js.contains("ɵɵpipeBind1"), "percent pipe should be registered. Output:\n{js}");
    insta::assert_snapshot!("template_literal_with_pipe", js);
}

#[test]
fn test_template_literal_with_pipe_and_text() {
    // Template literal with mixed text and pipe: `Value: ${num | percent} done`
    let js = compile_template_to_js(
        r"@let num = 0.75; {{ `Value: ${num | percent} done` }}",
        "TestComponent",
    );
    assert!(
        js.contains("ɵɵpipeBind1"),
        "percent pipe should be registered in template literal with surrounding text. Output:\n{js}"
    );
    insta::assert_snapshot!("template_literal_with_pipe_and_text", js);
}

#[test]
fn test_template_literal_without_pipe() {
    // Template literal without pipe should still work correctly (regression guard).
    let js =
        compile_template_to_js(r"@let name = 'world'; {{ `Hello ${name}!` }}", "TestComponent");
    insta::assert_snapshot!("template_literal_without_pipe", js);
}

#[test]
fn test_template_literal_pipe_in_attribute_binding() {
    // Template literal with pipe used as an attribute binding value.
    // Real-world pattern: [label]="`${(count() | number)}`"
    // Before the fix the pipe was silently dropped, producing `${ctx.count()}` instead.
    let js = compile_template_to_js(
        r#"<div [title]="`${count() | number} items`"></div>"#,
        "TestComponent",
    );
    assert!(
        js.contains("ɵɵpipeBind1"),
        "number pipe should appear in attribute binding template literal. Output:\n{js}"
    );
    insta::assert_snapshot!("template_literal_pipe_in_attribute_binding", js);
}

#[test]
fn test_template_literal_multiple_pipes() {
    // Two pipes inside one template literal. Both must be registered.
    let js = compile_template_to_js(
        r"@let a = 0.5; @let b = 1234; {{ `${a | percent} of ${b | number}` }}",
        "TestComponent",
    );
    assert!(
        js.matches("ɵɵpipeBind1").count() >= 2,
        "both percent and number pipes should appear. Output:\n{js}"
    );
    insta::assert_snapshot!("template_literal_multiple_pipes", js);
}

#[test]
fn test_template_literal_pipe_in_child_view() {
    // Template literal + pipe inside an @if child view.
    // Pipe must be registered in the child view's create block.
    let js = compile_template_to_js(
        r"@let n = 0.75; @if (true) { {{ `${n | percent}` }} }",
        "TestComponent",
    );
    assert!(
        js.contains("ɵɵpipeBind1"),
        "percent pipe should be registered in child view template literal. Output:\n{js}"
    );
    insta::assert_snapshot!("template_literal_pipe_in_child_view", js);
}

// ============================================================================
// @let self-reference / forward-reference Tests
// ============================================================================

#[test]
fn test_let_self_reference_replaced_with_undefined() {
    // A @let that references itself (self-reference) should have that reference
    // replaced with `undefined`, matching Angular's behavior.
    // Angular walks backward from the declaration op and replaces LexicalRead
    // in the declaration op itself.
    let js = compile_template_to_js(r"@let x = x + 1; <div>{{x}}</div>", "TestComponent");
    insta::assert_snapshot!("let_self_reference", js);
}

// ============================================================================
// Object Spread Tests
// ============================================================================

#[test]
fn test_object_spread_in_binding() {
    // { ...base, extra: 'val' } — spread was silently dropped, resulting in { extra: 'val' }
    // Keys/values in LiteralMap are parallel arrays; LiteralMapKey::Spread was skipped in
    // convert_ast_to_ir so the spread expression never reached the IR or emitter.
    let js = compile_template_to_js(
        r#"<div [title]="{ ...base, extra: 'val' }"></div>"#,
        "TestComponent",
    );
    // Angular wraps object literals in pure functions; spread appears as ...a0 in the
    // pure function body and ctx.base is passed as the argument.
    assert!(js.contains("...a0"), "object spread should be preserved in output. Output:\n{js}");
    assert!(js.contains("ctx.base"), "spread variable should be referenced. Output:\n{js}");
    insta::assert_snapshot!("object_spread_in_binding", js);
}

#[test]
fn test_object_spread_only() {
    let js = compile_template_to_js(r#"<div [title]="{ ...base }"></div>"#, "TestComponent");
    assert!(js.contains("...a0"), "spread-only object should emit spread. Output:\n{js}");
    assert!(js.contains("ctx.base"), "spread variable should be referenced. Output:\n{js}");
    insta::assert_snapshot!("object_spread_only", js);
}

#[test]
fn test_object_multiple_spreads() {
    let js = compile_template_to_js(
        r#"<div [title]="{ ...a, ...b, key: 'val' }"></div>"#,
        "TestComponent",
    );
    assert!(
        js.contains("...a0") && js.contains("...a1"),
        "multiple spreads should both appear. Output:\n{js}"
    );
    assert!(
        js.contains("ctx.a") && js.contains("ctx.b"),
        "both spread variables should be referenced. Output:\n{js}"
    );
    insta::assert_snapshot!("object_multiple_spreads", js);
}

#[test]
fn test_object_spread_with_pipe() {
    // Pipe inside the same object literal as a spread — pipe must still be registered.
    let js = compile_template_to_js(
        r#"<div [title]="{ ...base, val: num | percent }"></div>"#,
        "TestComponent",
    );
    assert!(js.contains("...a0"), "spread should be preserved alongside pipe. Output:\n{js}");
    assert!(
        js.contains("ɵɵpipeBind1"),
        "pipe inside object literal with spread should still be registered. Output:\n{js}"
    );
    insta::assert_snapshot!("object_spread_with_pipe", js);
}

#[test]
fn test_object_spread_at_end() {
    let js =
        compile_template_to_js(r#"<div [title]="{ key: 'val', ...base }"></div>"#, "TestComponent");
    assert!(js.contains("...a0"), "trailing spread should be preserved. Output:\n{js}");
    assert!(js.contains("ctx.base"), "spread variable should be referenced. Output:\n{js}");
    insta::assert_snapshot!("object_spread_at_end", js);
}

// ============================================================================
// Spread in Complex Expressions
// ============================================================================

#[test]
fn test_spread_in_arrow_function_body() {
    // Array spread inside an arrow function binding. Arrow functions fall through to the
    // ExpressionStore in ingest (not explicitly handled), so the LiteralArray with SpreadElement
    // reaches convert_angular_expression_with_ctx directly. Before the fix to the LiteralArray
    // arm in reify/angular_expression.rs, SpreadElement entries were silently unwrapped,
    // resulting in `() => [ctx.base,"extra"]` instead of `() => [...ctx.base,"extra"]`.
    let js = compile_template_to_js(
        r#"<button (click)="handler(() => [...base, 'extra'])">click</button>"#,
        "TestComponent",
    );
    assert!(
        js.contains("...ctx.base"),
        "spread inside arrow function body should be preserved. Output:\n{js}"
    );
    insta::assert_snapshot!("spread_in_arrow_function_body", js);
}

#[test]
fn test_object_spread_chained_bindings() {
    // Two property bindings on the same element force the chaining phase to run.
    // The chaining phase clones instruction args via clone_expression. Before the fix to
    // chaining.rs, LiteralMapEntry::new() was used (which always sets is_spread: false),
    // silently dropping spread info from any LiteralMap that clone_expression encountered.
    let js = compile_template_to_js(
        r#"<div [title]="{ ...base, extra: 'val' }" [id]="myId"></div>"#,
        "TestComponent",
    );
    assert!(
        js.contains("...a0"),
        "spread should be preserved when bindings are chained. Output:\n{js}"
    );
    assert!(
        js.contains("ctx.base"),
        "spread variable should be referenced when bindings are chained. Output:\n{js}"
    );
    insta::assert_snapshot!("object_spread_chained_bindings", js);
}

// ============================================================================
// Array Spread Tests
// ============================================================================

#[test]
fn test_array_spread_in_binding() {
    let js = compile_template_to_js(r#"<div [title]="[...base, 'extra']"></div>"#, "TestComponent");
    assert!(js.contains("...a0"), "array spread should be preserved in output. Output:\n{js}");
    assert!(js.contains("ctx.base"), "spread variable should be referenced. Output:\n{js}");
    insta::assert_snapshot!("array_spread_in_binding", js);
}

#[test]
fn test_array_multiple_spreads() {
    let js =
        compile_template_to_js(r#"<div [title]="[...a, ...b, 'val']"></div>"#, "TestComponent");
    assert!(
        js.contains("...a0") && js.contains("...a1"),
        "multiple array spreads should both appear. Output:\n{js}"
    );
    assert!(
        js.contains("ctx.a") && js.contains("ctx.b"),
        "both spread variables should be referenced. Output:\n{js}"
    );
    insta::assert_snapshot!("array_multiple_spreads", js);
}

#[test]
fn test_array_spread_vs_non_spread_pooling_distinct() {
    // Two array bindings whose entries are identical except for spread shape: `[a]` vs `[...a]`.
    // The pure-function pool deduplicates by body key, so if the key generation ignores the
    // spread metadata on DerivedLiteralArray entries, both bindings collide on the same pooled
    // helper and one binding gets the other's runtime semantics.
    let js = compile_template_to_js(
        r#"<div [title]="[a]"></div><div [id]="[...a]"></div>"#,
        "TestComponent",
    );
    // Each binding must produce its own pure function: one emitting `[a0]`, the other `[...a0]`.
    assert!(js.contains("[a0]"), "non-spread array binding should emit `[a0]` body. Output:\n{js}");
    assert!(
        js.contains("[...a0]"),
        "spread array binding should emit `[...a0]` body. Output:\n{js}"
    );
}

#[test]
fn test_object_spread_vs_non_spread_pooling_distinct() {
    // Object literal counterpart of the array test above. `{k: a}` and `{...a}` would collide
    // on the same pooled helper if spread metadata is excluded from the key.
    let js = compile_template_to_js(
        r#"<div [title]="{k: a}"></div><div [id]="{...a}"></div>"#,
        "TestComponent",
    );
    assert!(js.contains("...a0"), "object spread binding should emit `...a0`. Output:\n{js}");
    assert!(
        js.contains("k: a0") || js.contains("k:a0"),
        "non-spread object binding should emit `k: a0`. Output:\n{js}"
    );
}

// ============================================================================
// ng-content Tests
// ============================================================================

#[test]
fn test_ng_content() {
    let js = compile_template_to_js(r"<ng-content></ng-content>", "TestComponent");
    insta::assert_snapshot!("ng_content", js);
}

#[test]
fn test_ng_content_select() {
    let js =
        compile_template_to_js(r#"<ng-content select=".header"></ng-content>"#, "TestComponent");
    insta::assert_snapshot!("ng_content_select", js);
}

#[test]
fn test_ng_content_i18n_attr_not_in_projection() {
    // Verify i18n/i18n-* attrs are NOT included in ng-content projection attributes.
    // Angular's I18nMetaVisitor strips these before r3_template_transform runs.
    let js = compile_template_to_js(
        r#"<ng-content i18n select=".header"></ng-content>"#,
        "TestComponent",
    );
    assert!(
        !js.contains(r#""i18n""#),
        "i18n attribute should not appear in projection output. Got:\n{js}"
    );
}

#[test]
fn test_ng_content_with_bound_select() {
    // Tests that [select] binding on ng-content passes the binding name and value
    // as attributes to the projection instruction.
    // Angular treats ALL raw attrs on ng-content as TextAttributes, including bindings.
    // [select] with brackets is NOT the same as the static `select` attribute for the
    // CSS selector — the selector stays as "*" (wildcard).
    // Expected: ɵɵprojectionDef() with no args (single wildcard),
    //           ɵɵprojection(0, 0, ["[select]", "'[slot=expanded-content]'"])
    let js = compile_template_to_js(
        r#"<ng-content [select]="'[slot=expanded-content]'" />"#,
        "TestComponent",
    );
    insta::assert_snapshot!("ng_content_with_bound_select", js);
}

#[test]
fn test_ng_content_with_ng_project_as() {
    // Tests that ngProjectAs attribute generates the correct ProjectAs marker (5)
    // and parsed CSS selector in the attributes array.
    // Expected: ["ngProjectAs", "bit-label", 5, ["bit-label"]]
    let js = compile_template_to_js(
        r#"<ng-content ngProjectAs="bit-label"></ng-content>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("ng_content_with_ng_project_as", js);
}

#[test]
fn test_ng_content_with_ng_project_as_attribute_selector() {
    // Tests ngProjectAs with an attribute selector
    // Expected: ["ngProjectAs", "[card-content]", 5, ["", "card-content", ""]]
    let js = compile_template_to_js(
        r#"<ng-content ngProjectAs="[card-content]"></ng-content>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("ng_content_with_ng_project_as_attr_selector", js);
}

// ============================================================================
// ng-template Tests
// ============================================================================

#[test]
fn test_ng_template() {
    let js = compile_template_to_js(
        r"<ng-template #myTemplate><div>Template content</div></ng-template>",
        "TestComponent",
    );
    insta::assert_snapshot!("ng_template", js);
}

#[test]
fn test_ng_template_reference_in_binding() {
    // Tests that template references used in directive bindings (like ngIfElse)
    // generate proper ɵɵreference calls instead of ctx.propertyName
    let js = compile_template_to_js(
        r#"<ng-template #loadingState><div>Loading...</div></ng-template>
<div *ngIf="!loading; else loadingState">Content</div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("ng_template_reference_in_binding", js);
}

// ============================================================================
// Structural Directive Tests
// ============================================================================

#[test]
fn test_structural_directive_with_listener() {
    // Tests that listeners on elements with structural directives (like *ngIf)
    // are placed inside the embedded template, not at the parent level.
    // The click listener should be on the button inside Template_0, not on the
    // template instruction itself.
    let js = compile_template_to_js(
        r#"<button *ngIf="isVisible" (click)="handleClick()">Click me</button>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("structural_directive_with_listener", js);
}

#[test]
fn test_ngfor_attribute_ordering() {
    // Tests that *ngFor produces template attributes in the correct order:
    // ["ngFor", "ngForOf"] not ["ngForOf", "ngFor"]
    //
    // This is important for directive matching - the ordering must match Angular's
    // behavior where text attributes appear before bound attributes in the
    // template_attrs order.
    //
    // The fix ensures that text structural template attributes (like "ngFor")
    // go through the same code path as bound structural template attributes
    // (like "ngForOf"), preserving their original order from template_attrs.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<li *ngFor="let item of items">{{item}}</li>',
    standalone: true,
})
export class TestComponent {
    items = ['a', 'b', 'c'];
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    // The consts array should contain ["ngFor", "ngForOf"] in that order
    // This is emitted under the Template marker (4) in the attribute array
    // Template marker = 4, followed by attribute names
    //
    // Expected format: [4, "ngFor", "ngForOf", ...]
    // Wrong format:    [4, "ngForOf", "ngFor", ...]
    //
    // The regex looks for the consts array containing the Template marker (4)
    // followed by "ngFor" then "ngForOf"
    let has_correct_order = result.code.contains(r#"4,"ngFor","ngForOf""#);

    assert!(
        has_correct_order,
        "ngFor should appear before ngForOf in the consts array. Got:\n{}",
        result.code
    );

    // Also verify that the wrong order is NOT present
    let has_wrong_order = result.code.contains(r#"4,"ngForOf","ngFor""#);
    assert!(
        !has_wrong_order,
        "ngForOf should NOT appear before ngFor in the consts array. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("ngfor_attribute_ordering", result.code);
}

#[test]
fn test_event_before_property_in_bindings() {
    // Tests that in the bindings section of consts (marker 3), event bindings
    // come before property bindings. This is important because Angular expects:
    //   [3, "click", "disabled"]  (events first, then properties)
    // Not:
    //   [3, "disabled", "click"]  (properties before events - WRONG)
    //
    // This order is determined by the iteration order in attribute_extraction.ts,
    // which processes create ops (listeners) before update ops (properties).
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<button (click)="onClick()" [disabled]="isDisabled">Click</button>',
    standalone: true,
})
export class TestComponent {
    isDisabled = false;
    onClick() {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The consts array should contain the bindings marker (3) followed by
    // the event name first, then the property name.
    // Expected: [3, "click", "disabled"]
    // Wrong:    [3, "disabled", "click"]
    let has_correct_order = result.code.contains(r#"3,"click","disabled""#);

    assert!(
        has_correct_order,
        "Event binding 'click' should appear before property binding 'disabled' in the consts array. Got:\n{}",
        result.code
    );

    // Also verify that the wrong order is NOT present
    let has_wrong_order = result.code.contains(r#"3,"disabled","click""#);
    assert!(
        !has_wrong_order,
        "Property binding 'disabled' should NOT appear before event binding 'click' in the consts array. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("event_before_property_in_bindings", result.code);
}

// ============================================================================
// Compilation Mode Tests (Full vs DomOnly)
// ============================================================================

#[test]
fn test_standalone_component_uses_full_mode() {
    // OXC operates as a single-file compiler, equivalent to Angular's local compilation mode.
    // In local compilation mode, Angular ALWAYS sets hasDirectiveDependencies=true,
    // which means DomOnly mode is never used for component templates.
    // See: angular/packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts:1257
    //
    // This test ensures standalone components with no imports use Full mode instructions
    // (ɵɵelementStart) NOT DomOnly mode instructions (ɵɵdomElementStart).
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-external',
    template: '<div class="container"><h2>{{ title }}</h2><ul>@for (item of items; track item) { <li>{{ item }}</li> }</ul></div>',
    standalone: true,
})
export class ExternalComponent {
    title = 'External Component';
    items = ['Apple', 'Banana', 'Cherry'];
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Must use Full mode instructions (elementStart), not DomOnly (domElementStart)
    assert!(
        !result.code.contains("domElementStart"),
        "Standalone component should use Full mode (elementStart), not DomOnly mode (domElementStart).\n\
         OXC operates in local compilation mode where hasDirectiveDependencies is always true.\n\
         Output:\n{}",
        result.code
    );
    assert!(
        result.code.contains("elementStart"),
        "Expected Full mode instruction ɵɵelementStart in output.\nOutput:\n{}",
        result.code
    );

    insta::assert_snapshot!("standalone_component_uses_full_mode", result.code);
}

// ============================================================================
// Nested Control Flow Tests
// ============================================================================

/// Tests that bare component property tracks like `track id` generate a wrapper function,
/// NOT a direct `ctx.id` reference. Angular emits:
///   `function _forTrack($index,$item) { return this.id; }` then `ɵɵrepeaterCreate(..., _forTrack, true)`
/// Previously, oxc incorrectly optimized this to `ctx.id` passed directly, which changes
/// runtime semantics (passes a non-function value instead of a track function).
#[test]
fn test_for_track_bare_component_property() {
    let js = compile_template_to_js(
        r"@for (item of items; track id) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    // Angular wraps bare property reads in a function: _forTrack($index,$item){ return this.id; }
    // It should NOT be optimized to ctx.id directly
    assert!(
        js.contains("_forTrack"),
        "Bare component property track should generate a wrapper function _forTrack, not a direct ctx.id reference. Output:\n{js}"
    );
    assert!(
        js.contains("return this.id"),
        "Track wrapper function should use 'this.id' (not ctx.id) since usesComponentInstance is true. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_bare_component_property", js);
}

/// Tests that `track trackByFn` (bare method reference) also generates a wrapper function.
/// Angular does NOT optimize bare property reads like `track trackByFn` into direct references.
/// Only `track trackByFn($index)` or `track trackByFn($index, $item)` get optimized.
#[test]
fn test_for_track_bare_method_reference() {
    let js = compile_template_to_js(
        r"@for (item of items; track trackByFn) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    // Angular wraps bare method references in a function
    assert!(
        js.contains("_forTrack"),
        "Bare method reference track should generate a wrapper function _forTrack. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_bare_method_reference", js);
}

/// Tests that `track trackByFn($index)` inside a nested view (e.g. @if) uses
/// `componentInstance().trackByFn` instead of `ctx.trackByFn`.
///
/// Angular's `isTrackByFunctionCall` (track_fn_optimization.ts:84-115) only optimizes
/// when the context receiver's view is the root view. The main optimization then checks
/// `receiver.receiver.view === unit.xref` to decide between `ctx.method` (root view)
/// and `componentInstance().method` (non-root view).
///
/// When the @for is inside @if, the repeater is in a non-root view, so Angular uses
/// the `componentInstance().method` path.
#[test]
fn test_for_track_method_call_in_nested_view() {
    // @for inside @if: repeater is in a non-root view
    let js = compile_template_to_js(
        r"@if (showItems) { @for (item of items; track trackByFn($index)) { <div>{{item.name}}</div> } }",
        "TestComponent",
    );
    // In a nested view, Angular uses componentInstance().trackByFn
    assert!(
        js.contains("componentInstance"),
        "Track method call in nested view should use componentInstance().trackByFn. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_method_call_in_nested_view", js);
}

/// Tests that `track trackByFn($index)` in the root view uses `ctx.trackByFn`.
#[test]
fn test_for_track_method_call_in_root_view() {
    // @for directly in root view
    let js = compile_template_to_js(
        r"@for (item of items; track trackByFn($index)) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    // In the root view, Angular uses ctx.trackByFn directly
    assert!(
        js.contains("ctx.trackByFn"),
        "Track method call in root view should use ctx.trackByFn. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_method_call_in_root_view", js);
}

/// Tests that `track trackByFn($index, item)` in the root view uses `ctx.trackByFn`.
/// Note: The loop variable name (e.g. `item`) is used, not the literal `$item`.
/// `generateTrackVariables` converts `item` → `$item` ReadVarExpr, which is then optimizable.
#[test]
fn test_for_track_method_call_with_both_args() {
    let js = compile_template_to_js(
        r"@for (item of items; track trackByFn($index, item)) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    assert!(
        js.contains("ctx.trackByFn"),
        "Track method call with ($index, item) in root view should use ctx.trackByFn. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_method_call_with_both_args", js);
}

#[test]
fn test_nested_for_loops() {
    let js = compile_template_to_js(
        r"@for (group of groups; track group.id) { <div>@for (item of group.items; track item.id) { <span>{{item.name}}</span> }</div> }",
        "TestComponent",
    );
    insta::assert_snapshot!("nested_for_loops", js);
}

#[test]
fn test_nested_for_with_outer_scope_track() {
    // Reproduces the bug where inner @for track expression captures outer-scope variable.
    // The inner @for's `track group.id` references `group` from the outer @for.
    // Angular generates `function _forTrack1($index,$item) { return this.group.id; }` with
    // usesComponentInstance=true, NOT an arrow function with an out-of-scope identifier.
    let js = compile_template_to_js(
        r"@for (group of groups; track group.id) { @for (item of group.items; track group.id) { <span>{{item.name}}</span> } }",
        "TestComponent",
    );
    insta::assert_snapshot!("nested_for_with_outer_scope_track", js);
}

/// Tests that `track prefix() + item.id` generates a regular function (not arrow function).
/// When a binary expression in track contains a component method call, the generated
/// track function must use `function` declaration to properly bind `this`.
#[test]
fn test_for_track_binary_with_component_method() {
    let js = compile_template_to_js(
        r"@for (item of items; track prefix() + item.id) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    // Must generate a regular function, not an arrow function, because prefix() needs `this`
    assert!(
        js.contains("function _forTrack"),
        "Track with binary operator containing component method should generate a regular function. Output:\n{js}"
    );
    assert!(
        js.contains("this.prefix()"),
        "Track function should use 'this.prefix()' for component method access. Output:\n{js}"
    );
    // Must NOT be an arrow function (arrow functions don't bind `this`)
    assert!(
        !js.contains("const _forTrack"),
        "Should NOT generate an arrow function (const _forTrack = ...) for track expressions that reference component members. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_binary_with_component_method", js);
}

/// Tests that nullish coalescing (??) in track with component method generates a regular function.
/// This is the exact pattern from the original bug report: `track item.prefix ?? defaultPrefix()`
#[test]
fn test_for_track_nullish_coalescing_with_component_method() {
    let js = compile_template_to_js(
        r"@for (item of items; track item.prefix ?? defaultPrefix()) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    assert!(
        js.contains("function _forTrack"),
        "Track with ?? operator containing component method should generate a regular function. Output:\n{js}"
    );
    assert!(
        !js.contains("const _forTrack"),
        "Should NOT generate an arrow function for track with ?? referencing component members. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_nullish_coalescing_with_component_method", js);
}

/// Tests that ternary in track with component method generates a regular function.
#[test]
fn test_for_track_ternary_with_component_method() {
    let js = compile_template_to_js(
        r"@for (item of items; track useId() ? item.id : item.name) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    assert!(
        js.contains("function _forTrack"),
        "Track with ternary containing component method should generate a regular function. Output:\n{js}"
    );
    assert!(
        !js.contains("const _forTrack"),
        "Should NOT generate an arrow function for track with ternary referencing component members. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_ternary_with_component_method", js);
}

/// Tests that a complex track expression with multiple component references and binary operators
/// generates a regular function. Mirrors the original bug: `(tag.queryPrefix ?? queryPrefix()) + '.' + tag.key`
#[test]
fn test_for_track_complex_binary_with_nullish_coalescing() {
    let js = compile_template_to_js(
        r"@for (tag of visibleTags(); track (tag.queryPrefix ?? queryPrefix()) + '.' + tag.key) { <span>{{ tag.key }}</span> }",
        "TestComponent",
    );
    assert!(
        js.contains("function _forTrack"),
        "Complex track with ?? and + containing component method should generate a regular function. Output:\n{js}"
    );
    assert!(
        !js.contains("const _forTrack"),
        "Should NOT generate an arrow function. Output:\n{js}"
    );
    assert!(
        js.contains("this.queryPrefix()"),
        "Track function should use 'this.queryPrefix()' for component method access. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_complex_binary_with_nullish_coalescing", js);
}

/// Tests that a track expression with only item property reads in binary operators
/// correctly generates an arrow function (no component context needed).
#[test]
fn test_for_track_binary_without_component_context() {
    let js = compile_template_to_js(
        r"@for (item of items; track item.type + ':' + item.id) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    // This should be an arrow function since no component members are referenced
    assert!(
        js.contains("const _forTrack"),
        "Track with binary operator using only item properties should generate an arrow function. Output:\n{js}"
    );
    assert!(
        !js.contains("function _forTrack"),
        "Should NOT generate a regular function when no component members are referenced. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_binary_without_component_context", js);
}

/// Tests that negation (!) in track with component method generates a regular function.
#[test]
fn test_for_track_not_with_component_method() {
    let js = compile_template_to_js(
        r"@for (item of items; track !isDisabled()) { <div>{{item.name}}</div> }",
        "TestComponent",
    );
    assert!(
        js.contains("function _forTrack"),
        "Track with ! operator containing component method should generate a regular function. Output:\n{js}"
    );
    assert!(
        !js.contains("const _forTrack"),
        "Should NOT generate an arrow function. Output:\n{js}"
    );
    insta::assert_snapshot!("for_track_not_with_component_method", js);
}

#[test]
fn test_if_inside_for() {
    let js = compile_template_to_js(
        r"@for (item of items; track item.id) { @if (item.visible) { <div>{{item.name}}</div> } }",
        "TestComponent",
    );
    insta::assert_snapshot!("if_inside_for", js);
}

#[test]
fn test_for_inside_if() {
    let js = compile_template_to_js(
        r"@if (showItems) { @for (item of items; track item.id) { <div>{{item.name}}</div> } }",
        "TestComponent",
    );
    insta::assert_snapshot!("for_inside_if", js);
}

#[test]
fn test_switch_inside_for() {
    let js = compile_template_to_js(
        r"@for (item of items; track item.id) { @switch (item.type) { @case ('a') { <div>Type A</div> } @case ('b') { <div>Type B</div> } @default { <div>Unknown</div> } } }",
        "TestComponent",
    );
    insta::assert_snapshot!("switch_inside_for", js);
}

#[test]
fn test_for_with_context_variables() {
    let js = compile_template_to_js(
        r#"@for (item of items; track item.id; let idx = $index, first = $first, last = $last) { <div [class.first]="first" [class.last]="last">{{idx}}: {{item.name}}</div> }"#,
        "TestComponent",
    );
    insta::assert_snapshot!("for_with_context_variables", js);
}

/// Tests that $index is correctly resolved in listener handlers in @for loops.
/// This is a regression test for an issue where context variables like $index
/// would use a fallback name like `_unnamed_1757` instead of being properly
/// captured from the restored view context as `ɵ$index_xxx_rN.$index`.
#[test]
fn test_for_listener_with_index() {
    let js = compile_template_to_js(
        r#"@for (item of items; track item.id; let idx = $index) { <button (click)="remove(idx)">Remove #{{ idx }}</button> }"#,
        "TestComponent",
    );
    // The listener should have:
    // 1. A variable that reads $index from the restored view context
    // 2. Use that variable (like idx_r1) in the event handler expression
    // 3. NOT use _unnamed_XXX fallback names
    assert!(
        !js.contains("_unnamed_"),
        "Found _unnamed_ fallback variable - $index resolution failed"
    );
    insta::assert_snapshot!("for_listener_with_index", js);
}

/// Tests that $index is correctly resolved in two-way binding handlers in @for loops.
/// This is another regression test for the $index issue - the two-way binding
/// handler (e.g., [(ngModel)]="items[$index].value") needs proper $index resolution.
#[test]
fn test_for_two_way_binding_with_index() {
    let js = compile_template_to_js(
        r#"@for (item of items; track $index) { <input [(ngModel)]="items[$index].value"> }"#,
        "TestComponent",
    );
    // The two-way listener should have:
    // 1. A variable that reads $index from the restored view context
    // 2. Use that variable in the binding set expression: items[idx_rN].value = $event
    // 3. NOT use _unnamed_XXX fallback names
    assert!(
        !js.contains("_unnamed_"),
        "Found _unnamed_ fallback variable - $index resolution in two-way binding failed"
    );
    insta::assert_snapshot!("for_two_way_binding_with_index", js);
}

// ============================================================================
// Pipe Tests
// ============================================================================

#[test]
fn test_interpolation_with_pipe() {
    let js = compile_template_to_js(r"<div>{{name | uppercase}}</div>", "TestComponent");
    insta::assert_snapshot!("interpolation_with_pipe", js);
}

#[test]
fn test_pipe_with_arguments() {
    let js = compile_template_to_js(r"<div>{{date | date:'yyyy-MM-dd'}}</div>", "TestComponent");
    insta::assert_snapshot!("pipe_with_arguments", js);
}

#[test]
fn test_chained_pipes() {
    let js =
        compile_template_to_js(r"<div>{{name | lowercase | slice:0:10}}</div>", "TestComponent");
    insta::assert_snapshot!("chained_pipes", js);
}

#[test]
fn test_pipe_in_property_binding() {
    let js = compile_template_to_js(r#"<div [title]="name | uppercase"></div>"#, "TestComponent");
    insta::assert_snapshot!("pipe_in_property_binding", js);
}

#[test]
fn test_pipe_in_if_object_literal() {
    // Test pipes inside @if with object literal condition (alias pattern)
    // This should generate proper pipe declarations and pipeBind calls
    let js = compile_template_to_js(
        r"@if ({open: value | async, isOverlay: overlay | async}; as data) {
  <div>{{data.open}}</div>
}",
        "TestComponent",
    );
    insta::assert_snapshot!("pipe_in_if_object_literal", js);
}

#[test]
fn test_conditional_with_property_bindings_and_pipe() {
    // Test @if block with multiple property bindings including a pipe inside
    // This tests the var count calculation for embedded views
    // Expected: embedded view should have correct var count for:
    // - [routerLink] property binding (1 var)
    // - [icon] property binding (1 var)
    // - [ariaLabel] property binding with pipe (1 var + pipe vars)
    let js = compile_template_to_js(
        r#"@if (!hideLogo) {
  <a [routerLink]="['/']" class="tw-w-32">
    <bit-icon [icon]="logo" [ariaLabel]="'appLogoLabel' | i18n"></bit-icon>
  </a>
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("conditional_with_property_bindings_and_pipe", js);
}

// ============================================================================
// Safe Navigation Tests
// ============================================================================

#[test]
fn test_safe_navigation_interpolation() {
    let js = compile_template_to_js(r"<div>{{user?.name}}</div>", "TestComponent");
    insta::assert_snapshot!("safe_navigation_interpolation", js);
}

#[test]
fn test_safe_navigation_chain() {
    let js = compile_template_to_js(r"<div>{{user?.address?.city}}</div>", "TestComponent");
    insta::assert_snapshot!("safe_navigation_chain", js);
}

#[test]
fn test_safe_navigation_in_binding() {
    let js = compile_template_to_js(r#"<div [title]="user?.displayName"></div>"#, "TestComponent");
    insta::assert_snapshot!("safe_navigation_in_binding", js);
}

#[test]
fn test_safe_call() {
    let js = compile_template_to_js(r"<div>{{getData?.()}}</div>", "TestComponent");
    insta::assert_snapshot!("safe_call", js);
}

#[test]
fn test_safe_call_in_listener() {
    // When a listener handler contains `fn()?.method()`, Angular generates a temporary variable
    // because `fn()` is a function call that shouldn't be evaluated twice.
    // Expected output should contain `let tmp_N_0;` declaration inside the listener function.
    let js = compile_template_to_js(
        r#"<button (click)="getPopover()?.close()">Close</button>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("safe_call_in_listener", js);
}

#[test]
fn test_safe_property_read_with_call_receiver_in_listener() {
    // Pattern: `fn()?.prop` in listener — receiver is a call, needs tmp variable
    let js = compile_template_to_js(
        r#"<button (click)="getDialog()?.visible">Toggle</button>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("safe_property_read_with_call_receiver_in_listener", js);
}

#[test]
fn test_safe_call_in_listener_inside_conditional() {
    // When a listener is inside an embedded view (e.g., @if), handler_ops contains
    // restoreView and nextContext statements. The handler_expression (return value)
    // must be processed AFTER those ops, not once per op, to get the correct
    // tmp variable name (tmp_2_0, matching the op index after restoreView and nextContext).
    let js = compile_template_to_js(
        r#"@if (show) { <button (click)="getPopover()?.close()">Close</button> }"#,
        "TestComponent",
    );
    insta::assert_snapshot!("safe_call_in_listener_inside_conditional", js);
}

#[test]
fn test_pipe_in_binary_with_safe_property_read() {
    // Pattern from EmailCommentComponent: (comment$ | async || comment)?.new_mentioned_thread_count
    // When a pipe binding is inside a binary expression that is the receiver of a safe property read,
    // the compiler must generate a temporary variable to avoid evaluating the pipe twice.
    // TypeScript Angular compiler produces: (tmp = pipeBind(...) || fallback) == null ? null : tmp.prop
    // Without the fix, OXC duplicates the pipe call in both the guard and the access expression.
    let js = compile_template_to_js(
        r"<div>{{ ((data$ | async) || fallback)?.name }}</div>",
        "TestComponent",
    );
    insta::assert_snapshot!("pipe_in_binary_with_safe_property_read", js);
}

// ============================================================================
// Event Modifier Tests
// ============================================================================

#[test]
fn test_keyup_enter_event() {
    let js = compile_template_to_js(r#"<input (keyup.enter)="onSubmit()">"#, "TestComponent");
    insta::assert_snapshot!("keyup_enter_event", js);
}

#[test]
fn test_multiple_event_handlers() {
    let js = compile_template_to_js(
        r#"<div (click)="onClick()" (mouseover)="onHover()"></div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("multiple_event_handlers", js);
}

// ============================================================================
// Complex Template Tests
// ============================================================================

#[test]
fn test_todo_list() {
    let template = r#"
        <h1>{{title}}</h1>
        <input [(ngModel)]="newTodo" (keyup.enter)="addTodo()">
        @for (todo of todos; track todo.id) {
            <div [class.completed]="todo.done">
                <input type="checkbox" [(ngModel)]="todo.done">
                <span>{{todo.text}}</span>
                <button (click)="removeTodo(todo)">X</button>
            </div>
        } @empty {
            <p>No todos yet!</p>
        }
    "#;
    let js = compile_template_to_js(template, "TodoListComponent");
    insta::assert_snapshot!("todo_list", js);
}

// ============================================================================
// Component-Level Style Tests (Full File Transformation)
// ============================================================================

#[test]
fn test_component_with_inline_styles() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-styled',
    template: '<div class="container">Hello</div>',
    styles: ['.container { color: red; }']
})
export class StyledComponent {}
"#;

    let result = transform_angular_file(&allocator, "styled.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Verify styles are included in ɵcmp definition
    assert!(
        result.code.contains("styles:"),
        "Generated code should contain styles array in ɵcmp: {}",
        result.code
    );
    assert!(
        result.code.contains(".container"),
        "Generated code should contain the style content: {}",
        result.code
    );

    insta::assert_snapshot!("component_with_inline_styles", result.code);
}

#[test]
fn test_component_with_multiple_styles() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-multi-styled',
    template: '<div>Content</div>',
    styles: [
        '.first { color: blue; }',
        '.second { background: white; }'
    ]
})
export class MultiStyledComponent {}
";

    let result =
        transform_angular_file(&allocator, "multi-styled.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors());

    // Verify both styles are included
    assert!(result.code.contains(".first"), "Should contain first style");
    assert!(result.code.contains(".second"), "Should contain second style");

    insta::assert_snapshot!("component_with_multiple_styles", result.code);
}

#[test]
fn test_component_with_minified_styles() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-styled',
    template: '<div class="container">Hello</div>',
    styles: ['.container { color: red; background: transparent; }']
})
export class StyledComponent {}
"#;

    let mut options = ComponentTransformOptions::default();
    options.minify_component_styles = true;

    let result =
        transform_angular_file(&allocator, "styled.component.ts", source, Some(&options), None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    assert!(
        result.code.contains(".container[_ngcontent-%COMP%]{color:red;background:0 0}"),
        "Generated code should contain minified component styles: {}",
        result.code
    );
}

#[test]
fn test_component_without_styles_downgrades_encapsulation() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-no-styles',
    template: '<div>No styles</div>'
})
export class NoStylesComponent {}
";

    let result = transform_angular_file(&allocator, "no-styles.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors());

    // When there are no styles and encapsulation is Emulated (default),
    // it should be downgraded to None (value 2) or not set at all
    // per Angular compiler behavior (lines 315-323 of compiler.ts)
    insta::assert_snapshot!("component_without_styles", result.code);
}

#[test]
fn test_deeply_nested_if_in_for() {
    // Test case that reproduces the bug with extra standalone nextContext() calls.
    // Structure: @for > @if > component
    // The @if template should only have ONE nextContext() call to access $implicit,
    // not additional standalone nextContext() calls.
    let js = compile_template_to_js(
        r#"@for (account of accounts; track account.id) {
  @if (account.email != null) {
    <app-avatar [id]="account.id" [name]="account.name"></app-avatar>
  }
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("deeply_nested_if_in_for", js);
}

#[test]
fn test_reference_accessed_from_multiple_child_views() {
    // Test that reference variables are correctly named when accessed from multiple child views.
    // The reference #widget is declared on an element in the root view,
    // and accessed from multiple @if blocks (child views).
    // Both child views should share the same reference variable name.
    let js = compile_template_to_js(
        r#"<div #widget>
  @if (showA) {
    <button (click)="widget.focus()">A</button>
  }
  @if (showB) {
    <button (click)="widget.focus()">B</button>
  }
</div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("reference_accessed_from_multiple_child_views", js);
}

#[test]
fn test_reference_with_export_as_in_nested_views() {
    // Test that simulates the GridTableExample issue:
    // A reference with exportAs like #widget="ngGridCellWidget" is accessed
    // from multiple nested views at different depths.
    //
    // The expected behavior:
    // - `ɵɵreference(N)` should use the slot of the ELEMENT with the reference, not
    //   the slot where the variable is being used
    // - When the same reference is accessed from multiple child views, they should
    //   share the same variable name (e.g., `widget_r6` not `widget_r6`, `widget_r13`)
    let js = compile_template_to_js(
        r#"<div>
  <div #widget>
    @if (showA) {
      <button (click)="widget.focus()">A</button>
      @if (showNested) {
        <span>{{ widget.value }}</span>
      }
    }
    @if (showB) {
      <div (click)="widget.click()">B</div>
    }
  </div>
  <div #cb>
    @if (showC) {
      <span (click)="cb.reset()">C</span>
    }
  </div>
</div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("reference_with_export_as_in_nested_views", js);
}

#[test]
fn test_reference_in_sibling_child_views_with_for() {
    // More complex test: reference used in both @if and @for child views
    // This mimics patterns where references are accessed from multiple sibling views
    // at the same depth, as well as nested views.
    //
    // The key issue: when we have sibling child views that both access the same
    // reference from a parent view, they should share the same variable name.
    let js = compile_template_to_js(
        r#"<div>
  <ng-container>
    <div #widget>Content</div>
    @for (item of items; track item.id) {
      <span (click)="widget.select(item)">{{ item.name }}</span>
      @if (item.expanded) {
        <div (click)="widget.expand(item)">Details</div>
      }
    }
    @if (showAll) {
      <button (click)="widget.selectAll()">Select All</button>
    }
  </ng-container>
  <div #cb>Other</div>
  @if (showReset) {
    <button (click)="cb.reset()">Reset</button>
  }
</div>"#,
        "TestComponent",
    );
    insta::assert_snapshot!("reference_in_sibling_child_views_with_for", js);
}

#[test]
fn test_multiple_same_name_references_in_for() {
    // Test the GridTableExample pattern:
    // Inside a @for loop, multiple elements have the SAME reference name #widget.
    // Each #widget refers to a DIFFERENT element (different XrefId/slot).
    //
    // Expected behavior:
    // - Each #widget declaration creates a new variable with a unique suffix
    // - Each usage of `widget` resolves to the nearest `#widget` in its scope
    // - Different slots should result in different variable names
    //
    // This matches Angular's behavior where each element with #widget gets
    // its own slot and each usage of widget resolves to the correct slot.
    let js = compile_template_to_js(
        r#"@for (task of tasks(); track task) {
  <tr>
    <td>
      <div #widget (click)="widget.action1()">
        First Widget
      </div>
    </td>
    <td>
      <div #widget (click)="widget.action2()">
        Second Widget
      </div>
    </td>
  </tr>
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("multiple_same_name_references_in_for", js);
}

/// Test that attrs const is emitted for selectors with attributes.
///
/// When a selector like `span[bitBadge]` is used, Angular extracts the attribute
/// selector into a constant array that is referenced in the component definition.
///
/// Expected output:
/// ```javascript
/// const _c0 = ["bitBadge", ""];  // attrs const
/// BadgeComponent.ɵcmp = ɵɵdefineComponent({
///   attrs: _c0,
///   ...
/// })
/// ```
///
/// This was a bug where the attrs const was added to the pool during
/// `generate_component_definitions` but never emitted because `compile_template`
/// had already drained the pool to `declarations`.
#[test]
fn test_selector_attrs_const_emission() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'span[bitBadge]',
    template: '<ng-content></ng-content>',
    standalone: true,
})
export class BadgeComponent {}
";

    let result = transform_angular_file(&allocator, "badge.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The declarations_js should contain the attrs const
    // The const should look like: const _cN = ["bitBadge",""]
    // Note: no space after comma in the emitter output
    assert!(
        result.code.contains(r#"["bitBadge",""]"#),
        "Should emit attrs const with selector attribute. Got:\n{}",
        result.code
    );

    // The ɵcmp definition should reference the attrs const
    // Note: no space after colon in the emitter output
    assert!(
        result.code.contains("attrs:_c"),
        "Should reference attrs const in component definition. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("selector_attrs_const_emission", result.code);
}

// ============================================================================
// Multi-Component File Tests
// ============================================================================

/// Test that multiple components in the same file have unique constant names.
///
/// This test verifies the fix for the duplicate const declarations issue.
/// When a file contains multiple @Component classes, each should have unique
/// constant names (_c0, _c1, etc.) that don't conflict.
#[test]
fn test_multi_component_unique_const_names() {
    let allocator = Allocator::default();

    // Two components in the same file, each with @for loops that generate
    // pure functions (track expressions) which get pooled as constants.
    // This is a realistic scenario that triggers constant pool usage.
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-first',
    template: `
        @for (item of items; track item.id) {
            <div>{{ item.name }}</div>
        }
    `,
})
export class FirstComponent {
    items = [{id: 1, name: 'First'}];
}

@Component({
    selector: 'app-second',
    template: `
        @for (user of users; track user.id) {
            <span>{{ user.email }}</span>
        }
    `,
})
export class SecondComponent {
    users = [{id: 1, email: 'test@example.com'}];
}
";

    let result = transform_angular_file(&allocator, "multi.component.ts", source, None, None);

    assert_eq!(result.component_count, 2, "Should compile both components");
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Both components should generate track functions that are declared as constants
    // (const _forTrackN = ...). Check that all such declarations are unique.
    // Note: Angular's TypeScript translator uses 'const' for FINAL modifier.
    let track_var_pattern = "const _forTrack";
    let track_matches: Vec<_> = code.match_indices(track_var_pattern).collect();

    // Verify that all track variable names are unique
    let mut track_names = std::collections::HashSet::new();
    for (idx, _) in &track_matches {
        // Extract the full var name (e.g., "_forTrack0", "_forTrack1")
        let remaining = &code[*idx + 6..]; // Skip "const "
        let name_end = remaining.find(' ').unwrap_or(remaining.len());
        let var_name = &remaining[..name_end];
        assert!(
            track_names.insert(var_name.to_string()),
            "Duplicate track variable found: {var_name}. Full output:\n{code}"
        );
    }

    // Verify we have at least 2 unique track variables (one from each component)
    assert!(
        track_names.len() >= 2,
        "Expected at least 2 unique track variables, got {}. Variables: {:?}\nFull output:\n{}",
        track_names.len(),
        track_names,
        code
    );

    // Also check for function declarations with numbered suffixes
    // Both components generate template functions with unique names
    let template_func_pattern = "function FirstComponent_For_1_Template";
    let first_template_count = code.match_indices(template_func_pattern).count();
    assert_eq!(first_template_count, 1, "First component should have exactly 1 template function");

    let template_func_pattern2 = "function SecondComponent_For_1_Template";
    let second_template_count = code.match_indices(template_func_pattern2).count();
    assert_eq!(
        second_template_count, 1,
        "Second component should have exactly 1 template function"
    );

    // Also check for any _c variables (if any)
    // Note: Angular's JS emitter uses 'var' for all DeclareVarStmt.
    let var_decl_pattern = "var _c";
    let var_matches: Vec<_> = code.match_indices(var_decl_pattern).collect();

    // Verify that all var names are unique (if there are any)
    let mut var_names = std::collections::HashSet::new();
    for (idx, _) in &var_matches {
        let remaining = &code[*idx + 4..]; // Skip "var "
        let name_end = remaining.find(' ').unwrap_or(remaining.len());
        let var_name = &remaining[..name_end];
        assert!(
            var_names.insert(var_name.to_string()),
            "Duplicate var declaration found: {var_name}. Full output:\n{code}"
        );
    }
}

/// Test that host bindings and template compilation share constant pool indices.
///
/// This test verifies the fix for the duplicate const declarations issue where
/// host binding compilation would start with _c0 even if template compilation
/// had already used _c0, _c1, etc., causing "Identifier '_c0' has already been declared".
#[test]
fn test_host_bindings_share_constant_pool_with_template() {
    let allocator = Allocator::default();

    // Component with both a template that uses constants (via pure functions in @for)
    // AND host bindings that might generate constants
    let source = r"
import { Component, HostBinding, HostListener } from '@angular/core';

@Component({
    selector: '[appGrid]',
    template: `
        @for (item of items; track item.id) {
            <div>{{ item.name }}</div>
        }
    `,
    host: {
        '[class.active]': 'isActive',
        '[style.width.px]': 'width',
        '(click)': 'onClick($event)'
    }
})
export class GridComponent {
    items = [{id: 1, name: 'First'}];
    isActive = true;
    width = 100;
    onClick(event: Event) {}
}
";

    let result = transform_angular_file(&allocator, "grid.component.ts", source, None, None);

    assert_eq!(result.component_count, 1, "Should compile the component");
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Check for any _c constants
    let const_decl_pattern = "const _c";
    let const_matches: Vec<_> = code.match_indices(const_decl_pattern).collect();

    // Verify that all const names are unique (if there are any)
    let mut const_names = std::collections::HashSet::new();
    for (idx, _) in &const_matches {
        let remaining = &code[*idx + 6..]; // Skip "const "
        let name_end = remaining.find([' ', '='].as_ref()).unwrap_or(remaining.len());
        let const_name = &remaining[..name_end];
        assert!(
            const_names.insert(const_name.to_string()),
            "Duplicate const declaration found: {const_name}. Full output:\n{code}"
        );
    }

    // Check for track function constants too
    let track_const_pattern = "const _forTrack";
    let track_matches: Vec<_> = code.match_indices(track_const_pattern).collect();

    for (idx, _) in &track_matches {
        let remaining = &code[*idx + 6..]; // Skip "const "
        let name_end = remaining.find([' ', '='].as_ref()).unwrap_or(remaining.len());
        let const_name = &remaining[..name_end];
        assert!(
            const_names.insert(const_name.to_string()),
            "Duplicate track constant found: {const_name}. Full output:\n{code}"
        );
    }

    // Also verify there are no function declarations with the same name
    let func_pattern = "function ";
    let func_matches: Vec<_> = code.match_indices(func_pattern).collect();

    let mut func_names = std::collections::HashSet::new();
    for (idx, _) in &func_matches {
        let remaining = &code[*idx + 9..]; // Skip "function "
        let name_end = remaining.find('(').unwrap_or(remaining.len());
        let func_name = remaining[..name_end].trim();
        if !func_name.is_empty() {
            assert!(
                func_names.insert(func_name.to_string()),
                "Duplicate function declaration found: {func_name}. Full output:\n{code}"
            );
        }
    }
}

#[test]
fn test_component_with_export_as() {
    let allocator = Allocator::default();

    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-menu',
    template: '<div>Menu</div>',
    exportAs: 'menuComponent'
})
export class MenuComponent {}
";

    let result = transform_angular_file(&allocator, "menu.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Check that exportAs is emitted as an array
    // The emitter formats it as exportAs:["menuComponent"] (no spaces)
    assert!(
        code.contains(r#"exportAs:["menuComponent""#),
        "exportAs should be emitted as an array. Output:\n{code}"
    );
}

#[test]
fn test_component_with_multiple_export_as() {
    let allocator = Allocator::default();

    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-multi-menu',
    template: '<div>Multi Menu</div>',
    exportAs: 'menuComponent, menuAlias'
})
export class MultiMenuComponent {}
";

    let result = transform_angular_file(&allocator, "multi-menu.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Check that exportAs is emitted as an array with multiple values
    // The emitter formats it as exportAs:["menuComponent","menuAlias"] (no trailing comma)
    assert!(
        code.contains(r#"exportAs:["menuComponent","menuAlias""#),
        "exportAs should be emitted as an array with multiple values. Output:\n{code}"
    );
}

/// Test that two-way bindings don't duplicate event names in the consts array.
///
/// For `[(ngModel)]="value"`, the consts array should contain:
/// - `ngModelChange` (from the listener) once
/// - `ngModel` (from the property binding) once
///
/// This tests against the bug where `ngModelChange` appeared twice.
#[test]
fn test_two_way_binding_no_duplicate_in_consts() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<input [disabled]="isDisabled" [(ngModel)]="value">',
    standalone: true,
})
export class TestComponent {
    isDisabled = false;
    value = '';
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    println!("Generated code:\n{code}");

    // Count occurrences of "ngModelChange" in the code
    let ngmodel_change_count = code.matches("\"ngModelChange\"").count();

    // ngModelChange should appear exactly once in the consts array bindings
    // It may also appear once in the ɵɵtwoWayListener call, so we check for at most 2
    // But more importantly, it should NOT appear twice in the consts array
    assert!(
        ngmodel_change_count <= 2,
        "Expected at most 2 occurrences of 'ngModelChange' (once in consts, once in listener), got {ngmodel_change_count}.\nOutput:\n{code}"
    );

    // Check that the consts array doesn't have duplicate entries
    // Look for the pattern: 3,"ngModelChange","ngModelChange"
    // (where 3 is the Bindings marker)
    let has_duplicate = code.contains(r#""ngModelChange","ngModelChange""#);
    assert!(
        !has_duplicate,
        "The consts array should NOT have duplicate 'ngModelChange' entries.\nOutput:\n{code}"
    );
}

/// Test that duplicate property bindings are deduplicated in the consts array.
///
/// If the same binding name appears multiple times (which can happen with
/// certain edge cases), it should only appear once in the consts array.
/// This is ported from Angular's `isKnown` deduplication logic in const_collection.ts.
#[test]
fn test_duplicate_binding_deduplication() {
    let allocator = Allocator::default();
    // This test uses a click event and disabled property to verify deduplication.
    // If somehow the same binding name got extracted twice, it should still only appear once.
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<button (click)="onClick()" [disabled]="isDisabled">Click</button>',
    standalone: true,
})
export class TestComponent {
    isDisabled = false;
    onClick() {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Count occurrences of "click" and "disabled"
    let click_count = code.matches("\"click\"").count();
    let disabled_count = code.matches("\"disabled\"").count();

    // Each should appear exactly once in the consts array
    // "click" appears once in consts and once in listener call
    // "disabled" appears once in consts and potentially in the property call
    assert!(
        click_count <= 2,
        "Expected at most 2 occurrences of 'click', got {click_count}.\nOutput:\n{code}"
    );
    assert!(
        disabled_count <= 2,
        "Expected at most 2 occurrences of 'disabled', got {disabled_count}.\nOutput:\n{code}"
    );

    // Verify no duplicates in the consts array
    assert!(
        !code.contains(r#""click","click""#),
        "Should not have duplicate 'click' in consts.\nOutput:\n{code}"
    );
    assert!(
        !code.contains(r#""disabled","disabled""#),
        "Should not have duplicate 'disabled' in consts.\nOutput:\n{code}"
    );
}

// ============================================================================
// Animation Tests
// ============================================================================

#[test]
fn test_animate_enter_instruction() {
    // Tests that animate.enter attribute generates ɵɵanimateEnter instruction
    // Example: <p animate.enter="slide">Content</p>
    // Should output: i0.ɵɵanimateEnter("slide")
    let js = compile_template_to_js(r#"<p animate.enter="slide">Content</p>"#, "TestComponent");

    // Should have ɵɵanimateEnter instruction with the value
    assert!(
        js.contains(r#"i0.ɵɵanimateEnter("slide")"#),
        "Expected ɵɵanimateEnter instruction.\nGot:\n{js}"
    );

    // Should NOT use syntheticHostProperty
    assert!(
        !js.contains("syntheticHostProperty"),
        "Should not use syntheticHostProperty for animate.enter.\nGot:\n{js}"
    );
}

#[test]
fn test_animate_leave_instruction() {
    // Tests that animate.leave attribute generates ɵɵanimateLeave instruction
    // Example: <p animate.leave="fade">Content</p>
    // Should output: i0.ɵɵanimateLeave("fade")
    let js = compile_template_to_js(r#"<p animate.leave="fade">Content</p>"#, "TestComponent");

    // Should have ɵɵanimateLeave instruction with the value
    assert!(
        js.contains(r#"i0.ɵɵanimateLeave("fade")"#),
        "Expected ɵɵanimateLeave instruction.\nGot:\n{js}"
    );

    // Should NOT use syntheticHostProperty
    assert!(
        !js.contains("syntheticHostProperty"),
        "Should not use syntheticHostProperty for animate.leave.\nGot:\n{js}"
    );
}

#[test]
fn test_animate_enter_and_leave_together() {
    // Tests that both animate.enter and animate.leave work on the same element
    let js = compile_template_to_js(
        r#"<div animate.enter="fadeIn" animate.leave="fadeOut">Content</div>"#,
        "TestComponent",
    );

    // Should have both instructions
    assert!(
        js.contains(r#"i0.ɵɵanimateEnter("fadeIn")"#),
        "Expected ɵɵanimateEnter instruction.\nGot:\n{js}"
    );
    assert!(
        js.contains(r#"i0.ɵɵanimateLeave("fadeOut")"#),
        "Expected ɵɵanimateLeave instruction.\nGot:\n{js}"
    );
}

#[test]
fn test_host_animation_trigger_binding() {
    // Component with animation trigger in host property should emit ɵɵsyntheticHostProperty
    let source = r"
import { Component } from '@angular/core';
import { trigger, transition, style, animate } from '@angular/animations';

@Component({
    selector: 'app-slide',
    template: '<ng-content></ng-content>',
    animations: [trigger('slideIn', [transition(':enter', [style({ width: 0 }), animate('200ms')])])],
    host: {
        '[@slideIn]': 'animationState',
    }
})
export class SlideComponent {
    animationState = 'active';
}
";
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "slide.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Should have ɵɵsyntheticHostProperty in the hostBindings update block
    assert!(
        code.contains("syntheticHostProperty"),
        "Expected ɵɵsyntheticHostProperty for host animation trigger.\nGot:\n{code}"
    );
    assert!(
        code.contains(r#"syntheticHostProperty("@slideIn""#),
        "Expected syntheticHostProperty with @slideIn name.\nGot:\n{code}"
    );

    // Should NOT have ɵɵanimateEnter/ɵɵanimateLeave for [@trigger] bindings
    assert!(
        !code.contains("animateEnter") && !code.contains("animateLeave"),
        "Host [@trigger] bindings should not use animateEnter/animateLeave.\nGot:\n{code}"
    );
}

#[test]
fn test_directive_host_animation_trigger_binding() {
    // Directive with animation trigger in host property should emit ɵɵsyntheticHostProperty
    let source = r"
import { Directive } from '@angular/core';
import { trigger, transition, style, animate } from '@angular/animations';

@Directive({
    selector: '[appSlide]',
    host: {
        '[@slideIn]': 'animationState',
    }
})
export class SlideDirective {
    animationState = 'active';
}
";
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "slide.directive.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Should have ɵɵsyntheticHostProperty in the hostBindings update block
    assert!(
        code.contains(r#"syntheticHostProperty("@slideIn""#),
        "Expected syntheticHostProperty with @slideIn name for directive.\nGot:\n{code}"
    );

    // Should NOT use regular hostProperty for animation triggers
    assert!(
        !code.contains(r#"hostProperty("@slideIn""#),
        "Should not use hostProperty for animation triggers.\nGot:\n{code}"
    );
}

/// Test that multiple components with host bindings in the same file have unique constant names.
///
/// This test simulates the real-world scenario from Material Angular's fab.ts where
/// two components (MatFabButton and MatMiniFabButton) each have host bindings.
/// Each component's host bindings use the constant pool for hostAttrs, and the pool
/// indices must be shared across all components in the file to avoid duplicate
/// const declarations like "_c0" appearing twice.
#[test]
fn test_multi_component_host_bindings_unique_const_names() {
    let allocator = Allocator::default();

    // Two components in the same file, each with host bindings that generate constants.
    // This is similar to Material Angular's fab.ts with MatFabButton and MatMiniFabButton.
    let source = r"
import { Component, Input } from '@angular/core';

@Component({
    selector: 'button[mat-fab]',
    template: '<ng-content></ng-content>',
    host: {
        'class': 'mdc-fab mat-mdc-fab',
        '[class.mdc-fab--extended]': 'extended',
        '[class.mat-mdc-extended-fab]': 'extended',
    },
})
export class MatFabButton {
    @Input() extended: boolean = false;
}

@Component({
    selector: 'button[mat-mini-fab]',
    template: '<ng-content></ng-content>',
    host: {
        'class': 'mdc-fab mat-mdc-mini-fab',
    },
})
export class MatMiniFabButton {}
";

    let result = transform_angular_file(&allocator, "fab.ts", source, None, None);

    assert_eq!(result.component_count, 2, "Should compile both components");
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Collect all const/var declarations (_c0, _c1, etc.)
    // The emitter uses 'var' for these declarations
    let const_decl_pattern = "var _c";
    let const_matches: Vec<_> = code.match_indices(const_decl_pattern).collect();

    // Verify that all const names are unique
    let mut const_names = std::collections::HashSet::new();
    for (idx, _) in &const_matches {
        let remaining = &code[*idx + 4..]; // Skip "var "
        let name_end = remaining.find([' ', '='].as_ref()).unwrap_or(remaining.len());
        let const_name = &remaining[..name_end];
        assert!(
            const_names.insert(const_name.to_string()),
            "Duplicate const declaration found: {const_name}. Full output:\n{code}"
        );
    }

    // Both components should have attrs referencing _c constants (from selector)
    let attrs_pattern = "attrs:_c";
    let attrs_count = code.matches(attrs_pattern).count();
    assert!(
        attrs_count >= 2,
        "Expected at least 2 attrs references (one per component), found {attrs_count}. Output:\n{code}"
    );
}

/// Test that multiple components sharing the same template in the same file
/// have unique constant names.
///
/// This test reproduces the exact scenario from Material Angular's fab.ts where
/// both MatFabButton and MatMiniFabButton use templateUrl: 'button.html'.
/// The template generates multiple constants (for ng-content selectors, etc.),
/// and these must be unique across both components.
#[test]
fn test_multi_component_shared_template_unique_const_names() {
    let allocator = Allocator::default();

    // Simulate the button.html template that has:
    // - Class bindings (pure functions)
    // - Multiple ng-content selectors
    // This is the template used by MatFabButton and MatMiniFabButton
    let template = r#"
        <span
            class="mat-mdc-button-persistent-ripple"
            [class.mdc-button__ripple]="!_isFab"
            [class.mdc-fab__ripple]="_isFab">
        </span>
        <ng-content select=".material-icons:not([iconPositionEnd])"></ng-content>
        <span class="mdc-button__label"><ng-content></ng-content></span>
        <ng-content select=".material-icons[iconPositionEnd]"></ng-content>
    "#;

    // Two components using the same template (simulating templateUrl resolution)
    let source = format!(
        r"
import {{ Component, Input }} from '@angular/core';

@Component({{
    selector: 'button[mat-fab]',
    template: `{template}`,
    host: {{
        'class': 'mdc-fab mat-mdc-fab',
        '[class.mdc-fab--extended]': 'extended',
        '[class.mat-mdc-extended-fab]': 'extended',
    }},
}})
export class MatFabButton {{
    @Input() extended: boolean = false;
    _isFab = true;
}}

@Component({{
    selector: 'button[mat-mini-fab]',
    template: `{template}`,
    host: {{
        'class': 'mdc-fab mat-mdc-mini-fab',
    }},
}})
export class MatMiniFabButton {{
    _isFab = true;
}}
"
    );

    let result = transform_angular_file(&allocator, "fab.ts", &source, None, None);

    assert_eq!(result.component_count, 2, "Should compile both components");
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Collect all const/var declarations (_c0, _c1, etc.)
    let const_decl_pattern = "var _c";
    let const_matches: Vec<_> = code.match_indices(const_decl_pattern).collect();

    // Verify that all const names are unique
    let mut const_names = std::collections::HashSet::new();
    for (idx, _) in &const_matches {
        let remaining = &code[*idx + 4..]; // Skip "var "
        let name_end = remaining.find([' ', '='].as_ref()).unwrap_or(remaining.len());
        let const_name = &remaining[..name_end];
        assert!(
            const_names.insert(const_name.to_string()),
            "Duplicate const declaration found: {const_name}. Full output:\n{code}"
        );
    }

    // Should have multiple ng-content selectors pooled (at least one per template invocation)
    // Each component has 3 ng-content elements, so we expect multiple selector constants
    let ngcontent_pattern = "ngContentSelectors:_c";
    let ngcontent_count = code.matches(ngcontent_pattern).count();
    assert!(
        ngcontent_count >= 2,
        "Expected at least 2 ngContentSelectors references (one per component), found {ngcontent_count}. Output:\n{code}"
    );
}

/// Test that multiple components with external templates (templateUrl) have unique constant names.
///
/// This test simulates Material Angular's fab.ts where both MatFabButton and MatMiniFabButton
/// use templateUrl: 'button.html' - the same external template file.
#[test]
fn test_multi_component_external_template_unique_const_names() {
    let allocator = Allocator::default();

    // The actual button.html template from Material Angular
    let button_template = r#"
<span
    class="mat-mdc-button-persistent-ripple"
    [class.mdc-button__ripple]="!_isFab"
    [class.mdc-fab__ripple]="_isFab"></span>

<ng-content select=".material-icons:not([iconPositionEnd]), mat-icon:not([iconPositionEnd]), [matButtonIcon]:not([iconPositionEnd])">
</ng-content>

<span class="mdc-button__label"><ng-content></ng-content></span>

<ng-content select=".material-icons[iconPositionEnd], mat-icon[iconPositionEnd], [matButtonIcon][iconPositionEnd]">
</ng-content>

<span class="mat-focus-indicator"></span>
<span class="mat-mdc-button-touch-target"></span>
"#;

    // The fab.ts source with templateUrl references
    let source = r"
import { Component, Input, ViewEncapsulation, ChangeDetectionStrategy } from '@angular/core';

@Component({
    selector: 'button[mat-fab], a[mat-fab]',
    templateUrl: 'button.html',
    styleUrl: 'fab.css',
    host: {
        'class': 'mdc-fab mat-mdc-fab-base mat-mdc-fab',
        '[class.mdc-fab--extended]': 'extended',
        '[class.mat-mdc-extended-fab]': 'extended',
    },
    encapsulation: ViewEncapsulation.None,
    changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MatFabButton {
    @Input() extended: boolean = false;
    _isFab = true;
}

@Component({
    selector: 'button[mat-mini-fab], a[mat-mini-fab]',
    templateUrl: 'button.html',
    styleUrl: 'fab.css',
    host: {
        'class': 'mdc-fab mat-mdc-fab-base mdc-fab--mini mat-mdc-mini-fab',
    },
    encapsulation: ViewEncapsulation.None,
    changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MatMiniFabButton {
    _isFab = true;
}
";

    // Create resolved resources with the external template
    let mut templates = std::collections::HashMap::new();
    templates.insert("button.html".to_string(), button_template.to_string());

    let resources = ResolvedResources { templates, styles: std::collections::HashMap::new() };

    let result = transform_angular_file(&allocator, "fab.ts", source, None, Some(&resources));

    assert_eq!(result.component_count, 2, "Should compile both components");
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Collect all const/var declarations (_c0, _c1, etc.)
    let const_decl_pattern = "var _c";
    let const_matches: Vec<_> = code.match_indices(const_decl_pattern).collect();

    // Verify that all const names are unique
    let mut const_names = std::collections::HashSet::new();
    let mut duplicates = Vec::new();
    for (idx, _) in &const_matches {
        let remaining = &code[*idx + 4..]; // Skip "var "
        let name_end = remaining.find([' ', '='].as_ref()).unwrap_or(remaining.len());
        let const_name = &remaining[..name_end].to_string();
        if !const_names.insert(const_name.clone()) {
            duplicates.push(const_name.clone());
        }
    }

    assert!(
        duplicates.is_empty(),
        "Duplicate const declarations found: {duplicates:?}. Full output:\n{code}"
    );

    // Each component should have its own constants for attrs and ngContentSelectors
    let attrs_count = code.matches("attrs:_c").count();
    assert!(
        attrs_count >= 2,
        "Expected at least 2 attrs references (one per component), found {attrs_count}"
    );
}

/// Test three components in the same file to ensure constant pool index is correctly
/// shared across all of them.
///
/// This test simulates Material Angular's drawer.ts which has three components:
/// MatDrawerContent, MatDrawer, and MatDrawerContainer.
#[test]
fn test_three_components_unique_const_names() {
    let allocator = Allocator::default();

    // Three components with host bindings that generate constants
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'mat-drawer-content',
    template: '<ng-content></ng-content>',
    host: {
        'class': 'mat-drawer-content',
        '[style.margin-left.px]': 'marginLeft',
    },
})
export class MatDrawerContent {
    marginLeft = 0;
}

@Component({
    selector: 'mat-drawer',
    template: '<ng-content></ng-content>',
    host: {
        'class': 'mat-drawer',
        '[class.mat-drawer-opened]': 'opened',
    },
})
export class MatDrawer {
    opened = false;
}

@Component({
    selector: 'mat-drawer-container',
    template: '<ng-content></ng-content>',
    host: {
        'class': 'mat-drawer-container',
        '[class.mat-drawer-container-explicit-backdrop]': 'hasBackdrop',
    },
})
export class MatDrawerContainer {
    hasBackdrop = false;
}
";

    let result = transform_angular_file(&allocator, "drawer.ts", source, None, None);

    assert_eq!(result.component_count, 3, "Should compile all three components");
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Collect all const/var declarations (_c0, _c1, etc.)
    let const_decl_pattern = "var _c";
    let const_matches: Vec<_> = code.match_indices(const_decl_pattern).collect();

    // Verify that all const names are unique
    let mut const_names = std::collections::HashSet::new();
    let mut duplicates = Vec::new();
    for (idx, _) in &const_matches {
        let remaining = &code[*idx + 4..]; // Skip "var "
        let name_end = remaining.find([' ', '='].as_ref()).unwrap_or(remaining.len());
        let const_name = &remaining[..name_end].to_string();
        if !const_names.insert(const_name.clone()) {
            duplicates.push(const_name.clone());
        }
    }

    assert!(
        duplicates.is_empty(),
        "Duplicate const declarations found: {duplicates:?}. Full output:\n{code}"
    );

    // All three components should have ngContentSelectors constants
    let ngcontent_count = code.matches("ngContentSelectors:_c").count();
    assert!(
        ngcontent_count >= 3,
        "Expected at least 3 ngContentSelectors references (one per component), found {ngcontent_count}. Output:\n{code}"
    );
}

// ============================================================================
// Import Elision Tests
// ============================================================================

/// Test that @Input and @Output decorators are elided from imports.
///
/// Angular compiles these decorators into class metadata, so the decorator
/// imports become unused and should be removed.
#[test]
fn test_import_elision_input_output_decorators() {
    let allocator = Allocator::default();

    let source = r"
import { Component, Input, Output, EventEmitter } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<div>{{name}}</div>',
})
export class TestComponent {
    @Input() name: string = '';
    @Input('itemCount') count: number = 0;
    @Output() clicked = new EventEmitter<void>();
    @Output('valueChanged') changed = new EventEmitter<string>();

    onClick() {
        this.clicked.emit();
        this.changed.emit('clicked');
    }
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Angular keeps member decorator imports (Input, Output) for setClassMetadata
    // They are used by Angular DevTools and TestBed.overrideComponent
    // NOTE: OXC doesn't emit setClassMetadata, but we match Angular's import behavior
    let import_line =
        code.lines().find(|l| l.starts_with("import") && l.contains("@angular/core")).unwrap();

    assert!(
        import_line.contains("Input"),
        "Input should be preserved in imports (Angular keeps it for setClassMetadata). Import line: {import_line}"
    );
    assert!(
        import_line.contains("Output"),
        "Output should be preserved in imports (Angular keeps it for setClassMetadata). Import line: {import_line}"
    );

    // Component and EventEmitter should also be preserved
    // Component is a class decorator (kept at runtime)
    // EventEmitter is used in expressions (new EventEmitter())
    assert!(
        import_line.contains("Component"),
        "Component should be preserved. Import line: {import_line}"
    );
    assert!(
        import_line.contains("EventEmitter"),
        "EventEmitter should be preserved. Import line: {import_line}"
    );
}

/// Test that @ViewChild and @ContentChild decorators are preserved in imports.
/// Angular keeps member decorator imports for setClassMetadata (used by DevTools and TestBed).
#[test]
fn test_import_elision_query_decorators() {
    let allocator = Allocator::default();

    let source = r"
import { Component, ViewChild, ViewChildren, ContentChild, ContentChildren, QueryList, ElementRef, TemplateRef } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<div #myDiv><ng-template #myTpl></ng-template></div>',
})
export class TestComponent {
    @ViewChild('myDiv') div: ElementRef;
    @ViewChild('myTpl') template: TemplateRef<any>;
    @ViewChildren('items') items: QueryList<ElementRef>;
    @ContentChild('content') content: ElementRef;
    @ContentChildren('tabs') tabs: QueryList<ElementRef>;
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    let import_line =
        code.lines().find(|l| l.starts_with("import") && l.contains("@angular/core")).unwrap();

    // Query decorators should be preserved (Angular keeps them for setClassMetadata)
    for decorator in ["ViewChild", "ViewChildren", "ContentChild", "ContentChildren"] {
        assert!(
            import_line.contains(decorator),
            "{decorator} should be preserved in imports (Angular keeps for setClassMetadata). Import line: {import_line}"
        );
    }

    // Type-only imports should be elided
    // QueryList, ElementRef, TemplateRef are only used in type annotations
    for type_import in ["QueryList", "ElementRef", "TemplateRef"] {
        assert!(
            !import_line.contains(type_import),
            "{type_import} should be elided from imports (type-only). Import line: {import_line}"
        );
    }

    // Component should be preserved
    assert!(
        import_line.contains("Component"),
        "Component should be preserved. Import line: {import_line}"
    );
}

/// Test that constructor parameter decorators (@Optional, @Inject, etc.) are elided from imports.
/// Angular removes these decorators during compilation and encodes them in factory metadata.
#[test]
fn test_import_elision_ctor_param_decorators() {
    let allocator = Allocator::default();

    let source = r#"
import { Component, ElementRef, Optional, Inject } from "@angular/core";
import { DOCUMENT } from "@angular/common";
import { FormControlComponent } from "./form-control";

@Component({
    selector: 'bit-label',
    template: '<label></label>',
})
export class BitLabelComponent {
    constructor(
        private elementRef: ElementRef<HTMLInputElement>,
        @Optional() private parentFormControl: FormControlComponent,
        @Inject(DOCUMENT) private document: Document,
    ) {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    println!("=== OUTPUT ===\n{code}\n=== END ===");

    // Find the @angular/core import line
    let import_line = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    println!("Angular core import line: {import_line}");

    // Optional should be elided (only used as ctor param decorator)
    assert!(
        !import_line.contains("Optional"),
        "Optional should be elided from imports. Import line: {import_line}"
    );

    // Inject should be elided (only used as ctor param decorator)
    assert!(
        !import_line.contains("Inject"),
        "Inject should be elided from imports. Import line: {import_line}"
    );

    // ElementRef should be elided (only used in type annotation, DI comes from namespace)
    assert!(
        !import_line.contains("ElementRef"),
        "ElementRef should be elided from imports. Import line: {import_line}"
    );

    // Component should be preserved (class decorator)
    assert!(
        import_line.contains("Component"),
        "Component should be in imports. Import line: {import_line}"
    );

    // Find the @angular/common import line (if it exists)
    let common_import =
        code.lines().find(|l| l.starts_with("import") && l.contains("@angular/common"));

    // DOCUMENT should be elided (only used in @Inject argument)
    // If the import line exists, it should not contain DOCUMENT
    // Or the entire import should be removed
    if let Some(common_line) = common_import {
        assert!(
            !common_line.contains("DOCUMENT"),
            "DOCUMENT should be elided from imports. Import line: {common_line}"
        );
    }
}

/// Test that @HostBinding and @HostListener decorators are elided from imports.
#[test]
fn test_import_elision_host_decorators() {
    let allocator = Allocator::default();

    let source = r"
import { Component, HostBinding, HostListener } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<div>Test</div>',
})
export class TestComponent {
    @HostBinding('class.active') isActive = false;
    @HostBinding('attr.role') role = 'button';

    @HostListener('click', ['$event'])
    onClick(event: Event) {}

    @HostListener('keydown.enter')
    onEnter() {}
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // HostBinding and HostListener should be preserved in the import statement
    // Angular keeps member decorator imports for setClassMetadata (used by DevTools and TestBed)
    let import_line = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    assert!(
        import_line.contains("HostBinding"),
        "HostBinding should be preserved in imports (Angular keeps for setClassMetadata). Import line: {import_line}"
    );
    assert!(
        import_line.contains("HostListener"),
        "HostListener should be preserved in imports (Angular keeps for setClassMetadata). Import line: {import_line}"
    );

    // Component should be preserved
    assert!(
        import_line.contains("Component"),
        "Component should be preserved. Import line: {import_line}"
    );
}

// ============================================================================
// SVG Namespace Tests
// ============================================================================

/// Test that SVG elements inside @switch/@case inside @for have the correct namespace prefix.
///
/// This is a regression test for a bug where conditionalCreate was emitting "svg" instead
/// of ":svg:svg" for the tag parameter when the SVG element was inside a @switch/@case
/// that was nested inside a @for loop.
///
/// Expected: `conditionalCreate(0, ..., 2, 0, ":svg:svg", 0)`
/// Bug:      `conditionalCreate(0, ..., 2, 0, "svg", 0)`
#[test]
fn test_svg_namespace_in_switch_case_inside_for() {
    // Test with minimal whitespace - SVG is the ONLY child of @case block
    // When SVG is the single root element, the tag should include namespace prefix ":svg:svg"
    let js = compile_template_to_js(
        r#"@for (item of items; track item.id) {@switch (item.type) {@case ('circle') {<svg viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" /></svg>}@case ('rect') {<svg viewBox="0 0 100 100"><rect width="80" height="80" /></svg>}}}"#,
        "TestComponent",
    );

    // Verify that conditionalCreate uses ":svg:svg" not just "svg"
    assert!(
        js.contains(r#"":svg:svg""#),
        "conditionalCreate should use ':svg:svg' namespace prefix for SVG elements. Output:\n{js}"
    );
}

/// Test SVG namespace using the NAPI path (with whitespace removal).
/// This matches the actual NAPI binding behavior.
#[test]
fn test_svg_namespace_in_switch_case_inside_for_napi_path() {
    use oxc_angular_compiler::compile_template_to_js_with_options;

    let allocator = Allocator::default();
    let template = r#"
      @for (item of items; track item.id) {
        @switch (item.type) {
          @case ('circle') {
            <svg viewBox="0 0 100 100">
              <circle cx="50" cy="50" r="40" />
            </svg>
          }
          @case ('rect') {
            <svg viewBox="0 0 100 100">
              <rect width="80" height="80" />
            </svg>
          }
        }
      }
    "#;

    let options = ComponentTransformOptions::default();
    let result = compile_template_to_js_with_options(
        &allocator,
        template,
        "TestComponent",
        "test.ts",
        &options,
    );

    match result {
        Ok(output) => {
            // Verify that conditionalCreate uses ":svg:svg" not just "svg"
            assert!(
                output.code.contains(r#"":svg:svg""#),
                "NAPI path: conditionalCreate should use ':svg:svg' namespace prefix. Output:\n{}",
                output.code
            );
        }
        Err(e) => {
            panic!("Compilation failed: {e:?}");
        }
    }
}

/// Test SVG namespace using the NAPI path with DomOnly mode enabled.
/// This is the exact configuration used by the fixture tests for standalone components.
#[test]
fn test_svg_namespace_in_switch_case_inside_for_domonly_mode() {
    use oxc_angular_compiler::compile_template_to_js_with_options;

    let allocator = Allocator::default();
    let template = r#"
      @for (item of items; track item.id) {
        @switch (item.type) {
          @case ('circle') {
            <svg viewBox="0 0 100 100">
              <circle cx="50" cy="50" r="40" />
            </svg>
          }
          @case ('rect') {
            <svg viewBox="0 0 100 100">
              <rect width="80" height="80" />
            </svg>
          }
        }
      }
    "#;

    let options = ComponentTransformOptions::default();
    let result = compile_template_to_js_with_options(
        &allocator,
        template,
        "TestComponent",
        "test.ts",
        &options,
    );

    match result {
        Ok(output) => {
            // Verify that conditionalCreate uses ":svg:svg" not just "svg"
            assert!(
                output.code.contains(r#"":svg:svg""#),
                "DomOnly mode: conditionalCreate should use ':svg:svg' namespace prefix. Output:\n{}",
                output.code
            );
        }
        Err(e) => {
            panic!("Compilation failed: {e:?}");
        }
    }
}

/// Test SVG namespace using transform_angular_file (full file transformation).
/// This mimics exactly what the fixture tests do.
#[test]
fn test_svg_namespace_in_switch_case_using_transform_angular_file() {
    let allocator = Allocator::default();

    // This is the exact source code generated by the fixture's generateComponentSource
    let source = r#"
import { Component } from '@angular/core';

@Component({
  selector: 'app-svginswitchcase',
  standalone: true,
  template: `
      @for (item of items; track item.id) {
        @switch (item.type) {
          @case ('circle') {
            <svg viewBox="0 0 100 100">
              <circle cx="50" cy="50" r="40" />
            </svg>
          }
          @case ('rect') {
            <svg viewBox="0 0 100 100">
              <rect width="80" height="80" />
            </svg>
          }
        }
      }
    `
})
export class SvgInSwitchCaseComponent {}
"#;

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // Verify that conditionalCreate uses ":svg:svg" not just "svg"
    assert!(
        result.code.contains(r#"":svg:svg""#),
        "transform_angular_file: conditionalCreate should use ':svg:svg' namespace prefix. Output:\n{}",
        result.code
    );
}

#[test]
fn test_svg_in_switch_case_with_whitespace() {
    // Test with whitespace around SVG - multiple children means tag inference returns null
    // This matches TypeScript's behavior where whitespace text nodes prevent tag inference.
    // Angular documentation suggests this is by design - single root element pattern works
    // best without additional content.
    let js = compile_template_to_js(
        r#"@for (item of items; track item.id) {
  @switch (item.type) {
    @case ('circle') {
      <svg viewBox="0 0 100 100">
        <circle cx="50" cy="50" r="40" />
      </svg>
    }
  }
}"#,
        "TestComponent",
    );

    // When whitespace is present, tag inference returns null and is omitted.
    // The SVG element should still render correctly with namespaceSVG() call.
    assert!(
        js.contains("ɵɵnamespaceSVG()"),
        "SVG element should have namespaceSVG() call. Output:\n{js}"
    );
    assert!(
        js.contains(r#""svg""#),
        "SVG element should be rendered with tag name 'svg'. Output:\n{js}"
    );
}

#[test]
fn test_control_binding_attribute_extraction() {
    // Test that [formField] (control binding) is extracted into the consts array.
    // Before the fix, UpdateOp::Control was not handled in attribute extraction,
    // causing the control binding name ("formField") to be missing from the element's
    // extracted attributes. This resulted in duplicate/shifted const entries.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<cu-comp [formField]="myField" [open]="isOpen"></cu-comp>',
    standalone: true,
})
export class TestComponent {
    myField = 'test';
    isOpen = false;
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    eprintln!("OUTPUT:\n{}", result.code);

    // The consts array should contain "formField" as an extracted property binding name.
    // Without the fix, only "open" would appear (missing "formField"), resulting in
    // incorrect const array entries and shifted indices.
    assert!(
        result.code.contains(r#""formField""#),
        "Consts should contain 'formField' from control binding extraction. Output:\n{}",
        result.code
    );

    // Both "formField" and "open" should appear in the same consts entry (same element).
    // The property marker (3) should precede both names.
    // Expected: [3, "formField", "open"] (property marker followed by both binding names)
    // Without the fix: [3, "open"] (missing "formField")
    let has_both_in_same_const = result
        .code
        .lines()
        .any(|line| line.contains(r#""formField""#) && line.contains(r#""open""#));
    assert!(
        has_both_in_same_const,
        "Both 'formField' and 'open' should appear in the same consts entry. Output:\n{}",
        result.code
    );
}

#[test]
fn test_form_field_emits_property_and_zero_arg_control() {
    let js = compile_template_to_js(r#"<input [formField]="myField">"#, "TestComponent");

    assert!(
        js.contains(r#"ɵɵproperty("formField""#),
        "[formField] should emit a regular ɵɵproperty(\"formField\", ...). Got:\n{js}"
    );

    assert!(
        js.contains("ɵɵcontrol();"),
        "[formField] should emit zero-arg ɵɵcontrol(). Got:\n{js}"
    );

    assert!(
        !js.contains(r#"ɵɵcontrol(ctx.myField,"formField")"#),
        "[formField] should not emit legacy ɵɵcontrol(value, \"formField\"). Got:\n{js}"
    );
}

#[test]
fn test_form_field_maintains_mixed_property_order() {
    let js = compile_template_to_js(
        r#"<input type="radio" [formField]="value" [value]="'foo'" id="radio" /><input type="radio" [value]="'foo'" [formField]="value" id="radio" />"#,
        "TestComponent",
    );

    let compact: String = js.chars().filter(|c| !c.is_whitespace()).collect();
    let first_binding = r#"i0.ɵɵproperty("formField",ctx.value)("value","foo");i0.ɵɵcontrol();"#;
    let second_binding = r#"i0.ɵɵproperty("value","foo")("formField",ctx.value);i0.ɵɵcontrol();"#;

    assert!(
        compact.contains(first_binding),
        "Expected first radio input to keep [formField] before [value]. Got:\n{js}"
    );
    assert!(
        compact.contains(second_binding),
        "Expected second radio input to keep [value] before [formField]. Got:\n{js}"
    );
    assert!(
        compact.find(first_binding) < compact.find(second_binding),
        "Expected first radio binding sequence to appear before the second. Got:\n{js}"
    );
}

#[test]
fn test_form_field_extracted_consts_preserve_binding_order() {
    let allocator = Allocator::default();
    let source = r#"
import { Component, Directive, input } from '@angular/core';

@Directive({ selector: '[formField]' })
export class FormField {
    readonly formField = input<string>();
}

@Component({
    selector: 'test-comp',
    template: `
      <input
          type="radio"
          [formField]="value"
          [value]="'foo'"
          id="radio"
        />

        <input
          type="radio"
          [value]="'foo'"
          [formField]="value"
          id="radio"
        />
    `,
    imports: [FormField],
    standalone: true,
})
export class TestComponent {
    value = 'foo';
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let compact: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains(
            r#"consts:[["type","radio","id","radio",3,"formField","value"],["type","radio","id","radio",3,"value","formField"]]"#
        ),
        "Extracted const bindings should preserve per-element source order. Output:\n{}",
        result.code
    );
}

#[test]
fn test_form_field_does_not_inflate_vars_count() {
    let allocator = Allocator::default();
    let source = r#"
import { Component, Directive, input } from '@angular/core';

@Directive({ selector: '[formField]' })
export class FormField {
    readonly formField = input<string>();
}

@Component({
    selector: 'test-comp',
    template: `
      <input
          type="radio"
          [formField]="value"
          [value]="'foo'"
          id="radio"
        />

        <input
          type="radio"
          [value]="'foo'"
          [formField]="value"
          id="radio"
        />
    `,
    imports: [FormField],
    standalone: true,
})
export class TestComponent {
    value = 'foo';
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let compact: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains("decls:2,vars:4,consts:"),
        "[formField] should not inflate vars beyond Angular's control fixture count. Output:\n{}",
        result.code
    );
}

// ============================================================================
// Pipe Slot Propagation Through Control Ops Tests
// ============================================================================

/// Test that pipe slot index is correctly propagated through UpdateOp::Control.
///
/// When a pipe binding appears inside a control binding (e.g., `[field]="expr | async"`),
/// the pipe slot should be the pipe's actual allocated slot, NOT 0.
///
/// Before the fix, `propagate_slots_to_expressions` had a `_ => {}` catch-all that
/// silently skipped `UpdateOp::Control`, so PipeBindingExpr inside Control ops never
/// got their `target_slot.slot` updated from `None` to the correct `SlotId`.
/// During reify, `pipe.target_slot.slot.map_or(0, |s| s.0)` fell back to 0.
#[test]
fn test_pipe_slot_in_control_binding_exact_slot() {
    // Simple case: element with [field] control binding using a pipe.
    // Element is at slot 0, pipe is at slot 1.
    // The pipeBind1 call should reference slot 1, not slot 0.
    let js = compile_template_to_js(
        r#"<cu-comp [formField]="myField$ | async"></cu-comp>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Verify pipe declaration exists
    assert!(js.contains("ɵɵpipe("), "Should have pipe declaration. Output:\n{js}");

    // The pipe should use slot 1 (element at slot 0, pipe at slot 1)
    assert!(
        js.contains("pipeBind1(1,"),
        "pipeBind should use slot 1 (pipe slot), not slot 0 (element slot). Output:\n{js}"
    );
}

/// Test pipe slot for [field] control binding with safe navigation.
///
/// This reproduces the slot=0 bug where PipeBinding inside ControlOp
/// doesn't get its slot propagated properly.
#[test]
fn test_pipe_in_field_binding_with_safe_nav() {
    let js = compile_template_to_js(
        r#"<cu-comp [formField]="(settings$ | async)?.workload?.field" [title]="name | uppercase"></cu-comp>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");
    // The pipe inside [field] should NOT have slot 0 (element slot)
    // It should have its own pipe slot (1 or later)
    assert!(
        !js.contains("pipeBind1(0,"),
        "pipeBind should NOT use slot 0 (element slot). Output:\n{js}"
    );
}

/// Test pipe slot in [field] binding inside *ngIf embedded view.
#[test]
fn test_pipe_in_field_in_ngif() {
    let js = compile_template_to_js(
        r#"<div *ngIf="show"><cu-comp [formField]="(settings$ | async)?.workload?.field" [title]="name | uppercase"></cu-comp></div>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");
    assert!(!js.contains("pipeBind1(0,"), "pipeBind should NOT use slot 0. Output:\n{js}");
}

/// Test pipe slot in [field] binding inside @if block.
#[test]
fn test_pipe_in_field_in_if_block() {
    let js = compile_template_to_js(
        r#"@if (show) {<cu-comp [formField]="(settings$ | async)?.workload?.field" [title]="name | uppercase"></cu-comp>}"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");
    assert!(!js.contains("pipeBind1(0,"), "pipeBind should NOT use slot 0. Output:\n{js}");
}

/// Test multiple pipes with control binding in @if block.
///
/// Replicates the DashboardBoxComponent pattern: multiple property pipes + control pipe in @if.
#[test]
fn test_multiple_pipes_with_control_in_if() {
    let js = compile_template_to_js(
        r#"@if (show) {<cu-comp [prop1]="a$ | async" [prop2]="b$ | async" [field]="(settings$ | async)?.workload?.field" [prop3]="c$ | async"></cu-comp>}"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");
    assert!(!js.contains("pipeBind1(0,"), "pipeBind should NOT use slot 0. Output:\n{js}");
}

/// Test multiple pipes with control binding in *ngIf.
#[test]
fn test_multiple_pipes_with_control_in_ngif() {
    let js = compile_template_to_js(
        r#"<cu-comp *ngIf="show" [prop1]="a$ | async" [prop2]="b$ | async" [field]="(settings$ | async)?.workload?.field" [prop3]="c$ | async"></cu-comp>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");
    assert!(
        !js.contains("pipeBind1(0,"),
        "pipeBind should NOT use slot 0 in *ngIf template. Output:\n{js}"
    );
}

/// Test pipe with safe navigation in regular property binding.
///
/// Simpler case: pipe with safe navigation in property binding.
#[test]
fn test_pipe_with_safe_nav_in_property() {
    let js =
        compile_template_to_js(r#"<div [title]="(data$ | async)?.name"></div>"#, "TestComponent");
    eprintln!("OUTPUT:\n{js}");
    // The pipe should have slot 1 (after element at slot 0)
    assert!(js.contains("pipeBind1(1,"), "pipeBind should use slot 1. Output:\n{js}");
}

/// Test pipe in control binding with multiple pipe properties.
///
/// This matches the ViewsSidebar pattern: element with [field] pipe binding
/// + multiple other property pipe bindings + listener, inside @if.
#[test]
fn test_pipe_in_control_with_multiple_pipe_properties() {
    let js = compile_template_to_js(
        r#"@if (true) {
            <my-comp
                [field]="data$ | async"
                [prop1]="val1$ | async"
                [prop2]="val2$ | async"
                (myEvent)="onEvent($event)"
            ></my-comp>
        }"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Verify no pipe uses slot 0 (slot 0 is the element)
    assert!(!js.contains("pipeBind1(0,"), "No pipeBind should use slot 0. Output:\n{js}");
}

/// Test pipe in control with safe navigation chain.
///
/// This has pipe + safe navigation chain inside control binding.
#[test]
fn test_pipe_in_control_with_safe_nav_chain() {
    let js = compile_template_to_js(
        r#"<my-comp
            [field]="(data$ | async)?.nested?.value"
            [other]="other$ | async"
        ></my-comp>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // The pipe for control should NOT have slot 0 (slot 0 is the element)
    assert!(
        !js.contains("pipeBind1(0,"),
        "No pipeBind should use slot 0 (element slot). Output:\n{js}"
    );
}

/// Test pipe in control with safe navigation inside *ngIf.
///
/// Pattern inside *ngIf (ng-template) - matches DashboardBox exactly.
#[test]
fn test_pipe_in_control_with_safe_nav_in_ngif() {
    let js = compile_template_to_js(
        r#"<my-comp *ngIf="show"
            [field]="(settings$ | async)?.workload?.field"
            [disableTimeEstimate]="isGuest$ | async"
            [customFields]="fields$ | async"
        ></my-comp>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // No pipe should use slot 0
    assert!(!js.contains("pipeBind1(0,"), "No pipeBind should use slot 0. Output:\n{js}");
}

/// Test pipe in control with safe navigation using full file transform.
///
/// Uses transform_angular_file to match the e2e comparison path exactly.
#[test]
fn test_pipe_in_control_with_safe_nav_full_file() {
    let allocator = Allocator::default();

    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-box',
    template: `
        <div *ngIf="show">
            <div *ngIf="!isLoading">
                <cu-box-settings *ngIf="!isEmpty"
                    [field]="(boxSettings$ | async)?.workload?.field"
                    [disableTimeEstimate]="(isGuestTeam$ | async) && !((team$ | async)?.guest_settings?.can_see_time_estimated)"
                    [customFields]="customFields$ | async"
                    [customFieldId]="(boxSettings$ | async)?.workload?.customFieldId"
                ></cu-box-settings>
            </div>
        </div>
    `,
})
export class DashboardBoxComponent {
    show = true;
    isLoading = false;
    isEmpty = false;
    boxSettings$: any;
    isGuestTeam$: any;
    team$: any;
    customFields$: any;
}
"#;

    let result =
        transform_angular_file(&allocator, "dashboard-box.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    eprintln!("FULL OUTPUT:\n{code}");

    // Check that no pipeBind uses slot 0 (which would indicate unresolved slot)
    assert!(
        !code.contains("pipeBind1(0,"),
        "No pipeBind should use slot 0 (unresolved slot). Output:\n{code}"
    );
}

/// Test pipe in control with multiple pipe properties (v2).
///
/// More specific: exactly matching ViewsSidebar output pattern with
/// [field] binding first, then multiple other property bindings with pipes.
#[test]
fn test_pipe_in_control_with_multiple_pipe_properties_v2() {
    let js = compile_template_to_js(
        r#"@if (showForm) {
            <my-selector
                [forSidebarHeader]="true"
                [field]="newCustomField$ | async"
                [dropdownWidth]="209"
                [autoOpen]="autoOpenSelector$ | async"
                [autoFocus]="autoFocusSelector$ | async"
                [canCreate]="canCreate$ | async"
                (fieldSelected)="onFieldTypeChange($event)"
            ></my-selector>
        }"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // ALL pipes should have non-zero slots
    let pipe_decl_count = js.matches("ɵɵpipe(").count();
    let pipebind_zero = js.contains("pipeBind1(0,");

    assert!(pipe_decl_count >= 4, "Expected at least 4 pipe declarations. Output:\n{js}");
    assert!(
        !pipebind_zero,
        "No pipeBind should use slot 0 - all pipe slots should be > 0. Output:\n{js}"
    );
}

/// Test pipe slot propagation in reactive form control pattern.
///
/// This uses [field] with a pipe, matching the reactive form control pattern.
/// Without the UpdateOp::Control fix, the pipe slot would be 0.
#[test]
fn test_pipe_slot_in_reactive_form_control_binding() {
    let js = compile_template_to_js(r#"<input [field]="myField$ | async">"#, "TestComponent");
    eprintln!("OUTPUT:\n{js}");

    // Verify pipe declaration exists
    assert!(js.contains("ɵɵpipe("), "Should have pipe declaration. Output:\n{js}");

    // The pipe should have a non-zero slot
    assert!(
        !js.contains("pipeBind1(0,"),
        "pipeBind should NOT use slot 0. Slot 0 is the element slot. Output:\n{js}"
    );

    // Specifically, the pipe at slot 1 after the element at slot 0
    assert!(js.contains("pipeBind1(1,"), "pipeBind should use slot 1. Output:\n{js}");
}

/// Test pipe slot propagation through Control ops in an embedded view.
///
/// This tests that the fix works for pipes inside control bindings that are
/// themselves inside embedded views (e.g., @if blocks).
#[test]
fn test_pipe_slot_in_control_binding_inside_if_block() {
    let js = compile_template_to_js(
        r#"@if (showForm) {
            <cu-comp [field]="data$ | async" [title]="title$ | async"></cu-comp>
        }"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Count pipe declarations - should have 2 (one for each pipe)
    let pipe_count = js.matches("ɵɵpipe(").count();
    assert!(
        pipe_count >= 2,
        "Expected at least 2 pipe declarations, got {pipe_count}. Output:\n{js}"
    );

    // No pipe should use slot 0 (slot 0 is the element in the embedded view)
    assert!(
        !js.contains("pipeBind1(0,"),
        "No pipeBind should use slot 0 (element slot). Output:\n{js}"
    );
}

// ============================================================================
// Pipe Slot Propagation Through ArrowFunctionExpr Tests
// ============================================================================

/// Test pipe slot propagation in @if alias pattern (ArrowFunctionExpr context).
///
/// When pipes appear in expressions with aliases that trigger PureFunction extraction,
/// the pipe slot must still be correctly propagated through the ArrowFunctionExpr
/// ops that are stored in view.functions.
#[test]
fn test_pipe_slot_in_if_alias_with_pipe() {
    // @if with pipe and alias creates a conditional with a pipe binding.
    let js = compile_template_to_js(
        r"@if (data$ | async; as data) {
            <div>{{data.name}}</div>
        }",
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Verify pipe is present
    assert!(js.contains("ɵɵpipe("), "Should have pipe declaration. Output:\n{js}");

    // The pipe slot should not be 0
    assert!(!js.contains("pipeBind1(0,"), "pipeBind should NOT use slot 0. Output:\n{js}");

    // The pipe should use slot 1 (after the conditional at slot 0)
    assert!(js.contains("pipeBind1(1,"), "pipeBind should use slot 1. Output:\n{js}");
}

/// Test pipe slot propagation with multiple pipes in @if alias pattern.
///
/// This tests that all pipes in a complex @if condition with alias
/// get their slots correctly propagated.
#[test]
fn test_pipe_slot_in_if_alias_with_multiple_pipes() {
    let js = compile_template_to_js(
        r"@if ({open: value | async, isOverlay: overlay | async}; as data) {
            <div>{{data.open}}</div>
        }",
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Should have 2 pipe declarations
    let pipe_count = js.matches("ɵɵpipe(").count();
    assert!(
        pipe_count >= 2,
        "Expected at least 2 pipe declarations, got {pipe_count}. Output:\n{js}"
    );

    // No pipe should use slot 0 (slot 0 is the conditional)
    assert!(!js.contains("pipeBind1(0,"), "No pipeBind should use slot 0. Output:\n{js}");
}

/// Test pipe slots inside ArrowFunctionExpr ops in embedded views.
///
/// This uses a pipe inside an @if with alias, nested inside another @if,
/// to create a scenario where ArrowFunctionExpr ops exist in an embedded view
/// (not just the root view).
#[test]
fn test_pipe_slot_in_embedded_view_arrow_function() {
    let js = compile_template_to_js(
        r"@if (showForm) {
            @if (data$ | async; as data) {
                <div>{{data.name}}</div>
            }
        }",
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Verify pipe exists
    assert!(js.contains("ɵɵpipe("), "Should have pipe declaration. Output:\n{js}");

    // The pipe slot should not be 0
    assert!(
        !js.contains("pipeBind1(0,"),
        "pipeBind should NOT use slot 0 in embedded view. Output:\n{js}"
    );
}

// ============================================================================
// Parenthesized Safe Navigation Precedence Tests
// ============================================================================

/// Test that parenthesized safe navigation preserves correct precedence.
///
/// Template: `{{ (obj?.prop)[key] }}`
/// Without parentheses fix: `obj == null ? null : obj.prop[key]` (WRONG - keyed access inside ternary)
/// With parentheses fix:    `(obj == null ? null : obj.prop)[key]` (CORRECT - keyed access outside ternary)
///
/// The parentheses around `obj?.prop` create a grouping that acts as a barrier
/// in `has_safe_ternary_receiver`, preventing the keyed read from being absorbed
/// into the safe ternary expansion.
#[test]
fn test_parenthesized_safe_navigation_keyed_access() {
    let js = compile_template_to_js(r"<div>{{ (obj?.prop)[key] }}</div>", "TestComponent");
    eprintln!("OUTPUT:\n{js}");

    // The keyed access [key] must be OUTSIDE the ternary null check.
    // Correct pattern: `(... == null ? null : ...prop)[key]`
    // The output should contain `)[` pattern showing the keyed access
    // is applied to the result of the parenthesized ternary.
    assert!(
        js.contains(")["),
        "Keyed access should be outside the safe ternary (parenthesized). Output:\n{js}"
    );

    // The output should NOT have the keyed access inside the ternary false branch,
    // i.e., should not have `.prop[` without a preceding `)` from the parenthesized ternary.
    // Verify the ternary is wrapped in parentheses before the keyed access.
    let has_correct_precedence =
        js.contains("null)? null: ctx.obj.prop)") || js.contains("null : ctx.obj.prop)[");
    assert!(
        has_correct_precedence,
        "Safe ternary should be wrapped in parentheses before keyed access. Output:\n{js}"
    );
}

/// Test that standalone components WITH directive imports use Full mode (elementStart).
///
/// Angular determines compilation mode from component metadata:
///   meta.isStandalone && !meta.hasDirectiveDependencies → DomOnly
///   otherwise → Full
///
/// See: angular/packages/compiler/src/render3/view/compiler.ts lines 229-232
#[test]
fn test_dom_only_mode_not_used_when_component_has_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[appHighlight]', standalone: true })
export class HighlightDirective {}

@Component({
  selector: 'app-test',
  standalone: true,
  imports: [HighlightDirective],
  template: `
    <div>Hello</div>
    @for (item of items; track item) {
      <li>{{ item }}</li>
    }
  `
})
export class TestComponent {
  items: string[] = [];
}
";

    // OXC always uses Full mode (elementStart, not domElementStart)
    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // Should use elementStart (Full mode), NOT domElementStart (DomOnly mode)
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Component with imports should use ɵɵelementStart (Full mode), not domElementStart. Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Component with imports should NOT use ɵɵdomElementStart. Output:\n{}",
        result.code
    );
}

/// Test that standalone components WITHOUT imports use Full mode (local compilation).
///
/// OXC is a single-file compiler, equivalent to Angular's local compilation mode.
/// In local compilation mode, Angular ALWAYS sets hasDirectiveDependencies=true,
/// so DomOnly mode is never used for component templates.
/// See: angular/packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts:1257
#[test]
fn test_dom_only_mode_not_used_for_standalone_without_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: true,
  template: `
    <div>Hello</div>
    <span>World</span>
  `
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // OXC in local compilation mode: always Full mode for component templates
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Standalone component without imports should use ɵɵelementStart (Full mode). Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Standalone component without imports should NOT use ɵɵdomElementStart. Output:\n{}",
        result.code
    );
}

/// Test that non-standalone components use Full mode.
#[test]
fn test_dom_only_mode_not_used_for_non_standalone() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: false,
  template: `<div>Hello</div>`
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // Non-standalone should always use Full mode
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Non-standalone component should use ɵɵelementStart. Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Non-standalone component should NOT use ɵɵdomElementStart. Output:\n{}",
        result.code
    );
}

// ============================================================================
// Animation Binding Variable Naming Tests
// ============================================================================

#[test]
fn test_animation_enter_in_embedded_view_variable_naming() {
    // Animation bindings (animate.enter) in an embedded view (@if) should:
    // 1. Generate a SavedView (getCurrentView) for the embedded view
    // 2. The animation handler should restoreView to the saved view
    // 3. Variable naming (_rN) should be sequential without gaps
    //
    // This reproduces a bug where the animation handler's restoreView target
    // was not properly resolved, leading to _unnamed_N references and
    // off-by-one errors in variable naming indices.
    // [animate.enter] with brackets makes it a value binding (dynamic expression),
    // which creates an AnimationOp with handler function that needs save/restore view.
    let js = compile_template_to_js(
        r#"@if (show) { <div [animate.enter]="getValue()">{{label}}</div> }"#,
        "TestComponent",
    );
    assert!(
        !js.contains("_unnamed_"),
        "Generated JS contains _unnamed_ references, indicating animation handler naming bug.\nGenerated JS:\n{js}"
    );
    // Verify getCurrentView is generated for the embedded view.
    // Even though the animation handler's restoreView gets optimized away
    // (because getValue() doesn't reference view-specific variables),
    // the SavedView variable is eagerly created per Angular's spec.
    assert!(
        js.contains("getCurrentView"),
        "Embedded view with animation should have getCurrentView.\nGenerated JS:\n{js}"
    );
    insta::assert_snapshot!("animation_enter_in_embedded_view", js);
}

#[test]
fn test_animation_enter_in_nested_for_if_variable_naming() {
    // Reproduces the form.component.ts pattern:
    // @for creates outer embedded view, @if creates inner embedded view,
    // animate.enter in the inner view references variables from the @for scope.
    //
    // NG output pattern:
    //   const _r3 = i0.ɵɵgetCurrentView();
    //   i0.ɵɵelementStart(0, "div");
    //   i0.ɵɵanimateEnter(function() {
    //     i0.ɵɵrestoreView(_r3);
    //     const field_r4 = i0.ɵɵnextContext().$implicit;
    //     return i0.ɵɵresetView(field_r4.value);
    //   });
    //
    // OXC bug: missing getCurrentView, uses _unnamed_N in restoreView
    let js = compile_template_to_js(
        r#"@for (field of fields; track field) {
            @if (field.visible) {
                <div [animate.enter]="field.value">{{field.name}}</div>
            }
        }"#,
        "TestComponent",
    );
    assert!(
        !js.contains("_unnamed_"),
        "Generated JS contains _unnamed_ references in nested for/if animation.\nGenerated JS:\n{js}"
    );
    insta::assert_snapshot!("animation_enter_nested_for_if", js);
}

#[test]
fn test_animation_in_for_with_listener_variable_naming() {
    // Tests that when a view has both a regular listener AND an animation callback,
    // all variable names are consistent and properly sequenced.
    let js = compile_template_to_js(
        r#"@for (item of items; track item; let i = $index) {
            @if (item.active) {
                <div [animate.enter]="item.animation"
                     (click)="handleClick(item, i)">
                    {{item.label}}
                </div>
            }
        }"#,
        "TestComponent",
    );
    assert!(
        !js.contains("_unnamed_"),
        "Generated JS contains _unnamed_ references in animation+listener case.\nGenerated JS:\n{js}"
    );
    insta::assert_snapshot!("animation_in_for_with_listener", js);
}

#[test]
fn test_animation_enter_string_literal_in_embedded_view() {
    // Reproduces the edit-long-text-custom-field-value.component.ts issue:
    // An animation handler that returns a string literal in an embedded view (@if)
    // should still have getCurrentView/restoreView/resetView generated.
    //
    // Angular always adds restoreView/resetView to animation handlers in embedded views,
    // even when the handler expression is a simple string literal.
    // The variable_optimization phase may later optimize away the restoreView if the
    // handler doesn't actually access outer context, but the getCurrentView at the
    // view level must survive if there are other listeners that reference it.
    //
    // NG output pattern for animation returning string literal in embedded view:
    //   const _r1 = i0.ɵɵgetCurrentView();
    //   i0.ɵɵelementStart(0, "div", 0);
    //   i0.ɵɵanimateEnter(function ..._cb() {
    //     i0.ɵɵrestoreView(_r1);
    //     const ctx_r1 = i0.ɵɵnextContext();
    //     return i0.ɵɵresetView(ctx_r1.onClick());
    //   });
    //   i0.ɵɵlistener("click", function ..._listener() { ... });
    //
    // OXC bug: missing getCurrentView because the animation handler's restoreView
    // gets optimized away (string literal doesn't reference outer context), and if
    // the SavedView variable optimization also removes the getCurrentView, other
    // listeners in the same view lose their restoreView target.
    let js = compile_template_to_js(
        r#"@if (show) {
            <div [animate.enter]="'animate-in'" (click)="onClick()">
                {{label}}
            </div>
        }"#,
        "TestComponent",
    );
    assert!(
        !js.contains("_unnamed_"),
        "Generated JS contains _unnamed_ references.\nGenerated JS:\n{js}"
    );
    assert!(
        js.contains("getCurrentView"),
        "Embedded view with animation and listener should have getCurrentView.\nGenerated JS:\n{js}"
    );
    assert!(
        js.contains("restoreView"),
        "Listener in embedded view should have restoreView.\nGenerated JS:\n{js}"
    );
    insta::assert_snapshot!("animation_enter_string_literal_embedded_view", js);
}

#[test]
fn test_animation_enter_string_literal_only_in_embedded_view() {
    // Tests the case where an animation handler returning a string literal is the ONLY
    // listener-like op in an embedded view. Per Angular's variable_optimization.ts (lines 61-66),
    // Animation handlers also get `optimizeSaveRestoreView`, so when the return value is a
    // simple string literal that doesn't reference the view context, restoreView/resetView
    // should be optimized away.
    //
    // Expected output pattern:
    //   i0.ɵɵanimateEnter(function ...() {
    //     return "animate-in";
    //   });
    let js = compile_template_to_js(
        r#"@if (show) {
            <div [animate.enter]="'animate-in'">
                {{label}}
            </div>
        }"#,
        "TestComponent",
    );
    assert!(
        !js.contains("_unnamed_"),
        "Generated JS contains _unnamed_ references.\nGenerated JS:\n{js}"
    );
    // The animation handler only returns a string literal, so restoreView/resetView
    // should be optimized away. But the view's listener (if any) may still need it,
    // so we check that the animation callback itself doesn't have unnecessary wrapping.
    insta::assert_snapshot!("animation_enter_string_literal_only_embedded_view", js);
}

/// Test that implicit standalone components (no `standalone` in decorator) use Full mode.
///
/// Angular 19+ defaults `standalone` to `true` when not specified. However, OXC performs
/// single-file compilation without NgModule context. Angular's ngtsc (in local compilation
/// mode) always sets `hasDirectiveDependencies = true` because it can't fully inspect
/// dependencies. OXC should do the same: only use DomOnly mode when `standalone: true`
/// is EXPLICITLY set in the decorator.
///
/// Components that rely on the implicit default may be declared in NgModules, which
/// Angular's global compilation handles by setting `hasDirectiveDependencies =
/// !isStandalone || ...`. Without NgModule context, OXC must be conservative.
///
/// See: angular/packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts:1326-1339
#[test]
fn test_dom_only_mode_not_used_for_implicit_standalone() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  template: `
    <div>Hello</div>
    <span>World</span>
  `
})
export class TestComponent {}
";

    // Angular version 19+ defaults standalone to true, but implicit standalone
    // should NOT trigger DomOnly mode because the component might be in an NgModule
    let options = ComponentTransformOptions {
        angular_version: Some(AngularVersion::new(21, 0, 0)),
        ..Default::default()
    };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);

    // Should use Full mode (elementStart), NOT DomOnly (domElementStart)
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Implicit standalone component should use ɵɵelementStart (Full mode). Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Implicit standalone component should NOT use ɵɵdomElementStart (DomOnly). Output:\n{}",
        result.code
    );
}

/// Test that implicit standalone components with empty imports also use Full mode.
///
/// Even with an empty `imports: []` array, if `standalone` is not explicitly set,
/// OXC should use Full mode to be safe.
#[test]
fn test_dom_only_mode_not_used_for_implicit_standalone_with_empty_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  imports: [],
  template: `<div>Hello</div>`
})
export class TestComponent {}
";

    let options = ComponentTransformOptions {
        angular_version: Some(AngularVersion::new(21, 0, 0)),
        ..Default::default()
    };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);

    // Implicit standalone + empty imports should still use Full mode
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Implicit standalone with empty imports should use Full mode. Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Implicit standalone with empty imports should NOT use DomOnly. Output:\n{}",
        result.code
    );
}

/// Test that standalone components with empty imports use Full mode (local compilation).
///
/// OXC always uses Full mode for component templates, matching Angular's local compilation.
#[test]
fn test_dom_only_mode_not_used_for_standalone_with_empty_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: true,
  imports: [],
  template: `<div>Hello</div>`
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // OXC in local compilation mode: always Full mode for component templates
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Standalone with empty imports should use ɵɵelementStart (Full mode). Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Standalone with empty imports should NOT use ɵɵdomElementStart. Output:\n{}",
        result.code
    );
}

/// Test that standalone components with ONLY pipe imports use Full mode (local compilation).
///
/// OXC always uses Full mode for component templates, matching Angular's local compilation.
#[test]
fn test_dom_only_mode_not_used_for_standalone_with_pipe_only_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';
import { AsyncPipe } from '@angular/common';

@Component({
  selector: 'app-test',
  standalone: true,
  imports: [AsyncPipe],
  template: `<div>{{ data$ | async }}</div>`
})
export class TestComponent {
  data$ = null;
}
";

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // OXC in local compilation mode: always Full mode for component templates
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Standalone with pipe-only imports should use ɵɵelementStart (Full mode). Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Standalone with pipe-only imports should NOT use ɵɵdomElementStart. Output:\n{}",
        result.code
    );
}

/// Test that multiple pipe-only imports also use Full mode (local compilation).
///
/// OXC always uses Full mode for component templates, matching Angular's local compilation.
#[test]
fn test_dom_only_mode_not_used_for_standalone_with_multiple_pipe_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';
import { AsyncPipe, DatePipe, SlicePipe } from '@angular/common';

@Component({
  selector: 'app-test',
  standalone: true,
  imports: [AsyncPipe, DatePipe, SlicePipe],
  template: `<div>Hello</div>`
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    // OXC in local compilation mode: always Full mode for component templates
    assert!(
        result.code.contains("ɵɵelementStart"),
        "Multiple pipe-only imports should use Full mode. Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("ɵɵdomElementStart"),
        "Multiple pipe-only imports should NOT use DomOnly mode. Output:\n{}",
        result.code
    );
}

/// Test that mixed pipe + directive imports still use Full mode.
#[test]
fn test_full_mode_used_for_standalone_with_mixed_imports() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Directive } from '@angular/core';
import { AsyncPipe } from '@angular/common';

@Directive({ selector: '[appHighlight]', standalone: true })
export class HighlightDirective {}

@Component({
  selector: 'app-test',
  standalone: true,
  imports: [AsyncPipe, HighlightDirective],
  template: `<div>Hello</div>`
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.ts", source, None, None);

    assert!(
        result.code.contains("ɵɵelementStart"),
        "Mixed imports should use Full mode. Output:\n{}",
        result.code
    );
}

/// Test that @let declaration in nested @if correctly preserves context variable.
///
/// When a `@let` declaration and a subsequent expression both reference the component
/// context (via properties like `data$` and `canEdit$`), the compiler should:
/// 1. Create a single context variable (`ctx_r0 = nextContext(N)`)
/// 2. Use it for BOTH the @let's storeLet expression AND the conditional
/// 3. NOT inline the context variable (since it's used twice)
///
/// Bug: The context variable was incorrectly inlined into the storeLet, leaving
/// the second reference as `_unnamed_N` because the variable no longer existed
/// when the naming phase ran.
#[test]
fn test_let_declaration_with_multiple_context_refs_variable_naming() {
    let js = compile_template_to_js(
        r"@if (show) {
            @if (loading) {
                <span>Loading</span>
            } @else {
                @let data = items$ | async;
                @if (!(data?.length) && (canEdit$ | async)) {
                    <span>Empty</span>
                } @else {
                    <span>{{data}}</span>
                }
            }
        }",
        "TestComponent",
    );

    // The output must NOT contain _unnamed_ references
    assert!(
        !js.contains("_unnamed_"),
        "Context variable used by both @let and conditional should be named properly, not _unnamed_. Output:\n{js}"
    );
}

// ============================================================================
// Const reference index: i18n property binding extraction
// ============================================================================

/// Tests that pure property bindings with i18n markers are extracted as BindingKind::Property.
/// Pure property bindings like [heading]="title" i18n-heading keep Bindings marker (3) because
/// the runtime uses domProperty to set the value, not i18nAttributes. The I18n marker (6) is
/// only used for interpolated attributes that go through the i18n pipeline.
#[test]
fn test_i18n_property_binding_extracted_as_property_kind() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<my-comp [heading]="title" i18n-heading="@@my-heading">content</my-comp>',
    standalone: true,
})
export class TestComponent {
    title = 'hello';
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    // Pure property bindings keep Bindings marker (3), NOT I18n marker (6).
    // The i18n marker on a property binding is a no-op for directive matching.
    assert!(
        result.code.contains(r#"3,"heading""#),
        "Pure property binding with i18n marker should produce Bindings AttributeMarker (3). Output:\n{}",
        result.code
    );
}

/// Tests that interpolated attributes with i18n markers (e.g., heading="{{ name }}" i18n-heading)
/// are extracted as BindingKind::I18n (marker 6).
/// Angular's attribute_extraction.ts checks `op.i18nMessage !== null && op.templateKind === null`
/// and overrides the binding kind to I18n.
/// This matches the real-world pattern in ClickUp's old-join-team component.
#[test]
fn test_i18n_interpolated_attribute_extracted_as_i18n_kind() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<my-comp heading="Join the {{ name }} Workspace" i18n-heading="@@join-workspace">content</my-comp>',
    standalone: true,
})
export class TestComponent {
    name = 'hello';
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    // The consts array should contain [6,"heading"] (AttributeMarker.I18n = 6)
    // because the interpolated attribute has an i18n message (i18n-heading).
    assert!(
        result.code.contains(r#"6,"heading""#),
        "Interpolated attribute with i18n marker should produce I18n AttributeMarker (6). Output:\n{}",
        result.code
    );
    assert!(
        !result.code.contains(r#"3,"heading""#),
        "Interpolated attribute with i18n marker should NOT produce Bindings AttributeMarker (3). Output:\n{}",
        result.code
    );
}

/// Tests that i18n property bindings in control flow don't produce extra consts entries.
/// When a property binding has i18n-attr (e.g., [cuTooltip]="expr" i18n-cuTooltip),
/// the consts entry should use Bindings marker (3), matching the conditional insertion point.
/// This ensures the entries deduplicate and don't shift downstream consts indices.
#[test]
fn test_i18n_property_binding_in_control_flow_no_extra_consts() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: `
      <div data-test="body" class="body">
        @if (showTooltip) {
          <div data-test="inner"
            [cuTooltip]="someExpr"
            i18n-cuTooltip="@@copy-id">
            Content
          </div>
        }
      </div>
    `,
    standalone: true,
})
export class TestComponent {
    someExpr = 'hello';
    showTooltip = true;
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    // The consts array should NOT contain [6,"cuTooltip"] because [cuTooltip]="expr"
    // is a pure property binding, not an interpolated attribute.
    assert!(
        !result.code.contains(r#"6,"cuTooltip""#),
        "Pure property binding in control flow should NOT produce I18n AttributeMarker (6). Output:\n{}",
        result.code
    );
    // Should use Bindings marker (3) instead
    assert!(
        result.code.contains(r#"3,"cuTooltip""#),
        "Pure property binding in control flow should produce Bindings AttributeMarker (3). Output:\n{}",
        result.code
    );
}

/// Tests that i18n expressions with pipes maintain the correct template order.
/// When a text node inside an i18n span contains both plain expressions and pipe expressions,
/// the expressions must be emitted in their original template order.
/// Ported from Angular compliance test: r3_view_compiler_i18n/multiple_pipes.ts
#[test]
fn test_i18n_expression_ordering_with_pipes() {
    let js = compile_template_to_js(
        r"<span i18n>{{ a }} and {{ b }} and {{ c }} and {{ b | uppercase }}</span>",
        "TestComponent",
    );

    // Debug: print full output
    // The i18nExp calls should follow template order:
    // ctx.a, ctx.b, ctx.c, pipeBind1(..., ctx.b)
    assert!(
        js.contains("i18nExp(ctx.a)(ctx.b)(ctx.c)"),
        "Expressions should be in template order. Full output:\n{js}"
    );
}

/// Tests i18n expression ordering with ICU plural containing both plain and pipe expressions.
/// The credits-tooltip pattern: an i18n block with text interpolation + ICU plural where
/// the "other" case has a pipe expression that must come AFTER the plain expression.
///
/// Expected expression order (matching Angular ngtsc):
///   i18nExp(ctx.name)(ctx.count)(ctx.amount)(pipeBind1(..., ctx.count))
///
/// Bug: OXC was emitting pipeBind1 before ctx.amount (swapping expressions 2 and 3).
#[test]
fn test_i18n_expression_ordering_icu_plural_with_pipe() {
    let js = compile_template_to_js(
        r"<div i18n>{{ name }} {count, plural, =1 {({{ amount }} credits x 1 user)} other {({{ amount }} credits x {{ count | number }} users)}}</div>",
        "TestComponent",
    );

    // Extract the update block to check i18nExp ordering
    let update_start = js.find("if ((rf & 2))").expect("should have update block");
    let update_block = &js[update_start..];

    // The plain expression (ctx.amount) must come BEFORE the pipe expression (pipeBind1)
    // in the i18nExp chain. This matches Angular ngtsc behavior.
    let amount_pos = update_block.find("ctx.amount").expect("should have ctx.amount in i18nExp");
    let pipe_pos = update_block.find("pipeBind1").expect("should have pipeBind1 in i18nExp");

    assert!(
        amount_pos < pipe_pos,
        "ctx.amount (plain expression) must come before pipeBind1 (pipe expression) in i18nExp chain.\n\
         amount_pos={amount_pos}, pipe_pos={pipe_pos}\n\
         Update block:\n{update_block}"
    );
}

#[test]
fn test_nested_if_listener_ctx_reference() {
    // Test: nested @if where a listener in the inner @if accesses component properties.
    // The listener should use nextContext() to get the component context,
    // not bare `ctx` which would be the inner embedded view's context.
    let js = compile_template_to_js(
        r#"@if (show) {
  @if (active) {
    <button (click)="handleClick()">Click</button>
  }
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("nested_if_listener_ctx_reference", js);
}

#[test]
fn test_nested_if_alias_listener_ctx_reference() {
    // Test: @if with alias, nested @if where listener accesses both
    // the alias from the outer @if and a method from the component.
    // All context references inside the listener should use named variables (ctx_rN),
    // not bare `ctx`.
    let js = compile_template_to_js(
        r#"@if (getItem(); as item) {
  @if (item.active) {
    <button (click)="makePrivate(!(item.private && !item.shareWithTeam))">Toggle</button>
  }
}"#,
        "TestComponent",
    );
    insta::assert_snapshot!("nested_if_alias_listener_ctx_reference", js);
}

/// Tests that i18n attribute bindings before a @for block do not inflate the xref IDs
/// used for @for loop index variables.
///
/// In Angular's TypeScript compiler, BindingOp.i18nMessage stores a direct reference to
/// the i18n.Message object -- no xref is allocated during ingest. The xref for the i18n
/// context is only allocated later during the create_i18n_contexts phase.
///
/// If Oxc allocates extra xrefs for i18n messages during ingest, the @for body view's
/// xref will be higher than Angular's, causing the generated variable name ɵ$index_N to
/// use the wrong N value.
///
/// For the template `<div i18n-title title="Hello">text</div> @for (item of items; track $index) { {{$index}} }`:
///   - Angular TS: div=xref1, i18nAttrs=xref2, forBody=xref3 => ɵ$index_3
///   - Oxc (buggy): div=xref1, i18nMsg=xref2, i18nAttrs=xref3, forBody=xref4 => ɵ$index_4
#[test]
fn test_for_index_xref_with_i18n_attribute_binding() {
    let js = compile_template_to_js(
        r#"<div i18n-title title="Hello">text</div>
@for (item of items; track $index) { {{$index}} }"#,
        "TestComponent",
    );

    // Verify the output matches expected Angular behavior via snapshot.
    // The fix ensures Oxc doesn't allocate extra xrefs for i18n messages during ingest,
    // matching Angular TS which stores direct i18n.Message object references on BindingOp.
    insta::assert_snapshot!("for_index_xref_with_i18n_attribute_binding", js);
}

/// Tests that setClassMetadata uses namespace-prefixed type references for imported
/// constructor parameter types.
///
/// Angular's TypeScript compiler distinguishes between local and imported types in
/// the ɵsetClassMetadata constructor parameter metadata:
/// - Local types use bare names: `{ type: LocalService }`
/// - Imported types use namespace-prefixed names: `{ type: i1.ImportedService }`
///
/// This is because TypeScript type annotations are erased at runtime, so imported
/// types need namespace imports (i0, i1, i2...) to be available as runtime values.
/// The factory function (ɵfac) already handles this correctly via R3DependencyMetadata
/// and create_token_expression, but setClassMetadata was using bare names for all types.
#[test]
fn test_set_class_metadata_uses_namespace_for_imported_ctor_params() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';
import { SomeService } from './some.service';

@Component({
    selector: 'test-comp',
    template: '<div>hello</div>',
    standalone: true,
})
export class TestComponent {
    constructor(private svc: SomeService) {}
}
";

    let options = ComponentTransformOptions {
        emit_class_metadata: true,
        ..ComponentTransformOptions::default()
    };

    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Extract the setClassMetadata section specifically (not the factory function)
    let metadata_section = result
        .code
        .split("ɵsetClassMetadata")
        .nth(1)
        .expect("setClassMetadata should be present in output");

    // The ctor_parameters callback should use namespace-prefixed type for
    // the imported SomeService: `{type:i1.SomeService}` not `{type:SomeService}`
    assert!(
        metadata_section.contains("i1.SomeService"),
        "setClassMetadata ctor_parameters should use namespace-prefixed type (i1.SomeService) for imported constructor parameter. Metadata section:\n{metadata_section}"
    );
    assert!(
        !metadata_section.contains("type:SomeService}"),
        "setClassMetadata should NOT use bare type name for imported types. Metadata section:\n{metadata_section}"
    );
}

/// Tests that setClassMetadata uses namespace-prefixed type even when @Inject is present.
///
/// When a constructor parameter has both a type annotation and @Inject decorator pointing
/// to the same imported class, the metadata `type` field should still use namespace prefix.
/// The factory correctly uses bare names for @Inject tokens with named imports, but the
/// metadata type always represents the TypeScript type annotation which is erased at runtime.
///
/// Example:
/// - Factory: `ɵɵdirectiveInject(TagPickerComponent, 12)` (bare - ok, @Inject value import)
/// - Metadata: `{ type: i1.TagPickerComponent, decorators: [{type: Inject, ...}] }` (namespace)
#[test]
fn test_set_class_metadata_namespace_with_inject_decorator() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Inject, Optional, SkipSelf } from '@angular/core';
import { SomeService } from './some.service';

@Component({
    selector: 'test-comp',
    template: '<div>hello</div>',
    standalone: true,
})
export class TestComponent {
    constructor(
        @Optional() @SkipSelf() @Inject(SomeService) private svc: SomeService
    ) {}
}
";

    let options = ComponentTransformOptions {
        emit_class_metadata: true,
        ..ComponentTransformOptions::default()
    };

    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Extract the setClassMetadata section
    let metadata_section = result
        .code
        .split("ɵsetClassMetadata")
        .nth(1)
        .expect("setClassMetadata should be present in output");

    // Even with @Inject(SomeService), the type field should use namespace prefix
    // because the type annotation is erased by TypeScript
    assert!(
        metadata_section.contains("i1.SomeService"),
        "setClassMetadata should use namespace-prefixed type even with @Inject. Metadata section:\n{metadata_section}"
    );
}

/// Tests that when @Inject token differs from the type annotation (e.g., @Inject(DOCUMENT)
/// on a parameter typed as Document), the metadata type uses bare name since the type
/// annotation may reference a global or different module than the injection token.
#[test]
fn test_set_class_metadata_inject_differs_from_type() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Inject } from '@angular/core';
import { DOCUMENT } from '@angular/common';

@Component({
    selector: 'test-comp',
    template: '<div>hello</div>',
    standalone: true,
})
export class TestComponent {
    constructor(@Inject(DOCUMENT) private doc: Document) {}
}
";

    let options = ComponentTransformOptions {
        emit_class_metadata: true,
        ..ComponentTransformOptions::default()
    };

    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let metadata_section = result
        .code
        .split("ɵsetClassMetadata")
        .nth(1)
        .expect("setClassMetadata should be present in output");

    // The type should be bare "Document" (global type), not namespace-prefixed
    // even though the @Inject token (DOCUMENT) is from @angular/common
    assert!(
        metadata_section.contains("type:Document"),
        "setClassMetadata should use bare type for globals when @Inject token differs. Metadata section:\n{metadata_section}"
    );
    // Should NOT add namespace prefix for Document
    assert!(
        !metadata_section.contains("i1.Document"),
        "setClassMetadata should NOT namespace-prefix global types. Metadata section:\n{metadata_section}"
    );
}

// ============================================================================
// Namespace Attribute Const Collection Tests
// ============================================================================

/// Test that SVG elements with namespace attributes (xmlns:xlink) produce correct consts.
///
/// When a static attribute like `xmlns:xlink="..."` is ingested, the name should be
/// split into namespace="xmlns" and name="xlink" so that the consts array serializes it
/// as [AttributeMarker.NamespaceUri, "xmlns", "xlink", "..."] instead of
/// [":xmlns:xlink", "..."].
///
/// Without the fix, namespace attributes create duplicate consts entries because
/// the unsplit `:xmlns:xlink` format doesn't match the properly-split format,
/// preventing deduplication and shifting all subsequent consts indices.
#[test]
fn test_svg_namespace_attribute_consts() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-icon',
    standalone: true,
    template: '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" data-testid="icon"><use></use></svg>',
})
export class IconComponent {}
"#;

    let result = transform_angular_file(&allocator, "icon.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    let code = &result.code;
    eprintln!("OUTPUT:\n{code}");

    // The consts array should NOT contain ":xmlns:xlink" as a raw string.
    // Instead, namespace attributes should be serialized with the NamespaceUri marker (0).
    assert!(
        !code.contains(r#"":xmlns:xlink""#),
        "Consts should NOT contain raw ':xmlns:xlink' string. Namespace should be split. Output:\n{code}"
    );

    // The consts array SHOULD contain the proper namespace marker format:
    // 0 (NamespaceUri marker), "xmlns", "xlink"
    assert!(
        code.contains(r#"0,"xmlns","xlink""#),
        "Consts should contain namespace marker format: 0,\"xmlns\",\"xlink\". Output:\n{code}"
    );
}

/// Test that SVG with both namespace attributes and property bindings has correct consts indices.
///
/// This reproduces the real-world icon.component pattern where:
/// - An @if conditional wraps the SVG (creating a template function)
/// - The SVG has namespace attrs (xmlns:xlink) AND a property binding ([name])
/// - Both the conditional template and the SVG element need consts entries
///
/// Without the fix, a duplicate consts entry is created for the SVG element,
/// causing the template to reference the wrong consts index.
#[test]
fn test_svg_namespace_attrs_with_conditional_and_binding() {
    let allocator = Allocator::default();
    let source = r#"
import { Component, Input } from '@angular/core';

@Component({
    selector: 'app-icon',
    standalone: true,
    template: `@if (showIcon) {<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" data-testid="icon" class="svg"><use></use></svg>}`,
})
export class IconComponent {
    showIcon = true;
}
"#;

    let result = transform_angular_file(&allocator, "icon.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    let code = &result.code;
    eprintln!("OUTPUT:\n{code}");

    // Should NOT have duplicate consts entries. Count occurrences of "xmlns" in consts.
    // With the bug, there would be two entries - one with proper namespace format,
    // one with raw ":xmlns:xlink".
    assert!(
        !code.contains(r#"":xmlns:xlink""#),
        "Consts should NOT contain raw ':xmlns:xlink' string. Output:\n{code}"
    );
}

/// When an element has a structural directive (*ngIf) AND an i18n-translated attribute,
/// the hoisted static attributes must preserve the i18n info. Without this, the literal
/// text value is used in the consts array instead of the i18n variable reference, causing
/// incorrect deduplication when multiple similar elements exist.
///
/// Ported to match Angular TS behavior: ingestTemplateBindings passes attr.i18n to
/// createTemplateBinding for hoisted attributes (ingest.ts line 1497).
#[test]
fn test_i18n_attribute_on_structural_directive_element() {
    let allocator = Allocator::default();
    // Two buttons with *ngIf, both with i18n-cuTooltip but different custom IDs.
    // Each should get its own i18n variable (i18n_0, i18n_1) in the consts array.
    // Without the fix, both get literal "Clear date" and are deduplicated into one entry.
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    standalone: true,
    template: `
        <button *ngIf="showA" cuTooltip="Clear date" i18n-cuTooltip="@@clear-date-a" (click)="clearA()">Clear A</button>
        <button *ngIf="showB" cuTooltip="Clear date" i18n-cuTooltip="@@clear-date-b" (click)="clearB()">Clear B</button>
    `,
})
export class TestComponent {
    showA = true;
    showB = true;
    clearA() {}
    clearB() {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    let code = &result.code;
    eprintln!("OUTPUT:\n{code}");

    // Both buttons should use i18n variable references, not the literal "Clear date".
    // The consts array should contain i18n_0 and i18n_1 (or similar), NOT literal "Clear date".
    assert!(
        !code.contains(r#""cuTooltip","Clear date""#),
        "Consts should NOT contain literal 'Clear date' - should use i18n variable reference. Output:\n{code}"
    );
    // There should be two distinct i18n entries (i18n_0, i18n_1) for the two different @@ IDs
    assert!(code.contains("i18n_0"), "Should have i18n_0 variable. Output:\n{code}");
    assert!(code.contains("i18n_1"), "Should have i18n_1 variable. Output:\n{code}");
}

/// Test that pipe inside binary expression with safe navigation generates a temporary variable.
///
/// When a pipe result is wrapped in a binary expression (e.g., `(data$ | async) || fallback`)
/// and then used with safe navigation (`?.`), the compiler should generate a temporary variable
/// to avoid calling the pipe twice.
///
/// This is a port of the Angular TS behavior where `needsTemporaryInSafeAccess` checks through
/// `BinaryOperatorExpr` to find nested pipe expressions that need temporaries.
///
/// Without the fix, the compiler generates:
///   `(pipeBind1(...) || fallback) == null ? null : (pipeBind1(...) || fallback).prop`
///   (pipe called twice, slot indices doubled)
///
/// With the fix:
///   `(tmp_0_0 = pipeBind1(...) || fallback) == null ? null : tmp_0_0.prop`
///   (pipe called once, stored in temp variable)
#[test]
fn test_pipe_in_binary_with_safe_nav_uses_temp_variable() {
    let js = compile_template_to_js(
        r#"<div [title]="((data$ | async) || defaultVal)?.name"></div>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Should use a temporary variable to avoid double pipe evaluation
    assert!(
        js.contains("tmp_0_0"),
        "Should generate tmp_0_0 for pipe inside binary with safe nav. Output:\n{js}"
    );

    // The pipe should only appear ONCE in the expression (stored in tmp)
    let pipe_count = js.matches("pipeBind1(").count();
    assert_eq!(
        pipe_count, 1,
        "pipeBind1 should appear exactly once (not duplicated). Found {pipe_count} occurrences. Output:\n{js}"
    );
}

/// Test pipe in binary with safe navigation and chained property access.
///
/// More complex case: `((data$ | async) || fallback)?.nested?.value`
/// The entire safe navigation chain should use the temp variable.
#[test]
fn test_pipe_in_binary_with_safe_nav_chain() {
    let js = compile_template_to_js(
        r#"<div [title]="((data$ | async) || defaultVal)?.nested?.value"></div>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");

    // Should use a temporary variable
    assert!(
        js.contains("tmp_0_0"),
        "Should generate tmp_0_0 for pipe inside binary with safe nav chain. Output:\n{js}"
    );

    // The pipe should only appear ONCE
    let pipe_count = js.matches("pipeBind1(").count();
    assert_eq!(
        pipe_count, 1,
        "pipeBind1 should appear exactly once. Found {pipe_count}. Output:\n{js}"
    );
}

/// Tests that interpolations inside HTML elements within nested ICU plural branches
/// are correctly extracted as i18n expression placeholders.
///
/// When ICU case text contains `<strong>{{ expr }}</strong>`, the interpolation is
/// inside an HTML element node. `extract_placeholders_from_nodes` must recurse into
/// element children to find these interpolations. Without this, they are silently
/// dropped, leading to fewer i18nExp calls than expected.
///
/// This reproduces the undo-toast-items.component.ts mismatch where Angular emits 8
/// i18nExp args but OXC only emitted 5 due to missing interpolations inside `<strong>`.
#[test]
fn test_i18n_nested_icu_with_interpolations_inside_elements() {
    let js = compile_template_to_js(
        r"<span i18n>{count, plural, =1 {<strong>{{ name }}</strong> was deleted from {nestedCount, plural, =1 {<strong>{{ category }}</strong>} other {<strong>{{ category }}</strong> and {{ extra }} more}}} other {{{ count }} items deleted}}</span>",
        "TestComponent",
    );

    eprintln!("OUTPUT:\n{js}");

    // All interpolation expressions must appear in the i18nExp chain.
    // The expressions inside <strong> elements MUST be extracted:
    //   - name (inside <strong> in outer =1 branch)
    //   - category (inside <strong> in nested =1 branch)
    //   - category (inside <strong> in nested other branch)
    //   - extra (plain text in nested other branch)
    //   - count (plain text in outer other branch)
    // Plus the ICU switch variables:
    //   - count (outer plural VAR)
    //   - nestedCount (inner plural VAR)

    // Check that the expressions inside <strong> elements are present
    assert!(
        js.contains("ctx.name"),
        "ctx.name (inside <strong> in ICU) must be in i18nExp chain. Output:\n{js}"
    );
    assert!(
        js.contains("ctx.category"),
        "ctx.category (inside <strong> in nested ICU) must be in i18nExp chain. Output:\n{js}"
    );

    // Count the total number of i18nExp arguments.
    // There should be 7 expressions total:
    //   VAR: extra (innermost ICU), nestedCount (middle), count (outer) = 3 ICU vars
    //   INTERPOLATION: name, category, category, extra, count = varies
    // The exact count depends on deduplication, but name and category must be present.
    let i18n_exp_count = js.matches("i18nExp(").count();
    assert!(
        i18n_exp_count >= 1,
        "Should have at least one i18nExp call. Found {i18n_exp_count}. Output:\n{js}"
    );

    insta::assert_snapshot!("i18n_nested_icu_with_interpolations_inside_elements", js);
}

/// Tests that @defer loading timer consts are ordered AFTER i18n consts in the consts array.
///
/// Angular's TS compiler wraps defer timer configs in ConstCollectedExpr (phase 19), which are
/// resolved later in collectConstExpressions (phase 53) — AFTER i18n consts are added (phase 52).
/// This means i18n consts always appear before defer timer consts in the consts array.
///
/// Previously, OXC directly called job.add_const() in the defer_configs phase, placing the timer
/// const [100, null] at the front of the array and shifting all i18n indices by +1.
///
/// The template pattern: an i18n message + @defer with @loading(minimum 100ms).
/// Expected consts ordering: [i18n_0, [100, null], ...]
/// Bug consts ordering:      [[100, 0], i18n_0, ...]
#[test]
fn test_defer_loading_timer_consts_after_i18n_consts() {
    let js = compile_template_to_js(
        r#"<span i18n="@@my-label">Hello</span>
@defer (on viewport; prefetch on idle) {
  <div>Deferred content</div>
} @loading (minimum 100ms) {
  <div>Loading...</div>
}"#,
        "TestComponent",
    );

    // The i18n message should reference const index 0 (i18n_0 is first in consts array)
    // The @defer instruction should reference the timer config at a later index
    //
    // NG expected output has:
    //   consts: [...] => return [i18n_0, [100, null], ...]
    //   i18n(N, 0)       — i18n at const index 0
    //   defer(M, ..., 1, ..., timerScheduling)  — timer config at const index 1
    //
    // The bug would produce:
    //   consts: [...] => return [[100, 0], i18n_0, ...]
    //   i18n(N, 1)       — i18n at const index 1 (wrong!)
    //   defer(M, ..., 0, ..., timerScheduling)  — timer config at const index 0 (wrong!)

    // Check that i18n references const index 0 (not 1)
    assert!(
        js.contains("i18n(1,0)") || js.contains("i18n(1, 0)"),
        "i18n should reference const index 0 (before defer timer const). Output:\n{js}"
    );

    insta::assert_snapshot!("defer_loading_timer_consts_after_i18n_consts", js);
}

// =============================================================================
// Regression tests for runtime bugs found during ClickUp integration
// =============================================================================

/// Regression: Directive constructor DI tokens must be namespace-prefixed.
///
/// Bug: `@Directive` classes that inject services from external modules (e.g., `Store` from
/// `@ngrx/store`) emitted bare identifiers like `ɵɵdirectiveInject(Store)` instead of
/// namespace-prefixed `ɵɵdirectiveInject(i1.Store)`. At runtime TypeScript elides the bare
/// `Store` import (it's type-only), causing `ASSERTION ERROR: token must be defined`.
///
/// Root cause (two parts):
/// 1. `extract_param_token()` in directive/decorator.rs returned `ReadProp(i0.TypeName)` with
///    a hardcoded `i0` prefix, instead of `ReadVar(TypeName)` like injectable/pipe/ng_module.
/// 2. `transform_angular_file()` did not call `resolve_factory_dep_namespaces()` for directive
///    deps, so even corrected `ReadVar` tokens would not get namespace-resolved.
///
/// Fix: Return `ReadVar(TypeName)` from `extract_param_token()` AND call
/// `resolve_factory_dep_namespaces()` for directive deps in transform.rs.
#[test]
fn test_directive_factory_deps_use_namespace_prefixed_tokens() {
    let allocator = Allocator::default();

    // Simulate the ClickUp pattern: a directive injecting services from multiple modules
    let source = r"
import { Directive } from '@angular/core';
import { Store } from '@ngrx/store';
import { ToastService } from './toast.service';

@Directive({
    selector: '[appToastPosition]',
    standalone: true,
})
export class ToastPositionHelperDirective {
    constructor(
        private store: Store,
        private toastService: ToastService,
    ) {}
}
";

    let result = transform_angular_file(
        &allocator,
        "toast-position-helper.directive.ts",
        source,
        None,
        None,
    );

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Factory function should use namespace-prefixed tokens for DI
    // Store comes from @ngrx/store (i1), ToastService from ./toast.service (i2)
    assert!(
        code.contains("i1.Store"),
        "Factory should use namespace-prefixed i1.Store for @ngrx/store import. Output:\n{code}"
    );
    assert!(
        code.contains("i2.ToastService"),
        "Factory should use namespace-prefixed i2.ToastService for ./toast.service import. Output:\n{code}"
    );

    // Must NOT have bare (un-prefixed) tokens in directiveInject calls
    // A bare `directiveInject(Store)` would fail at runtime because TypeScript elides the import
    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");

    assert!(
        !factory_section.contains("directiveInject(Store)"),
        "Factory must NOT use bare 'Store' - TypeScript would elide this import. Factory:\n{factory_section}"
    );
    assert!(
        !factory_section.contains("directiveInject(ToastService)"),
        "Factory must NOT use bare 'ToastService' - TypeScript would elide this import. Factory:\n{factory_section}"
    );
}

/// Regression: Directive with multiple DI deps from different modules gets correct namespace indices.
///
/// Each imported module should get a unique namespace alias (i0=@angular/core, i1=first import,
/// i2=second import, etc.). This test verifies the namespace registry correctly assigns indices
/// when a directive has dependencies from multiple external modules.
#[test]
fn test_directive_multiple_deps_different_modules_correct_namespaces() {
    let allocator = Allocator::default();

    let source = r"
import { Directive, ElementRef } from '@angular/core';
import { Router } from '@angular/router';
import { HttpClient } from '@angular/common/http';
import { FormBuilder } from '@angular/forms';

@Directive({
    selector: '[appMultiDep]',
    standalone: true,
})
export class MultiDepDirective {
    constructor(
        private el: ElementRef,
        private router: Router,
        private http: HttpClient,
        private fb: FormBuilder,
    ) {}
}
";

    let result = transform_angular_file(&allocator, "multi-dep.directive.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // ElementRef is from @angular/core (i0) - should be i0.ElementRef
    assert!(
        code.contains("i0.ElementRef"),
        "ElementRef from @angular/core should use i0 namespace. Output:\n{code}"
    );

    // Each external module should get its own namespace (i1, i2, i3)
    // The exact indices depend on registration order, but each should be namespace-prefixed
    assert!(code.contains(".Router"), "Router should be namespace-prefixed. Output:\n{code}");
    assert!(
        code.contains(".HttpClient"),
        "HttpClient should be namespace-prefixed. Output:\n{code}"
    );
    assert!(
        code.contains(".FormBuilder"),
        "FormBuilder should be namespace-prefixed. Output:\n{code}"
    );

    // Count the namespace import declarations - should have i0 + at least 3 more
    let namespace_imports: Vec<&str> =
        code.lines().filter(|l| l.contains("import * as i")).collect();
    assert!(
        namespace_imports.len() >= 4,
        "Should have at least 4 namespace imports (i0..i3). Found {}:\n{}",
        namespace_imports.len(),
        namespace_imports.join("\n")
    );
}

/// Regression: `ɵɵi18nPostprocess` must use namespace prefix `i0.ɵɵi18nPostprocess`.
///
/// Bug: `wrap_with_postprocess()` in i18n_const_collection.rs used a bare
/// `ReadVar(ɵɵi18nPostprocess)` which emitted `ɵɵi18nPostprocess(...)` without the `i0.`
/// namespace prefix. At runtime: `ReferenceError: ɵɵi18nPostprocess is not defined`.
///
/// The postprocess function is only called for ICU messages that need sub-expression
/// replacement (e.g., nested plural/select with multiple sub-messages). Simple i18n
/// messages don't trigger this code path.
///
/// Fix: Changed to `ReadProp(i0.ɵɵi18nPostprocess)` matching all other Angular runtime calls.
#[test]
fn test_i18n_icu_postprocess_uses_namespace_prefix() {
    let allocator = Allocator::default();

    // An ICU plural with sub-messages triggers ɵɵi18nPostprocess.
    // This is the pattern from ClickUp's ChatBotTriggerComponent.
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-chatbot',
    standalone: true,
    template: `<span i18n>{count, plural, =1 {<strong>{{ name }}</strong> item} other {{{ count }} items}}</span>`,
})
export class ChatBotTriggerComponent {
    count = 0;
    name = '';
}
";

    let result =
        transform_angular_file(&allocator, "chatbot-trigger.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // If i18nPostprocess is present, it MUST be namespace-prefixed
    if code.contains("i18nPostprocess") {
        assert!(
            code.contains("i0.ɵɵi18nPostprocess"),
            "ɵɵi18nPostprocess must be namespace-prefixed as i0.ɵɵi18nPostprocess. \
             A bare 'ɵɵi18nPostprocess' causes ReferenceError at runtime. Output:\n{code}"
        );

        // Must NOT have a bare (un-prefixed) call
        // Check that there's no `ɵɵi18nPostprocess` without `i0.` before it
        for (i, _) in code.match_indices("ɵɵi18nPostprocess") {
            let prefix = &code[..i];
            assert!(
                prefix.ends_with("i0."),
                "Found bare ɵɵi18nPostprocess without i0. prefix at position {i}. Output:\n{code}"
            );
        }
    }
}

/// Multiple `@ViewChild` decorators on the same component should emit
/// as a single chained `ɵɵviewQuery(p1)(p2)(p3)` call — matches upstream
/// ngtsc's `instructionChainAfter()` emit. `ɵɵviewQuery` returns
/// `typeof ɵɵviewQuery` (core/src/render3/instructions/queries.ts:58),
/// so chaining is safe.
///
/// Earlier versions of this compiler emitted three separate statements
/// based on an incorrect "returns void" reading of Angular's source.
#[test]
fn test_multiple_view_queries_emit_chained_call() {
    let allocator = Allocator::default();

    let source = r"
import { Component, ViewChild, ElementRef } from '@angular/core';

@Component({
    selector: 'app-login',
    template: '<input #emailInput /><input #passwordInput /><button #submitBtn>Login</button>',
})
export class LoginFormComponent {
    @ViewChild('emailInput') emailInput: ElementRef;
    @ViewChild('passwordInput') passwordInput: ElementRef;
    @ViewChild('submitBtn') submitBtn: ElementRef;
}
";

    // Chained query emit requires Angular ≥ 21.0.4 (queries return
    // `typeof <fn>`, not `void`). Pin the version explicitly so this
    // test exercises the chained path; without it the compiler falls
    // back to the older safe separate-statement form.
    let options = ComponentTransformOptions {
        angular_version: Some(AngularVersion::new(22, 0, 0)),
        ..Default::default()
    };
    let result =
        transform_angular_file(&allocator, "login-form.component.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Three decorator-form view queries — one chained call expression has
    // the root identifier `ɵɵviewQuery` appearing exactly once, with two
    // additional `(...)` argument groups appended via chaining.
    let view_query_count = code.matches("ɵɵviewQuery(").count();
    assert_eq!(
        view_query_count, 1,
        "Three consecutive decorator queries should chain into a single \
         `ɵɵviewQuery(...)...` expression. Found {view_query_count} root calls. \
         Output:\n{code}"
    );

    // The single `ɵɵviewQuery(` must be followed (after its closing `)`)
    // by another `(` — the chain continuation.
    let query_fn = "ɵɵviewQuery(";
    let mut chain_followups = 0;
    for (start_idx, _) in code.match_indices(query_fn) {
        let after_fn = &code[start_idx + query_fn.len()..];
        let mut depth = 1;
        let mut end = 0;
        for (i, ch) in after_fn.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if after_fn[end..].trim_start().starts_with('(') {
            chain_followups += 1;
        }
    }
    assert_eq!(
        chain_followups, 1,
        "Chained queries should produce one continuation `(...)` after the \
         root call. Got {chain_followups}. Output:\n{code}"
    );
}

/// Multiple `@ContentChild`/`@ContentChildren` queries should emit
/// as a single chained `ɵɵcontentQuery(...)(...)(...)` call. Same
/// contract as `ɵɵviewQuery` — `ɵɵcontentQuery` returns
/// `typeof ɵɵcontentQuery` (core/src/render3/instructions/queries.ts:40).
#[test]
fn test_multiple_content_queries_emit_chained_call() {
    let allocator = Allocator::default();

    let source = r"
import { Component, ContentChild, ContentChildren, QueryList, TemplateRef } from '@angular/core';

@Component({
    selector: 'app-tabs',
    template: '<ng-content></ng-content>',
})
export class TabsComponent {
    @ContentChild('header') header: TemplateRef<any>;
    @ContentChildren('tab') tabs: QueryList<TemplateRef<any>>;
    @ContentChild('footer') footer: TemplateRef<any>;
}
";

    // Pin to v22 — see note on the view-query chained test above.
    let options = ComponentTransformOptions {
        angular_version: Some(AngularVersion::new(22, 0, 0)),
        ..Default::default()
    };
    let result =
        transform_angular_file(&allocator, "tabs.component.ts", source, Some(&options), None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    let content_query_count = code.matches("ɵɵcontentQuery(").count();
    assert_eq!(
        content_query_count, 1,
        "Three consecutive decorator content queries should chain into a single \
         `ɵɵcontentQuery(...)...` expression. Found {content_query_count} root calls. \
         Output:\n{code}"
    );

    let query_fn = "ɵɵcontentQuery(";
    let mut chain_followups = 0;
    for (start_idx, _) in code.match_indices(query_fn) {
        let after_fn = &code[start_idx + query_fn.len()..];
        let mut depth = 1;
        let mut end = 0;
        for (i, ch) in after_fn.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if after_fn[end..].trim_start().starts_with('(') {
            chain_followups += 1;
        }
    }
    assert_eq!(
        chain_followups, 1,
        "Should produce one continuation `(...)` after the root content-query call. \
         Got {chain_followups}. Output:\n{code}"
    );
}

/// Mixed signal + decorator view queries: consecutive same-kind calls
/// chain (`ɵɵviewQuerySignal(...)(...)` for the two signal queries), but
/// the chain breaks at the boundary to the decorator-form
/// (`ɵɵviewQuery(...)`) because the two runtime symbols aren't
/// interchangeable.
#[test]
fn test_mixed_signal_and_decorator_view_queries_break_chain_on_boundary() {
    let allocator = Allocator::default();

    let source = r"
import { Component, ViewChild, viewChild, viewChildren, ElementRef } from '@angular/core';

@Component({
    selector: 'app-mixed',
    template: '<div #a></div><div #b></div><div #c></div>',
})
export class MixedQueryComponent {
    a = viewChild<ElementRef>('a');
    b = viewChildren<ElementRef>('b');
    @ViewChild('c') c: ElementRef;
}
";

    // Pin to v22 — see note on the view-query chained test above.
    let options = ComponentTransformOptions {
        angular_version: Some(AngularVersion::new(22, 0, 0)),
        ..Default::default()
    };
    let result = transform_angular_file(
        &allocator,
        "mixed-query.component.ts",
        source,
        Some(&options),
        None,
    );

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Exactly one root ɵɵviewQuerySignal call (the two signal queries chain
    // into it) and exactly one root ɵɵviewQuery call (the decorator one).
    let signal_roots = code.matches("ɵɵviewQuerySignal(").count();
    let decorator_roots = code.matches("ɵɵviewQuery(").count();
    assert_eq!(
        signal_roots, 1,
        "Two signal view queries should chain into one root call. \
         Found {signal_roots}. Output:\n{code}"
    );
    assert_eq!(
        decorator_roots, 1,
        "The single decorator view query should produce one root call after \
         the signal chain breaks. Found {decorator_roots}. Output:\n{code}"
    );

    // Signal chain should have exactly one continuation `(...)` (the second
    // signal call). The decorator call should have zero continuations.
    fn count_chain_followups(code: &str, query_fn: &str) -> usize {
        let mut followups = 0;
        for (start_idx, _) in code.match_indices(query_fn) {
            let after_fn = &code[start_idx + query_fn.len()..];
            let mut depth = 1;
            let mut end = 0;
            for (i, ch) in after_fn.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if after_fn[end..].trim_start().starts_with('(') {
                followups += 1;
            }
        }
        followups
    }
    assert_eq!(
        count_chain_followups(code, "ɵɵviewQuerySignal("),
        1,
        "Two consecutive signal queries should chain (one continuation). \
         Output:\n{code}"
    );
    assert_eq!(
        count_chain_followups(code, "ɵɵviewQuery("),
        0,
        "Single decorator query should have no chain continuation. \
         Output:\n{code}"
    );
}

/// Compatibility guard. Two contracts at once:
///
/// 1. **`None` = assume latest.** Crates-wide convention: unset
///    `angular_version` means the consumer is on latest, so the
///    compiler emits the modern (chained) form. Mirrors
///    `supports_implicit_standalone`/`supports_service_decorator`'s
///    `map_or(true, …)`.
/// 2. **Explicit pre-v21.0.4 falls back.** On Angular 19, 20, and
///    v21.0.0–v21.0.3 the runtime query functions return `void`, so
///    chained emit would throw at runtime. Consumers targeting those
///    versions must pass `angular_version` explicitly to opt out of
///    the chained form.
#[test]
fn test_query_chaining_obeys_angular_version_gate() {
    let allocator = Allocator::default();

    let source = r"
import { Component, ViewChild, ElementRef } from '@angular/core';

@Component({
    selector: 'app-login',
    template: '<input #a /><input #b /><input #c />',
})
export class LoginFormComponent {
    @ViewChild('a') a: ElementRef;
    @ViewChild('b') b: ElementRef;
    @ViewChild('c') c: ElementRef;
}
";

    // Versions where chained emit would crash at runtime — must produce
    // three separate `ɵɵviewQuery(…)` statements.
    let unsafe_versions: [AngularVersion; 3] = [
        AngularVersion::new(19, 2, 0),
        AngularVersion::new(20, 0, 0),
        AngularVersion::new(21, 0, 3),
    ];
    for version in unsafe_versions {
        let options =
            ComponentTransformOptions { angular_version: Some(version), ..Default::default() };
        let result =
            transform_angular_file(&allocator, "login.component.ts", source, Some(&options), None);
        assert!(!result.has_errors(), "v{version:?} should compile: {:?}", result.diagnostics);

        let code = &result.code;
        let root_calls = code.matches("ɵɵviewQuery(").count();
        assert_eq!(
            root_calls, 3,
            "v{version:?}: expected 3 separate ɵɵviewQuery statements (chained emit \
             unsafe pre-v21.0.4). Output:\n{code}"
        );
    }

    // Versions where chained emit is safe — including `None` (assume
    // latest) per the crate's convention.
    let chained_versions: [Option<AngularVersion>; 3] =
        [None, Some(AngularVersion::new(21, 0, 4)), Some(AngularVersion::new(22, 0, 0))];
    for version in chained_versions {
        let options = ComponentTransformOptions { angular_version: version, ..Default::default() };
        let result =
            transform_angular_file(&allocator, "login.component.ts", source, Some(&options), None);
        let code = &result.code;
        assert_eq!(
            code.matches("ɵɵviewQuery(").count(),
            1,
            "v{version:?}: should chain — `None` defaults to assume-latest, \
             v21.0.4+ has `typeof <fn>` return. Output:\n{code}"
        );
    }
}

#[test]
fn test_for_loop_multiple_index_aliases_in_track() {
    // When multiple aliases reference $index (e.g., `let i = $index, j = $index`),
    // ALL aliases must be rewritten to $index in the track expression.
    // Angular stores all $index aliases in a Set<string> and checks membership,
    // while a bug in OXC previously stored only the last alias (overwriting earlier ones).
    // Reference: Angular's ingest.ts uses `indexVarNames = new Set<string>()` and `.add()`.
    let js = compile_template_to_js(
        r"@for (item of items; track i + j; let i = $index, j = $index) { {{item}} }",
        "TestComponent",
    );
    // The track function should rewrite both `i` and `j` to `$index`.
    // Expected: ($index,$item)=>($index + $index)
    // Bug behavior: ($index,$item)=>(this.i + $index)  (only `j` rewritten, `i` left as `this.i`)
    assert!(
        !js.contains("this.i"),
        "Track expression should rewrite all $index aliases, but `i` was not rewritten.\nGenerated JS:\n{js}"
    );
    assert!(
        !js.contains("this.j"),
        "Track expression should rewrite all $index aliases, but `j` was not rewritten.\nGenerated JS:\n{js}"
    );
}

// ============================================================================
// Error Recovery Conformance Tests (R3 Transform Level)
// ============================================================================

/// Transforms an Angular template to R3 AST and returns the nodes + error messages.
/// Unlike `compile_template_to_js`, this does NOT panic on parse/transform errors.
fn transform_to_r3(template: &str) -> (std::vec::Vec<String>, bool) {
    let allocator = Allocator::default();

    let parser = HtmlParser::with_expansion_forms(&allocator, template, "test.html");
    let html_result = parser.parse();

    let mut errors: std::vec::Vec<String> =
        html_result.errors.iter().map(|e| e.msg.clone()).collect();

    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&html_result.nodes);

    errors.extend(r3_result.errors.iter().map(|e| e.msg.clone()));

    // Check if any ForLoopBlock nodes exist in the result
    let has_for_block = r3_result.nodes.iter().any(|n| matches!(n, R3Node::ForLoopBlock(_)));
    (errors, has_for_block)
}

/// Returns (errors, r3_nodes_debug) for deeper node inspection.
fn transform_to_r3_nodes(template: &str) -> (std::vec::Vec<String>, std::vec::Vec<String>) {
    let allocator = Allocator::default();

    let parser = HtmlParser::with_expansion_forms(&allocator, template, "test.html");
    let html_result = parser.parse();

    let mut errors: std::vec::Vec<String> =
        html_result.errors.iter().map(|e| e.msg.clone()).collect();

    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&html_result.nodes);

    errors.extend(r3_result.errors.iter().map(|e| e.msg.clone()));

    let node_types: std::vec::Vec<String> = r3_result
        .nodes
        .iter()
        .map(|n| match n {
            R3Node::ForLoopBlock(_) => "ForLoopBlock".to_string(),
            R3Node::IfBlock(b) => format!("IfBlock(branches={})", b.branches.len()),
            R3Node::SwitchBlock(_) => "SwitchBlock".to_string(),
            R3Node::Text(_) => "Text".to_string(),
            R3Node::Element(_) => "Element".to_string(),
            R3Node::BoundText(_) => "BoundText".to_string(),
            other => format!("{other:?}").chars().take(30).collect(),
        })
        .collect();

    (errors, node_types)
}

#[test]
fn test_for_block_no_expression_returns_none() {
    // @for with no expression should return None (no ForLoopBlock node),
    // matching Angular's behavior where parseForLoopParameters returns null.
    let (errors, has_for_block) = transform_to_r3("@for { <div></div> }");
    assert!(
        !has_for_block,
        "Angular returns null node when @for expression fails to parse, but Rust emitted a ForLoopBlock"
    );
    assert!(!errors.is_empty(), "Should report a parse error for @for without expression");
}

#[test]
fn test_for_block_missing_track_returns_none() {
    // @for with valid expression but missing track should return None,
    // matching Angular's behavior (params.trackBy === null → node stays null).
    let (errors, has_for_block) = transform_to_r3("@for (item of items) { <div></div> }");
    assert!(
        !has_for_block,
        "Angular returns null node when @for has no track expression, but Rust emitted a ForLoopBlock"
    );
    assert!(
        errors.iter().any(|e| e.contains("track")),
        "Should report an error about missing track expression. Errors: {errors:?}"
    );
}

#[test]
fn test_if_block_no_expression_skips_main_branch() {
    // @if with no parameters should not push a main branch,
    // matching Angular where parseConditionalBlockParameters returns null.
    let (errors, node_types) = transform_to_r3_nodes("@if { <div></div> }");
    // The IfBlock should have 0 branches (main branch skipped)
    for node_type in &node_types {
        if node_type.starts_with("IfBlock") {
            assert_eq!(
                node_type, "IfBlock(branches=0)",
                "Angular skips the main branch when parameters are missing, expected 0 branches"
            );
        }
    }
    assert!(!errors.is_empty(), "Should report a parse error for @if without expression");
}

// ============================================================================
// Regression: @switch with @default first should preserve source order
// ============================================================================

#[test]
fn test_switch_default_first_preserves_source_order() {
    // When @default appears first in source, Angular TS preserves source order:
    // Case_0 = default (Other), Case_1 = case(1) (One), Case_2 = case(2) (Two)
    // The conditional expression puts default's slot as the ternary fallback.
    let js = compile_template_to_js(
        r"@switch (value) { @default { <div>Other</div> } @case (1) { <div>One</div> } @case (2) { <div>Two</div> } }",
        "TestComponent",
    );

    // Case_0 should be the default (Other), NOT reordered
    assert!(js.contains("Case_0_Template"), "Expected Case_0_Template in output. Got:\n{js}");
    let case0_start = js.find("Case_0_Template").unwrap();
    let case0_body = &js[case0_start..case0_start + 200];
    assert!(
        case0_body.contains("Other"),
        "Case_0 should render 'Other' (default in source order). Got:\n{js}"
    );

    // Conditional ternary: default slot (0) should be the fallback base
    // Expected: (tmp === 1) ? 1 : (tmp === 2) ? 2 : 0
    assert!(js.contains("2: 0)"), "Ternary fallback should be slot 0 (default). Got:\n{js}");
}

// ============================================================================
// Regression: [field] should be a regular property, not a control binding
// ============================================================================

#[test]
fn test_field_property_not_control_binding() {
    // [field] is a regular property binding, NOT a form control binding.
    // Only [formField] should trigger control binding behavior.
    // Before fix: [field] emitted controlCreate()/control() instructions.
    // After fix: [field] emits regular property() instruction.
    let js = compile_template_to_js(r#"<cu-comp [field]="myField"></cu-comp>"#, "TestComponent");

    // Should NOT have controlCreate
    assert!(!js.contains("controlCreate"), "[field] should NOT produce controlCreate. Got:\n{js}");

    // Should NOT have control() call
    assert!(!js.contains("ɵɵcontrol("), "[field] should NOT produce ɵɵcontrol(). Got:\n{js}");

    // Should have regular property binding
    assert!(
        js.contains(r#"ɵɵproperty("field""#),
        "[field] should produce regular ɵɵproperty(\"field\", ...). Got:\n{js}"
    );
}

// ============================================================================
// Regression: optimize_save_restore_view should remove restoreView/resetView
// when the listener body doesn't reference any parent context variables.
// ============================================================================

#[test]
fn test_optimize_save_restore_view_stop_propagation() {
    // Listener in embedded view that returns $event.stopPropagation()
    // should NOT have restoreView/resetView wrapping because the listener
    // doesn't reference any parent context variables.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<div *ngIf="show"><button (click)="$event.stopPropagation()">Click</button></div>',
    standalone: true,
})
export class TestComponent {
    show = true;
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Angular optimizes away restoreView/resetView when the listener doesn't
    // reference any parent context. The output should just be:
    //   return $event.stopPropagation();
    // NOT:
    //   i0.ɵɵrestoreView(_r1);
    //   return i0.ɵɵresetView($event.stopPropagation());
    assert!(
        !code.contains("restoreView") || !code.contains("$event.stopPropagation"),
        "Listener that only calls $event.stopPropagation() should not have restoreView/resetView. Got:\n{code}"
    );
}

#[test]
fn test_optimize_save_restore_view_return_false() {
    // Listener in embedded view that returns false
    // should NOT have restoreView/resetView wrapping.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<div *ngIf="show"><a (click)="false">Link</a></div>',
    standalone: true,
})
export class TestComponent {
    show = true;
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // The listener should just return false without restoreView/resetView
    assert!(
        !code.contains("restoreView"),
        "Listener that returns false should not have restoreView. Got:\n{code}"
    );
    assert!(
        !code.contains("resetView"),
        "Listener that returns false should not have resetView. Got:\n{code}"
    );
}

#[test]
fn test_optimize_save_restore_view_prevent_default() {
    // Listener in embedded view that returns $event.preventDefault()
    // should NOT have restoreView/resetView wrapping.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<div *ngIf="show"><form (submit)="$event.preventDefault()">Form</form></div>',
    standalone: true,
})
export class TestComponent {
    show = true;
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // The listener should just return $event.preventDefault() without restoreView/resetView
    assert!(
        !code.contains("restoreView"),
        "Listener that only calls $event.preventDefault() should not have restoreView. Got:\n{code}"
    );
    assert!(
        !code.contains("resetView"),
        "Listener that only calls $event.preventDefault() should not have resetView. Got:\n{code}"
    );
}

// ============================================================================
// Unicode / Non-ASCII Character Tests
// ============================================================================

#[test]
fn test_unicode_text_not_escaped() {
    // Unicode characters like en-dash should be emitted as raw UTF-8, not escaped.
    // Angular's TypeScript emitter does NOT escape non-ASCII printable characters.
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<span>Hello \u{2013} World</span>',
    standalone: true,
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    // Should contain raw en-dash, not escaped
    assert!(
        code.contains("\u{2013}") && !code.contains("\\u2013"),
        "En-dash should be emitted as raw UTF-8, not as \\u2013. Got:\n{code}"
    );
}

#[test]
fn test_unicode_non_breaking_space_not_escaped() {
    // Non-breaking space (U+00A0) should also be emitted as raw UTF-8
    let allocator = Allocator::default();
    let source = "
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<span>Hello\u{00A0}World</span>',
    standalone: true,
})
export class TestComponent {}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    // Should contain raw non-breaking space, not escaped
    assert!(
        code.contains("\u{00A0}") && !code.contains("\\u00A0"),
        "Non-breaking space should be emitted as raw UTF-8, not as \\u00A0. Got:\n{code}"
    );
}

/// Test that variable naming in embedded view listeners matches Angular TS output.
///
/// This is based on a real-world example from the ClickUp codebase where Angular TS
/// generates `_r1` and `ctx_r1` but OXC generates `_r2` and `ctx_r2` (off by 1).
///
/// The root cause is that the root view's SavedView variable (prepended by
/// save_and_restore_view) uses up a counter slot even though it gets optimized
/// away. The naming phase should produce the same numbering as Angular TS.
#[test]
fn test_variable_naming_in_embedded_view_listener() {
    let allocator = Allocator::default();
    // Template has ng-template with a listener that accesses the component context.
    // This is a simplified version of ObjectMentionsSelectV2Component.
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: `
        <ng-template>
            <button (click)="onClick($event)">Click</button>
        </ng-template>
    `,
    standalone: true,
})
export class TestComponent {
    onClick(e: any) {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Angular TS generates:
    //   const _r1 = i0.ɵɵgetCurrentView();   <-- SavedView in embedded view
    //   ...
    //   i0.ɵɵrestoreView(_r1);
    //   const ctx_r1 = i0.ɵɵnextContext();    <-- Context variable
    //
    // If the root view's unused SavedView variable consumes a counter slot,
    // OXC would produce _r2 and ctx_r2 instead.
    assert!(
        code.contains("_r1") && !code.contains("_r2"),
        "Embedded view should use _r1 for saved view, not _r2. Got:\n{code}"
    );
}

/// Test that variable naming matches Angular TS when root view has listeners.
///
/// When the root view has listeners (that don't need RestoreView), the root's
/// SavedView variable should still be optimized away. Angular TS's root template
/// does NOT have getCurrentView() in this case.
#[test]
fn test_variable_naming_root_listener_no_savedview() {
    let allocator = Allocator::default();
    // Template: root view has a listener + an ng-template with its own listener.
    // The root listener uses `ctx` directly, so the root's SavedView is unnecessary.
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: `
        <div (click)="onRootClick()">Root</div>
        <ng-template>
            <button (click)="onClick($event)">Click</button>
        </ng-template>
    `,
    standalone: true,
})
export class TestComponent {
    onRootClick() {}
    onClick(e: any) {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // The root template should NOT have getCurrentView() since the root listener
    // doesn't need RestoreView (it accesses ctx directly).
    // The embedded view should use _r1 (not _r2).
    //
    // If the root's unused SavedView leaks into the naming counter,
    // the embedded view would use _r2 instead of _r1.
    let root_template_section = code.split("function TestComponent_Template(").nth(1).unwrap_or("");

    let has_root_getcurrentview =
        root_template_section.split("function ").next().unwrap_or("").contains("getCurrentView");

    assert!(
        !has_root_getcurrentview,
        "Root template should NOT have getCurrentView() since root listeners don't need RestoreView. Got:\n{code}"
    );

    // The embedded view should use _r1, not _r2
    assert!(code.contains("_r1"), "Embedded view should use _r1 for saved view. Got:\n{code}");
    // _r2 should NOT appear
    assert!(
        !code.contains("_r2"),
        "No _r2 should appear (would indicate counter offset from unused root SavedView). Got:\n{code}"
    );
}

/// Test variable naming with a template reference (like #notifyFooter).
///
/// Based on the real ClickUp ObjectMentionsSelectV2Component which uses:
///   <ng-template #notifyFooter> with a listener inside
/// Angular TS uses _r1 for the embedded view, but OXC may use _r2 if the
/// root's SavedView consumes the counter.
#[test]
fn test_variable_naming_with_template_ref() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: `
        <div (click)="onRootClick()">Root</div>
        <ng-template #myRef>
            <button (click)="onClick($event)">Click</button>
        </ng-template>
    `,
    standalone: true,
})
export class TestComponent {
    onRootClick() {}
    onClick(e: any) {}
}
"#;

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    // Angular TS uses _r1 for the embedded view's saved view
    assert!(code.contains("_r1"), "Embedded view should use _r1 for saved view. Got:\n{code}");
    // If the root's SavedView consumed the counter, we'd see _r2
    assert!(!code.contains("_r2"), "No _r2 should appear. Got:\n{code}");
}

/// Test that host binding pure function declarations are emitted in the output.
///
/// When a component has a host `[class]` binding with an array literal containing
/// a dynamic value, the compiler extracts a pure function constant (e.g., `_c0`).
/// This constant must be emitted in the output — not silently dropped.
///
/// Guards against host binding pool constants not being emitted in
/// compile_template_to_js_with_options path.
#[test]
fn test_host_binding_pure_function_declarations_emitted() {
    use oxc_angular_compiler::{HostMetadataInput, compile_template_to_js_with_options};

    let allocator = Allocator::default();
    let template = "<p>hello</p>";

    let options = ComponentTransformOptions {
        host: Some(HostMetadataInput {
            properties: vec![("[class]".to_string(), r#"["my-class", typeClass()]"#.to_string())],
            attributes: vec![],
            listeners: vec![],
            class_attr: None,
            style_attr: None,
        }),
        selector: Some("my-comp".to_string()),
        ..Default::default()
    };

    let result = compile_template_to_js_with_options(
        &allocator,
        template,
        "MyComponent",
        "test.ts",
        &options,
    );

    match result {
        Ok(output) => {
            let code = &output.code;
            // The pure function constant DEFINITION (e.g., `const _c0 = ...`) must be present.
            // Without the fix, the host binding function body references _c0 but the
            // const definition is silently dropped, causing a runtime ReferenceError.
            assert!(
                code.contains("const _c"),
                "Host binding pure function constant definition (const _c0 = ...) must be emitted. Got:\n{code}"
            );
            // The pureFunction1 call must reference the constant
            assert!(
                code.contains("pureFunction1"),
                "Host binding should use pureFunction1 for array literal with dynamic value. Got:\n{code}"
            );
        }
        Err(e) => {
            panic!("Compilation failed: {e:?}");
        }
    }
}

// ============================================================================
// Standalone Emission Tests (Issue #95)
// ============================================================================

/// Test that a standalone component does NOT emit `standalone:true` in ɵɵdefineComponent.
///
/// Angular's TS compiler (compiler.ts:96-98) only emits `standalone: false` when
/// isStandalone === false. When standalone is true, it's omitted because the Angular
/// runtime (definition.ts:637) defaults `standalone` to `true` via `?? true`.
///
/// OXC matches this behavior exactly.
#[test]
fn test_standalone_component_omits_standalone_field() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<div>test</div>'
})
export class TestComponent {}
";

    let options = ComponentTransformOptions::default();
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Scope the check to the ɵɵdefineComponent({...}) literal. The setClassMetadata
    // emission (now on by default, matching ngc) faithfully preserves the user's
    // source `standalone: true` for TestBed — that's expected and not what this test
    // is asserting against.
    let define_start =
        result.code.find("ɵɵdefineComponent(").expect("expected ɵɵdefineComponent call in output");
    let define_end = result.code[define_start..]
        .find("});")
        .map(|i| define_start + i)
        .unwrap_or(result.code.len());
    let define_block = &result.code[define_start..define_end];
    let normalized_define = define_block.replace([' ', '\n', '\t'], "");
    assert!(
        !normalized_define.contains("standalone:true"),
        "ɵɵdefineComponent should NOT emit `standalone:true` (runtime defaults to true). \
         defineComponent block:\n{}",
        define_block
    );
}

/// Test that a non-standalone component emits `standalone:false` in ɵɵdefineComponent.
#[test]
fn test_non_standalone_component_emits_standalone_false() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-legacy',
  standalone: false,
  template: '<div>legacy</div>'
})
export class LegacyComponent {}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains("standalone:false"),
        "Non-standalone component MUST emit `standalone:false` in ɵɵdefineComponent. Output:\n{}",
        result.code
    );
}

/// Test that an implicit standalone component (Angular 19+ default) omits `standalone` field.
///
/// Angular 19+ defaults `standalone` to `true`. The Angular TS compiler omits the field
/// when true, and the runtime defaults it via `?? true`. OXC matches this behavior.
#[test]
fn test_implicit_standalone_with_imports_omits_standalone_field() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { NgIf } from '@angular/common';

@Component({
  selector: 'app-implicit',
  imports: [NgIf],
  template: '<div *ngIf="true">implicit standalone</div>'
})
export class ImplicitStandaloneComponent {}
"#;

    let options = ComponentTransformOptions::default();
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    // Angular TS compiler omits standalone when true (runtime defaults to true via ?? true)
    assert!(
        !normalized.contains("standalone:true"),
        "Implicit standalone component should NOT emit `standalone:true` (runtime defaults to true). Output:\n{}",
        result.code
    );
}

// ============================================================================
// JIT Compilation Tests
// ============================================================================

#[test]
fn test_jit_component_with_inline_template() {
    // When jit: true, the compiler should NOT compile templates.
    // Instead, it should keep the decorator and downlevel it using __decorate.
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-root',
    template: '<h1>Hello</h1>',
    standalone: true,
})
export class AppComponent {}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have __decorate import from tslib
    assert!(
        result.code.contains("import { __decorate } from \"tslib\""),
        "JIT output should import __decorate from tslib. Got:\n{}",
        result.code
    );

    // Should NOT have ɵcmp or ɵfac (AOT-style definitions)
    assert!(
        !result.code.contains("ɵcmp") && !result.code.contains("ɵfac"),
        "JIT output should NOT contain AOT definitions (ɵcmp/ɵfac). Got:\n{}",
        result.code
    );

    // Should have __decorate call with Component
    assert!(
        result.code.contains("__decorate("),
        "JIT output should use __decorate. Got:\n{}",
        result.code
    );

    // Should keep the template property as-is (inline template)
    assert!(
        result.code.contains("template:"),
        "JIT output should preserve inline template. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_inline_template", result.code);
}

#[test]
fn test_jit_component_with_template_url() {
    // When jit: true and templateUrl is used, it should be replaced with
    // an import from angular:jit:template:file;./path
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-root',
    templateUrl: './app.html',
    standalone: true,
})
export class AppComponent {}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have resource import for template
    assert!(
        result.code.contains("angular:jit:template:file;./app.html"),
        "JIT output should import template via angular:jit:template:file. Got:\n{}",
        result.code
    );

    // Should replace templateUrl with template referencing the import
    assert!(
        !result.code.contains("templateUrl"),
        "JIT output should replace templateUrl with template. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_template_url", result.code);
}

#[test]
fn test_jit_component_with_style_url() {
    // When jit: true and styleUrl/styleUrls is used, it should be replaced with
    // imports from angular:jit:style:file;./path
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-root',
    template: '<h1>Hello</h1>',
    styleUrl: './app.css',
})
export class AppComponent {}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have resource import for style
    assert!(
        result.code.contains("angular:jit:style:file;./app.css"),
        "JIT output should import style via angular:jit:style:file. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_style_url", result.code);
}

#[test]
fn test_jit_component_with_constructor_deps() {
    // JIT compilation should generate ctorParameters for constructor dependencies
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';
import { TitleService } from './title.service';

@Component({
    selector: 'app-root',
    template: '<h1>Hello</h1>',
})
export class AppComponent {
    constructor(private titleService: TitleService) {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have ctorParameters static property
    assert!(
        result.code.contains("ctorParameters"),
        "JIT output should contain ctorParameters. Got:\n{}",
        result.code
    );

    // Should reference TitleService type
    assert!(
        result.code.contains("TitleService"),
        "JIT ctorParameters should reference dependency type. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_constructor_deps", result.code);
}

#[test]
fn test_jit_component_class_restructuring() {
    // JIT should restructure: export class X {} → let X = class X {}; X = __decorate([...], X); export { X };
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
    selector: 'app-root',
    template: '<h1>Hello</h1>',
})
export class AppComponent {
    title = 'app';
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have let declaration
    assert!(
        result.code.contains("let AppComponent = class AppComponent"),
        "JIT output should use 'let X = class X' pattern. Got:\n{}",
        result.code
    );

    // Should have export statement
    assert!(
        result.code.contains("export { AppComponent }"),
        "JIT output should have named export. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_class_restructuring", result.code);
}

#[test]
fn test_jit_directive() {
    // @Directive should also be JIT-transformed with __decorate
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input } from '@angular/core';

@Directive({
    selector: '[appHighlight]',
    standalone: true,
})
export class HighlightDirective {
    @Input() color: string = 'yellow';
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "highlight.directive.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have __decorate with Directive
    assert!(
        result.code.contains("__decorate("),
        "JIT directive output should use __decorate. Got:\n{}",
        result.code
    );

    // Should NOT have ɵdir or ɵfac
    assert!(
        !result.code.contains("ɵdir") && !result.code.contains("ɵfac"),
        "JIT directive output should NOT contain AOT definitions. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_directive", result.code);
}

#[test]
fn test_jit_service_decorator() {
    // @Service (Angular v22+) should be JIT-downleveled exactly like @Injectable:
    // decorator removed, static decorators/ctorParameters emitted, and __decorate applied.
    let allocator = Allocator::default();
    let source = r"
import { Service } from '@angular/core';

@Service()
export class CounterService {
    constructor(private http: HttpClient) {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "counter.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The @Service decorator should be removed and lowered through __decorate.
    assert!(
        !result.code.contains("@Service"),
        "JIT output should NOT contain the @Service decorator. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("__decorate("),
        "JIT service output should use __decorate. Got:\n{}",
        result.code
    );

    // ctorParameters reflecting the constructor dependency should be emitted.
    assert!(
        result.code.contains("ctorParameters") && result.code.contains("HttpClient"),
        "JIT service output should emit ctorParameters with HttpClient. Got:\n{}",
        result.code
    );

    // JIT must NOT emit AOT definitions; the runtime's compileService handles that.
    assert!(
        !result.code.contains("ɵsvc") && !result.code.contains("ɵfac"),
        "JIT service output should NOT contain AOT definitions. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_service_decorator", result.code);
}

#[test]
fn test_jit_service_decorator_version_gated() {
    // Targeting Angular < 22 must not downlevel @Service (the runtime lacks
    // compileService). The decorator is left in source and a diagnostic is surfaced.
    let allocator = Allocator::default();
    let source = r"
import { Service } from '@angular/core';

@Service()
export class CounterService {}
";

    let options = ComponentTransformOptions {
        jit: true,
        angular_version: Some(AngularVersion::new(21, 0, 0)),
        ..Default::default()
    };
    let result =
        transform_angular_file(&allocator, "counter.service.ts", source, Some(&options), None);

    // A diagnostic should be surfaced for the unsupported decorator.
    assert!(
        result.has_errors(),
        "Targeting v21 with @Service should produce a diagnostic. Got none.\n{}",
        result.code
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.to_string().contains("@Service") && d.to_string().contains("v22")),
        "Diagnostic should mention @Service and v22. Got: {:?}",
        result.diagnostics
    );

    // The decorator should remain in the source (pass-through, not downleveled).
    assert!(
        result.code.contains("@Service"),
        "JIT output for v21 should leave the @Service decorator unchanged. Got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("__decorate("),
        "JIT output for v21 should NOT downlevel @Service via __decorate. Got:\n{}",
        result.code
    );
}

#[test]
fn test_jit_non_angular_service_decorator_does_not_shadow_injectable() {
    // A `@Service()` decorator from a non-Angular library must not cause the JIT
    // pipeline to misclassify the class as v22 `@Service`, nor (on pre-v22
    // targets) swallow a sibling `@Injectable` via the version-gate's early
    // `continue`. Name matching alone is insufficient — `Service` is a common
    // export name in DI containers and web frameworks.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';
import { Service } from 'some-other-lib';

@Service()
@Injectable()
export class CounterService {
    constructor(private http: HttpClient) {}
}
";

    let options = ComponentTransformOptions {
        jit: true,
        angular_version: Some(AngularVersion::new(21, 0, 0)),
        ..Default::default()
    };
    let result =
        transform_angular_file(&allocator, "counter.service.ts", source, Some(&options), None);

    // No "@Service requires v22" diagnostic should fire — this isn't Angular's
    // Service.
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.to_string().contains("@Service") && d.to_string().contains("v22")),
        "Non-Angular @Service should not trigger the v22 diagnostic. Got: {:?}",
        result.diagnostics
    );

    // The real @Injectable must still be lowered.
    assert!(
        result.code.contains("__decorate("),
        "JIT output should still lower @Injectable via __decorate. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("ctorParameters") && result.code.contains("HttpClient"),
        "JIT output should emit ctorParameters for the lowered @Injectable. Got:\n{}",
        result.code
    );
}

#[test]
fn test_jit_full_component_example() {
    // Full example matching the issue #97 scenario
    let allocator = Allocator::default();
    let source = r"
import { Component, signal } from '@angular/core';
import { RouterOutlet } from '@angular/router';
import { Lib1 } from 'lib1';
import { TitleService } from './title.service';

@Component({
    selector: 'app-root',
    imports: [RouterOutlet, Lib1],
    templateUrl: './app.html',
    styleUrl: './app.css',
})
export class App {
    titleService;
    title = signal('app');
    constructor(titleService: TitleService) {
        this.titleService = titleService;
        this.title.set(this.titleService.getTitle());
    }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should have all JIT characteristics
    assert!(
        result.code.contains("import { __decorate } from \"tslib\""),
        "Missing __decorate import"
    );
    assert!(
        result.code.contains("angular:jit:template:file;./app.html"),
        "Missing template resource import"
    );
    assert!(
        result.code.contains("angular:jit:style:file;./app.css"),
        "Missing style resource import"
    );
    assert!(result.code.contains("let App = class App"), "Missing class restructuring");
    assert!(result.code.contains("ctorParameters"), "Missing ctorParameters");
    assert!(result.code.contains("__decorate("), "Missing __decorate call");
    assert!(result.code.contains("export { App }"), "Missing named export");

    // Should NOT have AOT output
    assert!(!result.code.contains("ɵcmp"), "Should not contain ɵcmp");
    assert!(!result.code.contains("ɵfac"), "Should not contain ɵfac");
    assert!(!result.code.contains("defineComponent"), "Should not contain defineComponent");

    insta::assert_snapshot!("jit_full_component", result.code);
}

#[test]
fn test_jit_prop_decorators_emitted() {
    // Bug fix: member decorators (@Input, @Output, etc.) must be downleveled
    // to static propDecorators so Angular's JIT runtime can discover inputs/outputs.
    // Without this, @Input/@Output decorators are silently lost, breaking data binding.
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input, Output, HostBinding, EventEmitter } from '@angular/core';

@Directive({
    selector: '[appHighlight]',
})
export class HighlightDirective {
    @Input() color: string = 'yellow';
    @Input('aliasName') title: string = '';
    @Output() colorChange = new EventEmitter<string>();
    @HostBinding('class.active') isActive = false;
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "highlight.directive.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // propDecorators must be present — Angular's JIT runtime reads this
    assert!(
        result.code.contains("propDecorators"),
        "JIT output must emit static propDecorators. Got:\n{}",
        result.code
    );

    // Each decorated member should appear in propDecorators
    assert!(result.code.contains("color:"), "propDecorators should list 'color'");
    assert!(result.code.contains("title:"), "propDecorators should list 'title'");
    assert!(result.code.contains("colorChange:"), "propDecorators should list 'colorChange'");
    assert!(result.code.contains("isActive:"), "propDecorators should list 'isActive'");

    // The decorator type references should be present
    assert!(result.code.contains("type: Input"), "propDecorators should reference Input");
    assert!(result.code.contains("type: Output"), "propDecorators should reference Output");
    assert!(
        result.code.contains("type: HostBinding"),
        "propDecorators should reference HostBinding"
    );

    // The original decorators must be removed from the class body
    assert!(!result.code.contains("@Input()"), "@Input decorator must be removed from class body");
    assert!(
        !result.code.contains("@Output()"),
        "@Output decorator must be removed from class body"
    );

    insta::assert_snapshot!("jit_prop_decorators", result.code);
}

#[test]
fn test_jit_union_type_ctor_params() {
    // Angular-aligned union type behavior for ctorParameters.
    // Angular's typeReferenceToExpression filters ONLY `null` literal types.
    // If exactly one non-null type remains, it resolves; otherwise unresolvable.
    //
    // `T | null`                  → resolves to T   (1 non-null type)
    // `undefined | T`             → unresolvable     (2 non-null types: undefined + T)
    // `null | undefined | T`      → unresolvable     (2 non-null types: undefined + T)
    //
    // See: angular/packages/compiler-cli/src/ngtsc/transform/jit/src/downlevel_decorators_transform.ts
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';
import { ServiceA } from './a.service';
import { ServiceB } from './b.service';
import { ServiceC } from './c.service';

@Component({ selector: 'test', template: '' })
export class TestComponent {
    constructor(
        svcA: undefined | ServiceA,
        svcB: null | undefined | ServiceB,
        svcC: ServiceC | null,
    ) {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // `ServiceC | null` resolves correctly (1 non-null type)
    assert!(
        result.code.contains("type: ServiceC"),
        "ctorParameters should resolve 'ServiceC | null' to ServiceC. Got:\n{}",
        result.code
    );

    // `undefined | ServiceA` and `null | undefined | ServiceB` are unresolvable per Angular spec
    // (2 non-null types remain after filtering null)
    assert!(
        !result.code.contains("type: ServiceA"),
        "ctorParameters must not resolve 'undefined | ServiceA' (2 non-null types). Got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("type: ServiceB"),
        "ctorParameters must not resolve 'null | undefined | ServiceB' (2 non-null types). Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_union_type_ctor_params", result.code);
}

#[test]
fn test_jit_abstract_class() {
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

@Injectable()
export abstract class BaseProvider {
    protected abstract get name(): string;
    protected abstract initialize(): void;

    public greet(): string {
        return `Hello from ${this.name}`;
    }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "base.provider.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The abstract keyword should NOT appear before "class" in the output
    // (JIT converts to class expression which can't be abstract)
    assert!(
        !result.code.contains("abstract class"),
        "JIT output should not contain 'abstract class'. Got:\n{}",
        result.code
    );

    // Should have proper class expression
    assert!(
        result.code.contains("let BaseProvider = class BaseProvider"),
        "JIT output should have class expression. Got:\n{}",
        result.code
    );

    // Should have __decorate call
    assert!(
        result.code.contains("__decorate("),
        "JIT output should use __decorate. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_abstract_class", result.code);
}

#[test]
fn test_jit_non_angular_class_decorators_lowered() {
    // When a class has both Angular and non-Angular class-level decorators,
    // ALL decorators must be lowered into the __decorate() call.
    // Non-Angular decorators left as raw @Decorator syntax on a class expression
    // cause TS1206 (decorators are not valid on class expressions).
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';
import { State } from '@ngxs/store';

interface TodoStateModel {
    items: string[];
}

@State<TodoStateModel>({ name: 'todo', defaults: { items: [] } })
@Injectable()
export class TodoState {}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "todo.state.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // No raw @State decorator should remain in the output
    assert!(
        !result.code.contains("@State"),
        "Non-Angular class decorators should be lowered, not left as raw syntax. Got:\n{}",
        result.code
    );

    // Both decorators should appear in the __decorate call
    assert!(
        result.code.contains("State("),
        "Non-Angular class decorator State should appear in __decorate call. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("Injectable()"),
        "Angular class decorator Injectable should appear in __decorate call. Got:\n{}",
        result.code
    );

    // Decorator order should be preserved (State before Injectable)
    let state_pos = result.code.find("State(").unwrap();
    let injectable_pos = result.code.find("Injectable()").unwrap();
    assert!(
        state_pos < injectable_pos,
        "Decorator order should be preserved (State before Injectable). Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_non_angular_class_decorators", result.code);
}

#[test]
fn test_jit_non_angular_method_decorators_lowered() {
    // Non-Angular method decorators should be lowered to __decorate() calls
    // on the class prototype (for instance methods) or class itself (for static methods).
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';
import { State, Action, Selector } from '@ngxs/store';

@State({ name: 'todo' })
@Injectable()
export class TodoState {
    @Selector()
    static todos(state: any): any[] { return state.items; }

    @Action(AddTodo)
    add(ctx: any, action: any) { ctx.setState(action); }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "todo.state.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // No raw @Selector or @Action decorator should remain
    assert!(
        !result.code.contains("@Selector"),
        "Non-Angular method decorators should be lowered. Got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("@Action"),
        "Non-Angular method decorators should be lowered. Got:\n{}",
        result.code
    );

    // Static method → __decorate([Selector()], TodoState, "todos", null)
    assert!(
        result.code.contains("__decorate([Selector()], TodoState, \"todos\", null)"),
        "Static method decorator should use class directly (no .prototype). Got:\n{}",
        result.code
    );

    // Instance method → __decorate([Action(AddTodo)], TodoState.prototype, "add", null)
    assert!(
        result.code.contains("__decorate([Action(AddTodo)], TodoState.prototype, \"add\", null)"),
        "Instance method decorator should use .prototype. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_non_angular_method_decorators", result.code);
}

#[test]
fn test_jit_full_ngxs_example() {
    // Full example with NGXS-style decorators: @State, @Selector, @Action combined with @Injectable
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';
import { State, Action, Selector, StateContext } from '@ngxs/store';

interface TodoStateModel {
    items: TodoItem[];
    filter: string;
}

interface TodoItem {
    text: string;
    done: boolean;
}

class AddTodo {
    static readonly type = '[Todo] Add';
    constructor(public text: string) {}
}

class ToggleTodo {
    static readonly type = '[Todo] Toggle';
    constructor(public index: number) {}
}

@State<TodoStateModel>({ name: 'todo', defaults: { items: [], filter: 'all' } })
@Injectable()
export class TodoState {
    @Selector()
    static todos(state: TodoStateModel): TodoItem[] { return state.items; }

    @Selector()
    static filter(state: TodoStateModel): string { return state.filter; }

    @Action(AddTodo)
    add(ctx: StateContext<TodoStateModel>, action: AddTodo) { /* ... */ }

    @Action(ToggleTodo)
    toggle(ctx: StateContext<TodoStateModel>, action: ToggleTodo) { /* ... */ }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "todo.state.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // No raw decorators should remain anywhere
    assert!(
        !result.code.contains("@State")
            && !result.code.contains("@Injectable")
            && !result.code.contains("@Selector")
            && !result.code.contains("@Action"),
        "No raw decorator syntax should remain in output. Got:\n{}",
        result.code
    );

    // Member __decorate calls should come before class __decorate
    let selector_decorate =
        result.code.find("__decorate([Selector()], TodoState, \"todos\"").unwrap();
    let class_decorate = result.code.find("TodoState = __decorate(").unwrap();
    assert!(
        selector_decorate < class_decorate,
        "Member decorators should be emitted before class decorator. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_full_ngxs_example", result.code);
}

#[test]
fn test_jit_non_angular_property_decorator_uses_void_0() {
    // TypeScript uses `void 0` (not `null`) as the 4th argument for property decorators
    // because properties don't have an existing descriptor on the prototype.
    // Methods use `null` which tells __decorate to call Object.getOwnPropertyDescriptor.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

function Validate() { return function(t: any, k: string) {}; }
function Log(target: any, key: string, desc: PropertyDescriptor) {}

@Injectable()
export class MyService {
    @Validate()
    name: string = '';

    @Log
    greet() { return 'hello'; }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "my.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Property decorator should use `void 0`
    assert!(
        result.code.contains("__decorate([Validate()], MyService.prototype, \"name\", void 0)"),
        "Property decorator should use `void 0` as 4th arg. Got:\n{}",
        result.code
    );

    // Method decorator should use `null`
    assert!(
        result.code.contains("__decorate([Log], MyService.prototype, \"greet\", null)"),
        "Method decorator should use `null` as 4th arg. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_property_decorator_void_0", result.code);
}

#[test]
fn test_jit_mixed_angular_and_non_angular_decorators_on_same_member() {
    // When a member has both Angular and non-Angular decorators, the Angular
    // decorator goes into propDecorators while the non-Angular one is lowered
    // to a __decorate() call. Both must be stripped from the class body.
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input, Output, EventEmitter } from '@angular/core';

function Required() { return function(t: any, k: string) {}; }
function Throttle(ms: number) { return function(t: any, k: string, d: any) {}; }

@Directive({ selector: '[appField]' })
export class FieldDirective {
    @Required()
    @Input()
    value: string = '';

    @Throttle(300)
    @Output()
    valueChange = new EventEmitter<string>();

    @Throttle(100)
    onChange() {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "field.directive.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // No raw decorators should remain
    assert!(
        !result.code.contains("@Required")
            && !result.code.contains("@Input")
            && !result.code.contains("@Throttle")
            && !result.code.contains("@Output"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    // Angular decorators should appear in propDecorators
    assert!(
        result.code.contains("propDecorators"),
        "Angular member decorators should be in propDecorators. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("type: Input"),
        "propDecorators should contain Input. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("type: Output"),
        "propDecorators should contain Output. Got:\n{}",
        result.code
    );

    // Non-Angular decorators should be lowered via __decorate()
    assert!(
        result
            .code
            .contains("__decorate([Required()], FieldDirective.prototype, \"value\", void 0)"),
        "Non-Angular property decorator should use __decorate with void 0. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains(
            "__decorate([Throttle(300)], FieldDirective.prototype, \"valueChange\", void 0)"
        ),
        "Non-Angular property decorator should use __decorate with void 0. Got:\n{}",
        result.code
    );
    assert!(
        result
            .code
            .contains("__decorate([Throttle(100)], FieldDirective.prototype, \"onChange\", null)"),
        "Non-Angular method decorator should use __decorate with null. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_mixed_angular_non_angular_same_member", result.code);
}

#[test]
fn test_jit_multiple_non_angular_decorators_on_same_member() {
    // Multiple non-Angular decorators on the same member should all appear
    // in a single __decorate() call for that member.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

function Log() { return function(t: any, k: string, d: any) {}; }
function Memoize() { return function(t: any, k: string, d: any) {}; }
function Validate() { return function(t: any, k: string) {}; }

@Injectable()
export class MyService {
    @Log()
    @Memoize()
    compute() { return 42; }

    @Validate()
    @Log()
    name: string = '';
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "my.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Multiple decorators on method should be in single __decorate call, in source order
    assert!(
        result
            .code
            .contains("__decorate([Log(), Memoize()], MyService.prototype, \"compute\", null)"),
        "Multiple method decorators should be in one __decorate call. Got:\n{}",
        result.code
    );

    // Multiple decorators on property should also be in single __decorate call
    assert!(
        result
            .code
            .contains("__decorate([Validate(), Log()], MyService.prototype, \"name\", void 0)"),
        "Multiple property decorators should be in one __decorate call. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_multiple_decorators_same_member", result.code);
}

#[test]
fn test_jit_multiple_decorated_classes_in_same_file() {
    // Multiple Angular-decorated classes in the same file should each get
    // their own class expression conversion and __decorate calls.
    let allocator = Allocator::default();
    let source = r"
import { Component, Injectable } from '@angular/core';

function Logger() { return function(t: any) { return t; }; }

@Component({ selector: 'app-foo', template: '<p>foo</p>' })
export class FooComponent {}

@Logger()
@Injectable()
export class FooService {
    @Logger()
    doWork() {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "foo.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Both classes should be converted to class expressions
    assert!(
        result.code.contains("let FooComponent = class FooComponent"),
        "FooComponent should be a class expression. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("let FooService = class FooService"),
        "FooService should be a class expression. Got:\n{}",
        result.code
    );

    // Both should have __decorate calls
    assert!(
        result.code.contains("FooComponent = __decorate("),
        "FooComponent should have a __decorate call. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("FooService = __decorate("),
        "FooService should have a __decorate call. Got:\n{}",
        result.code
    );

    // No raw decorators
    assert!(
        !result.code.contains("@Component")
            && !result.code.contains("@Injectable")
            && !result.code.contains("@Logger"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    // FooService should include Logger in its class __decorate
    let service_decorate_pos = result.code.find("FooService = __decorate(").unwrap();
    let service_decorate_section = &result.code[service_decorate_pos..];
    assert!(
        service_decorate_section.contains("Logger()"),
        "FooService __decorate should include Logger. Got:\n{}",
        result.code
    );

    // FooService member decorator should also be lowered
    assert!(
        result.code.contains("__decorate([Logger()], FooService.prototype, \"doWork\", null)"),
        "FooService method decorator should be lowered. Got:\n{}",
        result.code
    );

    // Both should be re-exported
    assert!(
        result.code.contains("export { FooComponent }"),
        "FooComponent should be re-exported. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("export { FooService }"),
        "FooService should be re-exported. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_multiple_classes_same_file", result.code);
}

#[test]
fn test_jit_non_exported_class_with_decorators() {
    // A non-exported Angular class with non-Angular decorators should still
    // be lowered but without an export statement.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

function Singleton() { return function(t: any) { return t; }; }

@Singleton()
@Injectable()
class InternalService {
    @Singleton()
    getInstance() {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "internal.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should be converted to class expression
    assert!(
        result.code.contains("let InternalService = class InternalService"),
        "Non-exported class should still be converted. Got:\n{}",
        result.code
    );

    // No raw decorators
    assert!(
        !result.code.contains("@Singleton") && !result.code.contains("@Injectable"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    // Should NOT have an export statement
    assert!(
        !result.code.contains("export {") && !result.code.contains("export default"),
        "Non-exported class should not get an export statement. Got:\n{}",
        result.code
    );

    // Both class decorators should be in __decorate
    assert!(
        result.code.contains("InternalService = __decorate("),
        "Should have class __decorate. Got:\n{}",
        result.code
    );

    // Member decorator should be lowered
    assert!(
        result.code.contains(
            "__decorate([Singleton()], InternalService.prototype, \"getInstance\", null)"
        ),
        "Member decorator should be lowered. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_non_exported_class", result.code);
}

#[test]
fn test_jit_default_exported_class_with_decorators() {
    // A default-exported Angular class with non-Angular decorators should
    // be lowered with `export default ClassName` at the end.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

function Logger() { return function(t: any) { return t; }; }

@Logger()
@Injectable()
export default class AppService {
    @Logger()
    process() {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "app.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Should be class expression
    assert!(
        result.code.contains("let AppService = class AppService"),
        "Default-exported class should be converted. Got:\n{}",
        result.code
    );

    // Should have `export default AppService` (not `export { AppService }`)
    assert!(
        result.code.contains("export default AppService"),
        "Should use export default. Got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("export { AppService }"),
        "Should NOT use named export for default export. Got:\n{}",
        result.code
    );

    // No raw decorators
    assert!(
        !result.code.contains("@Logger") && !result.code.contains("@Injectable"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_default_export_class", result.code);
}

#[test]
fn test_jit_getter_setter_decorators() {
    // Decorators on getter/setter methods should be lowered like regular methods
    // (using null, not void 0, since they are accessor methods not property fields).
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input } from '@angular/core';

function Validate() { return function(t: any, k: string, d: any) {}; }
function Transform() { return function(t: any, k: string, d: any) {}; }

@Directive({ selector: '[appField]' })
export class FieldDirective {
    private _value = '';

    @Validate()
    @Input()
    get value() { return this._value; }
    set value(v: string) { this._value = v; }

    @Transform()
    get computed() { return this._value.toUpperCase(); }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "field.directive.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // No raw decorators
    assert!(
        !result.code.contains("@Validate")
            && !result.code.contains("@Input")
            && !result.code.contains("@Transform"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    // Getter decorator should use null (method/accessor, not property)
    assert!(
        result.code.contains("__decorate([Validate()], FieldDirective.prototype, \"value\", null)"),
        "Getter decorator should use null (accessor). Got:\n{}",
        result.code
    );
    assert!(
        result
            .code
            .contains("__decorate([Transform()], FieldDirective.prototype, \"computed\", null)"),
        "Getter decorator should use null (accessor). Got:\n{}",
        result.code
    );

    // Angular decorator should be in propDecorators
    assert!(
        result.code.contains("type: Input"),
        "Angular getter decorator should be in propDecorators. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_getter_setter_decorators", result.code);
}

#[test]
fn test_jit_decorator_with_complex_arguments() {
    // Decorators with complex arguments (objects, arrays, arrow functions,
    // template literals) should have their argument text preserved verbatim.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

function Config(opts: any) { return function(t: any) { return t; }; }
function Transform(fn: any) { return function(t: any, k: string, d: any) {}; }

@Config({
    name: 'test',
    deps: [ServiceA, ServiceB],
    factory: () => new TestService(),
})
@Injectable()
export class TestService {
    @Transform((val: string) => val.trim())
    process() {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "test.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // No raw decorators should remain
    assert!(
        !result.code.contains("@Config")
            && !result.code.contains("@Injectable")
            && !result.code.contains("@Transform"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    // Complex arguments should be preserved in the __decorate call
    assert!(
        result.code.contains("Config("),
        "Config decorator with complex args should be in __decorate. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("factory: () => new TestService()"),
        "Arrow function argument should be preserved. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("deps: [ServiceA, ServiceB]"),
        "Array argument should be preserved. Got:\n{}",
        result.code
    );

    // Method decorator with arrow function arg
    assert!(
        result.code.contains("Transform((val) => val.trim())"),
        "Arrow function in method decorator should be preserved. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_complex_decorator_arguments", result.code);
}

#[test]
fn test_jit_angular_param_decorators_not_in_member_decorate() {
    // Angular parameter decorators (@Inject, @Optional, @Self, @SkipSelf, @Host, @Attribute)
    // should NOT be emitted in __decorate() calls if they appear on a member.
    // While these are designed for constructor params, if someone puts them on a member,
    // they should be treated as Angular decorators (not lowered via __decorate).
    let allocator = Allocator::default();
    let source = r"
import { Injectable, Inject, Optional } from '@angular/core';

function Custom() { return function(t: any, k: string) {}; }

@Injectable()
export class MyService {
    @Inject('TOKEN')
    token: any;

    @Optional()
    optionalDep: any;

    @Custom()
    customProp: string = '';
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "my.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // @Custom should be lowered via __decorate (it's non-Angular)
    assert!(
        result.code.contains("__decorate([Custom()], MyService.prototype, \"customProp\", void 0)"),
        "Non-Angular decorator should be in __decorate. Got:\n{}",
        result.code
    );

    // @Inject and @Optional should NOT appear in __decorate calls for members
    // They are Angular decorators and should not be treated as non-Angular
    let member_decorate_calls: Vec<&str> = result
        .code
        .lines()
        .filter(|l| l.contains("__decorate(") && l.contains(".prototype"))
        .collect();
    for call in &member_decorate_calls {
        assert!(
            !call.contains("Inject(") && !call.contains("Optional()"),
            "Angular param decorators should not appear in member __decorate calls. Got:\n{call}"
        );
    }

    insta::assert_snapshot!("jit_angular_param_decorators_on_members", result.code);
}

// =========================================================================
// Reference output comparison tests
// =========================================================================
// These tests compare our output against the actual output from Angular's
// official JIT compiler (@angular/compiler-cli) + TypeScript emit pipeline.
// Reference outputs were generated by compiling TypeScript files with
// Angular's downlevel_decorators_transform followed by tsc emit.

#[test]
fn test_jit_reference_ngxs_animals_state() {
    // Reference: AnimalsState from Angular's actual JIT output
    // Non-Angular @State class decorator + @Injectable, with @Selector (static) and @Action (instance)
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';
import { State, Action, Selector } from '@ngxs/store';

@State({
    name: 'animals',
    defaults: []
})
@Injectable()
class AnimalsState {
    @Selector()
    static getAnimals(state: string[]): string[] {
        return state;
    }

    @Action({ type: 'AddAnimal' })
    addAnimal(ctx: any, action: any): void {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "animals.state.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Angular reference output (from full-compiled-output.js):
    //   __decorate([Action({type:'AddAnimal'})], AnimalsState.prototype, "addAnimal", null);
    //   __decorate([Selector()], AnimalsState, "getAnimals", null);
    //   AnimalsState = __decorate([State({...}), Injectable()], AnimalsState);

    // Instance method → prototype, null
    assert!(
        result.code.contains("__decorate([Action({ type: \"AddAnimal\" })], AnimalsState.prototype, \"addAnimal\", null)"),
        "Instance method should match Angular reference output. Got:\n{}",
        result.code
    );

    // Static method → class directly, null
    assert!(
        result.code.contains("__decorate([Selector()], AnimalsState, \"getAnimals\", null)"),
        "Static method should match Angular reference output. Got:\n{}",
        result.code
    );

    // Instance __decorate calls should come before static ones (TypeScript ordering)
    let instance_pos = result.code.find("AnimalsState.prototype").unwrap();
    let static_pos = result.code.find("AnimalsState, \"getAnimals\"").unwrap();
    assert!(
        instance_pos < static_pos,
        "Instance member __decorate should come before static. Got:\n{}",
        result.code
    );

    // Class __decorate should include both State and Injectable in source order
    let class_decorate = result.code.find("AnimalsState = __decorate(").unwrap();
    let class_section = &result.code[class_decorate..];
    assert!(
        class_section.contains("State(") && class_section.contains("Injectable()"),
        "Class __decorate should include both decorators. Got:\n{}",
        result.code
    );

    // No raw decorators
    assert!(
        !result.code.contains("@State")
            && !result.code.contains("@Injectable")
            && !result.code.contains("@Selector")
            && !result.code.contains("@Action"),
        "No raw decorator syntax should remain. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("jit_reference_animals_state", result.code);
}

#[test]
fn test_jit_reference_ordering() {
    // Reference: OrderTestState from Angular's actual JIT output
    // Tests that instance members are emitted before static members,
    // each group in source order. This matches TypeScript's emit behavior.
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';
import { State, Action, Selector } from '@ngxs/store';

@State({ name: 'order', defaults: {} })
@Injectable()
class OrderTestState {
    @Action({ type: 'First' })
    instanceFirst(ctx: any): void {}

    @Selector()
    static staticSecond(state: any): any { return state; }

    @Action({ type: 'Third' })
    instanceThird(ctx: any): void {}

    @Selector()
    static staticFourth(state: any): any { return state; }
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "order.state.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Angular reference output ordering (from decorate-patterns-output.js):
    //   __decorate([Action({type:'First'})], OrderTestState.prototype, "instanceFirst", null);
    //   __decorate([Action({type:'Third'})], OrderTestState.prototype, "instanceThird", null);
    //   __decorate([Selector()], OrderTestState, "staticSecond", null);
    //   __decorate([Selector()], OrderTestState, "staticFourth", null);
    //   OrderTestState = __decorate([State({...}), Injectable()], OrderTestState);

    let first_pos = result.code.find("\"instanceFirst\"").unwrap();
    let third_pos = result.code.find("\"instanceThird\"").unwrap();
    let second_pos = result.code.find("\"staticSecond\"").unwrap();
    let fourth_pos = result.code.find("\"staticFourth\"").unwrap();
    let class_pos = result.code.find("OrderTestState = __decorate(").unwrap();

    // Instance members first (in source order)
    assert!(first_pos < third_pos, "instanceFirst before instanceThird");
    // Then static members (in source order)
    assert!(third_pos < second_pos, "instance group before static group");
    assert!(second_pos < fourth_pos, "staticSecond before staticFourth");
    // Class decorator last
    assert!(fourth_pos < class_pos, "member decorators before class decorator");

    insta::assert_snapshot!("jit_reference_ordering", result.code);
}

#[test]
fn test_jit_reference_decorate_patterns() {
    // Reference: TestDecoratePatternsService from Angular's actual JIT output
    // Tests property/method/static/getter/setter decorator patterns
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

function CustomPropDecorator(): any { return () => {}; }
function CustomMethodDecorator(): any { return () => {}; }

@Injectable()
class TestDecoratePatternsService {
    @CustomPropDecorator()
    myProp: string = 'hello';

    @CustomMethodDecorator()
    myMethod(): void {}

    @CustomMethodDecorator()
    static myStaticMethod(): void {}

    @CustomPropDecorator()
    get myGetter(): string { return ''; }

    @CustomPropDecorator()
    set mySetter(val: string) {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "patterns.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Angular reference output (from decorate-patterns-output.js):
    //   __decorate([CustomPropDecorator()], X.prototype, "myProp", void 0);
    //   __decorate([CustomMethodDecorator()], X.prototype, "myMethod", null);
    //   __decorate([CustomPropDecorator()], X.prototype, "myGetter", null);
    //   __decorate([CustomPropDecorator()], X.prototype, "mySetter", null);
    //   __decorate([CustomMethodDecorator()], X, "myStaticMethod", null);

    // Property → void 0
    assert!(
        result.code.contains("__decorate([CustomPropDecorator()], TestDecoratePatternsService.prototype, \"myProp\", void 0)"),
        "Property decorator should use void 0 (Angular reference). Got:\n{}",
        result.code
    );

    // Method → null
    assert!(
        result.code.contains("__decorate([CustomMethodDecorator()], TestDecoratePatternsService.prototype, \"myMethod\", null)"),
        "Method decorator should use null (Angular reference). Got:\n{}",
        result.code
    );

    // Static method → class, null
    assert!(
        result.code.contains("__decorate([CustomMethodDecorator()], TestDecoratePatternsService, \"myStaticMethod\", null)"),
        "Static method should use class directly (Angular reference). Got:\n{}",
        result.code
    );

    // Getter → null (accessor, not property)
    assert!(
        result.code.contains("__decorate([CustomPropDecorator()], TestDecoratePatternsService.prototype, \"myGetter\", null)"),
        "Getter should use null (Angular reference). Got:\n{}",
        result.code
    );

    // Setter → null (accessor, not property)
    assert!(
        result.code.contains("__decorate([CustomPropDecorator()], TestDecoratePatternsService.prototype, \"mySetter\", null)"),
        "Setter should use null (Angular reference). Got:\n{}",
        result.code
    );

    // Ordering: instance members first (myProp, myMethod, myGetter, mySetter), then static
    let prop_pos = result.code.find("\"myProp\"").unwrap();
    let method_pos = result.code.find("\"myMethod\"").unwrap();
    let getter_pos = result.code.find("\"myGetter\"").unwrap();
    let setter_pos = result.code.find("\"mySetter\"").unwrap();
    let static_pos = result.code.find("\"myStaticMethod\"").unwrap();

    assert!(prop_pos < static_pos, "instance before static");
    assert!(method_pos < static_pos, "instance before static");
    assert!(getter_pos < static_pos, "instance before static");
    assert!(setter_pos < static_pos, "instance before static");

    insta::assert_snapshot!("jit_reference_decorate_patterns", result.code);
}

#[test]
fn test_jit_reference_angular_member_decorators() {
    // Reference: MyService from Angular's actual JIT output
    // Angular member decorators go into propDecorators, constructor params into ctorParameters
    let allocator = Allocator::default();
    let source = r"
import { Injectable, Inject, Optional, Input, Output, ViewChild, HostListener, HostBinding, ContentChild } from '@angular/core';

@Injectable()
class MyService {
    @Input()
    myInput: string = '';

    @Output()
    myOutput: any;

    @ViewChild('ref')
    myViewChild: any;

    @HostBinding('class.active')
    isActive: boolean = false;

    @HostListener('click', ['$event'])
    onClick(event: Event): void {}

    @ContentChild('content')
    myContent: any;

    constructor(
        @Inject('TOKEN') private token: string,
        @Optional() private optService: any,
    ) {}

    normalMethod(): void {}
}
";

    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "my.service.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // Angular reference: propDecorators should contain all Angular member decorators
    // From full-compiled-output.js:
    //   static propDecorators = {
    //       myInput: [{ type: Input }],
    //       myOutput: [{ type: Output }],
    //       myViewChild: [{ type: ViewChild, args: ['ref',] }],
    //       isActive: [{ type: HostBinding, args: ['class.active',] }],
    //       onClick: [{ type: HostListener, args: ['click', ['$event'],] }],
    //       myContent: [{ type: ContentChild, args: ['content',] }]
    //   };

    assert!(
        result.code.contains("propDecorators"),
        "Should have propDecorators. Got:\n{}",
        result.code
    );
    assert!(result.code.contains("type: Input"), "propDecorators: Input. Got:\n{}", result.code);
    assert!(result.code.contains("type: Output"), "propDecorators: Output. Got:\n{}", result.code);
    assert!(
        result.code.contains("type: ViewChild"),
        "propDecorators: ViewChild. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("type: HostBinding"),
        "propDecorators: HostBinding. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("type: HostListener"),
        "propDecorators: HostListener. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("type: ContentChild"),
        "propDecorators: ContentChild. Got:\n{}",
        result.code
    );

    // Angular reference: ctorParameters should contain constructor param types and decorators
    // From full-compiled-output.js:
    //   static ctorParameters = () => [
    //       { type: String, decorators: [{ type: Inject, args: ['TOKEN',] }] },
    //       { type: undefined, decorators: [{ type: Optional }] }
    //   ];
    assert!(
        result.code.contains("ctorParameters"),
        "Should have ctorParameters. Got:\n{}",
        result.code
    );
    assert!(result.code.contains("type: Inject"), "ctorParameters: Inject. Got:\n{}", result.code);
    assert!(
        result.code.contains("type: Optional"),
        "ctorParameters: Optional. Got:\n{}",
        result.code
    );

    // No raw Angular decorators should remain
    assert!(
        !result.code.contains("@Input")
            && !result.code.contains("@Output")
            && !result.code.contains("@ViewChild")
            && !result.code.contains("@HostBinding")
            && !result.code.contains("@HostListener")
            && !result.code.contains("@ContentChild")
            && !result.code.contains("@Inject")
            && !result.code.contains("@Optional"),
        "No raw Angular decorator syntax should remain. Got:\n{}",
        result.code
    );

    // No __decorate calls for Angular member decorators (they go in propDecorators instead)
    // Only the class __decorate([Injectable()], ...) should exist
    let decorate_count = result.code.matches("__decorate(").count();
    assert!(
        decorate_count == 1,
        "Should have exactly 1 __decorate call (class only, not members). Got {} calls:\n{}",
        decorate_count,
        result.code
    );

    insta::assert_snapshot!("jit_reference_angular_member_decorators", result.code);
}

// =========================================================================
// Source map tests
// =========================================================================

#[test]
fn test_sourcemap_aot_mode() {
    // Issue #99: transformAngularFile should return a source map when sourcemap: true
    let allocator = Allocator::default();
    let source = r"import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<h1>Hello World</h1>',
    standalone: true,
})
export class TestComponent {
}
";

    let options = ComponentTransformOptions { sourcemap: true, ..Default::default() };

    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);

    assert!(
        result.map.is_some(),
        "AOT mode should return a source map when sourcemap: true, but map was None"
    );

    let map = result.map.unwrap();
    // Verify it's valid JSON
    assert!(
        map.starts_with('{'),
        "Source map should be valid JSON, got: {}",
        &map[..50.min(map.len())]
    );
    // Verify it contains expected sourcemap fields
    assert!(map.contains("\"version\":3"), "Source map should have version 3");
    assert!(map.contains("\"mappings\""), "Source map should have mappings");
    assert!(map.contains("app.component.ts"), "Source map should reference the source file");
}

#[test]
fn test_sourcemap_jit_mode() {
    // Issue #99: JIT mode should also return a source map when sourcemap: true
    let allocator = Allocator::default();
    let source = r"import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<h1>Hello World</h1>',
    standalone: true,
})
export class TestComponent {
}
";

    let options = ComponentTransformOptions { sourcemap: true, jit: true, ..Default::default() };

    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);

    assert!(
        result.map.is_some(),
        "JIT mode should return a source map when sourcemap: true, but map was None"
    );

    let map = result.map.unwrap();
    assert!(map.starts_with('{'), "Source map should be valid JSON");
    assert!(map.contains("\"version\":3"), "Source map should have version 3");
}

#[test]
fn test_sourcemap_disabled_by_default() {
    // When sourcemap is false (default), map should be None
    let allocator = Allocator::default();
    let source = r"import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<h1>Hello</h1>',
    standalone: true,
})
export class TestComponent {
}
";

    let result = transform_angular_file(&allocator, "app.component.ts", source, None, None);

    assert!(result.map.is_none(), "Source map should be None when sourcemap option is false");
}

#[test]
fn test_sourcemap_with_external_template() {
    // Source map should work with resolved external templates
    let allocator = Allocator::default();
    let source = r"import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    templateUrl: './app.html',
    standalone: true,
})
export class TestComponent {
}
";

    let mut templates = std::collections::HashMap::new();
    templates.insert("./app.html".to_string(), "<h1>Hello World</h1>".to_string());
    let resolved = ResolvedResources { templates, styles: std::collections::HashMap::new() };

    let options = ComponentTransformOptions { sourcemap: true, ..Default::default() };

    let result = transform_angular_file(
        &allocator,
        "app.component.ts",
        source,
        Some(&options),
        Some(&resolved),
    );

    assert!(
        result.map.is_some(),
        "AOT with external template should return a source map when sourcemap: true"
    );
}

#[test]
fn test_sourcemap_no_angular_classes() {
    // A file with no Angular classes should still return a source map if requested
    let allocator = Allocator::default();
    let source = r"export class PlainService {
    getData() { return 42; }
}
";

    let options = ComponentTransformOptions { sourcemap: true, ..Default::default() };

    let result = transform_angular_file(&allocator, "plain.ts", source, Some(&options), None);

    // Even for files with no Angular components, if sourcemap is requested,
    // a trivial identity source map should be returned
    assert!(
        result.map.is_some(),
        "Should return a source map even for files with no Angular classes when sourcemap: true"
    );
}

// =============================================================================
// .d.ts Declaration Generation Tests (Issue #86)
// =============================================================================

#[test]
fn test_dts_component_basic() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-hello',
  standalone: true,
  template: '<p>Hello</p>'
})
export class HelloComponent {}
";

    let result = transform_angular_file(&allocator, "hello.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    // Should have exactly one dts declaration
    assert_eq!(result.dts_declarations.len(), 1, "Should have one dts declaration");

    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "HelloComponent");

    // Should contain ɵfac declaration
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<HelloComponent, never>;"),
        "Should contain ɵfac declaration. Got:\n{}",
        decl.members
    );

    // Should contain ɵcmp declaration with correct selector
    assert!(
        decl.members
            .contains("static ɵcmp: i0.ɵɵComponentDeclaration<HelloComponent, \"app-hello\""),
        "Should contain ɵcmp declaration with selector. Got:\n{}",
        decl.members
    );

    // Should include standalone: true
    assert!(
        decl.members.contains("true, never>;"),
        "Should include standalone=true. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_with_inputs_outputs() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Input, Output, EventEmitter } from '@angular/core';

@Component({
  selector: 'app-user',
  standalone: true,
  template: '<p>{{name}}</p>'
})
export class UserComponent {
  @Input() name: string = '';
  @Input({ required: true, alias: 'userId' }) id!: number;
  @Output() clicked = new EventEmitter<void>();
}
";

    let result = transform_angular_file(&allocator, "user.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "UserComponent");

    // Should contain input map with proper metadata
    assert!(
        decl.members.contains(r#""name": { "alias": "name"; "required": false; }"#),
        "Should contain name input metadata. Got:\n{}",
        decl.members
    );
    assert!(
        decl.members.contains(r#""id": { "alias": "userId"; "required": true; }"#),
        "Should contain id input metadata with alias. Got:\n{}",
        decl.members
    );

    // Should contain output map
    assert!(
        decl.members.contains(r#""clicked": "clicked""#),
        "Should contain output metadata. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_non_standalone() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-legacy',
  standalone: false,
  template: '<p>Legacy</p>'
})
export class LegacyComponent {}
";

    let result = transform_angular_file(&allocator, "legacy.component.ts", source, None, None);
    assert!(!result.has_errors());

    let decl = &result.dts_declarations[0];
    // Should include standalone: false
    assert!(
        decl.members.contains("false, never>;"),
        "Should include standalone=false. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_with_export_as() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';

@Component({
  selector: 'app-tooltip',
  standalone: true,
  exportAs: 'tooltip',
  template: '<ng-content></ng-content>'
})
export class TooltipComponent {}
";

    let result = transform_angular_file(&allocator, "tooltip.component.ts", source, None, None);
    assert!(!result.has_errors());

    let decl = &result.dts_declarations[0];
    assert!(
        decl.members.contains(r#"["tooltip"]"#),
        "Should contain exportAs array. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_directive() {
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({
  selector: '[appHighlight]',
  standalone: true,
  exportAs: 'highlight'
})
export class HighlightDirective {
  @Input() color: string = 'yellow';
  @Output() highlighted = new EventEmitter<boolean>();
}
";

    let result = transform_angular_file(&allocator, "highlight.directive.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "HighlightDirective");

    // Should have ɵfac
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<HighlightDirective, never>;"),
        "Should contain ɵfac. Got:\n{}",
        decl.members
    );

    // Should have ɵdir (not ɵcmp)
    assert!(
        decl.members.contains(
            "static ɵdir: i0.ɵɵDirectiveDeclaration<HighlightDirective, \"[appHighlight]\""
        ),
        "Should contain ɵdir with selector. Got:\n{}",
        decl.members
    );

    // Should contain exportAs
    assert!(
        decl.members.contains(r#"["highlight"]"#),
        "Should contain exportAs. Got:\n{}",
        decl.members
    );

    // Should contain input metadata
    assert!(
        decl.members.contains(r#""color": { "alias": "color"; "required": false; }"#),
        "Should contain input metadata. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_pipe() {
    let allocator = Allocator::default();
    let source = r"
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({
  name: 'capitalize',
  standalone: true
})
export class CapitalizePipe implements PipeTransform {
  transform(value: string): string {
    return value.charAt(0).toUpperCase() + value.slice(1);
  }
}
";

    let result = transform_angular_file(&allocator, "capitalize.pipe.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "CapitalizePipe");

    // Should have ɵfac
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<CapitalizePipe, never>;"),
        "Should contain ɵfac. Got:\n{}",
        decl.members
    );

    // Should have ɵpipe with correct name and standalone
    assert!(
        decl.members
            .contains(r#"static ɵpipe: i0.ɵɵPipeDeclaration<CapitalizePipe, "capitalize", true>;"#),
        "Should contain ɵpipe declaration. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_pipe_no_name() {
    let allocator = Allocator::default();
    let source = r"
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({
  standalone: true
})
export class MyPipe implements PipeTransform {
  transform(value: string): string {
    return value;
  }
}
";

    let result = transform_angular_file(&allocator, "my.pipe.ts", source, None, None);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "MyPipe");

    // When pipe name is None, should emit `null` (not `""`)
    assert!(
        decl.members.contains("static ɵpipe: i0.ɵɵPipeDeclaration<MyPipe, null, true>;"),
        "Should use null for missing pipe name. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_ng_module() {
    let allocator = Allocator::default();
    let source = r"
import { NgModule } from '@angular/core';
import { CommonModule } from '@angular/common';

@NgModule({
  declarations: [MyComponent],
  imports: [CommonModule],
  exports: [MyComponent]
})
export class MyModule {}
";

    let result = transform_angular_file(&allocator, "my.module.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "MyModule");

    // Should have ɵfac
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<MyModule, never>;"),
        "Should contain ɵfac. Got:\n{}",
        decl.members
    );

    // Should have ɵmod with declarations, imports, exports
    assert!(
        decl.members.contains("static ɵmod: i0.ɵɵNgModuleDeclaration<MyModule,"),
        "Should contain ɵmod. Got:\n{}",
        decl.members
    );
    assert!(
        decl.members.contains("typeof MyComponent"),
        "Should reference MyComponent with typeof. Got:\n{}",
        decl.members
    );
    assert!(
        decl.members.contains("typeof CommonModule"),
        "Should reference CommonModule with typeof. Got:\n{}",
        decl.members
    );

    // Should have ɵinj
    assert!(
        decl.members.contains("static ɵinj: i0.ɵɵInjectorDeclaration<MyModule>;"),
        "Should contain ɵinj. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_injectable() {
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

@Injectable({
  providedIn: 'root'
})
export class DataService {
  getData() { return []; }
}
";

    let result = transform_angular_file(&allocator, "data.service.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "DataService");

    // Should have ɵfac
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<DataService, never>;"),
        "Should contain ɵfac. Got:\n{}",
        decl.members
    );

    // Should have ɵprov
    assert!(
        decl.members.contains("static ɵprov: i0.ɵɵInjectableDeclaration<DataService>;"),
        "Should contain ɵprov. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_generic_injectable() {
    let allocator = Allocator::default();
    let source = r"
import { Injectable } from '@angular/core';

@Injectable({
  providedIn: 'root'
})
export class GenericService<T, U> {
  getData(): T | U { return null!; }
}
";

    let result = transform_angular_file(&allocator, "generic.service.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "GenericService");

    // Should have ɵfac with type parameters filled as `any`
    assert!(
        decl.members
            .contains("static ɵfac: i0.ɵɵFactoryDeclaration<GenericService<any, any>, never>;"),
        "Should contain ɵfac with generic params. Got:\n{}",
        decl.members
    );

    // Should have ɵprov with type parameters filled as `any`
    assert!(
        decl.members
            .contains("static ɵprov: i0.ɵɵInjectableDeclaration<GenericService<any, any>>;"),
        "Should contain ɵprov with generic params. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_generic_pipe() {
    let allocator = Allocator::default();
    let source = r"
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({ name: 'genericPipe', standalone: true })
export class GenericPipe<T> implements PipeTransform {
  transform(value: T): T { return value; }
}
";

    let result = transform_angular_file(&allocator, "generic.pipe.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "GenericPipe");

    // Should have ɵfac with type parameter filled as `any`
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<GenericPipe<any>, never>;"),
        "Should contain ɵfac with generic param. Got:\n{}",
        decl.members
    );

    // Should have ɵpipe with type parameter filled as `any`
    assert!(
        decl.members.contains("i0.ɵɵPipeDeclaration<GenericPipe<any>,"),
        "Should contain ɵpipe with generic param. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_generic_directive() {
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input } from '@angular/core';

@Directive({
  selector: '[appGeneric]',
  standalone: true
})
export class GenericDirective<T, U> {
  @Input() value!: T;
  @Input() extra!: U;
}
";

    let result = transform_angular_file(&allocator, "generic.directive.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "GenericDirective");

    // Should have ɵfac with type parameters filled as `any`
    assert!(
        decl.members
            .contains("static ɵfac: i0.ɵɵFactoryDeclaration<GenericDirective<any, any>, never>;"),
        "Should contain ɵfac with generic params. Got:\n{}",
        decl.members
    );

    // Should have ɵdir with type parameters filled as `any`
    assert!(
        decl.members.contains("i0.ɵɵDirectiveDeclaration<GenericDirective<any, any>,"),
        "Should contain ɵdir with generic params. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_generic_ng_module() {
    let allocator = Allocator::default();
    let source = r"
import { NgModule } from '@angular/core';

@NgModule({})
export class GenericModule<T> {}
";

    let result = transform_angular_file(&allocator, "generic.module.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "GenericModule");

    // Should have ɵfac with type parameter filled as `any`
    assert!(
        decl.members.contains("static ɵfac: i0.ɵɵFactoryDeclaration<GenericModule<any>, never>;"),
        "Should contain ɵfac with generic param. Got:\n{}",
        decl.members
    );

    // Should have ɵmod with type parameter filled as `any`
    assert!(
        decl.members.contains("i0.ɵɵNgModuleDeclaration<GenericModule<any>,"),
        "Should contain ɵmod with generic param. Got:\n{}",
        decl.members
    );

    // Should have ɵinj with type parameter filled as `any`
    assert!(
        decl.members.contains("static ɵinj: i0.ɵɵInjectorDeclaration<GenericModule<any>>;"),
        "Should contain ɵinj with generic param. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_multiple_classes_in_file() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Injectable, Pipe, PipeTransform } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class MyService {}

@Pipe({ name: 'myPipe', standalone: true })
export class MyPipe implements PipeTransform {
  transform(v: any) { return v; }
}

@Component({
  selector: 'app-multi',
  standalone: true,
  template: '<p>{{value | myPipe}}</p>'
})
export class MultiComponent {}
";

    let result = transform_angular_file(&allocator, "multi.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    // Should have declarations for all 3 classes
    assert_eq!(
        result.dts_declarations.len(),
        3,
        "Should have 3 dts declarations. Got: {:?}",
        result.dts_declarations.iter().map(|d| &d.class_name).collect::<Vec<_>>()
    );

    let class_names: Vec<&str> =
        result.dts_declarations.iter().map(|d| d.class_name.as_str()).collect();
    assert!(class_names.contains(&"MyService"), "Should have MyService");
    assert!(class_names.contains(&"MyPipe"), "Should have MyPipe");
    assert!(class_names.contains(&"MultiComponent"), "Should have MultiComponent");
}

#[test]
fn test_dts_no_declarations_for_plain_class() {
    let allocator = Allocator::default();
    let source = r"
export class PlainClass {
  doStuff() { return 42; }
}
";

    let result = transform_angular_file(&allocator, "plain.ts", source, None, None);

    // Should have no dts declarations for plain classes
    assert!(
        result.dts_declarations.is_empty(),
        "Should have no dts declarations for plain classes"
    );
}

#[test]
fn test_dts_component_with_signal_input() {
    let allocator = Allocator::default();
    let source = r"
import { Component, input } from '@angular/core';

@Component({
  selector: 'app-signal',
  standalone: true,
  template: '<p>{{name()}}</p>'
})
export class SignalComponent {
  name = input<string>('default');
  required = input.required<number>();
}
";

    let result = transform_angular_file(&allocator, "signal.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    assert_eq!(result.dts_declarations.len(), 1);
    let decl = &result.dts_declarations[0];

    // Signal inputs should have isSignal: true in the input map
    assert!(
        decl.members.contains(r#""isSignal": true"#),
        "Signal inputs should have isSignal: true. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_ctor_deps_with_attribute() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Attribute } from '@angular/core';
import { MyService } from './my.service';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<p>Test</p>'
})
export class TestComponent {
  constructor(
    private svc: MyService,
    @Attribute('title') title: string
  ) {}
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    // When any dep has @Attribute, should emit a tuple with object types
    // First dep has no flags -> null, second has attribute -> {attribute: "title"}
    assert!(
        decl.members.contains("[null, { attribute: \"title\" }]"),
        "Should emit tuple with attribute object type. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_ctor_deps_with_optional() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Optional } from '@angular/core';
import { MyService } from './my.service';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<p>Test</p>'
})
export class TestComponent {
  constructor(
    @Optional() private svc: MyService
  ) {}
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    // @Optional without @Attribute -> dep has optional flag but no attribute
    // Since optional is a flag, it should appear in the object type
    assert!(
        decl.members.contains("[{ optional: true }]"),
        "Should emit tuple with optional object type. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_ctor_deps_no_flags() {
    let allocator = Allocator::default();
    let source = r"
import { Component } from '@angular/core';
import { MyService } from './my.service';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<p>Test</p>'
})
export class TestComponent {
  constructor(private svc: MyService) {}
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    // No special flags -> never
    assert!(
        decl.members.contains("ɵɵFactoryDeclaration<TestComponent, never>"),
        "Should emit never for deps with no flags. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_directive_ctor_deps_with_optional_and_host() {
    let allocator = Allocator::default();
    let source = r"
import { Directive, Optional, Host } from '@angular/core';
import { MyService } from './my.service';
import { OtherService } from './other.service';

@Directive({
  selector: '[appTest]',
  standalone: true,
})
export class TestDirective {
  constructor(
    @Optional() @Host() private svc: MyService,
    private other: OtherService
  ) {}
}
";

    let result = transform_angular_file(&allocator, "test.directive.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    // First dep has optional + host, second has no flags -> null
    assert!(
        decl.members.contains("[{ optional: true, host: true }, null]"),
        "Should emit tuple with optional+host object type and null. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_with_ng_content_selectors() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
  selector: 'app-layout',
  standalone: true,
  template: `
    <ng-content select="header"></ng-content>
    <ng-content></ng-content>
    <ng-content select=".footer"></ng-content>
  `
})
export class LayoutComponent {}
"#;

    let result = transform_angular_file(&allocator, "layout.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];

    // Should contain ng-content selectors as a tuple type in the ɵcmp declaration
    assert!(
        decl.members.contains(r#"["header", "*", ".footer"]"#),
        "Should contain ng-content selectors tuple. Got:\n{}",
        decl.members
    );

    // Verify the full ɵcmp declaration structure has selectors in the correct position
    // (after QueryFields=never, before IsStandalone=true)
    assert!(
        decl.members.contains(r#"never, ["header", "*", ".footer"], true"#),
        "NgContentSelectors should be in the correct position between QueryFields and IsStandalone. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_component_with_input_transform() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Input, booleanAttribute } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<p>test</p>'
})
export class TestComponent {
  @Input({transform: booleanAttribute}) disabled: boolean = false;
  @Input() name: string = '';
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "TestComponent");

    // Input with transform should get ngAcceptInputType_* field
    assert!(
        decl.members.contains("static ngAcceptInputType_disabled: unknown;"),
        "Should contain ngAcceptInputType_disabled for transformed input. Got:\n{}",
        decl.members
    );

    // Input without transform should NOT get ngAcceptInputType_* field
    assert!(
        !decl.members.contains("ngAcceptInputType_name"),
        "Should NOT contain ngAcceptInputType_name for non-transformed input. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_directive_with_input_transform() {
    let allocator = Allocator::default();
    let source = r"
import { Directive, Input, booleanAttribute } from '@angular/core';

@Directive({
  selector: '[appTest]',
  standalone: true,
})
export class TestDirective {
  @Input({transform: booleanAttribute}) disabled: boolean = false;
  @Input() name: string = '';
}
";

    let result = transform_angular_file(&allocator, "test.directive.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "TestDirective");

    // Input with transform should get ngAcceptInputType_* field
    assert!(
        decl.members.contains("static ngAcceptInputType_disabled: unknown;"),
        "Should contain ngAcceptInputType_disabled for transformed input. Got:\n{}",
        decl.members
    );

    // Input without transform should NOT get ngAcceptInputType_* field
    assert!(
        !decl.members.contains("ngAcceptInputType_name"),
        "Should NOT contain ngAcceptInputType_name for non-transformed input. Got:\n{}",
        decl.members
    );
}

#[test]
fn test_dts_signal_input_with_transform_no_accept_type() {
    let allocator = Allocator::default();
    let source = r"
import { Component, input, booleanAttribute } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<p>test</p>'
})
export class TestComponent {
  disabled = input(false, {transform: booleanAttribute});
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should compile without errors: {:?}", result.diagnostics);

    let decl = &result.dts_declarations[0];
    assert_eq!(decl.class_name, "TestComponent");

    // Signal inputs should NOT get ngAcceptInputType_* fields
    assert!(
        !decl.members.contains("ngAcceptInputType_"),
        "Signal inputs should NOT get ngAcceptInputType_* fields. Got:\n{}",
        decl.members
    );
}

// ============================================================================
// Angular Version Gating Tests (Issue #105)
// ============================================================================
// These tests verify that when targeting Angular 19, the compiler emits
// ɵɵtemplate instead of ɵɵconditionalCreate/ɵɵconditionalBranchCreate
// for @if/@switch blocks, since those instructions don't exist in Angular 19.

#[test]
fn test_if_block_angular_v19() {
    let v19 = AngularVersion::new(19, 0, 0);
    let js = compile_template_to_js_with_version(
        r"@if (condition) { <div>Visible</div> }",
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use ɵɵtemplate, NOT ɵɵconditionalCreate
    assert!(
        js.contains("ɵɵtemplate("),
        "Angular 19 should emit ɵɵtemplate for @if blocks. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵconditionalCreate("),
        "Angular 19 should NOT emit ɵɵconditionalCreate. Got:\n{js}"
    );
    // Update instruction (ɵɵconditional) should still be emitted
    assert!(
        js.contains("ɵɵconditional("),
        "Angular 19 should still emit ɵɵconditional for update. Got:\n{js}"
    );
    insta::assert_snapshot!("if_block_angular_v19", js);
}

#[test]
fn test_if_else_block_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r"@if (condition) { <div>True</div> } @else { <div>False</div> }",
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use ɵɵtemplate for all branches, NOT conditionalCreate/conditionalBranchCreate
    assert!(
        js.contains("ɵɵtemplate("),
        "Angular 19 should emit ɵɵtemplate for @if/@else blocks. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵconditionalCreate("),
        "Angular 19 should NOT emit ɵɵconditionalCreate. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵconditionalBranchCreate("),
        "Angular 19 should NOT emit ɵɵconditionalBranchCreate. Got:\n{js}"
    );
    insta::assert_snapshot!("if_else_block_angular_v19", js);
}

#[test]
fn test_switch_block_angular_v19() {
    let v19 = AngularVersion::new(19, 0, 0);
    let js = compile_template_to_js_with_version(
        r"@switch (value) { @case (1) { <div>One</div> } @case (2) { <div>Two</div> } @default { <div>Other</div> } }",
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use ɵɵtemplate for all @switch cases
    assert!(
        js.contains("ɵɵtemplate("),
        "Angular 19 should emit ɵɵtemplate for @switch blocks. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵconditionalCreate("),
        "Angular 19 should NOT emit ɵɵconditionalCreate for @switch. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵconditionalBranchCreate("),
        "Angular 19 should NOT emit ɵɵconditionalBranchCreate for @switch. Got:\n{js}"
    );
    insta::assert_snapshot!("switch_block_angular_v19", js);
}

#[test]
fn test_if_block_angular_v20_default() {
    // Default (no version set) should emit conditionalCreate (Angular 20+ behavior)
    let js = compile_template_to_js_with_version(
        r"@if (condition) { <div>Visible</div> }",
        "TestComponent",
        None,
    );
    assert!(
        js.contains("ɵɵconditionalCreate("),
        "Default (latest) should emit ɵɵconditionalCreate. Got:\n{js}"
    );
}

#[test]
fn test_if_block_angular_v20_explicit() {
    let v20 = AngularVersion::new(20, 0, 0);
    let js = compile_template_to_js_with_version(
        r"@if (condition) { <div>Visible</div> }",
        "TestComponent",
        Some(v20),
    );
    assert!(
        js.contains("ɵɵconditionalCreate("),
        "Angular 20 should emit ɵɵconditionalCreate. Got:\n{js}"
    );
}

// ============================================================================
// Angular 19 Property/Attribute Interpolation Version Gating (Issue #107)
// ============================================================================
// These tests verify that when targeting Angular 19, the compiler emits
// ɵɵpropertyInterpolate*/ɵɵattributeInterpolate* instead of
// ɵɵproperty + nested ɵɵinterpolate*, and ɵɵhostProperty instead of ɵɵdomProperty.

#[test]
fn test_property_interpolation_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div [title]="'Hello ' + name">static</div>"#,
        "TestComponent",
        Some(v19),
    );
    // Non-interpolation property bindings should still use ɵɵproperty
    assert!(
        js.contains("ɵɵproperty("),
        "Angular 19 should use ɵɵproperty for non-interpolation bindings. Got:\n{js}"
    );
    // Should NOT emit ɵɵinterpolate for non-interpolation bindings
    assert!(
        !js.contains("ɵɵinterpolate"),
        "Angular 19 should NOT emit ɵɵinterpolate for non-interpolation bindings. Got:\n{js}"
    );
}

#[test]
fn test_property_interpolation_angular_v19_with_interpolation() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div title="Hello {{name}}">static</div>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use combined ɵɵpropertyInterpolate1 instruction
    assert!(
        js.contains("ɵɵpropertyInterpolate1("),
        "Angular 19 should emit ɵɵpropertyInterpolate1 for property interpolation. Got:\n{js}"
    );
    // Should NOT emit standalone ɵɵinterpolate
    assert!(
        !js.contains("ɵɵinterpolate1("),
        "Angular 19 should NOT emit standalone ɵɵinterpolate1. Got:\n{js}"
    );
    insta::assert_snapshot!("property_interpolation_angular_v19", js);
}

#[test]
fn test_property_interpolation_angular_v19_multiple() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div title="{{first}} and {{second}}">static</div>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use ɵɵpropertyInterpolate2 for 2 expressions
    assert!(
        js.contains("ɵɵpropertyInterpolate2("),
        "Angular 19 should emit ɵɵpropertyInterpolate2 for 2-expression interpolation. Got:\n{js}"
    );
    insta::assert_snapshot!("property_interpolation_angular_v19_multiple", js);
}

#[test]
fn test_property_interpolation_angular_v19_simple() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div title="{{name}}">static</div>"#,
        "TestComponent",
        Some(v19),
    );
    // Single expression with empty strings → ɵɵpropertyInterpolate
    assert!(
        js.contains("ɵɵpropertyInterpolate("),
        "Angular 19 should emit ɵɵpropertyInterpolate for simple interpolation. Got:\n{js}"
    );
    insta::assert_snapshot!("property_interpolation_angular_v19_simple", js);
}

#[test]
fn test_attribute_interpolation_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<svg attr.viewBox="0 0 {{svgSize}} {{svgSize}}"></svg>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use combined ɵɵattributeInterpolate2 instruction
    assert!(
        js.contains("ɵɵattributeInterpolate2("),
        "Angular 19 should emit ɵɵattributeInterpolate2 for attribute interpolation. Got:\n{js}"
    );
    // Should NOT emit standalone ɵɵinterpolate
    assert!(
        !js.contains("ɵɵinterpolate2("),
        "Angular 19 should NOT emit standalone ɵɵinterpolate2. Got:\n{js}"
    );
    insta::assert_snapshot!("attribute_interpolation_angular_v19", js);
}

#[test]
fn test_property_interpolation_angular_v20_default() {
    // Default (no version set) should use ɵɵproperty + ɵɵinterpolate1 (Angular 20+ behavior)
    let js = compile_template_to_js_with_version(
        r#"<div title="Hello {{name}}">static</div>"#,
        "TestComponent",
        None,
    );
    assert!(
        js.contains("ɵɵinterpolate1("),
        "Default (latest) should emit ɵɵinterpolate1. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵpropertyInterpolate"),
        "Default (latest) should NOT emit ɵɵpropertyInterpolate. Got:\n{js}"
    );
}

#[test]
fn test_property_interpolation_angular_v20_explicit() {
    let v20 = AngularVersion::new(20, 0, 0);
    let js = compile_template_to_js_with_version(
        r#"<div title="Hello {{name}}">static</div>"#,
        "TestComponent",
        Some(v20),
    );
    assert!(js.contains("ɵɵinterpolate1("), "Angular 20 should emit ɵɵinterpolate1. Got:\n{js}");
    assert!(
        !js.contains("ɵɵpropertyInterpolate"),
        "Angular 20 should NOT emit ɵɵpropertyInterpolate. Got:\n{js}"
    );
}

// ========================================================================
// Angular 19 version-gating: stylePropInterpolate, styleMapInterpolate, classMapInterpolate
// ========================================================================

#[test]
fn test_style_prop_interpolation_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    // Use interpolation syntax (not binding syntax) for style prop
    let js = compile_template_to_js_with_version(
        r#"<div style.width="prefix{{expr}}suffix"></div>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use combined ɵɵstylePropInterpolate1 instruction
    assert!(
        js.contains("ɵɵstylePropInterpolate1("),
        "Angular 19 should emit ɵɵstylePropInterpolate1 for style prop interpolation. Got:\n{js}"
    );
    // Should NOT use standalone interpolate nested in styleProp
    assert!(
        !js.contains("ɵɵinterpolate1("),
        "Angular 19 should NOT emit standalone ɵɵinterpolate1 for style prop. Got:\n{js}"
    );
    insta::assert_snapshot!("style_prop_interpolation_angular_v19", js);
}

#[test]
fn test_style_prop_interpolation_angular_v19_with_unit() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div style.width.px="prefix{{size}}suffix"></div>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use combined ɵɵstylePropInterpolate1 with unit
    assert!(
        js.contains("ɵɵstylePropInterpolate1("),
        "Angular 19 should emit ɵɵstylePropInterpolate1 for style prop with unit. Got:\n{js}"
    );
    insta::assert_snapshot!("style_prop_interpolation_angular_v19_with_unit", js);
}

#[test]
fn test_style_prop_interpolation_angular_v20() {
    let v20 = AngularVersion::new(20, 0, 0);
    let js = compile_template_to_js_with_version(
        r#"<div style.width="prefix{{expr}}suffix"></div>"#,
        "TestComponent",
        Some(v20),
    );
    // Angular 20+ should use styleProp + standalone interpolate
    assert!(
        !js.contains("ɵɵstylePropInterpolate"),
        "Angular 20 should NOT emit ɵɵstylePropInterpolate. Got:\n{js}"
    );
}

#[test]
fn test_style_map_interpolation_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div style="width:{{expr}}px"></div>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use combined ɵɵstyleMapInterpolate1 instruction
    assert!(
        js.contains("ɵɵstyleMapInterpolate1("),
        "Angular 19 should emit ɵɵstyleMapInterpolate1 for style map interpolation. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵinterpolate1("),
        "Angular 19 should NOT emit standalone ɵɵinterpolate1 for style map. Got:\n{js}"
    );
    insta::assert_snapshot!("style_map_interpolation_angular_v19", js);
}

#[test]
fn test_style_map_interpolation_angular_v20() {
    let v20 = AngularVersion::new(20, 0, 0);
    let js = compile_template_to_js_with_version(
        r#"<div style="width:{{expr}}px"></div>"#,
        "TestComponent",
        Some(v20),
    );
    // Angular 20+ should use styleMap + standalone interpolate
    assert!(
        !js.contains("ɵɵstyleMapInterpolate"),
        "Angular 20 should NOT emit ɵɵstyleMapInterpolate. Got:\n{js}"
    );
}

#[test]
fn test_class_map_interpolation_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    let js = compile_template_to_js_with_version(
        r#"<div class="prefix{{expr}}suffix"></div>"#,
        "TestComponent",
        Some(v19),
    );
    // Angular 19 should use combined ɵɵclassMapInterpolate1 instruction
    assert!(
        js.contains("ɵɵclassMapInterpolate1("),
        "Angular 19 should emit ɵɵclassMapInterpolate1 for class map interpolation. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵinterpolate1("),
        "Angular 19 should NOT emit standalone ɵɵinterpolate1 for class map. Got:\n{js}"
    );
    insta::assert_snapshot!("class_map_interpolation_angular_v19", js);
}

#[test]
fn test_class_map_interpolation_angular_v20() {
    let v20 = AngularVersion::new(20, 0, 0);
    let js = compile_template_to_js_with_version(
        r#"<div class="prefix{{expr}}suffix"></div>"#,
        "TestComponent",
        Some(v20),
    );
    // Angular 20+ should use classMap + standalone interpolate
    assert!(
        !js.contains("ɵɵclassMapInterpolate"),
        "Angular 20 should NOT emit ɵɵclassMapInterpolate. Got:\n{js}"
    );
}

#[test]
fn test_style_prop_singleton_collapsed_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    // Singleton interpolation style.color="{{expr}}" is collapsed to plain styleProp
    // for both v19 and v20+. Angular v19's collateInterpolationArgs maps singleton
    // empty-string interpolations to the plain instruction (index 0).
    let js = compile_template_to_js_with_version(
        r#"<div style.color="{{color}}"></div>"#,
        "TestComponent",
        Some(v19),
    );
    assert!(
        js.contains("ɵɵstyleProp("),
        "Angular 19 singleton style interpolation should use plain ɵɵstyleProp. Got:\n{js}"
    );
    // Should NOT use standalone interpolate or combined interpolate
    assert!(
        !js.contains("ɵɵinterpolate1(") && !js.contains("ɵɵstylePropInterpolate"),
        "Angular 19 singleton should NOT emit interpolate instructions. Got:\n{js}"
    );
}

#[test]
fn test_property_singleton_interpolation_with_sanitizer_angular_v19() {
    let v19 = AngularVersion::new(19, 2, 0);
    // Singleton property interpolation `src="{{url}}"` with sanitizer (img src → URL sanitizer).
    // Must select ɵɵpropertyInterpolate (non-numbered), NOT ɵɵpropertyInterpolate1.
    // propertyInterpolate(propName, value, sanitizer?) — takes value directly.
    // propertyInterpolate1(propName, prefix, v0, suffix, sanitizer?) — expects prefix/suffix.
    let js =
        compile_template_to_js_with_version(r#"<img src="{{url}}">"#, "TestComponent", Some(v19));
    // Must use ɵɵpropertyInterpolate (simple variant), not ɵɵpropertyInterpolate1
    assert!(
        js.contains("ɵɵpropertyInterpolate("),
        "Singleton interpolation with sanitizer should use ɵɵpropertyInterpolate, not ɵɵpropertyInterpolate1. Got:\n{js}"
    );
    assert!(
        !js.contains("ɵɵpropertyInterpolate1("),
        "Should NOT emit ɵɵpropertyInterpolate1 for singleton. Got:\n{js}"
    );
    // Must include sanitizer function
    assert!(js.contains("ɵɵsanitizeUrl"), "Should include ɵɵsanitizeUrl sanitizer. Got:\n{js}");
    insta::assert_snapshot!("property_singleton_interpolation_with_sanitizer_v19", js);
}

// ============================================================================
// Host Directive Alias Tests
// ============================================================================

/// Test host directives with simple aliased inputs/outputs.
///
/// Mirrors the compliance test `host_directives_with_inputs_outputs.ts`.
/// The mapping array must use `[internalName, publicName]` ordering.
#[test]
fn test_host_directives_with_inputs_outputs() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Directive, EventEmitter, Input, Output } from '@angular/core';

@Directive({})
export class HostDir {
  @Input() value = 0;
  @Input() color = '';
  @Output() opened = new EventEmitter();
  @Output() closed = new EventEmitter();
}

@Component({
    selector: 'my-component',
    template: '',
    hostDirectives: [{
        directive: HostDir,
        inputs: ['value', 'color: colorAlias'],
        outputs: ['opened', 'closed: closedAlias'],
    }],
    standalone: false,
})
export class MyComponent {
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");

    // Input mappings: 'value' (no alias) → ["value", "value"], 'color: colorAlias' → ["color", "colorAlias"]
    // The array must be [internalName, publicName, ...] i.e. ["value", "value", "color", "colorAlias"]
    assert!(
        normalized.contains(r#"inputs:["value","value","color","colorAlias"]"#),
        "Input mappings should be [internalName, publicName]. Got:\n{}",
        result.code
    );

    // Output mappings: 'opened' → ["opened", "opened"], 'closed: closedAlias' → ["closed", "closedAlias"]
    assert!(
        normalized.contains(r#"outputs:["opened","opened","closed","closedAlias"]"#),
        "Output mappings should be [internalName, publicName]. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("host_directives_with_inputs_outputs", result.code);
}

/// Test host directives where the directive has `@Input('alias')` and the host re-aliases.
///
/// Mirrors the compliance test `host_directives_with_host_aliases.ts`.
#[test]
fn test_host_directives_with_host_aliases() {
    let allocator = Allocator::default();
    let source = r"
import { Component, Directive, EventEmitter, Input, Output } from '@angular/core';

@Directive({})
export class HostDir {
  @Input('valueAlias') value = 1;
  @Input('colorAlias') color = '';
  @Output('openedAlias') opened = new EventEmitter();
  @Output('closedAlias') closed = new EventEmitter();
}

@Component({
    selector: 'my-component',
    template: '',
    hostDirectives: [{
        directive: HostDir,
        inputs: ['valueAlias', 'colorAlias: customColorAlias'],
        outputs: ['openedAlias', 'closedAlias: customClosedAlias'],
    }],
    standalone: false,
})
export class MyComponent {
}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");

    // Input mappings: 'valueAlias' → ["valueAlias", "valueAlias"], 'colorAlias: customColorAlias' → ["colorAlias", "customColorAlias"]
    assert!(
        normalized
            .contains(r#"inputs:["valueAlias","valueAlias","colorAlias","customColorAlias"]"#),
        "Input mappings should be [internalName, publicName]. Got:\n{}",
        result.code
    );

    // Output mappings: 'openedAlias' → ["openedAlias", "openedAlias"], 'closedAlias: customClosedAlias' → ["closedAlias", "customClosedAlias"]
    assert!(
        normalized
            .contains(r#"outputs:["openedAlias","openedAlias","closedAlias","customClosedAlias"]"#),
        "Output mappings should be [internalName, publicName]. Got:\n{}",
        result.code
    );

    insta::assert_snapshot!("host_directives_with_host_aliases", result.code);
}

// =============================================================================
// Issue #203: useFactory with block-body functions silently dropped in providers
// =============================================================================

#[test]
fn test_use_factory_block_body_arrow_preserved() {
    let allocator = Allocator::default();
    let source = r"
import { Component, inject } from '@angular/core';

const MY_TOKEN = 'MY_TOKEN';

@Component({
    selector: 'my-component',
    template: '<div>hello</div>',
    providers: [
        {
            provide: MY_TOKEN,
            useFactory: () => {
                const config = inject(AppConfig);
                if (config.useMock) {
                    return new MockService();
                }
                return new RealService(config);
            }
        }
    ]
})
export class MyComponent {}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    assert_eq!(result.component_count, 1);

    // The key assertion: the block-body arrow function should be preserved intact.
    // Before the fix, `const config = inject(AppConfig)` and `if (config.useMock) { ... }`
    // were silently dropped, leaving only `return new RealService(config)`.
    assert!(
        result.code.contains("const config = inject(AppConfig)"),
        "Block-body arrow: const declaration should be preserved. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("if (config.useMock)"),
        "Block-body arrow: if statement should be preserved. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("return new MockService()"),
        "Block-body arrow: return inside if should be preserved. Got:\n{}",
        result.code
    );
    assert!(
        result.code.contains("return new RealService(config)"),
        "Block-body arrow: final return should be preserved. Got:\n{}",
        result.code
    );
}

#[test]
fn test_use_factory_expression_body_arrow_still_works() {
    // Verify that expression-body arrows (which already worked) are not regressed
    let allocator = Allocator::default();
    let source = r"
import { Component, inject } from '@angular/core';

const MY_TOKEN = 'MY_TOKEN';

@Component({
    selector: 'my-component',
    template: '<div>hello</div>',
    providers: [
        {
            provide: MY_TOKEN,
            useFactory: () => new RealService(inject(AppConfig))
        }
    ]
})
export class MyComponent {}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    assert!(
        result.code.contains("new RealService(inject(AppConfig))"),
        "Expression-body arrow should be preserved. Got:\n{}",
        result.code
    );
}

#[test]
fn test_providers_with_function_expression_preserved() {
    // function() expressions should also be preserved
    let allocator = Allocator::default();
    let source = r"
import { Component, inject } from '@angular/core';

const MY_TOKEN = 'MY_TOKEN';

@Component({
    selector: 'my-component',
    template: '<div>hello</div>',
    providers: [
        {
            provide: MY_TOKEN,
            useFactory: function() { return new RealService(); }
        }
    ]
})
export class MyComponent {}
";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    assert!(
        result.code.contains("function()"),
        "function expression should be preserved. Got:\n{}",
        result.code
    );
}

// =============================================================================
// Regression: @Inject(TOKEN) on pipe constructor parameters
// =============================================================================
// The `extract_param_dependency` function in `pipe/decorator.rs` previously did
// not handle the `@Inject(TOKEN)` decorator, so the token was silently taken
// from the TypeScript type annotation instead. When the type was an interface
// (erased at runtime) this left the DI token undefined, which Angular 20's
// `assertDefined(token)` guard rejects immediately. See commit b2dd390.

/// `@Inject(TOKEN)` on a pipe constructor param must produce a factory that
/// injects the TOKEN identifier, not the erased type annotation.
#[test]
fn test_pipe_factory_uses_inject_token_over_interface_type() {
    let allocator = Allocator::default();
    let source = r"
import { Pipe, PipeTransform, Inject, InjectionToken } from '@angular/core';

export interface Config {
    locale: string;
}

export const CONFIG = new InjectionToken<Config>('CONFIG');

@Pipe({ name: 'localized', standalone: true })
export class LocalizedPipe implements PipeTransform {
    constructor(@Inject(CONFIG) private config: Config) {}
    transform(value: string): string { return value; }
}
";

    let result = transform_angular_file(&allocator, "localized.pipe.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");

    // Factory must inject CONFIG (the @Inject token), not Config (the interface type).
    assert!(
        factory_section.contains("CONFIG"),
        "Pipe factory should inject the @Inject(CONFIG) token. Factory:\n{factory_section}"
    );
    assert!(
        !factory_section.contains("directiveInject(Config)")
            && !factory_section.contains("ɵɵinject(Config)"),
        "Pipe factory must NOT inject the erased interface type 'Config'. Factory:\n{factory_section}"
    );
}

/// When `@Inject(TOKEN)` is used alongside modifier decorators (`@Optional`,
/// `@SkipSelf`), the factory must still pick up the TOKEN and forward the
/// correct DI flags.
#[test]
fn test_pipe_factory_inject_token_with_optional_skip_self() {
    let allocator = Allocator::default();
    let source = r"
import { Pipe, PipeTransform, Inject, Optional, SkipSelf, InjectionToken } from '@angular/core';

export const MY_TOKEN = new InjectionToken<string>('MY_TOKEN');

@Pipe({ name: 'tagged', standalone: true })
export class TaggedPipe implements PipeTransform {
    constructor(
        @Optional() @SkipSelf() @Inject(MY_TOKEN) private tag: string | null,
    ) {}
    transform(value: string): string { return value; }
}
";

    let result = transform_angular_file(&allocator, "tagged.pipe.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");

    // MY_TOKEN must be present in the factory as the DI token.
    assert!(
        factory_section.contains("directiveInject(MY_TOKEN"),
        "Pipe factory should inject MY_TOKEN. Factory:\n{factory_section}"
    );

    // The DI flag bitmask must include Optional (8) | SkipSelf (4). Angular's
    // pipe compilation also ORs in ForPipe (16), yielding 28. We only require
    // that both Optional and SkipSelf bits are set.
    let flags = factory_section
        .split("directiveInject(MY_TOKEN,")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .expect("Factory should encode numeric DI flags");
    assert!(
        flags & 8 != 0 && flags & 4 != 0,
        "Pipe factory flags should include Optional (8) and SkipSelf (4). Got: {flags}. Factory:\n{factory_section}"
    );
}

/// Without `@Inject`, the factory must still fall back to the type annotation
/// so that plain class-typed dependencies continue to resolve correctly.
#[test]
fn test_pipe_factory_without_inject_still_uses_type_annotation() {
    let allocator = Allocator::default();
    let source = r"
import { Pipe, PipeTransform } from '@angular/core';

export class Logger {
    log(msg: string): void {}
}

@Pipe({ name: 'logged', standalone: true })
export class LoggedPipe implements PipeTransform {
    constructor(private logger: Logger) {}
    transform(value: string): string { return value; }
}
";

    let result = transform_angular_file(&allocator, "logged.pipe.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;
    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");

    assert!(
        factory_section.contains("Logger"),
        "Pipe factory should fall back to the type annotation (Logger). Factory:\n{factory_section}"
    );
}

// ============================================================================
// outputFromObservable tests
// ============================================================================

#[test]
fn test_output_from_observable_simple() {
    let allocator = Allocator::default();
    let source = r#"
import { Component, EventEmitter } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
    selector: 'test-comp',
    standalone: true,
    template: '',
})
export class TestComponent {
    readonly queryChanged = outputFromObservable(new EventEmitter<string>());
}
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"ɵɵdefineComponent("#)
            && normalized.contains(r#"outputs:{queryChanged:"queryChanged"}"#),
        "outputFromObservable property should appear in outputs:{{}} inside ɵɵdefineComponent.\nCode:\n{}",
        result.code
    );
    insta::assert_snapshot!("output_from_observable_simple", result.code);
}

#[test]
fn test_output_from_observable_property_reference() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
    selector: 'test-comp',
    standalone: true,
    template: '',
})
export class TestComponent {
    readonly valueChanged = outputFromObservable(this.someService.value$);
}
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"ɵɵdefineComponent("#)
            && normalized.contains(r#"outputs:{valueChanged:"valueChanged"}"#),
        "outputFromObservable with property reference should appear in outputs:{{}} inside ɵɵdefineComponent.\nCode:\n{}",
        result.code
    );
    insta::assert_snapshot!("output_from_observable_property_reference", result.code);
}

#[test]
fn test_output_from_observable_piped() {
    // Regression: the reported real-world case — complex piped observable as argument.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
    selector: 'test-comp',
    standalone: true,
    template: '',
})
export class TestComponent {
    readonly queryChanged = outputFromObservable(
        this.dataService.value$.pipe(
            skip(1),
            debounceTime(300),
        ),
    );
}
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"ɵɵdefineComponent("#)
            && normalized.contains(r#"outputs:{queryChanged:"queryChanged"}"#),
        "outputFromObservable with piped observable should appear in outputs:{{}} inside ɵɵdefineComponent.\nCode:\n{}",
        result.code
    );
    insta::assert_snapshot!("output_from_observable_piped", result.code);
}

#[test]
fn test_output_from_observable_with_alias() {
    // The alias option is the second argument; the class property name maps to the alias binding name.
    let allocator = Allocator::default();
    let source = r#"
import { Component, EventEmitter } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
    selector: 'test-comp',
    standalone: true,
    template: '',
})
export class TestComponent {
    readonly _clicked = outputFromObservable(new EventEmitter<void>(), { alias: 'clicked' });
}
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    // Class property name '_clicked' must map to binding name 'clicked' (the alias value).
    assert!(
        normalized.contains(r#"ɵɵdefineComponent("#)
            && normalized.contains(r#"outputs:{_clicked:"clicked"}"#),
        "outputFromObservable alias should become the binding property name in outputs:{{}}.\nCode:\n{}",
        result.code
    );
    // Also verify the class property name itself is NOT used as the binding name.
    assert!(
        !normalized.contains(r#"outputs:{_clicked:"_clicked"}"#),
        "Class property name '_clicked' should NOT be used as binding name when alias is set.\nCode:\n{}",
        result.code
    );
    insta::assert_snapshot!("output_from_observable_with_alias", result.code);
}

#[test]
fn test_output_from_observable_mixed_with_output() {
    // Both output() and outputFromObservable() in the same class must both appear in outputs.
    let allocator = Allocator::default();
    let source = r#"
import { Component, EventEmitter, output } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
    selector: 'test-comp',
    standalone: true,
    template: '',
})
export class TestComponent {
    readonly clicked = output<void>();
    readonly queryChanged = outputFromObservable(new EventEmitter<string>());
}
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"ɵɵdefineComponent("#)
            && normalized.contains(r#"outputs:{clicked:"clicked",queryChanged:"queryChanged"}"#),
        "Both output() and outputFromObservable() must appear in outputs:{{}} inside ɵɵdefineComponent.\nCode:\n{}",
        result.code
    );
    insta::assert_snapshot!("output_from_observable_mixed_with_output", result.code);
}

/// Host attribute key referencing a same-file `const` must emit `hostAttrs` in
/// `ɵɵdefineDirective`, matching the official Angular compiler.
#[test]
fn host_attribute_identifier_key_emits_host_attrs() {
    let allocator = Allocator::default();
    let source = r#"
import { Directive } from '@angular/core';

export const MARKER_ATTR = 'data-marker';

@Directive({
    selector: '[marker]',
    host: { [MARKER_ATTR]: '' },
})
export class MarkerDirective {}
"#;
    let result = transform_angular_file(&allocator, "marker.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"hostAttrs:["data-marker",""]"#),
        "Expected hostAttrs:[\"data-marker\",\"\"] in directive definition.\nCode:\n{}",
        result.code
    );
}

/// Host attribute value referencing a same-file `const` must resolve and emit
/// `hostAttrs` with the resolved string.
#[test]
fn host_attribute_identifier_value_emits_host_attrs() {
    let allocator = Allocator::default();
    let source = r#"
import { Directive } from '@angular/core';

const BTN_TYPE = 'submit';

@Directive({
    selector: '[d]',
    host: { type: BTN_TYPE },
})
export class D {}
"#;
    let result = transform_angular_file(&allocator, "d.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"hostAttrs:["type","submit"]"#),
        "Expected hostAttrs:[\"type\",\"submit\"] in directive definition.\nCode:\n{}",
        result.code
    );
}

/// An unresolved identifier in a computed host key must be silently dropped —
/// matching existing behavior for any unrecognized host metadata.
#[test]
fn host_attribute_unknown_identifier_dropped() {
    let allocator = Allocator::default();
    let source = r#"
import { Directive } from '@angular/core';

@Directive({
    selector: '[d]',
    host: { [UNRESOLVED]: '' },
})
export class D {}
"#;
    let result = transform_angular_file(&allocator, "d.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        !normalized.contains("hostAttrs:"),
        "Unresolved identifier must not produce hostAttrs entry.\nCode:\n{}",
        result.code
    );
}

// ---------------------------------------------------------------------------
// Issue #286: `${...}` template-literal interpolation in decorator metadata.
//
// Angular's partial evaluator constant-folds template literals whose `${...}`
// expressions reference same-file `const`-bound string values. OXC must match
// this behavior for `template:`, `selector:`, `styles:`, etc. — otherwise an
// AOT component silently never matches its tag (or fails to compile at all).
// ---------------------------------------------------------------------------

/// `template:` may be a template literal interpolating a module-level `const`.
/// The interpolation must be folded so the component emits a real `ɵcmp` with
/// the resolved template.
#[test]
fn component_template_literal_const_interpolation_in_template() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const twBtn = `px-4 py-2 rounded`;

@Component({
    selector: 'app-btn',
    template: `<button class="${twBtn}">x</button>`,
    standalone: true,
})
export class BtnComponent {}
"#;
    let result = transform_angular_file(&allocator, "btn.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The component MUST have a real definition emitted.
    assert!(
        result.code.contains("ɵɵdefineComponent("),
        "Expected ɵɵdefineComponent in output (component must be compiled, not skipped).\nCode:\n{}",
        result.code
    );

    // The resolved class attribute must reach the output. Angular's template
    // compiler tokenizes the class string into a `consts: [[1, ...]]` array
    // (AttributeMarker.Classes == 1), so we check for the tokenized form.
    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"consts:[[1,"px-4","py-2","rounded"]]"#),
        "Expected resolved class attribute tokens in compiled output.\nCode:\n{}",
        result.code
    );
}

/// `selector:` may be a template literal interpolating a module-level `const`.
/// The interpolation must be folded so the selector matches its tag at runtime.
#[test]
fn component_template_literal_const_interpolation_in_selector() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const PREFIX = 'app';

@Component({
    selector: `${PREFIX}-btn`,
    template: '<button>x</button>',
    standalone: true,
})
export class BtnComponent {}
"#;
    let result = transform_angular_file(&allocator, "btn.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"selectors:[["app-btn"]]"#),
        "Expected selectors:[[\"app-btn\"]] from folded `${{PREFIX}}-btn`.\nCode:\n{}",
        result.code
    );
}

/// `@Directive` selector must also resolve template-literal interpolation.
#[test]
fn directive_template_literal_const_interpolation_in_selector() {
    let allocator = Allocator::default();
    let source = r#"
import { Directive } from '@angular/core';

const NAME = 'highlight';

@Directive({
    selector: `[${NAME}]`,
})
export class HighlightDirective {}
"#;
    let result = transform_angular_file(&allocator, "highlight.directive.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"selectors:[[""#) && normalized.contains(r#""highlight""#),
        "Expected resolved selector containing \"highlight\" attribute in directive definition.\nCode:\n{}",
        result.code
    );
}

/// Multiple `${...}` interpolations in a single template literal must all fold.
#[test]
fn component_template_literal_multiple_const_interpolations() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const A = 'hello';
const B = 'world';

@Component({
    selector: 'app-multi',
    template: `<span>${A} ${B}!</span>`,
    standalone: true,
})
export class MultiComponent {}
"#;
    let result = transform_angular_file(&allocator, "multi.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.code.contains("ɵɵdefineComponent("),
        "Expected ɵɵdefineComponent in output.\nCode:\n{}",
        result.code
    );
    assert!(
        result.code.contains("hello world!"),
        "Expected folded text \"hello world!\" in compiled template.\nCode:\n{}",
        result.code
    );
}

/// Chained consts: a const whose initializer interpolates another const must
/// still resolve when referenced from decorator metadata.
#[test]
fn component_chained_const_template_literal_interpolation() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const PREFIX = 'app';
const TAG = `${PREFIX}-chained`;

@Component({
    selector: TAG,
    template: '<span>x</span>',
    standalone: true,
})
export class ChainedComponent {}
"#;
    let result = transform_angular_file(&allocator, "chained.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let normalized = result.code.replace([' ', '\n', '\t'], "");
    assert!(
        normalized.contains(r#"selectors:[["app-chained"]]"#),
        "Expected selectors:[[\"app-chained\"]] from chained const resolution.\nCode:\n{}",
        result.code
    );
}

/// `styles:` array elements may contain `${...}` interpolations referencing
/// same-file consts; they must fold like `template:` does.
#[test]
fn component_styles_template_literal_const_interpolation() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const COLOR = 'red';

@Component({
    selector: 'app-s',
    template: '<span>x</span>',
    styles: [`:host { color: ${COLOR}; }`],
    standalone: true,
})
export class StyledComponent {}
"#;
    let result = transform_angular_file(&allocator, "styled.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.code.contains("color: red"),
        "Expected folded `color: red` in styles output.\nCode:\n{}",
        result.code
    );
}

/// JIT mode: `templateUrl` may be a template literal interpolating a
/// module-level `const`. The rewriter must fold and emit the
/// `angular:jit:template:file;...` import with the resolved path.
#[test]
fn jit_template_url_template_literal_const_interpolation() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const DIR = './cmp';

@Component({
    selector: 'app-root',
    templateUrl: `${DIR}/x.html`,
    standalone: true,
})
export class AppComponent {}
"#;
    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.code.contains("angular:jit:template:file;./cmp/x.html"),
        "JIT output should import folded template path via angular:jit:template:file.\nCode:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("templateUrl"),
        "JIT output should replace templateUrl with template.\nCode:\n{}",
        result.code
    );
}

/// JIT mode: `templateUrl` may be a bare identifier referencing a same-file
/// string `const`. Must also fold.
#[test]
fn jit_template_url_const_identifier() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const TPL = './app.html';

@Component({
    selector: 'app-root',
    templateUrl: TPL,
    standalone: true,
})
export class AppComponent {}
"#;
    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.code.contains("angular:jit:template:file;./app.html"),
        "JIT output should import resolved template path via angular:jit:template:file.\nCode:\n{}",
        result.code
    );
}

/// JIT mode: `styleUrl` may be a template literal interpolating a `const`.
#[test]
fn jit_style_url_template_literal_const_interpolation() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const DIR = './cmp';

@Component({
    selector: 'app-root',
    template: '<h1>x</h1>',
    styleUrl: `${DIR}/x.css`,
})
export class AppComponent {}
"#;
    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.code.contains("angular:jit:style:file;./cmp/x.css"),
        "JIT output should import folded style path via angular:jit:style:file.\nCode:\n{}",
        result.code
    );
}

/// JIT mode: `styleUrls:` array elements may be template literals
/// interpolating a `const`. Each element must fold individually.
#[test]
fn jit_style_urls_template_literal_const_interpolation() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

const DIR = './cmp';

@Component({
    selector: 'app-root',
    template: '<h1>x</h1>',
    styleUrls: [`${DIR}/a.css`, './b.css'],
})
export class AppComponent {}
"#;
    let options = ComponentTransformOptions { jit: true, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "app.component.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.code.contains("angular:jit:style:file;./cmp/a.css"),
        "JIT output should import folded first style path.\nCode:\n{}",
        result.code
    );
    assert!(
        result.code.contains("angular:jit:style:file;./b.css"),
        "JIT output should also import the literal second style path.\nCode:\n{}",
        result.code
    );
}

/// An interpolated `${...}` whose identifier is NOT a known const must NOT
/// crash and must NOT produce a partial/garbage selector — the field is
/// dropped (same fallback as today for any unresolvable identifier).
///
/// Scope: this test asserts ONLY on the `ɵcmp` selectors field. Since #299
/// turned `emit_class_metadata` on by default, the raw `${UNRESOLVED}-tag`
/// template literal is intentionally preserved verbatim inside
/// `ɵsetClassMetadata(..., [{ type: Component, args: [...] }], ...)` to
/// mirror ngc's behavior — that's metadata for runtime tooling and is not
/// the compiled selector itself.
#[test]
fn component_template_literal_unresolved_identifier_drops_field() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: `${UNRESOLVED}-tag`,
    template: '<span>x</span>',
    standalone: true,
})
export class UnresolvedComponent {}
"#;
    let result = transform_angular_file(&allocator, "u.component.ts", source, None, None);
    // Must not crash.
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The unresolved interpolation must not appear inside the `ɵcmp`'s
    // `selectors:` slot — that's the compiled selector that actually drives
    // template matching.
    let cmp_start = result.code.find("ɵɵdefineComponent({").expect("ɵcmp missing");
    let cmp_section = &result.code[cmp_start..];
    let cmp_end = cmp_section.find("})").expect("ɵcmp not terminated");
    let cmp_def = &cmp_section[..cmp_end];
    assert!(
        !cmp_def.contains("${UNRESOLVED}-tag"),
        "Unresolved interpolation must not leak verbatim into ɵcmp.\nɵcmp:\n{cmp_def}"
    );
    // And the compiled selector must fall back to the default tag, matching
    // ngc's behavior when a metadata interpolation can't be resolved.
    assert!(
        cmp_def.contains(r#"selectors:[["ng-component"]]"#),
        "Selector should fall back to `ng-component`.\nɵcmp:\n{cmp_def}"
    );
}

// =============================================================================
// Issue #287: TDZ-safe hoisting of consts referenced by emitted Ivy definitions
// =============================================================================
// When `@Component` metadata references a `const` (or other binding) declared
// *after* the class, the emitted Ivy definition (`ɵcmp` static field) evaluates
// the providers array eagerly in the class body. Because the const is still in
// the temporal dead zone at that point, this throws `ReferenceError: Cannot
// access 'TOKEN' before initialization` at module load.
//
// Angular's official compiler hoists such consts above the class declaration.
// These tests pin that behavior.

/// A `const` referenced by `providers` and declared after the class must be
/// hoisted above the class so the eagerly-evaluated `ɵɵProvidersFeature` does
/// not hit the TDZ at class-init time.
#[test]
fn component_providers_const_after_class_is_hoisted() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN, useValue: 1 }] })
export class TestComponent {}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The const TOKEN must appear before `class TestComponent` in the output
    // so it is initialized before the static `ɵcmp` field evaluates providers.
    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    assert!(
        token_pos < class_pos,
        "`const TOKEN` must be hoisted above `class TestComponent`. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );

    // Must only appear once: the original must have been deleted from its
    // original location.
    let count = result.code.matches("const TOKEN").count();
    assert_eq!(
        count, 1,
        "`const TOKEN` should appear exactly once (original deleted). Got {count}.\nCode:\n{}",
        result.code
    );
}

/// `viewProviders` is also evaluated eagerly via `ɵɵProvidersFeature` — consts
/// it references must be hoisted too.
#[test]
fn component_view_providers_const_after_class_is_hoisted() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({
    selector: 'x',
    template: '',
    viewProviders: [{ provide: VIEW_TOKEN, useValue: 2 }],
})
export class TestComponent {}
const VIEW_TOKEN = 'view-tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result.code.find("const VIEW_TOKEN").unwrap_or_else(|| {
        panic!("Expected `const VIEW_TOKEN` to be present.\nCode:\n{}", result.code)
    });
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    assert!(
        token_pos < class_pos,
        "`const VIEW_TOKEN` must be hoisted above `class TestComponent`. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
}

/// Multiple distinct providers consts after the class — all referenced by
/// metadata — must be hoisted, preserving their original relative order.
#[test]
fn component_multiple_provider_consts_after_class_are_hoisted_in_order() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({
    selector: 'x',
    template: '',
    providers: [
        { provide: TOKEN_A, useValue: 1 },
        { provide: TOKEN_B, useValue: 2 },
    ],
})
export class TestComponent {}
const TOKEN_A = 'a';
const TOKEN_B = 'b';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let a_pos = result.code.find("const TOKEN_A").expect("TOKEN_A missing");
    let b_pos = result.code.find("const TOKEN_B").expect("TOKEN_B missing");
    let class_pos = result.code.find("class TestComponent").expect("class missing");
    assert!(
        a_pos < class_pos && b_pos < class_pos,
        "Both consts must be hoisted above the class. \
         a@{a_pos} b@{b_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        a_pos < b_pos,
        "Relative order of consts must be preserved (A before B).\nCode:\n{}",
        result.code
    );
}

/// `useFactory` referencing a const declared later still hoists the const,
/// because the const is captured in the providers array argument which
/// `ɵɵProvidersFeature` evaluates at class-init time. Note: identifiers
/// referenced *inside* the factory's arrow-function body fire lazily when the
/// factory is invoked, so they don't need hoisting — only top-level metadata
/// references do.
#[test]
fn component_use_factory_dependency_const_is_hoisted_when_referenced_at_top_level() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({
    selector: 'x',
    template: '',
    providers: [{ provide: TOKEN, useFactory: () => 'val', deps: [DEP_TOKEN] }],
})
export class TestComponent {}
const TOKEN = 'tok';
const DEP_TOKEN = 'dep';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").expect("class missing");
    let token_pos = result.code.find("const TOKEN").expect("TOKEN missing");
    let dep_pos = result.code.find("const DEP_TOKEN").expect("DEP_TOKEN missing");
    assert!(token_pos < class_pos, "TOKEN (provider key) must be hoisted.\nCode:\n{}", result.code);
    assert!(
        dep_pos < class_pos,
        "DEP_TOKEN (deps array entry) must be hoisted.\nCode:\n{}",
        result.code
    );
}

/// Two `@Component` classes in the same file that both reference the same
/// later-declared const must hoist it exactly once, ahead of the earliest
/// referencing class.
#[test]
fn component_shared_provider_const_is_hoisted_once_for_multiple_classes() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'a', template: '', providers: [{ provide: SHARED, useValue: 1 }] })
export class A {}
@Component({ selector: 'b', template: '', providers: [{ provide: SHARED, useValue: 2 }] })
export class B {}
const SHARED = 'shared';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let count = result.code.matches("const SHARED").count();
    assert_eq!(count, 1, "`const SHARED` should appear exactly once.\nCode:\n{}", result.code);

    let shared_pos = result.code.find("const SHARED").unwrap();
    let a_pos = result.code.find("class A").unwrap();
    let b_pos = result.code.find("class B").unwrap();
    assert!(
        shared_pos < a_pos && shared_pos < b_pos,
        "const must be hoisted above both classes.\nshared@{shared_pos} a@{a_pos} b@{b_pos}\nCode:\n{}",
        result.code
    );
}

/// Identifiers referenced *only* inside a factory function body fire when
/// the factory is invoked, never at class-definition time. They do NOT need
/// to be hoisted. This guards against over-hoisting that could break code
/// that relies on the original declaration order (e.g. a const initialized
/// using values not yet computed at module load).
#[test]
fn component_const_referenced_only_inside_factory_body_is_not_hoisted() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({
    selector: 'x',
    template: '',
    providers: [{ provide: 'k', useFactory: () => LAZY_VALUE }],
})
export class TestComponent {}
const LAZY_VALUE = 'lazy';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let lazy_pos = result.code.find("const LAZY_VALUE").expect("LAZY_VALUE missing");
    let class_pos = result.code.find("class TestComponent").expect("class missing");
    assert!(
        lazy_pos > class_pos,
        "Const referenced only inside the factory body should NOT be hoisted.\n\
         lazy@{lazy_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
}

/// A const declared *before* the class must NOT be moved — only post-class
/// declarations need hoisting. The compiler must not pointlessly rewrite
/// already-valid code.
#[test]
fn component_provider_const_before_class_is_not_hoisted() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
const TOKEN = 'tok';
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN, useValue: 1 }] })
export class TestComponent {}
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The const must still appear once (we did not duplicate it).
    let count = result.code.matches("const TOKEN").count();
    assert_eq!(count, 1, "`const TOKEN` should still appear once.\nCode:\n{}", result.code);

    // And it must come before the class (its original position).
    let token_pos = result.code.find("const TOKEN").unwrap();
    let class_pos = result.code.find("class TestComponent").unwrap();
    assert!(token_pos < class_pos, "Order should be preserved.\nCode:\n{}", result.code);
}

/// When two bindings from the *same*
/// multi-declarator statement (`const A = 1, B = 2;`) are referenced by
/// different decorated classes, the hoist plan keys entries by binding name,
/// producing two `HoistEntry` values that share the same `stmt_start` but
/// carry different `insert_at` targets. The dedup loop in `collect_hoist_edits`
/// keeps whichever entry HashMap iteration visits first and drops the other —
/// so the chosen `insert_at` is nondeterministic, and can land *after* the
/// earliest referencing class. That leaves the earlier class still inside the
/// TDZ of the hoisted statement.
///
/// Scenario:
///   * `class A` (decorated) references `B`.
///   * `class C` (decorated) references `A`.
///   * Both classes are declared *before* `const A = 1, B = 2;`.
///
/// The correct behavior is to hoist the shared statement to *above the
/// earliest* referencing class (class A here), so both `A` and `B` are
/// initialized before either decorator runs.
#[test]
fn component_shared_multideclarator_const_hoists_above_earliest_referencer() {
    let allocator = Allocator::default();
    // `Acomp` references `Bval` in its decorator metadata.
    // `Ccomp` references `Aval` in its decorator metadata.
    // The const declaring both `Aval` and `Bval` is declared *after* both
    // classes, so both must be hoisted above the earliest class (`Acomp`).
    let source = r#"
import { Component } from '@angular/core';
@Component({
    selector: 'a-comp',
    template: '',
    providers: [{ provide: 'k', useValue: Bval }],
})
export class Acomp {}
@Component({
    selector: 'c-comp',
    template: '',
    providers: [{ provide: 'k', useValue: Aval }],
})
export class Ccomp {}
const Aval = 1, Bval = 2;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    // The shared declaration must appear exactly once (original deleted, single
    // hoisted copy emitted).
    let const_count = result.code.matches("const Aval").count();
    assert_eq!(
        const_count, 1,
        "`const Aval = 1, Bval = 2;` should appear exactly once. Got {const_count}.\nCode:\n{}",
        result.code
    );

    let const_pos = result.code.find("const Aval").expect("`const Aval` must appear in the output");
    let acomp_pos =
        result.code.find("class Acomp").expect("`class Acomp` must appear in the output");
    let ccomp_pos =
        result.code.find("class Ccomp").expect("`class Ccomp` must appear in the output");

    // The hoisted shared statement must precede BOTH classes — not just the
    // later one (`Ccomp`). If the dedup logic picks `Ccomp`'s `insert_at`,
    // the const will land between the two classes, leaving `Acomp` in the
    // TDZ of `Bval`.
    assert!(
        const_pos < acomp_pos,
        "`const Aval, Bval` must be hoisted above the *earliest* referencer (Acomp). \
         const@{const_pos} Acomp@{acomp_pos} Ccomp@{ccomp_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        const_pos < ccomp_pos,
        "`const Aval, Bval` must also be hoisted above Ccomp. \
         const@{const_pos} Acomp@{acomp_pos} Ccomp@{ccomp_pos}\nCode:\n{}",
        result.code
    );
}

/// Guards transitive TDZ deps: when decorator metadata references an
/// aggregate binding (e.g. `providers: PROVIDERS`) and that aggregate's
/// initializer transitively references *another* later-declared top-level
/// binding (`TOKEN`), the hoister must pull both bindings above the class.
///
/// Without this, `PROVIDERS` gets moved above the class but `TOKEN` stays
/// below, so `PROVIDERS`'s own initializer throws `ReferenceError: Cannot
/// access 'TOKEN' before initialization` at module evaluation — strictly
/// worse than before the hoist.
#[test]
fn component_provider_aggregate_const_pulls_in_transitive_tdz_dep() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: PROVIDERS })
export class TestComponent {}
const PROVIDERS = [{ provide: TOKEN, useValue: 1 }];
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let providers_pos = result.code.find("const PROVIDERS").unwrap_or_else(|| {
        panic!("Expected `const PROVIDERS` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    // Both must be hoisted above the class.
    assert!(
        providers_pos < class_pos,
        "`const PROVIDERS` must be hoisted above the class. \
         providers@{providers_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        token_pos < class_pos,
        "`const TOKEN` (transitively referenced by PROVIDERS' initializer) \
         must also be hoisted above the class to avoid TDZ. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );

    // And `TOKEN` must come before `PROVIDERS` so PROVIDERS' initializer can
    // actually read it at module load.
    assert!(
        token_pos < providers_pos,
        "`const TOKEN` must precede `const PROVIDERS` in the hoisted region. \
         token@{token_pos} providers@{providers_pos}\nCode:\n{}",
        result.code
    );

    // Neither should be duplicated.
    assert_eq!(
        result.code.matches("const PROVIDERS").count(),
        1,
        "`const PROVIDERS` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// When `providers: PROVIDERS` references a `const PROVIDERS = makeProviders()`
/// whose initializer *calls* a later-declared `function makeProviders()`, and
/// that function reads another later-declared `const TOKEN`, the hoister must
/// also pull `TOKEN` above the class — otherwise the hoisted `PROVIDERS`
/// initializer invokes `makeProviders()` before `TOKEN` is initialized and
/// throws `ReferenceError: Cannot access 'TOKEN' before initialization`.
#[test]
fn component_provider_const_via_function_call_pulls_in_transitive_tdz_dep() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: PROVIDERS })
class TestComponent {}
const TOKEN = 'tok';
const PROVIDERS = makeProviders();
function makeProviders() { return [{ provide: TOKEN }]; }
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let providers_pos = result.code.find("const PROVIDERS").unwrap_or_else(|| {
        panic!("Expected `const PROVIDERS` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    // Both must be hoisted above the class.
    assert!(
        providers_pos < class_pos,
        "`const PROVIDERS` must be hoisted above the class. \
         providers@{providers_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        token_pos < class_pos,
        "`const TOKEN` (transitively read by makeProviders() at module init) \
         must also be hoisted above the class to avoid TDZ. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );

    // And `TOKEN` must come before `PROVIDERS` so `makeProviders()` can read it
    // when the hoisted `PROVIDERS` initializer evaluates at module load.
    assert!(
        token_pos < providers_pos,
        "`const TOKEN` must precede `const PROVIDERS` in the hoisted region. \
         token@{token_pos} providers@{providers_pos}\nCode:\n{}",
        result.code
    );

    // Neither should be duplicated.
    assert_eq!(
        result.code.matches("const PROVIDERS").count(),
        1,
        "`const PROVIDERS` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `class.body.span.end` is the exclusive byte offset one past the closing
/// `}`. A `VariableDeclaration` whose statement starts at *exactly* that
/// offset (no whitespace between `}` and `const`) is positioned immediately
/// after the class body and is still in the TDZ when the class's static
/// fields evaluate. The hoist must move it; using `<=` for the
/// "before-class" check accidentally skips this boundary case.
#[test]
fn component_provider_const_immediately_after_class_brace_is_hoisted() {
    let allocator = Allocator::default();
    // No whitespace at all between `}` and `const` — `const` starts at
    // exactly `class.body.span.end`.
    let source = "import { Component } from '@angular/core';\n\
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN, useValue: 1 }] })\n\
export class TestComponent {}const TOKEN = 'tok';\n";

    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    assert!(
        token_pos < class_pos,
        "Boundary-case `const TOKEN` (decl at exactly class.body.span.end) must \
         still be hoisted above the class. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once (original deleted).\nCode:\n{}",
        result.code
    );
}

/// A top-level function referenced from decorator metadata as a *value*
/// (e.g. `useFactory: makeFactory`) is NOT called at class-definition time —
/// Angular's injector calls it later, when the provider is actually resolved.
/// So later-declared bindings reachable only through that function's body
/// must NOT be hoisted. Hoisting them would create a NEW TDZ that didn't
/// exist in the original source.
#[test]
fn component_provider_useFactory_function_value_does_not_hoist_body_deps() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useFactory: makeFactory }] })
class TestComponent {}
function makeFactory() { return TOKEN; }
const TOKEN = TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));

    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert!(
        token_pos > class_pos,
        "`const TOKEN` must NOT be hoisted — `makeFactory` is stored as a value, not \
         called at module load. Hoisting `TOKEN` above the class would TDZ on \
         `TestComponent`. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
}

/// `Expression::ChainExpression` (optional chaining, `TOKEN?.id` or `f?.()`)
/// must contribute identifier references to the decorator-metadata symbol
/// scan, so that the referenced top-level binding gets hoisted.
#[test]
fn component_provider_optional_chain_token_is_hoisted() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN?.id, useValue: 1 }] })
class TestComponent {}
const TOKEN = { id: 'tok' };
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    assert!(
        token_pos < class_pos,
        "`const TOKEN` (referenced via `TOKEN?.id` in providers) must be \
         hoisted above the class. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Top-level destructuring patterns must be indexed: `const { TOKEN } = X;`
/// binds `TOKEN`, and decorator metadata referencing `TOKEN` must hoist that
/// declaration above the class.
#[test]
fn component_provider_destructured_top_level_token_is_hoisted() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
const TOKENS = { TOKEN: 'tok' };
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN, useValue: 1 }] })
class TestComponent {}
const { TOKEN } = TOKENS;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result.code.find("const { TOKEN }").unwrap_or_else(|| {
        panic!("Expected `const {{ TOKEN }}` to be present.\nCode:\n{}", result.code)
    });
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    assert!(
        token_pos < class_pos,
        "`const {{ TOKEN }}` (destructured from `TOKENS`) must be hoisted \
         above the class. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const { TOKEN }").count(),
        1,
        "`const {{ TOKEN }}` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// A multi-declarator `const TOKEN = 'tok', BACKREF = TestComponent;`
/// statement is referenced (via `TOKEN`) in the decorator metadata. The
/// statement's *other* declarator initializer references `TestComponent`
/// itself, which lives below. Hoisting the whole statement above the class
/// would put `BACKREF = TestComponent` ahead of `class TestComponent`,
/// introducing a *new* TDZ on the class.
///
/// The safe-skip guard refuses to hoist a statement when any of its
/// initializer symbols resolves to a top-level class declared at position
/// `>= effective_start` of the class being protected.
#[test]
fn component_provider_multi_declarator_with_class_self_ref_skips_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN, useValue: 1 }] })
class TestComponent {}
const TOKEN = 'tok', BACKREF = TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    // The original `const TOKEN = 'tok', BACKREF = TestComponent;` statement
    // must remain in its original position (below the class). It must NOT be
    // duplicated/hoisted above the class.
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` must not be duplicated (no hoist + keep). \
         Hoisting this multi-declarator statement would put \
         `BACKREF = TestComponent` ahead of the class.\nCode:\n{}",
        result.code
    );
    if let Some(token_pos) = result.code.find("const TOKEN") {
        assert!(
            token_pos > class_pos,
            "`const TOKEN ... BACKREF = TestComponent` must NOT be hoisted \
             above the class — that would introduce a new TDZ on `TestComponent`. \
             token@{token_pos} class@{class_pos}\nCode:\n{}",
            result.code
        );
    }
}

/// `providers: (() => [{ provide: TOKEN, useValue: 1 }])()` — the IIFE
/// is invoked *eagerly* at class-definition time, so the references inside
/// the arrow body must be treated as eager. The general lazy-bodies rule
/// (skip arrow/function bodies) doesn't apply when the function is its own
/// callee.
#[test]
fn component_provider_iife_metadata_hoists_inner_token() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: (() => [{ provide: TOKEN, useValue: 1 }])() })
class TestComponent {}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    assert!(
        token_pos < class_pos,
        "`const TOKEN` (referenced inside an IIFE in `providers`) must be \
         hoisted above the class — the IIFE runs eagerly. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `foo` is referenced as a value (`useFactory: foo`) in TestComponent's
/// decorator metadata — NOT called there. The global `eagerly_called`
/// closure adds `foo` because *another* top-level statement
/// (`const X = foo()`) calls it. The BFS for TestComponent must not chase
/// `foo`'s body just because some unrelated module-level statement happens
/// to invoke `foo`. Otherwise it pulls in `TOKEN` and hoists
/// `const TOKEN = TestComponent;` above the class → new TDZ on the class.
///
/// Per-class eagerly_called scoping (seeded only from THIS class's
/// `decorator_called`) prevents this leak.
#[test]
fn component_provider_useFactory_value_does_not_chase_global_eager_caller() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
function foo() { return TOKEN; }
const X = foo();
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useFactory: foo }] })
class TestComponent {}
const TOKEN = TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    // `const TOKEN = TestComponent;` must NOT be hoisted above the class —
    // that would put `TestComponent` reference ahead of its own declaration.
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` must not be duplicated.\nCode:\n{}",
        result.code
    );
    if let Some(token_pos) = result.code.find("const TOKEN") {
        assert!(
            token_pos > class_pos,
            "`const TOKEN = TestComponent` must NOT be hoisted above the class \
             — that would introduce a new TDZ on `TestComponent`. \
             `foo` is referenced as a value in `useFactory: foo`, not called \
             by this class's decorator metadata. \
             token@{token_pos} class@{class_pos}\nCode:\n{}",
            result.code
        );
    }
}

/// When a hoisted initializer eagerly calls a top-level function whose
/// *parameter default expression* reads a later-declared binding, the
/// param-default reference is just as TDZ-relevant as a body reference:
/// defaults evaluate at call time, before the function body runs.
///
/// Here, the BFS sees `PROVIDERS = makeProviders()`, marks `makeProviders`
/// as eagerly called, and must chase BOTH `makeProviders`'s body refs AND
/// the refs inside its parameter default `token = TOKEN`. Otherwise `TOKEN`
/// is left below the class and the hoisted `const PROVIDERS = makeProviders()`
/// throws `ReferenceError: Cannot access 'TOKEN' before initialization` when
/// the parameter default fires.
#[test]
fn component_provider_eager_call_chases_param_default_refs() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: PROVIDERS })
class TestComponent {}
const PROVIDERS = makeProviders();
function makeProviders(token = TOKEN) { return [{ provide: token }]; }
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let providers_pos = result.code.find("const PROVIDERS").unwrap_or_else(|| {
        panic!("Expected `const PROVIDERS` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read by makeProviders's parameter default at call time) \
         must be hoisted above the class to avoid TDZ. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        providers_pos < class_pos,
        "`const PROVIDERS` must be hoisted above the class. \
         providers@{providers_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        token_pos < providers_pos,
        "`const TOKEN` must precede `const PROVIDERS` so the parameter default \
         `token = TOKEN` can read it when `makeProviders()` runs at module init. \
         token@{token_pos} providers@{providers_pos}\nCode:\n{}",
        result.code
    );

    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const PROVIDERS").count(),
        1,
        "`const PROVIDERS` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// A destructuring binding `const { TOKEN = FALLBACK } = {}` introduces
/// `TOKEN` (used in decorator metadata) but its initializer is `{}`, so the
/// `FALLBACK` default fires when the destructuring statement evaluates.
/// The hoister must chase defaults inside the binding pattern, otherwise
/// `FALLBACK` stays below the class and the hoisted destructuring throws
/// `ReferenceError: Cannot access 'FALLBACK' before initialization` at
/// runtime.
#[test]
fn component_destructured_default_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: TOKEN, useValue: 0 }] })
class TestComponent {}
const { TOKEN = FALLBACK } = {};
const FALLBACK = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let fallback_pos = result.code.find("const FALLBACK").unwrap_or_else(|| {
        panic!("Expected `const FALLBACK` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result.code.find("const { TOKEN").unwrap_or_else(|| {
        panic!("Expected `const {{ TOKEN ...` to be present.\nCode:\n{}", result.code)
    });
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        fallback_pos < token_pos,
        "`const FALLBACK` must precede `const {{ TOKEN = FALLBACK }} = {{}}` so the \
         destructuring default can read it. fallback@{fallback_pos} token@{token_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        token_pos < class_pos,
        "`const {{ TOKEN ... }}` must be hoisted above the class. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        fallback_pos < class_pos,
        "`const FALLBACK` must also be hoisted above the class. \
         fallback@{fallback_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );

    assert_eq!(
        result.code.matches("const FALLBACK").count(),
        1,
        "`const FALLBACK` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const { TOKEN").count(),
        1,
        "destructuring statement should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `provideThing` is eagerly called from the decorator. Its body contains an
/// IIFE `(() => [TOKEN])()` whose body executes at the call site, so the
/// `TOKEN` reference is TDZ-relevant. `FunctionBodyIdentVisitor` must walk
/// IIFE callee bodies the same way `collect_expr_symbols` does, or `TOKEN`
/// is left below the class and the eagerly-called function throws at module
/// init.
#[test]
fn component_eager_fn_body_iife_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [provideThing()] })
class TestComponent {}
function provideThing() { return (() => [TOKEN])(); }
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside an IIFE in the body of an eagerly-called \
         function) must be hoisted above the class to avoid TDZ. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `make` is eagerly invoked from the decorator. Inside `make`, a *local*
/// function declaration `inner` is defined and immediately called. `inner`'s
/// body reads a later-declared top-level `const TOKEN`, so `TOKEN` is
/// TDZ-relevant: at module load, the hoisted decorator-eval runs
/// `make() → inner() → TOKEN` before the const initializer fires.
///
/// `FunctionBodyIdentVisitor::visit_function` must descend into named nested
/// `Function` nodes so the locally-declared `inner` contributes its body
/// references (and its own callees) to the enclosing function's eager
/// surface. Without that, the BFS never observes that `make()` transitively
/// reads `TOKEN` and the const stays below the class.
#[test]
fn component_eager_fn_body_local_fn_decl_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
function make() {
  function inner() { return TOKEN; }
  return inner();
}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside a locally-declared function called from \
         the body of an eagerly-called function) must be hoisted above the \
         class to avoid TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `make` is eagerly invoked from the decorator. Inside `make`, a *local*
/// arrow expression is assigned to a `const inner` binding and then
/// immediately called via `inner()`. `inner`'s body reads a later-declared
/// top-level `const TOKEN`, so `TOKEN` is TDZ-relevant: at module load, the
/// hoisted decorator-eval runs `make() → inner() → TOKEN` before the const
/// initializer fires.
///
/// Unlike a *named* nested function (handled by walking through
/// `visit_function`), arrows assigned to local bindings need a separate
/// indexing step: `FunctionBodyIdentVisitor` must record arrow-valued local
/// bindings inside the function body it walks, then fold those bodies in at
/// each call site so calls to local arrows transitively contribute their
/// reads to the enclosing eager surface.
#[test]
fn component_eager_fn_body_local_arrow_binding_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
function make() {
  const inner = () => TOKEN;
  return inner();
}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside a local arrow binding called from the \
         body of an eagerly-called function) must be hoisted above the class \
         to avoid TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Sibling of `component_eager_fn_body_local_arrow_binding_chases_late_const`
/// that locks in laziness: when a local arrow binding is stored in a provider
/// (`useFactory: lazy`) but is NEVER called inside the enclosing function's
/// body, the arrow's body refs must NOT force a hoist via the local-arrow
/// indexing. The hoist might still happen because other analysis paths treat
/// the provider shape as eager, but the transform must at minimum not error.
#[test]
fn component_eager_fn_body_lazy_local_arrow_does_not_force_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
function make() {
  const lazy = () => TOKEN;
  return [{ provide: 'tok', useFactory: lazy }];
}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
}

/// Laziness sibling for *named* nested function declarations: inside an
/// eagerly-called `make()`, a locally-declared `function unused()` reads
/// `TOKEN`, but the function is never invoked. `make()` returns `[]`, so no
/// eager read of `TOKEN` actually happens at decorator-eval time. The
/// transform must NOT fold `unused`'s body into the eager surface — doing so
/// would falsely hoist `TOKEN` above the class even though no value-passed
/// reference fires.
///
/// The original source places `const TOKEN` after the class. With correct
/// laziness, the transform leaves that ordering intact.
#[test]
fn component_eager_fn_body_uncalled_nested_fn_decl_does_not_force_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
function make() {
  function unused() { return TOKEN; }
  return [];
}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));

    assert!(
        class_pos < token_pos,
        "`const TOKEN` must NOT be hoisted: `unused` is declared but never \
         called, so its body refs are lazy. class@{class_pos} \
         token@{token_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// JS function declarations are hoisted inside their enclosing scope, so a
/// call to `inner()` can appear in source *before* the `function inner()`
/// declaration and still resolve at runtime. The visitor walks in source
/// order, so it sees `return inner();` before it indexes `inner`. The
/// fold-at-call-site path must therefore pre-index nested function
/// declarations within each function body / block before walking the
/// statements — otherwise the call site cannot resolve `inner` and `TOKEN`
/// stays unhoisted.
#[test]
fn component_eager_fn_body_hoisted_fn_decl_call_still_chases() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
function make() {
  return inner();
  function inner() { return TOKEN; }
}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read by a hoisted nested function declaration called \
         from above its source position inside an eagerly-called function) \
         must be hoisted above the class to avoid TDZ. token@{token_pos} \
         class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// The safe-skip guard must refuse to hoist a `var TOKEN = make()` initializer
/// when the eagerly-called `make()`'s body reads a later-declared top-level
/// class. Without the fix, hoisting `var TOKEN = make()` above
/// `class TestComponent` invents a fresh TDZ on the class: `make()` runs at
/// the hoisted initializer's evaluation time and reads `TestComponent` before
/// the class binding is initialized.
///
/// The user's existing TDZ on `TOKEN` is NOT our problem to fix — we must
/// just not introduce a NEW class TDZ. So we only assert that `class
/// TestComponent` still precedes `var TOKEN`.
#[test]
fn component_eager_fn_body_class_ref_blocks_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useValue: TOKEN }] })
class TestComponent {}
var TOKEN = make();
function make() { return TestComponent; }
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("var TOKEN")
        .unwrap_or_else(|| panic!("Expected `var TOKEN` to be present.\nCode:\n{}", result.code));

    assert!(
        class_pos < token_pos,
        "`var TOKEN = make()` must NOT be hoisted above the class because \
         `make()`'s body reads `TestComponent`. Hoisting would invent a fresh \
         class TDZ. class@{class_pos} token@{token_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("var TOKEN").count(),
        1,
        "`var TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// A decorator-metadata `AssignmentExpression` (`(cached = TOKEN)`) carries
/// identifier references on both its `left` and `right`. The
/// `collect_expr_symbols` walker must not silently drop these — otherwise
/// `TOKEN` never enters the BFS and stays declared below the class, while
/// the class's emitted Ivy definition reads `TOKEN` eagerly.
#[test]
fn component_assignment_expression_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
let cached;
@Component({ selector: 'x', template: '', providers: [(cached = TOKEN)] })
class TestComponent {}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read by an AssignmentExpression in decorator \
         metadata) must be hoisted above the class to avoid TDZ. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Transitive dependency cascade. The BFS pops `TOKEN` whose
/// only directly-called function is `make()`; the closure of
/// `init_called_symbols` brings in nothing class-relevant from `make`'s
/// body (it just calls `BACKREF` whose binding is a non-function const).
/// So the safe-skip guard at `TOKEN`'s site passes — `TOKEN`'s statement
/// is planned. The BFS then pushes `make`'s body refs onto the worklist,
/// pops `BACKREF`, and *its* guard detects `BACKREF = TestComponent` reading
/// a later class — so `BACKREF` is skipped. But `TOKEN`'s plan entry is
/// still there, leaving the runtime broken: hoisted `var TOKEN = make()`
/// invokes `make()` which reads not-yet-initialized `BACKREF`, which the
/// guard correctly identified would read `TestComponent` if it ran.
///
/// Required: when a dependency is guard-skipped, every transitively
/// dependent already-planned statement must be un-planned too. Without
/// the fix, `var TOKEN` lands above `class TestComponent` in the output.
#[test]
fn component_eager_fn_body_transitive_class_ref_unplans_chain() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useValue: TOKEN }] })
class TestComponent {}
var TOKEN = make();
function make() { return BACKREF; }
const BACKREF = TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("var TOKEN")
        .unwrap_or_else(|| panic!("Expected `var TOKEN` to be present.\nCode:\n{}", result.code));
    let backref_pos = result.code.find("const BACKREF").unwrap_or_else(|| {
        panic!("Expected `const BACKREF` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        class_pos < token_pos,
        "`var TOKEN = make()` must NOT be hoisted above the class because \
         its transitive dep `BACKREF` reads `TestComponent`. class@{class_pos} \
         token@{token_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        class_pos < backref_pos,
        "`const BACKREF = TestComponent` must NOT be hoisted above the class. \
         class@{class_pos} backref@{backref_pos}\nCode:\n{}",
        result.code
    );
}

/// Function-valued `const`/`let` bindings hide eager class
/// reads. The BFS pops `TOKEN` whose `init_called_symbols = {make}`.
/// `make` is a `const` arrow, not a function decl — so it's missing from
/// `fn_body_*` maps. The closure expansion finds nothing; the guard
/// passes; `TOKEN` gets hoisted above the class. At runtime: hoisted
/// `make()` reads `TestComponent` in TDZ.
///
/// Required: top-level `const`/`let`/`var` bindings whose initializer is
/// *directly* an `ArrowFunctionExpression` / `FunctionExpression` (after
/// peeling parens / TS wrappers) must be indexed into `fn_body_*` maps
/// keyed by the binding symbol, so the existing safe-skip guard catches
/// the transitive class read.
#[test]
fn component_eager_fn_value_const_arrow_class_ref_blocks_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useValue: TOKEN }] })
class TestComponent {}
var TOKEN = make();
const make = () => TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("var TOKEN")
        .unwrap_or_else(|| panic!("Expected `var TOKEN` to be present.\nCode:\n{}", result.code));

    assert!(
        class_pos < token_pos,
        "`var TOKEN = make()` must NOT be hoisted above the class because \
         the `const make = () => TestComponent` arrow body reads the class. \
         class@{class_pos} token@{token_pos}\nCode:\n{}",
        result.code
    );
}

/// Member-call shapes `fn.call(...)` / `fn.apply(...)` aren't
/// recognized as eager calls. `record_direct_callee` peels parens / TS
/// wrappers but stops at `StaticMemberExpression`, so `make.call(null)`
/// records nothing in `called`. The guard's `stmt_called` is empty, the
/// transitive class-ref check never inspects `make`'s body, and `TOKEN`
/// gets hoisted above the class. At runtime: hoisted `make.call(null)`
/// reads `TestComponent` in TDZ.
///
/// Required: extend `record_direct_callee` (or a wrapper) to recognize
/// the static call shapes `fn.call(...)`, `fn.apply(...)`, and
/// `fn.bind(...)()` on top-level function symbols.
#[test]
fn component_eager_member_call_class_ref_blocks_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useValue: TOKEN }] })
class TestComponent {}
var TOKEN = make.call(null);
function make() { return TestComponent; }
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("var TOKEN")
        .unwrap_or_else(|| panic!("Expected `var TOKEN` to be present.\nCode:\n{}", result.code));

    assert!(
        class_pos < token_pos,
        "`var TOKEN = make.call(null)` must NOT be hoisted above the class \
         because `make()`'s body reads `TestComponent`. class@{class_pos} \
         token@{token_pos}\nCode:\n{}",
        result.code
    );
}

/// Cross-class `insert_at` ordering. Two
/// `@Component`-decorated classes (C1 first, C2 second) with an
/// undecorated `class Mid` between them. C1 plans `var TOKEN = make()` at
/// `insert_at = pos_C1`; its BFS chases `make`'s body to `X` but the
/// safe-skip guard rejects `X` for C1 because `X = Mid` reads class `Mid`
/// which is declared *after* C1. C2's BFS reaches `X` independently (via
/// `useValue: X`) and the safe-skip passes for C2 (Mid is declared
/// *before* C2). So `X` lands in the plan at `insert_at = pos_C2 >
/// pos_C1`.
///
/// The cascade un-planning loop previously treated "X is in plan" as a
/// safe dep — but X's `insert_at` is *later* than TOKEN's, so at runtime
/// hoisted TOKEN runs before hoisted X and `make()` TDZ-reads `X`. The
/// fix changes the cascade check to "dep planned at an `insert_at` ≤ S's
/// `insert_at`" (drop S otherwise).
///
/// We assert `class C1` precedes `var TOKEN = make()`: TOKEN must NOT be
/// hoisted because its dep X can't be hoisted to the same insertion
/// position. (TOKEN's user-authored TDZ on X persists — not our problem;
/// we just must not introduce a fresh hoist-induced TDZ between the two
/// hoisted statements.)
#[test]
fn component_cascade_cross_class_insert_order_unplans_dependent() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'a', template: '', providers: [{ provide: 'x', useValue: TOKEN }] })
class C1 {}
var TOKEN = make();
function make() { return X; }
class Mid {}
@Component({ selector: 'b', template: '', providers: [{ provide: 'y', useValue: X }] })
class C2 {}
const X = Mid;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let c1_pos = result
        .code
        .find("class C1")
        .unwrap_or_else(|| panic!("Expected `class C1` to be present.\nCode:\n{}", result.code));
    let token_pos = result
        .code
        .find("var TOKEN")
        .unwrap_or_else(|| panic!("Expected `var TOKEN` to be present.\nCode:\n{}", result.code));

    assert!(
        c1_pos < token_pos,
        "`var TOKEN = make()` must NOT be hoisted above `class C1` because \
         its transitive dep `X` is only planned at `insert_at` for \
         `class C2`, which is *later* in source. Hoisting TOKEN above C1 \
         leaves it running before the hoisted X lands. c1@{c1_pos} \
         token@{token_pos}\nCode:\n{}",
        result.code
    );
}

/// Per-S eager-call set. Class A uses
/// `makeRef` as a value (`useFactory: makeRef`); class B *calls* `make()`
/// (`providers: [make()]`). The cascade pass currently uses
/// `combined_eagerly_called` (the union across all classes) so `make` —
/// only eagerly invoked from B — over-expands A's `makeRef` statement
/// closure through `make`'s body refs. A's safe hoist gets dropped even
/// though A never calls `make`.
///
/// With the fix, the cascade computes a per-S eager-call set from
/// `info.init_called_symbols` closed under `fn_body_called_symbols`. A's
/// statement `const makeRef = make;` calls nothing, so its eager set is
/// empty and the closure doesn't chase `make`'s body. A's `makeRef` hoist
/// survives.
#[test]
fn component_cascade_value_only_ref_does_not_over_expand() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'a', template: '', providers: [{ provide: 'x', useFactory: makeRef }] })
class A {}
@Component({ selector: 'b', template: '', providers: [make()] })
class B {}
const makeRef = make;
function make() { return BACKREF; }
const BACKREF = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let a_pos = result
        .code
        .find("class A")
        .unwrap_or_else(|| panic!("Expected `class A` to be present.\nCode:\n{}", result.code));
    let make_ref_pos = result.code.find("const makeRef").unwrap_or_else(|| {
        panic!("Expected `const makeRef` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        make_ref_pos < a_pos,
        "`const makeRef = make;` must be hoisted above `class A` because A \
         only references `makeRef` as a value — A never *calls* `make`, so \
         `make`'s body refs are irrelevant to A's safe-skip. The cascade \
         must compute a per-S eager-call set so `make`'s eager evaluation \
         from class B doesn't bleed into A's closure. makeRef@{make_ref_pos} \
         a@{a_pos}\nCode:\n{}",
        result.code
    );
}

/// Multi-declarator function-valued bindings.
/// `index_fn_valued_binding` currently only runs when
/// `decl.declarations.len() == 1`. The shape
/// `const make = () => TestComponent, other = 0;` skips indexing, so
/// `make`'s arrow body is never visible to the safe-skip guard. An eager
/// caller (`var TOKEN = make()`) then hoists above the class and TDZ-reads
/// `TestComponent` at runtime.
///
/// The fix lifts the indexing into the per-declarator loop so each
/// declarator with a plain identifier binding and a direct arrow/function
/// initializer gets indexed regardless of how many siblings share the
/// statement. Assert `class TestComponent` precedes `var TOKEN`.
#[test]
fn component_multi_declarator_fn_valued_binding_blocks_caller_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'x', useValue: TOKEN }] })
class TestComponent {}
var TOKEN = make();
const make = () => TestComponent, other = 0;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let token_pos = result
        .code
        .find("var TOKEN")
        .unwrap_or_else(|| panic!("Expected `var TOKEN` to be present.\nCode:\n{}", result.code));

    assert!(
        class_pos < token_pos,
        "`var TOKEN = make()` must NOT be hoisted above the class because \
         the multi-declarator binding `const make = () => TestComponent, \
         other = 0;` declares `make` whose arrow body reads `TestComponent`. \
         class@{class_pos} token@{token_pos}\nCode:\n{}",
        result.code
    );
}

/// A top-level `const make = () => DEP`
/// populates BOTH `symbol_to_stmt[make]` (binding) AND
/// `fn_body_symbol_refs[make]` (because `index_fn_valued_binding` indexes
/// arrow/function-valued bindings as if they were function declarations).
/// When the BFS pops `make` and `eagerly_called.contains(&make)` (because
/// decorator metadata called `make()`), the `if let Some(&stmt_start) =
/// symbol_to_stmt.get(&make)` branch fires first and plans `make`'s
/// statement — then the `else if eagerly_called.contains(&symbol)` body-
/// chase NEVER runs. Result: `TOKEN`, which `make`'s arrow body reads, is
/// never pushed onto the worklist and stays declared below the class. At
/// runtime, hoisted `makeProviders()` reads `TOKEN` in TDZ.
///
/// Required: when the BFS pops a symbol that has BOTH a `symbol_to_stmt`
/// entry AND a `fn_body_symbol_refs` entry, AND is in `eagerly_called`,
/// the binding-planning branch must ALSO chase the function body refs —
/// the symbol acts as both a binding AND a function.
///
/// Assert: `const TOKEN` appears before `const makeProviders` in output,
/// and `const makeProviders` appears before `class TestComponent` — the
/// chase must reach `TOKEN` so it gets hoisted too.
#[test]
fn component_eager_fn_valued_const_chases_body_refs() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: makeProviders() })
class TestComponent {}
const makeProviders = () => [{ provide: TOKEN, useValue: 0 }];
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let make_pos = result.code.find("const makeProviders").unwrap_or_else(|| {
        panic!("Expected `const makeProviders` to be present.\nCode:\n{}", result.code)
    });
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < make_pos,
        "`const TOKEN` (read inside `makeProviders`'s arrow body which is \
         eagerly invoked by decorator metadata) must be hoisted above \
         `const makeProviders`. token@{token_pos} make@{make_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        make_pos < class_pos,
        "`const makeProviders` must be hoisted above `class TestComponent`. \
         make@{make_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Locks in symmetric per-stmt eager-call
/// reasoning between the cascade un-planning pass and `topological_order`.
/// The cascade was changed to compute a per-S `stmt_called` (closure of
/// `init_called_symbols` under `fn_body_called_symbols`); the topo sort
/// was still passing the global `combined_eagerly_called`. The asymmetry
/// can in principle create spurious dependency edges between planned
/// statements; in practice the cycle-break path is contrived. This test
/// is a regression guardrail: build a case where class A only references
/// `makeRef = make` as a value and class B eagerly calls `make()`. The
/// cascade decides A's hoist is safe; the topological sort must emit A's
/// statement in an order consistent with the cascade's view (i.e. not
/// reorder or drop it).
///
/// Locks in symmetric per-stmt eager-call reasoning between cascade and
/// topological_order.
#[test]
fn component_topo_uses_per_stmt_eager_set() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'a', template: '', providers: [{ provide: 'x', useFactory: makeRef }] })
class A {}
@Component({ selector: 'b', template: '', providers: [make()] })
class B {}
const makeRef = make;
function make() { return BACKREF; }
const BACKREF = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let make_ref_pos = result.code.find("const makeRef").unwrap_or_else(|| {
        panic!("Expected `const makeRef` to be present.\nCode:\n{}", result.code)
    });
    let a_pos = result
        .code
        .find("class A")
        .unwrap_or_else(|| panic!("Expected `class A` to be present.\nCode:\n{}", result.code));

    // The cascade pass already proves A is safe to hoist; symmetric topo
    // must agree — `const makeRef` must precede `class A`.
    assert!(
        make_ref_pos < a_pos,
        "`const makeRef = make;` must be hoisted above `class A` — A only \
         references `makeRef` as a value. The topological sort must reason \
         against the same per-stmt eager-call set the cascade used, so the \
         global `make` eager-call (from class B) doesn't introduce a \
         spurious edge that reorders A's hoist. makeRef@{make_ref_pos} \
         a@{a_pos}\nCode:\n{}",
        result.code
    );
    // Basic ordering invariant: a single `const makeRef` survives.
    assert_eq!(
        result.code.matches("const makeRef").count(),
        1,
        "`const makeRef` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// A function-valued `const`
/// binding whose ARROW BODY reads a top-level class can escape BOTH the
/// safe-skip guard AND the cascade un-planning when the binding ITSELF
/// is eagerly called from a decorator.
///
/// Trace:
/// - `decorator_called = {make}`. Per-class `eagerly_called = {make}`.
/// - BFS pops `make`. `symbol_to_stmt[make]` is present → enter the
///   binding branch.
/// - Safe-skip guard inspects `info.init_symbols` (refs in the
///   *initializer expression*). For `const make = () => TestComponent;`,
///   the initializer is an `ArrowFunctionExpression` — `collect_expr_symbols`
///   treats arrow bodies as lazy, so `init_symbols = {}` and
///   `init_called_symbols = {}`. Guard passes.
/// - Plan adds `const make = () => TestComponent;`. The Round-5 fix then
///   chases `fn_body_symbol_refs[make] = {TestComponent}`, pushing
///   `TestComponent` onto the worklist. BFS pops `TestComponent` — it's
///   a class, not a binding, not in `eagerly_called` — falls through.
///
/// Result: `make` is hoisted above the class. At Ivy decorator-eval time,
/// hoisted `make()` reads `TestComponent` in TDZ → ReferenceError.
///
/// Fix: the safe-skip guard must also include the body refs of every
/// fn-valued binding declared by this statement whose binding symbol is
/// in the per-class `eagerly_called` set — those body refs fire when the
/// binding is invoked at module load.
///
/// Assert: `class TestComponent` precedes `const make` in the output —
/// `make`'s hoisting must be blocked because its body reads
/// `TestComponent`.
#[test]
fn component_eager_fn_valued_const_reading_class_blocks_hoist() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
const make = () => TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let make_pos = result
        .code
        .find("const make")
        .unwrap_or_else(|| panic!("Expected `const make` to be present.\nCode:\n{}", result.code));

    assert!(
        class_pos < make_pos,
        "`class TestComponent` must precede `const make` — `make`'s arrow \
         body reads `TestComponent`, and the decorator eagerly invokes \
         `make()`. Hoisting `make` above the class introduces a fresh TDZ \
         on `TestComponent`. class@{class_pos} make@{make_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const make").count(),
        1,
        "`const make` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Round 6 transitive variant: the cascade un-planning loop must also
/// consult fn-valued bindings' body refs.
///
/// Trace:
/// - `@Component({ providers: make() }) class TestComponent {}`.
/// - `const make = () => BACKREF;` — guard passes (arrow body lazy),
///   `make` planned.
/// - Body chase pushes `BACKREF`. BFS pops `BACKREF`. Its stmt's
///   `init_symbols = {TestComponent}` → safe-skip blocks. `BACKREF` is
///   NOT planned.
/// - Cascade pass for `make`: `info.init_symbols = {}` (arrow body lazy),
///   so `expand_through_functions(init_symbols={}, …)` returns empty
///   closure. The cascade never sees that `make`'s body reads `BACKREF`,
///   which isn't planned → cascade doesn't drop `make`.
/// - Result: `make` is hoisted above the class, `BACKREF` stays below;
///   at runtime hoisted `make()` reads `BACKREF` in TDZ.
///
/// Fix: the cascade un-planning loop's closure seed must include each
/// fn-valued binding's symbol (so `expand_through_functions` descends
/// into its body), gated by `combined_eagerly_called` — only when the
/// binding's symbol is actually eagerly invoked somewhere.
///
/// Assert: `class TestComponent` precedes BOTH `const make` and
/// `const BACKREF` in the output — neither got hoisted.
#[test]
fn component_eager_fn_valued_const_transitive_class_ref_unplans() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
const make = () => BACKREF;
const BACKREF = TestComponent;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });
    let make_pos = result
        .code
        .find("const make")
        .unwrap_or_else(|| panic!("Expected `const make` to be present.\nCode:\n{}", result.code));
    let backref_pos = result.code.find("const BACKREF").unwrap_or_else(|| {
        panic!("Expected `const BACKREF` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        class_pos < make_pos,
        "`class TestComponent` must precede `const make` — `make`'s arrow \
         body reads `BACKREF` which transitively reads `TestComponent`. \
         Hoisting `make` introduces a TDZ. class@{class_pos} \
         make@{make_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        class_pos < backref_pos,
        "`class TestComponent` must precede `const BACKREF` — `BACKREF` \
         directly reads `TestComponent`. The original guard already \
         blocks `BACKREF`'s hoist; this assertion locks that in. \
         class@{class_pos} backref@{backref_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const make").count(),
        1,
        "`const make` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const BACKREF").count(),
        1,
        "`const BACKREF` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Top-level class declarations' constructor
/// bodies are NOT indexed into `fn_body_symbol_refs` /
/// `fn_body_called_symbols`. When a hoisted initializer eagerly invokes
/// `new ClassName()`, the constructor body runs at module load — and any
/// later-declared top-level binding it reads will TDZ-throw.
///
/// Trace:
/// - `@Component({ providers: PROVIDERS }) class TestComponent {}`
/// - `class S { constructor() { TOKEN; } }` declared above.
/// - `const PROVIDERS = [new S()];` below the decorated class.
/// - `const TOKEN = 1;` below `PROVIDERS`.
///
/// BFS pops `PROVIDERS`: `init_symbols = {S}`, `init_called_symbols = {S}`
/// (recorded by `record_direct_callee` on `new S()`). Without class
/// indexing, the closure of `init_called_symbols` under
/// `fn_body_called_symbols` stays `{S}` and `fn_body_symbol_refs.get(&S)`
/// is empty. Safe-skip guard passes. `PROVIDERS` is planned. BFS chases
/// `S` (transitive): not in `symbol_to_stmt`, not in `eagerly_called`
/// (since `S` is a class, not a function decl) → nothing happens. `TOKEN`
/// never enters the worklist; it stays below the class. At runtime,
/// hoisted `new S()` reads `TOKEN` in TDZ.
///
/// Fix: index every top-level class declaration's constructor body (and
/// eager class parts) into `fn_body_symbol_refs` / `fn_body_called_symbols`.
/// Then `S` becomes `eagerly_called` once `PROVIDERS`'s
/// `init_called_symbols` is folded in, and the BFS chases the class
/// "body" refs (which include `TOKEN`).
///
/// Assert: `const TOKEN` precedes `const PROVIDERS` AND `const PROVIDERS`
/// precedes `class TestComponent` — both transitively hoisted.
#[test]
fn component_eager_new_class_constructor_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
class S { constructor() { TOKEN; } }
@Component({ selector: 'x', template: '', providers: PROVIDERS })
class TestComponent {}
const PROVIDERS = [new S()];
const TOKEN = 1;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let providers_pos = result.code.find("const PROVIDERS").unwrap_or_else(|| {
        panic!("Expected `const PROVIDERS` to be present.\nCode:\n{}", result.code)
    });
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < providers_pos,
        "`const TOKEN` (read inside `class S`'s constructor body which is \
         eagerly invoked by `new S()` in `PROVIDERS`) must be hoisted above \
         `const PROVIDERS`. token@{token_pos} providers@{providers_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        providers_pos < class_pos,
        "`const PROVIDERS` must be hoisted above `class TestComponent`. \
         providers@{providers_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const PROVIDERS").count(),
        1,
        "`const PROVIDERS` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `E::ClassExpression(_) => {}` in
/// `collect_expr_symbols` drops the eager parts of a class expression —
/// the `super_class` expression, computed keys, static field initializers,
/// and static blocks. Those fire when the class expression is *defined*,
/// not lazily when its methods run.
///
/// Trace:
/// - `@Component({ providers: PROVIDERS }) class TestComponent {}`
/// - `const PROVIDERS = [class extends BASE {}];`
/// - `const BASE = class {};`
///
/// Without the fix, `PROVIDERS`'s `init_symbols` is empty (class expr is
/// opaque), so `BASE` never enters the worklist. `PROVIDERS` is hoisted
/// above `TestComponent` but `BASE` stays below — at runtime, hoisted
/// `[class extends BASE {}]` evaluates and reads `BASE` in TDZ.
///
/// Fix: walk `super_class`, computed keys on all members, static field
/// initializers, static accessor initializers, and static blocks.
///
/// Assert: `const BASE` precedes `const PROVIDERS` AND `const PROVIDERS`
/// precedes `class TestComponent`.
#[test]
fn component_class_expr_super_class_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: PROVIDERS })
class TestComponent {}
const PROVIDERS = [class extends BASE {}];
const BASE = class {};
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let base_pos = result
        .code
        .find("const BASE")
        .unwrap_or_else(|| panic!("Expected `const BASE` to be present.\nCode:\n{}", result.code));
    let providers_pos = result.code.find("const PROVIDERS").unwrap_or_else(|| {
        panic!("Expected `const PROVIDERS` to be present.\nCode:\n{}", result.code)
    });
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        base_pos < providers_pos,
        "`const BASE` (read by `class extends BASE {{}}` inside `PROVIDERS`) \
         must be hoisted above `const PROVIDERS`. base@{base_pos} \
         providers@{providers_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        providers_pos < class_pos,
        "`const PROVIDERS` must be hoisted above `class TestComponent`. \
         providers@{providers_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const BASE").count(),
        1,
        "`const BASE` should appear exactly once.\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const PROVIDERS").count(),
        1,
        "`const PROVIDERS` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `make()` is eagerly invoked by the decorator. Inside `make`'s body an
/// inline class expression `class extends TOKEN {}` evaluates eagerly, so the
/// `super_class` reference to `TOKEN` should flow into the eager-evaluation
/// set. `FunctionBodyIdentVisitor::visit_class` is a no-op which silently
/// drops these refs unless it walks the class's eager parts via
/// `walk_class_eager_parts`.
#[test]
fn component_eager_fn_body_inline_class_extends_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
function make() { return class extends TOKEN {}; }
const TOKEN = class {};
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read by `class extends TOKEN {{}}` inside the body of \
         an eagerly-called function) must be hoisted above the class to avoid \
         TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// `(cond ? makeA : makeB)()` invokes one of `makeA`/`makeB`. Both branches
/// can run, so `record_direct_callee` must descend into the consequent and
/// alternate of a `ConditionalExpression` callee and add both identifiers to
/// `called`. Without this, neither callee body is chased and `TOKEN` stays
/// declared below the class.
#[test]
fn component_eager_conditional_callee_chases_both_branches() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
const cond = true;
@Component({ selector: 'x', template: '', providers: (cond ? makeA : makeB)() })
class TestComponent {}
function makeA() { return TOKEN; }
function makeB() { return TOKEN; }
const TOKEN = 1;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside both branches of a conditional callee \
         `(cond ? makeA : makeB)()`) must be hoisted above the class to avoid \
         TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Inside an eagerly-called function `outer()`, a tagged template
/// `` tag`hello` `` invokes `tag`. `FunctionBodyIdentVisitor` must override
/// `visit_tagged_template_expression` and record the tag as a callee
/// (direct/indirect/bind) — otherwise the default walk adds `tag` to `out`
/// only, `tag` never enters `eagerly_called`, and the late `TOKEN` reference
/// inside `tag`'s body is never chased.
#[test]
fn component_eager_fn_body_tagged_template_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: outer() })
class TestComponent {}
function outer() { return tag`hello`; }
function tag(_strings: TemplateStringsArray) { return TOKEN; }
const TOKEN = 1;
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside the body of a tagged-template tag invoked \
         from an eagerly-called function) must be hoisted above the class to \
         avoid TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Decorator metadata uses a tagged template whose tag is produced by
/// `.bind` / `.call` / `.apply`. The tag function fires at class-definition
/// time, so its body refs must enter the eagerly-called closure — same
/// treatment `E::CallExpression` / `E::NewExpression` already get.
#[test]
fn component_tagged_template_bind_tag_chases_late_const() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make.bind(null)`hello` })
class TestComponent {}
function make() { return [{ provide: TOKEN, useValue: 0 }]; }
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside `make`'s body, called via a `.bind`-tagged \
         template in decorator metadata) must be hoisted above the class to avoid \
         TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// An eagerly-called function-valued binding declared *before* the
/// decorated class is itself already initialized — but the function body
/// it stores still fires when the decorator calls it, and that body's
/// later-declared reads (`TOKEN` below) are TDZ-relevant. The BFS used
/// to skip the body chase entirely when the binding's stmt_start was
/// before the class's body end, leaving `TOKEN` unhoisted and the
/// emitted Ivy definition throwing at module load.
#[test]
fn component_pre_class_fn_valued_binding_chases_body_refs() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
const make = () => [{ provide: TOKEN, useValue: 0 }];
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside the body of a pre-class fn-valued binding \
         called by the decorator) must be hoisted above the class to avoid TDZ. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Decorator metadata invokes a `.call`-style indirect callee whose
/// receiver is a conditional expression: `(cond ? makeA : makeB).call(null)`.
/// `record_indirect_callee` must descend through the conditional/logical/
/// sequence wrapper to reach the underlying identifiers — otherwise neither
/// `makeA` nor `makeB` enters the eagerly-called closure and `TOKEN` (read
/// from their bodies) is left unhoisted.
#[test]
fn component_eager_indirect_callee_descends_through_conditional() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
const cond = true;
@Component({ selector: 'x', template: '', providers: (cond ? makeA : makeB).call(null) })
class TestComponent {}
function makeA() { return [{ provide: TOKEN, useValue: 0 }]; }
function makeB() { return [{ provide: TOKEN, useValue: 1 }]; }
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside both branches of a conditional indirect callee \
         `(cond ? makeA : makeB).call(null)`) must be hoisted above the class to \
         avoid TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Decorator metadata invokes a `.bind`-style callee whose receiver is a
/// conditional expression: `(cond ? makeA : makeB).bind(null)()`.
/// `record_bind_callee` must descend through the conditional/logical/
/// sequence wrapper on the bind receiver to reach the underlying
/// identifiers — otherwise neither `makeA` nor `makeB` enters the
/// eagerly-called closure and `TOKEN` (read from their bodies) is left
/// unhoisted.
#[test]
fn component_eager_bind_callee_descends_through_conditional() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
const cond = true;
@Component({ selector: 'x', template: '', providers: (cond ? makeA : makeB).bind(null)() })
class TestComponent {}
function makeA() { return [{ provide: TOKEN, useValue: 0 }]; }
function makeB() { return [{ provide: TOKEN, useValue: 1 }]; }
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` (read inside both branches of a conditional bind callee \
         `(cond ? makeA : makeB).bind(null)()`) must be hoisted above the class to \
         avoid TDZ. token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

/// Both the cascade un-planning pass and the topological-precompute pass
/// derive a per-statement `stmt_called` set. They must compute it with the
/// SAME shape — seed with `init_called_symbols`, fold in fn-valued binding
/// symbols (when eagerly called), then close under `fn_body_called_symbols`.
/// If the two passes disagree, the topo edge expansion may miss a dependency
/// edge through a fn-valued binding's body chain, leaving a hoisted
/// dependent emitted before its dependee.
///
/// Engineered shape: `make = () => inner()` calls `inner()`, whose body
/// reads `TOKEN`. The final emission order must place `const TOKEN` BEFORE
/// the hoisted `const make = () => inner();` so that when `make()` runs at
/// module load the eventual `TOKEN` read is initialized.
#[test]
fn component_topo_symmetric_eager_set_with_fn_valued_binding_chain() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';
@Component({ selector: 'x', template: '', providers: make() })
class TestComponent {}
const make = () => inner();
function inner() { return TOKEN; }
const TOKEN = 'tok';
"#;
    let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let token_pos = result
        .code
        .find("const TOKEN")
        .unwrap_or_else(|| panic!("Expected `const TOKEN` to be present.\nCode:\n{}", result.code));
    let make_pos = result
        .code
        .find("const make")
        .unwrap_or_else(|| panic!("Expected `const make` to be present.\nCode:\n{}", result.code));
    let class_pos = result.code.find("class TestComponent").unwrap_or_else(|| {
        panic!("Expected `class TestComponent` to be present.\nCode:\n{}", result.code)
    });

    assert!(
        token_pos < class_pos,
        "`const TOKEN` must be hoisted above the class. \
         token@{token_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        make_pos < class_pos,
        "`const make` must be hoisted above the class. \
         make@{make_pos} class@{class_pos}\nCode:\n{}",
        result.code
    );
    assert!(
        token_pos < make_pos,
        "`const TOKEN` must precede `const make` so that `make()` (called at \
         module load via the decorator) reads an initialized `TOKEN` through \
         `inner()`. token@{token_pos} make@{make_pos}\nCode:\n{}",
        result.code
    );
    assert_eq!(
        result.code.matches("const TOKEN").count(),
        1,
        "`const TOKEN` should appear exactly once.\nCode:\n{}",
        result.code
    );
}

// =============================================================================
// Regression tests for issue #288 — type-only constructor parameters
// =============================================================================
//
// When a constructor parameter's type annotation comes from a type-only import
// (`import type { X }` or `import { type X }`), TypeScript erases the import at
// runtime. The Angular reference compiler responds with `ɵɵinvalidFactory()`
// (`ValueUnavailableKind.TYPE_ONLY_IMPORT`) instead of an `ɵɵdirectiveInject(X)`
// call that would crash with `token must be defined`.

/// Issue #288: `import type { MyService }` constructor params must not emit a
/// runtime DI token.
///
/// A type-only import is erased at runtime, so neither a namespace import for
/// the module nor a `directiveInject(i1.MyService)` call may be generated. The
/// factory body must instead be `ɵɵinvalidFactory()`.
#[test]
fn test_component_type_only_import_emits_invalid_factory() {
    let allocator = Allocator::default();

    let source = r"
import { Component } from '@angular/core';
import type { MyService } from './my-service';

@Component({
    selector: 'x',
    template: '',
    standalone: true,
})
export class X {
    constructor(private svc: MyService) {}
}
";

    let result = transform_angular_file(&allocator, "x.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    assert!(
        !code.lines().any(|l| l.trim_start().starts_with("import * as ")
            && (l.contains("'./my-service'") || l.contains("\"./my-service\""))),
        "type-only import './my-service' must not be promoted to a runtime namespace import.\nOutput:\n{code}"
    );

    assert!(
        !code.contains("i1.MyService"),
        "Must not emit namespace-prefixed `i1.MyService` for a type-only import.\nOutput:\n{code}"
    );
    assert!(
        !code.contains("directiveInject(MyService)"),
        "Must not emit a bare `directiveInject(MyService)` for a type-only import.\nOutput:\n{code}"
    );

    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");
    assert!(
        factory_section.contains("ɵɵinvalidFactory()"),
        "Type-only DI param must produce `ɵɵinvalidFactory()`. Factory:\n{factory_section}"
    );
}

/// Issue #288: `import { type MyService }` (inline type specifier) must behave
/// the same as a declaration-level `import type`.
#[test]
fn test_component_inline_type_specifier_emits_invalid_factory() {
    let allocator = Allocator::default();

    let source = r"
import { Component } from '@angular/core';
import { type MyService } from './my-service';

@Component({
    selector: 'x',
    template: '',
    standalone: true,
})
export class X {
    constructor(private svc: MyService) {}
}
";

    let result = transform_angular_file(&allocator, "x.component.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    assert!(
        !code.contains("from './my-service'") && !code.contains("from \"./my-service\""),
        "inline `type` specifier must not promote './my-service' to a runtime import.\nOutput:\n{code}"
    );
    assert!(
        !code.contains("i1.MyService"),
        "Must not emit namespace-prefixed `i1.MyService` for a type-only specifier.\nOutput:\n{code}"
    );

    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");
    assert!(
        factory_section.contains("ɵɵinvalidFactory()"),
        "Inline `type` specifier DI param must produce `ɵɵinvalidFactory()`. Factory:\n{factory_section}"
    );
}

/// Issue #288: directives with type-only DI tokens must also emit
/// `ɵɵinvalidFactory()` instead of a runtime namespace reference.
#[test]
fn test_directive_type_only_import_emits_invalid_factory() {
    let allocator = Allocator::default();

    let source = r"
import { Directive } from '@angular/core';
import type { MyService } from './my-service';

@Directive({
    selector: '[appX]',
    standalone: true,
})
export class XDirective {
    constructor(private svc: MyService) {}
}
";

    let result = transform_angular_file(&allocator, "x.directive.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    assert!(
        !code.lines().any(|l| l.trim_start().starts_with("import * as ")
            && (l.contains("'./my-service'") || l.contains("\"./my-service\""))),
        "type-only import './my-service' must not be promoted to a runtime namespace import.\nOutput:\n{code}"
    );
    assert!(
        !code.contains("i1.MyService"),
        "Must not emit namespace-prefixed `i1.MyService` for a type-only directive param.\nOutput:\n{code}"
    );

    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");
    assert!(
        factory_section.contains("ɵɵinvalidFactory()"),
        "Directive type-only DI param must produce `ɵɵinvalidFactory()`. Factory:\n{factory_section}"
    );
}

/// Issue #288: injectables (`@Injectable`) with type-only DI tokens must also
/// emit `ɵɵinvalidFactory()`.
#[test]
fn test_injectable_type_only_import_emits_invalid_factory() {
    let allocator = Allocator::default();

    let source = r"
import { Injectable } from '@angular/core';
import type { MyDep } from './my-dep';

@Injectable({ providedIn: 'root' })
export class MyService {
    constructor(private dep: MyDep) {}
}
";

    let result = transform_angular_file(&allocator, "my-service.ts", source, None, None);

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    assert!(
        !code.lines().any(|l| l.trim_start().starts_with("import * as ")
            && (l.contains("'./my-dep'") || l.contains("\"./my-dep\""))),
        "type-only import './my-dep' must not be promoted to a runtime namespace import.\nOutput:\n{code}"
    );

    let factory_section =
        code.split("ɵfac").nth(1).expect("Should have a factory definition (ɵfac)");
    assert!(
        factory_section.contains("ɵɵinvalidFactory()"),
        "Injectable type-only DI param must produce `ɵɵinvalidFactory()`. Factory:\n{factory_section}"
    );
}

/// Regression for https://github.com/voidzero-dev/oxc-angular-compiler/issues/291.
///
/// `transformAngularFile` must surface external `templateUrl` / `styleUrls`
/// paths in `result.dependencies` so build tools (the Vite plugin in
/// particular) can register them as watch dependencies.
#[test]
fn test_resource_dependencies_reported_when_resolved() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-x',
    templateUrl: './x.html',
    styleUrls: ['./x.css', './shared.css'],
    standalone: true,
})
export class XComponent {}
"#;

    let mut templates = std::collections::HashMap::new();
    templates.insert("./x.html".to_string(), "<div>x</div>".to_string());
    let mut styles = std::collections::HashMap::new();
    styles.insert("./x.css".to_string(), vec![".x{color:red}".to_string()]);
    styles.insert("./shared.css".to_string(), vec![".shared{}".to_string()]);
    let resources = ResolvedResources { templates, styles };

    let result =
        transform_angular_file(&allocator, "x.component.ts", source, None, Some(&resources));
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    assert!(
        result.dependencies.contains(&"./x.html".to_string()),
        "templateUrl should appear in dependencies, got: {:?}",
        result.dependencies
    );
    assert!(
        result.dependencies.contains(&"./x.css".to_string()),
        "styleUrl ./x.css should appear in dependencies, got: {:?}",
        result.dependencies
    );
    assert!(
        result.dependencies.contains(&"./shared.css".to_string()),
        "styleUrl ./shared.css should appear in dependencies, got: {:?}",
        result.dependencies
    );
}

/// Regression for https://github.com/voidzero-dev/oxc-angular-compiler/issues/291.
///
/// External resource paths must be reported in `result.dependencies` even when
/// no `ResolvedResources` map is supplied. Build tools call the compiler from
/// their loader, and they need to know which sibling files to watch *before*
/// they can pre-load and supply the resource contents. Returning an empty
/// `dependencies` list when `resolvedResources` is `None` silently breaks HMR.
#[test]
fn test_resource_dependencies_reported_without_resolved_resources() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-x',
    templateUrl: './x.html',
    styleUrls: ['./x.css', './shared.css'],
    standalone: true,
})
export class XComponent {}
"#;

    // No ResolvedResources passed — mirrors the path build tools hit on the
    // first transform pass before they've discovered the sibling files.
    let result = transform_angular_file(&allocator, "x.component.ts", source, None, None);

    assert!(
        result.dependencies.contains(&"./x.html".to_string()),
        "templateUrl must be reported in dependencies even when unresolved, got: {:?}",
        result.dependencies
    );
    assert!(
        result.dependencies.contains(&"./x.css".to_string())
            && result.dependencies.contains(&"./shared.css".to_string()),
        "styleUrls must be reported in dependencies even when unresolved, got: {:?}",
        result.dependencies
    );
}

/// styleUrls must be tracked even when the component uses an inline
/// `template:` string. styles and templates are independent resource axes.
#[test]
fn test_inline_template_with_external_styles_reports_style_dependencies() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-x',
    template: '<div>inline</div>',
    styleUrl: './x.css',
    standalone: true,
})
export class XComponent {}
"#;

    let result = transform_angular_file(&allocator, "x.component.ts", source, None, None);
    assert!(
        result.dependencies.contains(&"./x.css".to_string()),
        "styleUrl alongside an inline template must still appear in dependencies, got: {:?}",
        result.dependencies
    );
}

/// Regression for https://github.com/voidzero-dev/oxc-angular-compiler/issues/290.
///
/// A `@Component` template like `<div><span></div>` used to compile silently —
/// `result.diagnostics` was empty even though `</div>` jumps past an unclosed
/// `<span>`. The HTML parser now flags this exactly like Angular's reference
/// parser, and the diagnostic must propagate all the way out to the file-level
/// transform result so consumers (vite plugin, NAPI bindings) can surface it.
#[test]
fn test_malformed_template_surfaces_parse_diagnostic() {
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-bad',
    template: '<div><span></div>',
    standalone: true,
})
export class BadComponent {}
"#;

    let result = transform_angular_file(&allocator, "bad.component.ts", source, None, None);

    assert!(
        result.has_errors(),
        "Malformed template must produce diagnostics, but `result.diagnostics` was empty. Output:\n{}",
        result.code
    );
    let mentions_unclosed = result.diagnostics.iter().any(|d| {
        let s = format!("{d}");
        s.contains("Unexpected closing tag \"div\"")
    });
    assert!(
        mentions_unclosed,
        "Diagnostic should call out the unexpected closing tag, got: {:?}",
        result.diagnostics
    );
}

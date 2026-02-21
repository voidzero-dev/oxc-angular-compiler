//! Integration tests for the Angular template compiler.
//!
//! These tests verify the complete compilation pipeline from
//! HTML template string to JavaScript output.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    AngularVersion, ResolvedResources, TransformOptions as ComponentTransformOptions,
    output::ast::FunctionExpr,
    output::emitter::JsEmitter,
    parser::html::HtmlParser,
    pipeline::{emit::compile_template, ingest::ingest_component},
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions},
    transform_angular_file,
};
use oxc_span::Atom;

/// Compiles an Angular template to JavaScript.
fn compile_template_to_js(template: &str, component_name: &str) -> String {
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
    let mut job = ingest_component(&allocator, Atom::from(component_name), r3_result.nodes);

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
    // HTML entity &times; between two interpolations should produce \u00D7 in the output
    let js = compile_template_to_js("<div>{{ a }}&times;{{ b }}</div>", "TestComponent");
    // Should produce: textInterpolate2("", ctx.a, "\u00D7", ctx.b)
    // Note: × (multiplication sign) = U+00D7, escaped as \u00D7
    assert!(
        js.contains(r#"textInterpolate2("",ctx.a,"\u00D7",ctx.b)"#),
        "Expected textInterpolate2 with escaped times character. Got:\n{js}"
    );
}

#[test]
fn test_html_entity_at_start_of_interpolation() {
    // Entity at start: &times;{{ a }}
    let js = compile_template_to_js("<div>&times;{{ a }}</div>", "TestComponent");
    // Should produce: textInterpolate1("\u00D7", ctx.a)
    // Note: × (multiplication sign) = U+00D7, escaped as \u00D7
    assert!(
        js.contains(r#"textInterpolate1("\u00D7",ctx.a)"#)
            || js.contains(r#"textInterpolate("\u00D7",ctx.a)"#),
        "Expected textInterpolate with escaped times character at start. Got:\n{js}"
    );
}

#[test]
fn test_multiple_html_entities_between_interpolations() {
    // Multiple entities: {{ a }}&nbsp;&times;&nbsp;{{ b }}
    let js =
        compile_template_to_js("<div>{{ a }}&nbsp;&times;&nbsp;{{ b }}</div>", "TestComponent");
    // Should produce: textInterpolate2("", ctx.a, "\u00A0\u00D7\u00A0", ctx.b)
    // Note: &nbsp; = U+00A0, &times; = U+00D7, both escaped as \uNNNN
    assert!(
        js.contains(r#"textInterpolate2("",ctx.a,"\u00A0\u00D7\u00A0",ctx.b)"#),
        "Expected textInterpolate2 with escaped Unicode entities. Got:\n{js}"
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
        r#"<div i18n>
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
</div>"#,
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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

#[test]
fn test_nested_for_loops() {
    let js = compile_template_to_js(
        r"@for (group of groups; track group.id) { <div>@for (item of group.items; track item.id) { <span>{{item.name}}</span> }</div> }",
        "TestComponent",
    );
    insta::assert_snapshot!("nested_for_loops", js);
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
        r#"<div>{{ ((data$ | async) || fallback)?.name }}</div>"#,
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

    let result = transform_angular_file(
        &allocator,
        "styled.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "multi-styled.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

    assert_eq!(result.component_count, 1);
    assert!(!result.has_errors());

    // Verify both styles are included
    assert!(result.code.contains(".first"), "Should contain first style");
    assert!(result.code.contains(".second"), "Should contain second style");

    insta::assert_snapshot!("component_with_multiple_styles", result.code);
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

    let result = transform_angular_file(
        &allocator,
        "no-styles.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "badge.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "multi.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "grid.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "menu.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "multi-menu.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "fab.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "fab.ts",
        &source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "fab.ts",
        source,
        &ComponentTransformOptions::default(),
        Some(&resources),
    );

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

    let result = transform_angular_file(
        &allocator,
        "drawer.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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
    // Test that [field] (control binding) is extracted into the consts array.
    // Before the fix, UpdateOp::Control was not handled in attribute extraction,
    // causing the control binding name ("field") to be missing from the element's
    // extracted attributes. This resulted in duplicate/shifted const entries.
    let allocator = Allocator::default();
    let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'test-comp',
    template: '<cu-comp [field]="myField" [open]="isOpen"></cu-comp>',
    standalone: true,
})
export class TestComponent {
    myField = 'test';
    isOpen = false;
}
"#;

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    eprintln!("OUTPUT:\n{}", result.code);

    // The consts array should contain "field" as an extracted property binding name.
    // Without the fix, only "open" would appear (missing "field"), resulting in
    // incorrect const array entries and shifted indices.
    assert!(
        result.code.contains(r#""field""#),
        "Consts should contain 'field' from control binding extraction. Output:\n{}",
        result.code
    );

    // Both "field" and "open" should appear in the same consts entry (same element).
    // The property marker (3) should precede both names.
    // Expected: [3, "field", "open"] (property marker followed by both binding names)
    // Without the fix: [3, "open"] (missing "field")
    let has_both_in_same_const =
        result.code.lines().any(|line| line.contains(r#""field""#) && line.contains(r#""open""#));
    assert!(
        has_both_in_same_const,
        "Both 'field' and 'open' should appear in the same consts entry. Output:\n{}",
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
        r#"<cu-comp [field]="myField$ | async"></cu-comp>"#,
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
        r#"<cu-comp [field]="(settings$ | async)?.workload?.field" [title]="name | uppercase"></cu-comp>"#,
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
        r#"<div *ngIf="show"><cu-comp [field]="(settings$ | async)?.workload?.field" [title]="name | uppercase"></cu-comp></div>"#,
        "TestComponent",
    );
    eprintln!("OUTPUT:\n{js}");
    assert!(!js.contains("pipeBind1(0,"), "pipeBind should NOT use slot 0. Output:\n{js}");
}

/// Test pipe slot in [field] binding inside @if block.
#[test]
fn test_pipe_in_field_in_if_block() {
    let js = compile_template_to_js(
        r#"@if (show) {<cu-comp [field]="(settings$ | async)?.workload?.field" [title]="name | uppercase"></cu-comp>}"#,
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

    let result = transform_angular_file(
        &allocator,
        "dashboard-box.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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
    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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
    // listener-like op in an embedded view. Angular's ngtsc always keeps restoreView/resetView
    // in animation handler callbacks in embedded views, even when the return value is a simple
    // string literal that doesn't reference the view context.
    //
    // Expected NG output pattern:
    //   i0.ɵɵanimateEnter(function ...() {
    //     i0.ɵɵrestoreView(_r1);
    //     return i0.ɵɵresetView("animate-in");
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
    assert!(
        js.contains("restoreView"),
        "Animation handler in embedded view should keep restoreView.\nGenerated JS:\n{js}"
    );
    assert!(
        js.contains("resetView"),
        "Animation handler in embedded view should keep resetView.\nGenerated JS:\n{js}"
    );
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
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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
    let source = r#"
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
"#;

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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
    let source = r#"
import { Component } from '@angular/core';
import { AsyncPipe, DatePipe, SlicePipe } from '@angular/common';

@Component({
  selector: 'app-test',
  standalone: true,
  imports: [AsyncPipe, DatePipe, SlicePipe],
  template: `<div>Hello</div>`
})
export class TestComponent {}
"#;

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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
    let source = r#"
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
"#;

    let options = ComponentTransformOptions::default();
    let result = transform_angular_file(&allocator, "test.ts", source, &options, None);

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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
        r#"<span i18n>{{ a }} and {{ b }} and {{ c }} and {{ b | uppercase }}</span>"#,
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
        r#"<div i18n>{{ name }} {count, plural, =1 {({{ amount }} credits x 1 user)} other {({{ amount }} credits x {{ count | number }} users)}}</div>"#,
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
    let source = r#"
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
"#;

    let options = ComponentTransformOptions {
        emit_class_metadata: true,
        ..ComponentTransformOptions::default()
    };

    let result = transform_angular_file(&allocator, "test.component.ts", source, &options, None);

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
        "setClassMetadata ctor_parameters should use namespace-prefixed type (i1.SomeService) for imported constructor parameter. Metadata section:\n{}",
        metadata_section
    );
    assert!(
        !metadata_section.contains("type:SomeService}"),
        "setClassMetadata should NOT use bare type name for imported types. Metadata section:\n{}",
        metadata_section
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
    let source = r#"
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
"#;

    let options = ComponentTransformOptions {
        emit_class_metadata: true,
        ..ComponentTransformOptions::default()
    };

    let result = transform_angular_file(&allocator, "test.component.ts", source, &options, None);

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
        "setClassMetadata should use namespace-prefixed type even with @Inject. Metadata section:\n{}",
        metadata_section
    );
}

/// Tests that when @Inject token differs from the type annotation (e.g., @Inject(DOCUMENT)
/// on a parameter typed as Document), the metadata type uses bare name since the type
/// annotation may reference a global or different module than the injection token.
#[test]
fn test_set_class_metadata_inject_differs_from_type() {
    let allocator = Allocator::default();
    let source = r#"
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
"#;

    let options = ComponentTransformOptions {
        emit_class_metadata: true,
        ..ComponentTransformOptions::default()
    };

    let result = transform_angular_file(&allocator, "test.component.ts", source, &options, None);

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
        "setClassMetadata should use bare type for globals when @Inject token differs. Metadata section:\n{}",
        metadata_section
    );
    // Should NOT add namespace prefix for Document
    assert!(
        !metadata_section.contains("i1.Document"),
        "setClassMetadata should NOT namespace-prefix global types. Metadata section:\n{}",
        metadata_section
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

    let result = transform_angular_file(
        &allocator,
        "icon.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );
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

    let result = transform_angular_file(
        &allocator,
        "icon.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );
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

    let result = transform_angular_file(
        &allocator,
        "test.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );
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
        r#"<span i18n>{count, plural, =1 {<strong>{{ name }}</strong> was deleted from {nestedCount, plural, =1 {<strong>{{ category }}</strong>} other {<strong>{{ category }}</strong> and {{ extra }} more}}} other {{{ count }} items deleted}}</span>"#,
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
    let source = r#"
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
"#;

    let result = transform_angular_file(
        &allocator,
        "toast-position-helper.directive.ts",
        source,
        &ComponentTransformOptions::default(),
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

    let source = r#"
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
"#;

    let result = transform_angular_file(
        &allocator,
        "multi-dep.directive.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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
    let source = r#"
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
"#;

    let result = transform_angular_file(
        &allocator,
        "chatbot-trigger.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

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

/// Regression: Multiple view queries must emit separate statements, not chained calls.
///
/// Bug: Multiple `@ViewChild`/`@ViewChildren` queries were chained as
/// `ɵɵviewQuery(pred1)(pred2)`, treating the return value of `ɵɵviewQuery` as a callable.
/// Angular 20's `ɵɵviewQuery` returns `void`, so chaining causes:
/// `TypeError: i0.ɵɵviewQuery(...) is not a function`.
///
/// Fix: Emit each query as a separate statement:
///   `ɵɵviewQuery(pred1); ɵɵviewQuery(pred2);`
#[test]
fn test_multiple_view_queries_emit_separate_statements() {
    let allocator = Allocator::default();

    // Reproduce the ClickUp LoginFormComponent pattern: multiple @ViewChild decorators
    let source = r#"
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
"#;

    let result = transform_angular_file(
        &allocator,
        "login-form.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Count separate ɵɵviewQuery calls - should be 3 (one per @ViewChild)
    let view_query_count = code.matches("ɵɵviewQuery(").count();
    assert_eq!(
        view_query_count, 3,
        "Should have exactly 3 separate ɵɵviewQuery calls. Found {view_query_count}. Output:\n{code}"
    );

    // Must NOT have chained calls: ɵɵviewQuery(...)(...) pattern
    // This regex-free check: after each `ɵɵviewQuery(` find the matching `)` and check
    // the next non-whitespace char is NOT `(`
    let query_fn = "ɵɵviewQuery(";
    for (start_idx, _) in code.match_indices(query_fn) {
        let after_fn = &code[start_idx + query_fn.len()..];
        // Find the closing paren (handle nested parens)
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
        // Check what comes after the closing paren
        let after_close = after_fn[end..].trim_start();
        assert!(
            !after_close.starts_with('('),
            "Found chained ɵɵviewQuery call (return value used as function). \
             Angular 20's ɵɵviewQuery returns void. Output:\n{code}"
        );
    }
}

/// Regression: Multiple content queries must emit separate statements, not chained calls.
///
/// Same issue as view queries but for `@ContentChild`/`@ContentChildren`.
/// The fix applies to both `create_view_queries_function` and `create_content_queries_function`.
#[test]
fn test_multiple_content_queries_emit_separate_statements() {
    let allocator = Allocator::default();

    let source = r#"
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
"#;

    let result = transform_angular_file(
        &allocator,
        "tabs.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Count separate ɵɵcontentQuery calls - should be 3
    let content_query_count = code.matches("ɵɵcontentQuery(").count();
    assert_eq!(
        content_query_count, 3,
        "Should have exactly 3 separate ɵɵcontentQuery calls. Found {content_query_count}. Output:\n{code}"
    );

    // Must NOT have chained calls
    let query_fn = "ɵɵcontentQuery(";
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
        let after_close = after_fn[end..].trim_start();
        assert!(
            !after_close.starts_with('('),
            "Found chained ɵɵcontentQuery call. Angular 20's query functions return void. Output:\n{code}"
        );
    }
}

/// Regression: Mixed view queries (signal + decorator) must all be separate statements.
///
/// Signal-based queries (`viewChild()`, `viewChildren()`) and decorator-based queries
/// (`@ViewChild`, `@ViewChildren`) can coexist on the same component. All of them must
/// emit as separate statements.
#[test]
fn test_mixed_signal_and_decorator_view_queries_separate_statements() {
    let allocator = Allocator::default();

    let source = r#"
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
"#;

    let result = transform_angular_file(
        &allocator,
        "mixed-query.component.ts",
        source,
        &ComponentTransformOptions::default(),
        None,
    );

    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);

    let code = &result.code;

    // Should have signal query calls AND decorator query calls, all separate
    let total_query_calls = code.matches("ɵɵviewQuery").count();
    assert!(
        total_query_calls >= 3,
        "Should have at least 3 view query calls (signal + decorator). Found {total_query_calls}. Output:\n{code}"
    );

    // Verify no chaining for any view query variant
    for query_fn in ["ɵɵviewQuerySignal(", "ɵɵviewQuery("] {
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
            let after_close = after_fn[end..].trim_start();
            assert!(
                !after_close.starts_with('('),
                "Found chained {query_fn} call. Output:\n{code}"
            );
        }
    }
}

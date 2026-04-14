//! Reproduction of playground issue
use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    output::ast::FunctionExpr,
    output::emitter::JsEmitter,
    parser::html::{HtmlParser, remove_whitespaces},
    pipeline::{emit::compile_template, ingest::ingest_component},
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions},
};
use oxc_str::Ident;

fn compile_template_to_js(template: &str, component_name: &str) -> String {
    let allocator = Allocator::default();
    let parser = HtmlParser::new(&allocator, template, "test.html");
    let html_result = parser.parse();
    assert!(html_result.errors.is_empty(), "HTML parse errors: {:?}", html_result.errors);
    // Apply whitespace removal like Angular's preserveWhitespaces: false
    let nodes = remove_whitespaces(&allocator, &html_result.nodes, true);
    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&nodes);
    assert!(r3_result.errors.is_empty(), "Transform errors: {:?}", r3_result.errors);
    let mut job = ingest_component(&allocator, Ident::from(component_name), r3_result.nodes);
    let result = compile_template(&mut job);
    let emitter = JsEmitter::new();
    let mut output = String::new();
    for decl in &result.declarations {
        output.push_str(&emitter.emit_statement(decl));
        output.push('\n');
    }
    output.push_str(&emit_function(&emitter, &result.template_fn));
    output
}

fn emit_function(emitter: &JsEmitter, func: &FunctionExpr<'_>) -> String {
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

/// Test that all *ngIf bindings are emitted in the update phase.
/// Three templates with *ngIf should produce three property calls.
///
/// This was a regression where single-op groups were being unnecessarily
/// moved around in the ordering phase, causing stale pointers when the
/// insertion points were later used.
#[test]
fn test_multiple_ngif_bindings() {
    let template = r#"<app-header *ngIf="showHeader$ | async"></app-header>
<div id="container">
  <div class="loading" *ngIf="loading">...</div>
  <router-outlet *ngIf="!loading"></router-outlet>
</div>"#;

    let js = compile_template_to_js(template, "AppComponent");

    // Count property calls with 'ngIf'
    let ngif_count = js.matches("property(\"ngIf\"").count();

    assert_eq!(ngif_count, 3, "Expected 3 ngIf property calls, got {ngif_count}.\nOutput:\n{js}");
}

/// Test that vars count is correct for repeater body views with multiple bindings.
/// This was a regression where cursor.replace_current() didn't update the cursor
/// to point to the new node, causing subsequent bindings to be skipped during
/// binding_specialization phase.
#[test]
fn test_badge_list_vars() {
    // Template with @for containing:
    // - Two property bindings on the same element: [variant] and [truncate]
    // - Text interpolation: {{ item }}
    // - Nested @if with condition: !last || isFiltered()
    // Expected: 4 vars in repeater body (1 per property binding + 1 for text + 1 for conditional)
    let template = r#"<div class="tw-inline-flex tw-flex-wrap tw-gap-2">
  @for (item of filteredItems(); track item; let last = $last) {
    <span bitBadge [variant]="variant()" [truncate]="truncate()">
      {{ item }}
    </span>
    @if (!last || isFiltered()) {
      <span class="tw-sr-only">, </span>
    }
  }
  @if (isFiltered()) {
    <span bitBadge [variant]="variant()">
      {{ "plusNMore" | i18n: (items().length - filteredItems().length).toString() }}
    </span>
  }
</div>"#;

    let js = compile_template_to_js(template, "BadgeListComponent");

    // Verify the repeater body has 4 vars (matching TypeScript compiler)
    // The repeaterCreate call should look like:
    // ɵɵrepeaterCreate(slot, fn, decls, vars, ...)
    // where vars=4
    assert!(
        js.contains("repeaterCreate(") && js.contains(",4,"),
        "Expected repeater body to have 4 vars, but got: {js}"
    );
}

/// Test that property binding on an element with SVG children is properly emitted.
/// This was a bug where the [title] property binding on a span element was not being
/// generated in the update phase.
#[test]
fn test_title_property_binding_with_svg_child() {
    // Simplified template - just span with title binding
    let template = r#"<span [title]="title() || text()">content</span>"#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should be emitted in the update phase
    // as domProperty("title", (ctx.title() || ctx.text())) BEFORE any advance()
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test that property binding on an element with SVG children is properly emitted.
/// This variant adds leading whitespace which triggers the bug.
#[test]
fn test_title_property_binding_with_leading_whitespace() {
    // Adding leading whitespace before the span - this is the key difference
    let template = r#"
    <span [title]="title() || text()">content</span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should be emitted in the update phase
    // as domProperty("title", (ctx.title() || ctx.text()))
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test that property binding on span is emitted when it has an SVG child.
#[test]
fn test_title_property_binding_with_svg() {
    // Span with title binding containing an SVG child
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg">test</svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should be emitted in the update phase
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with SVG having bindings - this is closer to the actual problem case.
#[test]
fn test_title_property_binding_with_svg_having_bindings() {
    // SVG child has bindings, which might interfere with span's binding
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()">test</svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with SVG having text child with bindings
#[test]
fn test_title_property_binding_with_svg_text_child() {
    // SVG with text child that has bindings
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()">
        <text [attr.fill]="textColor()">hello</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with ngClass binding - might be the culprit
#[test]
fn test_title_property_binding_with_ngclass() {
    // SVG with ngClass binding
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [ngClass]="classList()">test</svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with interpolated attribute binding
#[test]
fn test_title_property_binding_with_interpolated_attr() {
    // SVG with interpolated viewBox
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" attr.viewBox="0 0 {{ svgSize }} {{ svgSize }}">test</svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with pointer-events static attribute - this might be the culprit!
#[test]
fn test_title_property_binding_with_pointer_events() {
    // SVG with pointer-events static attribute
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" pointer-events="none">test</svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with combined attributes that are in the full template
#[test]
fn test_title_property_binding_combined() {
    // Combined attributes closer to full template
    let template = r#"
    <span [title]="title() || text()">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        pointer-events="none"
        [style.backgroundColor]="backgroundColor()"
        [ngClass]="classList()"
        attr.viewBox="0 0 {{ svgSize }} {{ svgSize }}"
      >
        <text [attr.fill]="textColor()">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with style bindings on text element
#[test]
fn test_title_with_text_style_bindings() {
    // Text with style bindings - narrowing down the issue
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg">
        <text [style.fontWeight]="svgFontWeight" [style.fontSize.px]="svgFontSize">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with style bindings AND interpolation on text element
#[test]
fn test_title_with_text_style_and_interpolation() {
    // Text with style bindings AND interpolation - this is the trigger
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg">
        <text [style.fontWeight]="svgFontWeight" [style.fontSize.px]="svgFontSize">{{ displayChars() }}</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test combination of svg bindings + text bindings
#[test]
fn test_title_svg_and_text_bindings() {
    // This has all the svg and text bindings
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()" [ngClass]="classList()">
        <text [attr.fill]="textColor()" [style.fontWeight]="svgFontWeight" [style.fontSize.px]="svgFontSize">{{ displayChars() }}</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test: svg with style and ngClass bindings (without text child)
#[test]
fn test_title_svg_style_and_ngclass() {
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()" [ngClass]="classList()">test</svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test: svg with style and ngClass + text child with attr binding
#[test]
fn test_title_svg_bindings_and_text_attr() {
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()" [ngClass]="classList()">
        <text [attr.fill]="textColor()">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test: svg bindings + text with attr AND style bindings (no interpolation)
#[test]
fn test_title_svg_text_attr_and_style() {
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()" [ngClass]="classList()">
        <text [attr.fill]="textColor()" [style.fontWeight]="svgFontWeight">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Minimal: text with attr AND style binding (no svg bindings)
#[test]
fn test_title_text_attr_and_style_minimal() {
    let template = r#"
    <span [title]="title()">
      <svg xmlns="http://www.w3.org/2000/svg">
        <text [attr.fill]="textColor()" [style.fontWeight]="svgFontWeight">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// svg with style only + text with attr+style
#[test]
fn test_title_svg_style_only_text_attr_style() {
    let template = r#"
    <span [title]="title()">
      <svg xmlns="http://www.w3.org/2000/svg" [style.backgroundColor]="backgroundColor()">
        <text [attr.fill]="textColor()" [style.fontWeight]="svgFontWeight">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// svg with ngClass only + text with attr+style
#[test]
fn test_title_svg_ngclass_only_text_attr_style() {
    let template = r#"
    <span [title]="title()">
      <svg xmlns="http://www.w3.org/2000/svg" [ngClass]="classList()">
        <text [attr.fill]="textColor()" [style.fontWeight]="svgFontWeight">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// MINIMAL FAILING CASE: svg with style+ngClass + text with attr+style
#[test]
fn test_title_svg_style_ngclass_text_attr_style_minimal() {
    let template = r#"<span [title]="title()"><svg [style.a]="a()" [ngClass]="b()"><text [attr.c]="c()" [style.d]="d()">x</text></svg></span>"#;

    let js = compile_template_to_js(template, "AvatarComponent");

    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Regression test: Ensure binding ordering doesn't drop bindings when
/// groups of bindings on different elements are processed.
/// This was a bug where the ordering phase would use stale pointers,
/// causing bindings at the start of the list and at the end to be dropped.
#[test]
fn test_binding_ordering_preserves_all_bindings() {
    // This template has bindings on 3 elements: span, svg, and text
    // The ordering phase groups bindings by target element and reorders within each group.
    // Previously, this would drop the span's [title] binding and text's [style.d] binding.
    let template = r#"<span [title]="title()"><svg [style.a]="a()" [ngClass]="b()"><text [attr.c]="c()" [style.d]="d()">x</text></svg></span>"#;
    let js = compile_template_to_js(template, "Test");

    // All 5 bindings should be present in the output
    assert!(js.contains("property(\"title\""), "title binding on span should be present");
    assert!(js.contains("styleProp(\"a\""), "style.a binding on svg should be present");
    assert!(js.contains("property(\"ngClass\""), "ngClass binding on svg should be present");
    assert!(js.contains("attribute(\"c\""), "attr.c binding on text should be present");
    assert!(js.contains("styleProp(\"d\""), "style.d binding on text should be present");
}

/// Test with style bindings with units on text element
#[test]
fn test_title_property_binding_with_style_units() {
    // Text with style bindings including units
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg">
        <text [style.fontWeight]="svgFontWeight" [style.fontSize.px]="svgFontSize">test</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with text interpolation
#[test]
fn test_title_property_binding_with_text_interpolation() {
    // Text with interpolation - this was missing in the failing output
    let template = r#"
    <span [title]="title() || text()">
      <svg xmlns="http://www.w3.org/2000/svg">
        <text>{{ displayChars() }}</text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test with all the bindings from the failing template
#[test]
fn test_title_property_binding_almost_full() {
    // Almost full template - omit the static attributes on text
    let template = r#"
    <span [title]="title() || text()">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        pointer-events="none"
        [style.backgroundColor]="backgroundColor()"
        [ngClass]="classList()"
        attr.viewBox="0 0 {{ svgSize }} {{ svgSize }}"
      >
        <text
          [attr.fill]="textColor()"
          [style.fontWeight]="svgFontWeight"
          [style.fontSize.px]="svgFontSize"
        >
          {{ displayChars() }}
        </text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should still be emitted
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );
}

/// Test that property binding on an element with SVG children is properly emitted.
/// Full bitwarden template version.
#[test]
fn test_title_property_binding_with_svg_child_full() {
    // Exact template from bitwarden's AvatarComponent
    let template = r#"
    <span [title]="title() || text()">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        pointer-events="none"
        [style.backgroundColor]="backgroundColor()"
        [ngClass]="classList()"
        attr.viewBox="0 0 {{ svgSize }} {{ svgSize }}"
      >
        <text
          text-anchor="middle"
          y="50%"
          x="50%"
          dy="0.35em"
          pointer-events="auto"
          [attr.fill]="textColor()"
          [style.fontWeight]="svgFontWeight"
          [style.fontSize.px]="svgFontSize"
          font-family='Inter,"Helvetica Neue",Helvetica,Arial,sans-serif,"Apple Color Emoji","Segoe UI Emoji","Segoe UI Symbol"'
        >
          {{ displayChars() }}
        </text>
      </svg>
    </span>
  "#;

    let js = compile_template_to_js(template, "AvatarComponent");

    // The span's [title] property binding should be emitted in the update phase
    // as domProperty("title", (ctx.title() || ctx.text())) BEFORE any advance()
    assert!(
        js.contains("domProperty(\"title\"") || js.contains("property(\"title\""),
        "Expected title property binding, but got:\n{js}"
    );

    // Verify the title binding comes before the first advance()
    // TypeScript output: domProperty("title", ...) then advance()
    // Oxc output (bug): advance() first, missing title binding
    if let Some(title_pos) =
        js.find("domProperty(\"title\"").or_else(|| js.find("property(\"title\""))
        && let Some(advance_pos) = js.find("advance()")
    {
        assert!(
            title_pos < advance_pos,
            "Title binding should come before first advance(). Got:\n{js}"
        );
    }
}

/// Test that unused reference variables are removed from listener handlers.
/// This was a bug where all local refs in scope were being added to every
/// listener's handler_ops, even if the listener didn't use them.
#[test]
fn test_listener_unused_refs_removed() {
    // This is a simplified version of the bitwarden ChipSelectComponent template
    // The click handler calls ctx.setMenuWidth() which doesn't use any local refs,
    // so the reference variables for #menuTrigger, #chipSelectButton, #menu
    // should NOT appear in the click handler.
    let template = r#"<button
    #menuTrigger="menuTrigger"
    (click)="setMenuWidth()"
    #chipSelectButton
  >Click me</button>
  <bit-menu #menu></bit-menu>"#;

    let js = compile_template_to_js(template, "ChipSelectComponent");

    // Find the click listener (handles both single and double quotes)
    let listener_start = js
        .find("listener(\"click\"")
        .or_else(|| js.find("listener('click'"))
        .expect("Should have click listener");
    let listener_end = js[listener_start..].find("});").expect("Should find end of listener");
    let listener_body = &js[listener_start..listener_start + listener_end + 3];

    // The listener should NOT contain reference() calls since ctx.setMenuWidth()
    // doesn't use any of the local refs
    assert!(
        !listener_body.contains("reference("),
        "Click handler should NOT contain reference() calls for unused refs.\nListener body:\n{listener_body}"
    );

    // The listener should NOT contain restoreView/resetView since the listener
    // doesn't reference any parent context variables. The optimizer should
    // strip restoreView/resetView entirely, leaving just `return ctx.setMenuWidth();`
    assert!(
        !listener_body.contains("restoreView("),
        "Click handler should NOT contain restoreView - handler doesn't use refs.\nListener body:\n{listener_body}"
    );
    assert!(
        !listener_body.contains("resetView("),
        "Click handler should NOT contain resetView - handler doesn't use refs.\nListener body:\n{listener_body}"
    );
}

/// Test that the expression parser correctly handles pipes inside parentheses with nullish coalescing.
#[test]
fn test_pipe_in_nullish_coalescing_expression() {
    use oxc_allocator::Allocator;
    use oxc_angular_compiler::ast::expression::AngularExpression;
    use oxc_angular_compiler::parser::expression::Parser;

    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, "placeholder() ?? ('search' | i18n)");
    let result = parser.parse_simple_binding();

    // Should not have errors
    assert!(result.errors.is_empty(), "Should parse without errors, got: {:?}", result.errors);

    // The AST should contain a BindingPipe
    fn contains_pipe(expr: &AngularExpression<'_>) -> bool {
        match expr {
            AngularExpression::BindingPipe(_) => true,
            AngularExpression::Binary(b) => contains_pipe(&b.left) || contains_pipe(&b.right),
            AngularExpression::ParenthesizedExpression(p) => contains_pipe(&p.expression),
            _ => false,
        }
    }

    assert!(
        contains_pipe(&result.ast),
        "Should contain a BindingPipe expression. Got: {:?}",
        result.ast
    );
}

/// Test that pipes inside nullish coalescing are properly compiled.
#[test]
fn test_pipe_in_nullish_coalescing_compilation() {
    // This is the SearchComponent pattern - pipe inside nullish coalescing
    let template = r#"<input [placeholder]="placeholder() ?? ('search' | i18n)">"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // The pipe declaration should be present
    assert!(js.contains("pipe("), "Should have pipe declaration. Got:\n{js}");

    // The pipeBind call should be present in the update phase
    assert!(js.contains("pipeBind"), "Should have pipeBind call in update phase. Got:\n{js}");
}

/// Test that pipes are not duplicated when there are multiple pipes and a template.
/// This reproduces the bug where an extra pipe is created at slot 10.
#[test]
fn test_no_duplicate_pipes_with_template() {
    // Template with multiple pipes and a template
    let template = r#"<label>{{ 'searchItems' | i18n }}</label>
<input [placeholder]="placeholder ?? ('search' | i18n)">
<ng-template #tooltip let-item>{{item}}</ng-template>"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Count pipe declarations - should be exactly 2 (one for each unique pipe use)
    let pipe_count = js.matches("ɵɵpipe(").count();
    assert_eq!(
        pipe_count, 2,
        "Expected exactly 2 pipe declarations, got {pipe_count}.\nOutput:\n{js}"
    );
}

/// Test that pipe inside structural directive's attribute does NOT create duplicate pipe.
/// The pipe for `'resetSearch' | i18n` inside the *ngIf'd button should only appear
/// in the embedded view, not in the root view.
#[test]
fn test_pipe_in_structural_directive_attr_no_duplicate() {
    // Simplified SearchComponent template - pipe inside *ngIf'd element's attribute
    let template = r#"<input [placeholder]="placeholder ?? ('search' | i18n)">
<button *ngIf="showButton" [attr.aria-label]="'resetSearch' | i18n"></button>"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Count pipe declarations in the MAIN function only
    // The main function should have exactly 1 pipe (for placeholder)
    // The embedded view function should have 1 pipe (for aria-label)
    let main_fn_start = js.find("function TestComponent_Template").expect("main fn should exist");
    let main_fn_end = js[main_fn_start..].find("}\n").expect("main fn should end") + main_fn_start;
    let main_fn = &js[main_fn_start..main_fn_end];

    let main_pipe_count = main_fn.matches("ɵɵpipe(").count();
    assert_eq!(
        main_pipe_count, 1,
        "Main function should have exactly 1 pipe declaration (for placeholder), got {main_pipe_count}.\nMain function:\n{main_fn}"
    );
}

/// Test that pipes are correctly placed BEFORE listeners on the same element.
///
/// This is a regression test for the SearchComponent case where:
/// - An input element has [placeholder]="placeholder() ?? ('search' | i18n)"
/// - The same input has (ngModelChange), (focus), (blur) listeners
///
/// The expected order in create block should be:
/// 1. elementStart(slot, 'input', ...)
/// 2. pipe(slot+1, 'pipeName')   <-- MUST come before listeners
/// 3. listener('ngModelChange', ...)
/// 4. listener('focus', ...)
/// 5. listener('blur', ...)
/// 6. elementEnd()
///
/// The bug was that pipes were being placed AFTER templates instead of
/// right after their target element.
#[test]
fn test_pipe_placement_before_listeners() {
    // Simplified SearchComponent template - input with pipe binding AND listeners
    // Using a simple pipe expression without nullish coalescing for clarity
    let template = r#"<input
        [placeholder]="searchText | uppercase"
        (ngModelChange)="onInputChange($event)"
        (focus)="onFocus()"
        (blur)="onBlur()"
    >"#;

    let js = compile_template_to_js(template, "SearchComponent");
    println!("Generated JS:\n{js}");

    // The pipe declaration should be present
    assert!(js.contains("pipe("), "Should have pipe declaration. Got:\n{js}");

    // The pipe should come BEFORE any listener
    let pipe_pos = js.find("pipe(").expect("Should have pipe");
    let listener_pos = js.find("listener(").expect("Should have listener");

    assert!(
        pipe_pos < listener_pos,
        "Pipe should come before listeners.\nPipe at: {pipe_pos}, Listener at: {listener_pos}\nOutput:\n{js}"
    );
}

/// Test that pipes are correctly placed BEFORE listeners when there's also a template.
///
/// This is the full SearchComponent case where there's:
/// - An input element with pipe binding and listeners
/// - A template after the input
///
/// The bug was that pipes were being placed AFTER the template instead of
/// right after their target element (the input).
#[test]
fn test_pipe_placement_before_listeners_with_template() {
    // SearchComponent pattern: input with pipe + listeners, followed by template
    // Using a simple pipe expression for clarity
    let template = r#"<input
        [placeholder]="searchText | uppercase"
        (ngModelChange)="onInputChange($event)"
        (focus)="onFocus()"
    >
    <ng-template #resultTemplate let-item>
        <span>{{item}}</span>
    </ng-template>"#;

    let js = compile_template_to_js(template, "SearchComponent");
    println!("Generated JS:\n{js}");

    // The pipe declaration should be present
    assert!(js.contains("pipe("), "Should have pipe declaration. Got:\n{js}");

    // The pipe should come BEFORE any listener
    let pipe_pos = js.find("pipe(").expect("Should have pipe");
    let listener_pos = js.find("listener(").expect("Should have listener");

    assert!(
        pipe_pos < listener_pos,
        "Pipe should come before listeners.\nPipe at: {pipe_pos}, Listener at: {listener_pos}\nOutput:\n{js}"
    );

    // The pipe should also come BEFORE the template
    if let Some(template_pos) = js.find("template(") {
        assert!(
            pipe_pos < template_pos,
            "Pipe should come before template.\nPipe at: {pipe_pos}, Template at: {template_pos}\nOutput:\n{js}"
        );
    }
}

/// Test that listener handlers in deeply nested *ngIf templates generate correct nextContext() calls.
///
/// This is a critical regression test for the bug where:
/// - A button inside 3 levels of *ngIf nesting
/// - Has a click handler accessing component properties
/// - The handler should call nextContext(3) to access the component context
/// - But was incorrectly using raw `ctx` which refers to the embedded view's context
///
/// TypeScript Angular compiler generates:
/// ```js
/// i0.ɵɵrestoreView(_r4);
/// const ctx_r1 = i0.ɵɵnextContext(3);
/// return i0.ɵɵresetView((ctx_r1.activeOption = ctx_r1.Option.A));
/// ```
///
/// Oxc was generating (incorrectly):
/// ```js
/// i0.ɵɵrestoreView(_r4);
/// return i0.ɵɵresetView((ctx.activeOption = ctx.Option.A));
/// ```
#[test]
fn test_listener_in_deeply_nested_ngif_uses_next_context() {
    // Template with 3 levels of *ngIf nesting
    // The click handler accesses component properties (activeOption, Option)
    let template = r#"<ng-container *ngIf="showOuter">
  <div *ngIf="showMiddle">
    <button *ngIf="showInner" (click)="activeOption = Option.A">Click</button>
  </div>
</ng-container>"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the innermost button template function (3 levels deep)
    // It should be named something like TestComponent_ng_container_0_div_1_button_1_Template
    let button_fn_match = js.find("button_1_Template").or_else(|| js.find("button_0_Template"));

    assert!(button_fn_match.is_some(), "Should have a button template function. Got:\n{js}");

    let button_fn_pos = button_fn_match.unwrap();
    let button_fn_section = &js[button_fn_pos..];

    // Find the listener inside the button function (handles both single and double quotes)
    let listener_match = button_fn_section
        .find("listener(\"click\"")
        .or_else(|| button_fn_section.find("listener('click'"));
    assert!(listener_match.is_some(), "Should have a click listener. Got:\n{button_fn_section}");

    let listener_pos = listener_match.unwrap();
    let listener_end = button_fn_section[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &button_fn_section[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    // Since we're 3 levels deep (*ngIf creates embedded views), we need nextContext(3)
    // or multiple nextContext() calls that sum to 3
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The listener should NOT use raw `ctx.` without a nextContext() variable
    // Look for patterns like `ctx.activeOption` without a preceding `ctx_r` variable
    let has_raw_ctx = listener_body.contains("(ctx.") && !listener_body.contains("ctx_r");
    assert!(
        !has_raw_ctx,
        "Listener should NOT use raw ctx. directly - should use ctx_rN from nextContext().\nListener body:\n{listener_body}"
    );
}

/// Test that ng-template let-bindings are correctly resolved in child views.
///
/// This tests the fix for the issue where child views inside an ng-template
/// (like @if blocks) incorrectly resolve let-binding variables by going
/// all the way to the root component instead of the immediate parent ng-template.
///
/// Example from the issue:
/// ```html
/// <ng-template ng-label-tmp let-item="item" let-clear="clear">
///   <button (click)="clear(item)">
///     @if (item.icon != null) {
///       <i class="bwi bwi-fw {{ item.icon }}"></i>
///     }
///   </button>
/// </ng-template>
/// ```
///
/// The @if block should resolve `item` from the parent ng-template's context
/// (1 level up via nextContext()), not from the root component (2 levels up).
#[test]
fn test_ng_template_let_binding_child_view_context() {
    let template = r#"<ng-template let-item="item" let-clear="clear">
  <button (click)="clear(item)">
    @if (item.icon != null) {
      <i class="bwi bwi-fw {{ item.icon }}"></i>
    }
  </button>
</ng-template>"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the @if block's template function
    let if_fn_start = js.find("Conditional_2_Conditional");

    // The @if block should use nextContext() with 1 step (not 2) to access the
    // parent ng-template's context where item is defined
    if let Some(if_fn_pos) = if_fn_start {
        let if_fn_section = &js[if_fn_pos..];
        // Find the update block in the @if function
        if let Some(update_pos) = if_fn_section.find("(rf & 2)") {
            let update_section = &if_fn_section[update_pos..];

            // Should have nextContext() to go up to the ng-template
            assert!(
                update_section.contains("nextContext()"),
                "Should use nextContext() to access parent template context.\nGot:\n{js}"
            );

            // Should NOT have nextContext(2) which would skip over the ng-template
            // and go directly to the root component
            assert!(
                !update_section.contains("nextContext(2)"),
                "Should NOT use nextContext(2) - item is in the immediate parent ng-template, not the root.\nGot:\n{js}"
            );
        }
    }

    // Also verify the ng-template function extracts item and clear from its context
    let template_fn_start = js.find("TestComponent_ng_template_0_Template");
    if let Some(tmpl_pos) = template_fn_start {
        let tmpl_section = &js[tmpl_pos..];
        // Find the update block in the ng-template function
        if let Some(update_pos) = tmpl_section.find("(rf & 2)") {
            let update_section = &tmpl_section[update_pos..];

            // The ng-template should extract item from ctx.$implicit or ctx.item
            // and clear from ctx.clear (depending on how it's defined)
            let has_item_extraction =
                update_section.contains("ctx.item") || update_section.contains("ctx.$implicit");
            assert!(
                has_item_extraction || update_section.contains("item_r"),
                "ng-template should extract item variable from context.\nGot:\n{update_section}"
            );
        }
    }
}

/// Test that listener handlers in @switch case templates generate correct nextContext() calls.
///
/// This reproduces the issue from ImportComponent where a listener inside a @switch case
/// uses `ctx.formGroup` directly instead of `nextContext().formGroup`.
///
/// TypeScript Angular compiler generates:
/// ```js
/// i0.ɵɵdomListener("csvDataLoaded", function ..._listener($event) {
///   i0.ɵɵrestoreView(_r8);
///   const ctx_r2 = i0.ɵɵnextContext();
///   return i0.ɵɵresetView(ctx_r2.formGroup.controls.fileContents.setValue($event));
/// });
/// ```
///
/// Oxc was generating (incorrectly):
/// ```js
/// i0.ɵɵdomListener('csvDataLoaded', function ..._listener($event) {
///   i0.ɵɵrestoreView(_r7);
///   return i0.ɵɵresetView(ctx.formGroup.controls.fileContents.setValue($event));
/// });
/// ```
#[test]
fn test_listener_in_switch_case_uses_next_context() {
    // Simplified template: @switch with a listener inside the case
    let template = r#"@switch (format) {
  @case ("lastpass") {
    <import-lastpass (csvDataLoaded)="formGroup.controls.fileContents.setValue($event)"></import-lastpass>
  }
  @default {
    <div>Default content</div>
  }
}"#;

    let js = compile_template_to_js(template, "ImportComponent");
    println!("Generated JS:\n{js}");

    // Find the listener in the @case template function (handles both single and double quotes)
    let listener_match =
        js.find("listener(\"csvDataLoaded\"").or_else(|| js.find("listener('csvDataLoaded'"));
    assert!(listener_match.is_some(), "Should have a csvDataLoaded listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    // Since we're inside a @switch case (embedded view), we need nextContext()
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The listener should NOT use raw `ctx.formGroup` without nextContext
    // Should use something like `ctx_r2.formGroup` where ctx_r2 comes from nextContext()
    let has_raw_ctx_formgroup = listener_body.contains("ctx.formGroup");
    assert!(
        !has_raw_ctx_formgroup,
        "Listener should NOT use raw ctx.formGroup - should use ctx_rN from nextContext().\nListener body:\n{listener_body}"
    );
}

/// Test that listener handlers in @for loop templates generate correct nextContext() calls.
#[test]
fn test_listener_in_for_loop_uses_next_context() {
    // Template: @for with a listener that accesses component method
    let template = r#"@for (item of items; track item) {
  <button (click)="handleClick(item)">{{ item.name }}</button>
}"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the listener in the @for body template function (handles both single and double quotes)
    let listener_match = js.find("listener(\"click\"").or_else(|| js.find("listener('click'"));
    assert!(listener_match.is_some(), "Should have a click listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    // The @for body is an embedded view, so we need nextContext()
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );
}

/// Test that listeners in @for loops correctly extract loop variables from restoreView.
///
/// This is a regression test for the bug where listeners in @for loops used `ctx.option`
/// directly instead of extracting `option` from `restoreView(...).$implicit`.
///
/// Expected pattern (like TypeScript compiler):
/// ```javascript
/// const option_r8 = i0.ɵɵrestoreView(_r7).$implicit;
/// const ctx_r3 = i0.ɵɵnextContext(2);
/// return i0.ɵɵresetView((option_r8.children?.length ? ctx_r3.viewOption(option_r8, $event) : ctx_r3.selectOption(option_r8, $event)));
/// ```
#[test]
fn test_for_listener_extracts_loop_variable_from_restore_view() {
    // This is similar to ChipSelectComponent_Conditional_12_For_3_Template
    let template = r#"@for (option of options; track option) {
  <button (click)="option.children?.length ? viewOption(option, $event) : selectOption(option, $event)">
    {{ option.label }}
  </button>
}"#;

    let js = compile_template_to_js(template, "ChipSelectComponent");

    // Find the listener (handles both single and double quotes)
    let listener_start = js
        .find("listener(\"click\"")
        .or_else(|| js.find("listener('click'"))
        .expect("Should have click listener");
    let listener_end = js[listener_start..].find("});").unwrap_or(500);
    let listener_body = &js[listener_start..listener_start + listener_end + 3];

    // The listener should NOT use ctx.option (that's wrong - option is a loop variable, not a component property)
    assert!(
        !listener_body.contains("ctx.option"),
        "Listener should NOT use ctx.option. Loop variable 'option' should be extracted from restoreView.\nListener body:\n{listener_body}"
    );

    // The listener should have restoreView and extract the loop variable
    assert!(
        listener_body.contains("restoreView(") && listener_body.contains(".$implicit"),
        "Listener should extract loop variable from restoreView(...).$implicit.\nListener body:\n{listener_body}"
    );

    // The listener should use nextContext to access component methods
    assert!(
        listener_body.contains("nextContext("),
        "Listener should use nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The extracted variable should be used throughout the expression
    // Check that we see option_rN in the expression (not ctx.option)
    let uses_extracted_var =
        listener_body.contains("option_r") && !listener_body.contains("ctx.option");
    assert!(
        uses_extracted_var,
        "Listener should use extracted loop variable (option_rN) instead of ctx.option.\nListener body:\n{listener_body}"
    );
}

/// Test that twoWayListener handlers inside embedded views generate correct nextContext() calls.
///
/// This reproduces the InputPasswordComponent issue where a twoWayListener inside an *ngIf
/// uses `_unnamed_X.showPassword` instead of `nextContext().showPassword`.
///
/// Expected pattern (like TypeScript compiler):
/// ```javascript
/// i0.ɵɵtwoWayListener("toggledChange", function ..._listener($event) {
///   i0.ɵɵrestoreView(_r2);
///   const ctx_r0 = i0.ɵɵnextContext();  // <-- THIS IS MISSING
///   (i0.ɵɵtwoWayBindingSet(ctx_r0.showPassword, $event) || (ctx_r0.showPassword = $event));
///   return i0.ɵɵresetView($event);
/// });
/// ```
#[test]
fn test_two_way_listener_in_ngif_uses_next_context() {
    // Simplified template matching the InputPasswordComponent pattern:
    // *ngIf creates an embedded view, and the button inside has [(toggled)]="showPassword"
    let template = r#"<div *ngIf="showForm">
  <button [(toggled)]="showPassword">Toggle</button>
</div>"#;

    let js = compile_template_to_js(template, "InputPasswordComponent");
    println!("Generated JS:\n{js}");

    // Check what's in the embedded view function
    if let Some(div_fn_start) = js.find("function InputPasswordComponent_div_0_Template") {
        let div_fn = &js[div_fn_start..];
        if let Some(create_start) = div_fn.find("if ((rf & 1))") {
            let create_block = &div_fn[create_start..];
            println!(
                "\n=== CREATE BLOCK ===\n{}",
                &create_block[..create_block.find("if ((rf & 2))").unwrap_or(create_block.len())]
            );
        }
    }

    // Find the twoWayListener (handles both single and double quotes)
    let listener_match = js
        .find("twoWayListener(\"toggledChange\"")
        .or_else(|| js.find("twoWayListener('toggledChange'"));
    assert!(listener_match.is_some(), "Should have a toggledChange twoWayListener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("TwoWayListener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    // Since we're inside an *ngIf (embedded view), we need nextContext()
    assert!(
        listener_body.contains("nextContext("),
        "TwoWayListener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The listener should NOT use `_unnamed_` which indicates an unresolved variable
    assert!(
        !listener_body.contains("_unnamed_"),
        "TwoWayListener should NOT have _unnamed_ variables - context should be properly resolved.\nListener body:\n{listener_body}"
    );

    // The listener should NOT use raw `ctx.showPassword` without nextContext
    let has_raw_ctx = listener_body.contains("ctx.showPassword");
    assert!(
        !has_raw_ctx,
        "TwoWayListener should NOT use raw ctx.showPassword - should use ctx_rN from nextContext().\nListener body:\n{listener_body}"
    );
}

/// Test that listener handlers in @if block templates generate correct nextContext() calls.
///
/// This reproduces the issue from ImportComponent where a listener inside an @if branch
/// uses `ctx.formGroup` directly instead of `nextContext().formGroup`.
#[test]
fn test_listener_in_if_block_uses_next_context() {
    // Simplified template matching the actual ImportComponent structure:
    // @if (condition) { <component (event)="handler()"></component> }
    let template = r#"@if (showLastPassOptions) {
  <import-lastpass (csvDataLoaded)="formGroup.controls.fileContents.setValue($event)"></import-lastpass>
} @else {
  <div>Default content</div>
}"#;

    let js = compile_template_to_js(template, "ImportComponent");
    println!("Generated JS:\n{js}");

    // Find the listener in the @if branch template function (handles both single and double quotes)
    let listener_match =
        js.find("listener(\"csvDataLoaded\"").or_else(|| js.find("listener('csvDataLoaded'"));
    assert!(listener_match.is_some(), "Should have a csvDataLoaded listener. Got:\n{js}");

    let listener_pos = listener_match.expect("Already checked above");
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    // Since we're inside an @if branch (embedded view), we need nextContext()
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The listener should NOT use raw `ctx.formGroup` without nextContext
    let has_raw_ctx_formgroup = listener_body.contains("ctx.formGroup");
    assert!(
        !has_raw_ctx_formgroup,
        "Listener should NOT use raw ctx.formGroup - should use ctx_rN from nextContext().\nListener body:\n{listener_body}"
    );
}

/// Test that prefix not expressions (!) inside embedded view listeners correctly resolve names.
///
/// This is a regression test for the bug where expressions like `bold = !bold` in an
/// embedded view listener would generate `ctx_r1.bold = !ctx.bold` instead of
/// `ctx_r1.bold = !ctx_r1.bold`.
///
/// The issue was that `PrefixNot` expressions in `resolve_angular_expression` were not
/// being handled, so the nested property read (`bold` in `!bold`) was never resolved.
///
/// Expected pattern:
/// ```javascript
/// return i0.ɵɵresetView((ctx_r1.bold = !ctx_r1.bold));  // Both sides use ctx_r1
/// ```
///
/// Bug pattern (before fix):
/// ```javascript
/// return i0.ɵɵresetView((ctx_r1.bold = !ctx.bold));  // Right side incorrectly uses ctx
/// ```
#[test]
fn test_prefix_not_in_embedded_view_listener() {
    // Template: ng-template with a listener that has `bold = !bold` assignment
    // This is the pattern from the CDK menu example
    let template = r#"<ng-template #tmpl>
    <button (click)="bold = !bold">Toggle Bold</button>
</ng-template>"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the listener (may use single or double quotes)
    let listener_match = js.find("listener(\"click\"").or_else(|| js.find("listener('click'"));
    assert!(listener_match.is_some(), "Should have a click listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener should have nextContext() to access the component context
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The critical check: the right side of `bold = !bold` should NOT use raw `ctx.bold`
    // It should use `ctx_r1.bold` (or similar) from nextContext()
    //
    // Bug pattern to detect: `= !ctx.bold`
    // Correct pattern: `= !ctx_r` followed by `.bold`
    assert!(
        !listener_body.contains("!ctx.bold"),
        "Listener should NOT use `!ctx.bold` - the nested property read in PrefixNot should be resolved.\n\
         Both sides of `bold = !bold` should use the same context variable (ctx_rN from nextContext).\n\
         Listener body:\n{listener_body}"
    );

    // Also check that we're not using raw `ctx.bold` anywhere in the expression
    // (except for the parameter name which is just `ctx`)
    let uses_wrong_ctx = listener_body.contains("ctx.bold");
    assert!(
        !uses_wrong_ctx,
        "Listener should NOT use raw ctx.bold - should use ctx_rN.bold from nextContext().\n\
         Listener body:\n{listener_body}"
    );
}

/// Test that unary expressions (+expr, -expr) inside embedded view listeners correctly resolve names.
///
/// Similar to the prefix not test, but for unary plus/minus expressions.
#[test]
fn test_unary_expr_in_embedded_view_listener() {
    // Template: ng-template with a listener that uses unary expressions
    let template = r#"<ng-template #tmpl>
    <button (click)="value = -value">Negate</button>
</ng-template>"#;

    let js = compile_template_to_js(template, "TestComponent");

    // Find the listener (may use single or double quotes)
    let listener_match = js.find("listener(\"click\"").or_else(|| js.find("listener('click'"));
    assert!(listener_match.is_some(), "Should have a click listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener should have nextContext()
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The unary expression should not use raw ctx.value
    assert!(
        !listener_body.contains("-ctx.value"),
        "Listener should NOT use `-ctx.value` - unary expressions should be properly resolved.\nListener body:\n{listener_body}"
    );
}

/// Test that two-way binding event names are not duplicated in the consts array.
///
/// For a two-way binding like `[(ngModel)]="value"`, the compiled output should have:
/// - `ngModel` (the property binding name)
/// - `ngModelChange` (the event listener name)
///
/// But each name should appear only ONCE in the consts array bindings section.
/// This is a regression test for the issue where `ngModelChange` was appearing twice.
#[test]
fn test_two_way_binding_no_duplicate_event_in_consts() {
    // Simple two-way binding on an input
    let template = r#"<input [(ngModel)]="value">"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Count occurrences of "ngModelChange" in the consts array declaration
    // The const declaration looks like: const _c0 = [3, "ngModelChange", "ngModel"];
    // or similar, where 3 is the Bindings marker
    let ngmodel_change_count = js.matches("\"ngModelChange\"").count();

    assert_eq!(
        ngmodel_change_count, 1,
        "Expected exactly 1 occurrence of 'ngModelChange', got {ngmodel_change_count}.\nOutput:\n{js}"
    );

    // Also verify ngModel appears exactly once (the property binding)
    // Note: We need to match exactly "ngModel" not "ngModelChange"
    // Look for the pattern "ngModel" followed by something that's not "Change"
    let ngmodel_only_count = js
        .match_indices("\"ngModel\"")
        .filter(|(idx, _)| {
            // Check if followed by "Change"
            let rest = &js[*idx + 9..];
            !rest.starts_with("Change")
        })
        .count();

    assert_eq!(
        ngmodel_only_count, 1,
        "Expected exactly 1 occurrence of 'ngModel' (without Change suffix), got {ngmodel_only_count}.\nOutput:\n{js}"
    );
}

/// Test that two-way binding with a static class attribute produces correct consts array.
///
/// When there's both a two-way binding and static attributes on the same element,
/// a consts array is generated. The binding names should still not be duplicated.
#[test]
fn test_two_way_binding_with_static_attr_no_duplicate() {
    // Two-way binding with an attribute class - this should trigger consts array generation
    // We need to add the attr. prefix to make it a static attribute binding that extracts to consts
    let template = r#"<input attr.class="form-control" [(ngModel)]="value">"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Count occurrences of "ngModelChange" - should appear exactly once
    let ngmodel_change_count = js.matches("\"ngModelChange\"").count();

    assert_eq!(
        ngmodel_change_count, 1,
        "Expected exactly 1 occurrence of 'ngModelChange', got {ngmodel_change_count}.\nOutput:\n{js}"
    );

    // Count occurrences of "ngModel" (not followed by "Change") - should appear exactly once
    let ngmodel_only_count = js
        .match_indices("\"ngModel\"")
        .filter(|(idx, _)| {
            let rest = &js[*idx + 9..];
            !rest.starts_with("Change")
        })
        .count();

    assert_eq!(
        ngmodel_only_count, 1,
        "Expected exactly 1 occurrence of 'ngModel' (without Change suffix), got {ngmodel_only_count}.\nOutput:\n{js}"
    );
}

/// Test with a property binding (like [disabled]) and a two-way binding, which should trigger
/// the consts array with bindings.
#[test]
fn test_two_way_binding_with_property_binding_consts() {
    // Two-way binding with another property binding - this should trigger consts array
    let template = r#"<input [disabled]="isDisabled" [(ngModel)]="value">"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Count occurrences of "ngModelChange" - should appear exactly once
    let ngmodel_change_count = js.matches("\"ngModelChange\"").count();

    assert_eq!(
        ngmodel_change_count, 1,
        "Expected exactly 1 occurrence of 'ngModelChange', got {ngmodel_change_count}.\nOutput:\n{js}"
    );

    // Count occurrences of "ngModel" (not followed by "Change") - should appear exactly once
    let ngmodel_only_count = js
        .match_indices("\"ngModel\"")
        .filter(|(idx, _)| {
            let rest = &js[*idx + 9..];
            !rest.starts_with("Change")
        })
        .count();

    assert_eq!(
        ngmodel_only_count, 1,
        "Expected exactly 1 occurrence of 'ngModel' (without Change suffix), got {ngmodel_only_count}.\nOutput:\n{js}"
    );

    // Verify "disabled" appears exactly once
    assert_eq!(
        js.matches("\"disabled\"").count(),
        1,
        "Expected exactly 1 occurrence of 'disabled'.\nOutput:\n{js}"
    );
}

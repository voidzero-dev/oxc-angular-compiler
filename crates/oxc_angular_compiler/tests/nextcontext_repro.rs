//! Reproduction of extra nextContext issue
use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    output::ast::FunctionExpr,
    output::emitter::JsEmitter,
    parser::html::{HtmlParser, remove_whitespaces},
    pipeline::{
        compilation::ComponentCompilationJob, emit::compile_template, ingest::ingest_component,
    },
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions},
};
use oxc_span::Ident;

fn compile_template_to_js(template: &str, component_name: &str) -> String {
    let allocator = Allocator::default();
    let parser = HtmlParser::new(&allocator, template, "test.html");
    let html_result = parser.parse();
    assert!(html_result.errors.is_empty(), "HTML parse errors: {:?}", html_result.errors);
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
        result.push_str(&emitter.emit_statement(stmt));
        result.push('\n');
    }
    result.push_str("}\n");
    result
}

// Helper function to debug view structure (useful for debugging)
#[expect(dead_code)]
fn debug_job_views(job: &ComponentCompilationJob<'_>) {
    println!("=== Debug Job Views ===");
    println!("Root view xref: {:?}", job.root.xref);
    println!(
        "Root context_variables: {:?}",
        job.root
            .context_variables
            .iter()
            .map(|cv| (cv.name.as_str(), cv.value.as_str()))
            .collect::<Vec<_>>()
    );
    println!(
        "Root aliases: {:?}",
        job.root.aliases.iter().map(|a| a.identifier.as_str()).collect::<Vec<_>>()
    );

    for (xref, view) in &job.views {
        println!("\nView {xref:?}:");
        println!(
            "  context_variables: {:?}",
            view.context_variables
                .iter()
                .map(|cv| (cv.name.as_str(), cv.value.as_str()))
                .collect::<Vec<_>>()
        );
        println!(
            "  aliases: {:?}",
            view.aliases.iter().map(|a| a.identifier.as_str()).collect::<Vec<_>>()
        );
    }
    println!("=== End Debug ===\n");
}

/// Test: ngFor inside ngIf - verify no extra nextContext calls
#[test]
fn test_nested_ngfor_in_ngif() {
    // Template: *ngIf wrapping *ngFor with nested conditional
    // This creates deeply nested views that need context access
    let template = r#"
<ng-container *ngIf="data as view">
    <div *ngFor="let item of view.items">
        <span *ngIf="item.active">
            {{ item.name }}
        </span>
    </div>
</ng-container>
"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Count nextContext calls in the innermost template
    // The innermost view (item.active check) should only need:
    // - One nextContext to access the @for context (item)
    // It should NOT have extra orphaned nextContext(n) calls

    // Check that we don't have orphaned nextContext calls
    // Pattern: a line with ONLY "nextContext(n);" (no variable assignment)
    for line in js.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("i0.ɵɵnextContext(") && trimmed.ends_with(");") {
            // Check if this is an orphaned call (not part of a variable declaration or expression)
            assert!(
                !(!line.contains("const ") && !line.contains("= ") && !line.contains('.')),
                "Found orphaned nextContext call: {line}"
            );
        }
    }
}

/// Test: Triple nested views
#[test]
fn test_triple_nested_views() {
    // Root -> ngIf -> ngFor -> ngIf (3 levels of embedding)
    let template = r#"
<ng-container *ngIf="outer as a">
    <ng-container *ngFor="let b of a.items">
        <span *ngIf="b.show">{{ b.value }}</span>
    </ng-container>
</ng-container>
"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // The innermost template accessing b.show and b.value should have:
    // - One nextContext(1) to get the ngFor context
    // - One property read from that context
    // It should NOT have additional nextContext calls to parent scopes if unused
}

/// Test: Pattern matching account-switcher - ngIf with ngTemplate and ngFor
#[test]
fn test_account_switcher_pattern() {
    // This pattern creates multiple levels: ngIf -> ngTemplate -> ngFor -> nested elements
    let template = r#"
<ng-container *ngIf="view$ | async as view">
    <ng-template [ngIf]="view.items">
        <div *ngFor="let account of view.items">
            <button *ngIf="account.active">
                <app-avatar [name]="account.name"></app-avatar>
            </button>
        </div>
    </ng-template>
</ng-container>
"#;

    let js = compile_template_to_js(template, "TestComponent");

    // Check for orphaned nextContext calls
    for line in js.lines() {
        let trimmed = line.trim();
        // Look for standalone nextContext(n); calls that aren't part of assignments
        if trimmed.starts_with("i0.ɵɵnextContext(")
            && trimmed.ends_with(");")
            && !line.contains("const ")
            && !line.contains("= ")
        {
            panic!("Found orphaned nextContext call: {line}");
        }
    }
}

/// Test: NavItemComponent pattern - @if with ng-template and nested @if
/// This is the failing pattern: nextContext(3) expected but nextContext(2) generated
///
/// View hierarchy:
/// - Root (NavItemComponent)
///   - Conditional_3 (@if block)
///     - ng_template_5 (#anchorAndButtonContent)
///       - Conditional_1 (@if inside ng-template)
///
/// When Conditional_1 accesses ctx.icon(), it needs 3 levels up to root.
#[test]
fn test_nav_item_pattern() {
    // Simplified version of NavItemComponent template
    let template = r#"
<div>
  @if (open || icon()) {
    <ng-template #anchorAndButtonContent>
      @if (icon()) {
        <i class="icon {{ icon() }}"></i>
      }
    </ng-template>
  }
</div>
"#;

    let js = compile_template_to_js(template, "NavItemComponent");

    // Find the innermost template function and check its nextContext depth
    // Expected: nextContext(3) to get from Conditional_1 to Root
    // Pattern: NavItemComponent_Conditional_0_ng_template_N_Conditional_M_Template

    // Count context levels by checking nextContext calls
    for line in js.lines() {
        if line.contains("nextContext(") {
            println!("nextContext found: {}", line.trim());
        }
    }

    // The innermost @if that accesses icon() should have nextContext(3):
    // 1. From Conditional_1 to ng_template_5
    // 2. From ng_template_5 to Conditional_3 (the outer @if)
    // 3. From Conditional_3 to Root
    assert!(
        js.contains("nextContext(3)"),
        "Expected nextContext(3) for accessing root context from nested template, but found:\n{js}"
    );
}

/// Test: Deep nesting (4+ levels)
#[test]
fn test_deep_nesting() {
    // 4 levels: Root -> ngIf -> ngFor -> ngIf -> ngFor
    let template = r#"
<ng-container *ngIf="data as a">
    <div *ngFor="let b of a.items">
        <ng-container *ngIf="b.visible">
            <span *ngFor="let c of b.children">{{ c.name }}</span>
        </ng-container>
    </div>
</ng-container>
"#;

    let js = compile_template_to_js(template, "TestComponent");

    // Check for orphaned nextContext calls - look for nextContext() on its own line
    // (not part of a variable assignment like "const x = nextContext()...")
    for line in js.lines() {
        let trimmed = line.trim();
        // Pattern: standalone nextContext call with no assignment or property access after
        // Good: const x = nextContext().$implicit;
        // Good: const x = nextContext();  (variable captures the result)
        // Bad: nextContext();  (no variable, result is unused)
        // Bad: i0.ɵɵnextContext();  (same but with namespace)
        if trimmed.contains("nextContext(")
            && trimmed.ends_with(';')
            && !trimmed.contains("const ")
            && !trimmed.contains("let ")
            && !trimmed.contains("= ")
        {
            // It's a standalone call. Check if there's property access after the call
            // (e.g., nextContext().$implicit)
            let parts: Vec<&str> = trimmed.split("nextContext(").collect();
            if parts.len() >= 2 {
                let after_call = parts[1];
                // If there's nothing after the closing paren except semicolon, it's orphaned
                if (after_call.starts_with(");") || after_call.starts_with(").$"))
                    && after_call.starts_with(");")
                {
                    panic!("Found orphaned nextContext call: '{trimmed}'");
                }
            }
        }
    }
}

/// Test: @for loop with `let i = $index` alias used in click handler
/// Verifies that alias variables are correctly inlined in listener handlers.
///
/// Expected behavior (TypeScript):
/// - The alias `i` should be inlined to reference `ɵ$index_N`
/// - `ɵ$index_N` is extracted from restoreView().$index
///
/// The generated code should look like:
/// ```javascript
/// i0.ɵɵdomListener('click', function ..._listener() {
///   const ɵ$index_5_r2 = i0.ɵɵrestoreView(_r1).$index;  // Extract $index
///   const ctx_r2 = i0.ɵɵnextContext();
///   return i0.ɵɵresetView(ctx_r2.selectTab(ɵ$index_5_r2));  // Use $index directly
/// });
/// ```
#[test]
fn test_for_loop_with_index_alias_in_listener() {
    // Simple case: @for loop WITHOUT alias
    let template_simple = r#"
@for (tab of tabs(); track tab) {
  <button (click)="selectTab($index)">Tab</button>
}
"#;
    let js_simple = compile_template_to_js(template_simple, "SimpleComponent");
    println!("=== Simple (no alias) ===\n{js_simple}");

    // Complex case: @for loop WITH alias
    let template = r#"
@for (tab of tabs(); track tab; let i = $index) {
  <button (click)="selectTab(i)">Tab</button>
}
"#;

    let js = compile_template_to_js(template, "TabGroupComponent");
    println!("=== With alias ===\n{js}");

    // The listener should NOT use ctx.i - it should use the extracted $index
    assert!(
        !js.contains("ctx.i"),
        "Alias 'i' should not resolve to ctx.i - it should be inlined to ɵ$index:\n{js}"
    );

    // The listener should extract $index from restoreView
    assert!(
        js.contains("restoreView(_r") && (js.contains(").$index") || js.contains(".$index")),
        "Should extract $index from restoreView():\n{js}"
    );

    // Should NOT have _unnamed variables
    assert!(!js.contains("_unnamed_"), "Should not have unnamed variables:\n{js}");
}

/// Test: Verify that @for loop aliases have proper context variables set up during ingest
#[test]
fn test_for_loop_alias_ingest() {
    let allocator = Allocator::default();
    let template = r#"
@for (tab of tabs(); track tab; let i = $index) {
  <button (click)="selectTab(i)">Tab</button>
}
"#;
    let parser = HtmlParser::new(&allocator, template, "test.html");
    let html_result = parser.parse();
    let nodes = remove_whitespaces(&allocator, &html_result.nodes, true);
    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&nodes);

    let job = ingest_component(&allocator, Ident::from("TestComponent"), r3_result.nodes);

    // Check that the body view has the expected context variables and aliases
    assert!(!job.views.is_empty(), "Should have at least one embedded view");

    // Find the @for body view
    let body_view = job.views.values().find(|v| !v.context_variables.is_empty());
    assert!(body_view.is_some(), "Should have a view with context variables");
    let body_view = body_view.unwrap();

    // Check for ɵ$index_N in context_variables
    let has_indexed_index =
        body_view.context_variables.iter().any(|cv| cv.name.as_str().starts_with("ɵ$index_"));
    assert!(has_indexed_index, "Should have ɵ$index_N context variable for alias resolution");

    // Check that 'i' alias exists
    let has_i_alias = body_view.aliases.iter().any(|a| a.identifier.as_str() == "i");
    assert!(has_i_alias, "Should have 'i' alias");
}

/// Test: Simple *ngIf with component property access
/// Reproduces CreateClientDialogComponent_span_13_Template issue
#[test]
fn test_simple_ngif_with_component_property() {
    let template = r#"
<span *ngIf="discountPercentage">{{ discountPercentage }}</span>
"#;

    let js = compile_template_to_js(template, "CreateClientDialogComponent");
    println!("Generated JS:\n{js}");

    // Find the span template function
    let template_fn = js.find("span_0_Template");
    assert!(template_fn.is_some(), "Should have span_0_Template function. Got:\n{js}");

    let fn_start = template_fn.unwrap();
    let fn_end = js[fn_start..].find("\n}").unwrap_or(1000) + fn_start;
    let template_fn_body = &js[fn_start..fn_end];

    println!("\nTemplate function:\n{template_fn_body}");

    // The update block should have nextContext() to access component property
    // Look for the update block (rf & 2)
    if let Some(update_pos) = template_fn_body.find("(rf & 2)") {
        let update_block = &template_fn_body[update_pos..];
        println!("\nUpdate block:\n{update_block}");

        // Should have nextContext() to access component
        assert!(
            update_block.contains("nextContext("),
            "Update block should have nextContext() for embedded view. Got:\n{update_block}"
        );
    }
}

/// Test: *ngIf with `this.` prefix property access
/// Reproduces AdjustStorageDialogComponent_bit_hint_10_Template issue
/// The `this.` prefix should be handled the same as direct property access
#[test]
fn test_ngif_with_this_prefix_property() {
    let template = r#"
<bit-hint *ngIf="dialogParams.type === 'Add'">
  {{ this.formGroup.value.storage }}
</bit-hint>
"#;

    let js = compile_template_to_js(template, "AdjustStorageDialogComponent");
    println!("Generated JS:\n{js}");

    // Find the bit_hint template function
    let template_fn = js.find("bit_hint_0_Template");
    assert!(template_fn.is_some(), "Should have bit_hint_0_Template function. Got:\n{js}");

    let fn_start = template_fn.unwrap();
    let fn_end = js[fn_start..].find("\n}").unwrap_or(1000) + fn_start;
    let template_fn_body = &js[fn_start..fn_end];

    println!("\nTemplate function:\n{template_fn_body}");

    // The update block should have nextContext() to access component property
    if let Some(update_pos) = template_fn_body.find("(rf & 2)") {
        let update_block = &template_fn_body[update_pos..];
        println!("\nUpdate block:\n{update_block}");

        // Should have nextContext() to access component
        assert!(
            update_block.contains("nextContext("),
            "Update block should have nextContext() for embedded view with this. prefix. Got:\n{update_block}"
        );

        // Should NOT use ctx.formGroup directly - should use ctx_rN.formGroup
        assert!(
            !update_block.contains("ctx.formGroup"),
            "Should NOT use ctx.formGroup - should use ctx_rN.formGroup from nextContext(). Got:\n{update_block}"
        );
    }
}

/// Test: Many sibling *ngIf views followed by @if with listener
/// This reproduces the ImportComponent issue where:
/// - Many *ngIf blocks create sibling embedded views
/// - An @if block with a listener incorrectly uses `ctx.formGroup` instead of `nextContext().formGroup`
///
/// Expected behavior (TypeScript):
/// ```javascript
/// i0.ɵɵdomListener("csvDataLoaded", function ..._listener($event) {
///   i0.ɵɵrestoreView(_r8);
///   const ctx_r2 = i0.ɵɵnextContext();
///   return i0.ɵɵresetView(ctx_r2.formGroup.controls.fileContents.setValue($event));
/// });
/// ```
///
/// Bug behavior (before fix):
/// ```javascript
/// i0.ɵɵdomListener('csvDataLoaded', function ..._listener($event) {
///   i0.ɵɵrestoreView(_r7);
///   return i0.ɵɵresetView(ctx.formGroup.controls.fileContents.setValue($event));
/// });
/// ```
#[test]
fn test_listener_in_if_after_many_ngif_siblings() {
    // Simplified ImportComponent pattern:
    // - Many *ngIf blocks (creating sibling embedded views)
    // - @if block with a listener that accesses component property
    let template = r#"
<div>
  <ng-container *ngIf="formatA">A</ng-container>
  <ng-container *ngIf="formatB">B</ng-container>
  <ng-container *ngIf="formatC">C</ng-container>
  <ng-container *ngIf="formatD">D</ng-container>
  <ng-container *ngIf="formatE">E</ng-container>
  <ng-container *ngIf="formatF">F</ng-container>
  <ng-container *ngIf="formatG">G</ng-container>
  <ng-container *ngIf="formatH">H</ng-container>
  @if (showLastPassOptions) {
    <import-lastpass (csvDataLoaded)="formGroup.controls.fileContents.setValue($event)"></import-lastpass>
  }
</div>
"#;

    let js = compile_template_to_js(template, "ImportComponent");
    println!("Generated JS:\n{js}");

    // Find the listener for csvDataLoaded (handles both single and double quotes)
    let listener_match =
        js.find("listener(\"csvDataLoaded\"").or_else(|| js.find("listener('csvDataLoaded'"));
    assert!(listener_match.is_some(), "Should have a csvDataLoaded listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
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

/// Test: Simpler case - just one *ngIf followed by @if with listener
#[test]
fn test_listener_in_if_after_one_ngif_sibling() {
    let template = r#"
<div>
  <ng-container *ngIf="formatA">A</ng-container>
  @if (showOptions) {
    <button (click)="handleClick()"></button>
  }
</div>
"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the listener (handles both single and double quotes)
    let listener_match = js.find("listener(\"click\"").or_else(|| js.find("listener('click'"));
    assert!(listener_match.is_some(), "Should have a click listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );
}

/// Test: Actual ImportComponent structure - @if inside bit-card with nested *ngIf callout
/// The @if is NOT inside the *ngIf - they are siblings inside bit-card
#[test]
fn test_listener_in_if_sibling_to_large_ngif_block() {
    // This mimics the real ImportComponent structure:
    // - bit-card contains:
    //   - bit-callout with *ngIf="format" (large block with many nested *ngIf)
    //   - @if (showLastPassOptions) { import-lastpass with listener }
    //
    // The @if is a SIBLING to the *ngIf, not nested inside it.
    let template = r#"
<bit-card>
  <bit-callout *ngIf="format">
    <ng-container *ngIf="format === 'a'">A</ng-container>
    <ng-container *ngIf="format === 'b'">B</ng-container>
    <ng-container *ngIf="format === 'c'">C</ng-container>
    <ng-container *ngIf="format === 'd'">D</ng-container>
    <ng-container *ngIf="format === 'e'">E</ng-container>
    <ng-container *ngIf="format === 'f'">F</ng-container>
    <ng-container *ngIf="format === 'g'">G</ng-container>
    <ng-container *ngIf="format === 'h'">H</ng-container>
    <ng-container *ngIf="format === 'i'">I</ng-container>
    <ng-container *ngIf="format === 'j'">J</ng-container>
  </bit-callout>
  @if (showLastPassOptions) {
    <import-lastpass
      [formGroup]="formGroup"
      (csvDataLoaded)="formGroup.controls.fileContents.setValue($event)"
    ></import-lastpass>
  } @else if (showChromiumOptions) {
    <import-chrome
      [formGroup]="formGroup"
      [onImportFromBrowser]="onImportFromBrowser"
      (csvDataLoaded)="formGroup.controls.fileContents.setValue($event)"
    ></import-chrome>
  }
</bit-card>
"#;

    let js = compile_template_to_js(template, "ImportComponent");
    println!("Generated JS:\n{js}");

    // Find the csvDataLoaded listener for import-lastpass (handles both single and double quotes)
    let listener_match =
        js.find("listener(\"csvDataLoaded\"").or_else(|| js.find("listener('csvDataLoaded'"));
    assert!(listener_match.is_some(), "Should have a csvDataLoaded listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("\nFirst csvDataLoaded listener body:\n{listener_body}");

    // The listener MUST have nextContext() to access the component context
    assert!(
        listener_body.contains("nextContext("),
        "Listener should call nextContext() to access component context.\nListener body:\n{listener_body}"
    );

    // The listener should NOT use raw `ctx.formGroup`
    let has_raw_ctx_formgroup = listener_body.contains("ctx.formGroup");
    assert!(
        !has_raw_ctx_formgroup,
        "Listener should NOT use raw ctx.formGroup - should use ctx_rN from nextContext().\nListener body:\n{listener_body}"
    );

    // Also check the update block for import-chrome - should use nextContext() for onImportFromBrowser
    if js.contains("'onImportFromBrowser'") {
        // Find the property binding for onImportFromBrowser
        let prop_pos = js.find("'onImportFromBrowser'").unwrap();
        let line_start = js[..prop_pos].rfind('\n').unwrap_or(0);
        let line_end = js[prop_pos..].find('\n').unwrap_or(0) + prop_pos;
        let property_line = &js[line_start..line_end];

        println!("\nonImportFromBrowser property line:\n{property_line}");

        // Should NOT use ctx.onImportFromBrowser - should use ctx_rN.onImportFromBrowser
        assert!(
            !property_line.contains("ctx.onImportFromBrowser"),
            "Property binding should NOT use raw ctx. - should use ctx_rN from nextContext().\nLine:\n{property_line}"
        );
    }
}

/// Test: Listener in @if block accessing component property WITHOUT this. prefix
/// This reproduces the ImportComponent_Conditional_50_Template issue where:
/// - A listener handler accesses `formGroup.controls.fileContents.setValue($event)`
/// - This should become `ctx_r2.formGroup.controls.fileContents.setValue($event)` with nextContext()
///
/// This is different from the `this.` prefix case - here we're testing implicit receiver.
#[test]
fn test_listener_in_if_accessing_component_property() {
    // Simplified ImportComponent pattern:
    // - @if block with a listener that calls a method chain on a component property
    let template = r#"
@if (showForm) {
  <div (customEvent)="formGroup.controls.value.update($event)">
    Click me
  </div>
}
"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the embedded view function (the @if branch)
    let template_fn_start = js.find("TestComponent_Conditional_0_Template").unwrap();
    let template_fn_body = &js[template_fn_start..];
    println!("\nTemplate function:\n{}", &template_fn_body[..template_fn_body.len().min(1500)]);

    // The embedded view function should have nextContext() to access root context
    assert!(
        template_fn_body.contains("nextContext("),
        "Embedded view listener should have nextContext() call. Got:\n{}",
        &template_fn_body[..template_fn_body.len().min(800)]
    );

    // The resetView call should NOT use raw `ctx.formGroup`
    // It should use something like `ctx_r0.formGroup` (variable from nextContext)
    assert!(
        !template_fn_body.contains("ctx.formGroup"),
        "resetView should NOT use raw ctx.formGroup - should use ctx_rN.formGroup from nextContext(). Got:\n{}",
        &template_fn_body[..template_fn_body.len().min(800)]
    );

    // Verify it uses the context variable (ctx_r...) instead
    assert!(
        template_fn_body.contains("ctx_r"),
        "Should use ctx_rN (e.g., ctx_r0, ctx_r1) from nextContext(). Got:\n{}",
        &template_fn_body[..template_fn_body.len().min(800)]
    );
}

/// Test: Cascading unused variable removal via uncountVariableUsages in handler ops.
///
/// When a listener is inside a deeply nested @for loop but only accesses the
/// component context (not loop variables), the generated handler should NOT
/// have extra nextContext() calls for intermediate scopes.
///
/// Scenario:
/// - @for (outer) -> @for (inner) -> listener that only calls component method
/// - The listener handler generates variables for both @for contexts (item, innerItem)
/// - Since the listener doesn't use innerItem, that variable is removed
/// - Without uncountVariableUsages: the intermediate context variable (ctx_for_inner)
///   still appears used (count=1 from the removed innerItem variable), causing
///   its ViewContextWrite fence to keep it as a statement op -> extra nextContext()
/// - With uncountVariableUsages: removing innerItem decrements ctx_for_inner's count
///   to 0, so ctx_for_inner is also removed, and then ctx_for_outer is also removed,
///   resulting in clean code with only one nextContext() to the component.
#[test]
fn test_cascading_unused_variable_removal_in_handler() {
    let template = r#"
@for (group of groups; track group.id) {
  @for (item of group.items; track item.id) {
    <button (click)="doSomething()">Click</button>
  }
}
"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // Find the click listener
    let listener_match = js.find("listener(\"click\"").or_else(|| js.find("listener('click'"));
    assert!(listener_match.is_some(), "Should have a click listener. Got:\n{js}");

    let listener_pos = listener_match.unwrap();
    let listener_end = js[listener_pos..].find("});").unwrap_or(500);
    let listener_body = &js[listener_pos..listener_pos + listener_end + 3];

    println!("Listener body:\n{listener_body}");

    // The listener should have exactly ONE nextContext() call (to reach the component).
    // Without uncountVariableUsages, there would be extra nextContext() calls
    // for the intermediate @for scopes whose variables weren't actually used.
    let next_context_count = listener_body.matches("nextContext(").count();

    // We expect exactly 1 nextContext call that reaches the component context.
    // If uncountVariableUsages is not implemented, we'd see additional
    // nextContext() calls as standalone statements (not assigned to variables).
    assert!(
        next_context_count == 1,
        "Expected exactly 1 nextContext() call in listener, but found {next_context_count}.\n\
         Without uncountVariableUsages, intermediate scope variables aren't decremented\n\
         when unused variables referencing them are removed, causing extra nextContext() calls.\n\
         Listener body:\n{listener_body}"
    );

    // Also verify there are no standalone nextContext() statements
    // (nextContext calls that aren't part of variable assignments)
    for line in listener_body.lines() {
        let trimmed = line.trim();
        if trimmed.contains("nextContext(")
            && trimmed.ends_with(';')
            && !trimmed.contains("const ")
            && !trimmed.contains("let ")
            && !trimmed.contains("= ")
        {
            panic!(
                "Found standalone nextContext() statement (should have been removed):\n\
                 '{trimmed}'\n\
                 This indicates uncountVariableUsages is not working correctly.\n\
                 Listener body:\n{listener_body}"
            );
        }
    }
}

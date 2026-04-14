//! Test for listener handlers in @for loops
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

/// Test: *ngFor with nested *ngIf and listener
/// This tests the pattern from CustomFieldsComponent where a click handler inside
/// a nested *ngIf needs to access loop variables from the parent *ngFor
#[test]
fn test_ngfor_nested_ngif_listener() {
    let template = r#"
<div *ngFor="let field of fields.controls; let i = index">
  <button
    (click)="openDialog({ index: i, label: field.value.name })"
    *ngIf="canEdit(field.value.type)"
  >Edit</button>
</div>
"#;

    let js = compile_template_to_js(template, "TestComponent");
    println!("Generated JS:\n{js}");

    // The listener function should have:
    // 1. A nextContext() call to get the ngFor context
    // 2. Extract $implicit (field) from that context
    // 3. Extract index (i) from that context
    // 4. Another nextContext(N) to get the component context
    // 5. Use the extracted variables in the method call

    // Check that we're extracting from $implicit
    assert!(
        js.contains("$implicit") || js.contains("ctx_r"),
        "Should extract loop variable from $implicit or ctx_r\nOutput:\n{js}"
    );

    // Check that we're extracting index
    assert!(
        js.contains(".index") || js.contains("i_r"),
        "Should extract index from context\nOutput:\n{js}"
    );

    // The expression should NOT be ctx.i or ctx.field (direct property access on root context)
    // It should use captured variables like i_r6 and field_r3
    // Look at the listener function
    let listener_start = js.find("_listener(").expect("Should have a listener");
    let listener_end =
        js[listener_start..].find("});").map_or(js.len(), |i| listener_start + i + 3);
    let listener_section = &js[listener_start..listener_end];

    // The listener should NOT use ctx.i or ctx.field for loop variables
    let uses_wrong_ctx_access = listener_section.contains("ctx.i")
        || listener_section.contains("ctx.field")
        || listener_section.contains("{index:ctx");

    assert!(
        !uses_wrong_ctx_access,
        "Listener is using direct ctx access for loop variables instead of captured variables.\n\
         This means generate_variables is not adding Variable ops for ngFor context.\n\
         Listener section:\n{listener_section}"
    );
}

/// Test: Full chip-select template pattern
#[test]
fn test_full_chip_select() {
    // Exact structure from chip-select.component.html
    let template = r#"
<div [ngClass]="{ 'class': someCondition }">
  <button
    [bitMenuTriggerFor]="menu"
    [disabled]="disabled()"
    [title]="label"
    #menuTrigger="menuTrigger"
    (click)="setMenuWidth()"
    #chipSelectButton
  >
    <span>
      <i [ngClass]="icon"></i>
      <span>{{ label }}</span>
    </span>
    @if (!selectedOption) {
      <i [ngClass]="menuTrigger.isOpen ? 'up' : 'down'"></i>
    }
  </button>

  @if (selectedOption) {
    <button
      [disabled]="disabled()"
      (click)="clear()"
    >
      <i></i>
    </button>
  }
</div>

<bit-menu #menu (closed)="handleMenuClosed()">
  @if (renderedOptions) {
    <div [ngStyle]="menuWidth && { width: menuWidth + 'px' }">
      @if (getParent(renderedOptions); as parent) {
        <button
          bitMenuItem
          (click)="viewOption(parent, $event)"
          [title]="label"
        >
          <i slot="start"></i>
          {{ label }}
        </button>
        <button
          bitMenuItem
          (click)="selectOption(renderedOptions, $event)"
          [title]="renderedOptions.label"
        >
          <i slot="start"></i>
          {{ renderedOptions.label }}
        </button>
      }
      @for (option of renderedOptions.children; track option) {
        <button
          bitMenuItem
          (click)="option.children?.length ? viewOption(option, $event) : selectOption(option, $event)"
          [disabled]="option.disabled"
          [title]="option.label"
          [attr.aria-haspopup]="option.children?.length ? 'menu' : null"
        >
          @if (option.icon) {
            <i slot="start" [ngClass]="option.icon"></i>
          }
          {{ option.label }}
          @if (option.children?.length) {
            <i slot="end"></i>
          }
        </button>
      }
    </div>
  }
</bit-menu>
"#;

    let js = compile_template_to_js(template, "ChipSelectComponent");

    // Find the For_N_Template function that has the complex click handler
    // and verify it uses $implicit and nextContext correctly
    // NOTE: We specifically look for "_For_N_Template(" to avoid matching nested
    // Conditional templates inside the For (like _For_3_Conditional_1_Template)
    let lines: Vec<_> = js.lines().collect();
    let for_template_idx = lines.iter().position(|l| {
        // Match pattern like "_For_3_Template(rf,ctx)" but not "_For_3_Conditional_1_Template"
        // The function name has _For_N_Template where N is a number with no _Conditional_ after _For_
        if let Some(for_pos) = l.find("_For_") {
            let after_for = &l[for_pos + 5..]; // Skip past "_For_"
            // Find "_Template(" after the For
            if let Some(template_pos) = after_for.find("_Template(rf,ctx)") {
                // Check that there's no "_Conditional_" between _For_ and _Template
                let between = &after_for[..template_pos];
                return !between.contains("_Conditional_");
            }
        }
        false
    });

    if let Some(idx) = for_template_idx {
        let for_template_section: String = lines[idx..]
            .iter()
            .take_while(|l| !l.starts_with("function Chip") || l.contains(lines[idx]))
            .take(30)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");

        // Check that the listener properly extracts from $implicit
        assert!(
            for_template_section.contains("restoreView")
                && for_template_section.contains("$implicit"),
            "@for listener should extract option from restoreView().$implicit"
        );

        // Check for nextContext
        assert!(
            for_template_section.contains("nextContext"),
            "@for listener should use nextContext to access component methods"
        );
    } else {
        panic!("Could not find @for template function in output:\n{js}");
    }
}

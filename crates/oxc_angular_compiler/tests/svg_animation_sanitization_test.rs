//! Issue #315 sub-gap 2: SVG animation *value* attributes must be validated.
//!
//! Binding `[attr.to]` / `[attr.from]` / `[attr.values]` on an SVG `<animate>`
//! (or `[attr.to]` on `<set>`) animates the referenced attribute's *value* at
//! runtime. This is an XSS vector identical to `attributeName`, so upstream
//! registers these as `ATTRIBUTE_NO_BINDING`, which resolves to the
//! `ɵɵvalidateAttribute` validator in generated code.
//!
//! Before the schema fix these emitted a plain `ɵɵattribute("to", ctx.x)` with no
//! validator. After the fix they emit
//! `ɵɵattribute("to", ctx.x, ɵɵvalidateAttribute)`.
//!
//! The harness here mirrors `integration_test.rs::compile_template_to_js`
//! (the same public pipeline API), kept self-contained to avoid disturbing the
//! large shared integration file.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    output::ast::FunctionExpr,
    output::emitter::JsEmitter,
    parser::html::HtmlParser,
    pipeline::{emit::compile_template, ingest::ingest_component},
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions},
};
use oxc_str::Ident;

/// Compiles an Angular template to JavaScript via the full pipeline.
fn compile_template_to_js(template: &str, component_name: &str) -> String {
    let allocator = Allocator::default();

    let parser = HtmlParser::with_expansion_forms(&allocator, template, "test.html");
    let html_result = parser.parse();
    if !html_result.errors.is_empty() {
        let errors: Vec<_> = html_result.errors.iter().map(|e| e.msg.clone()).collect();
        panic!("HTML parse errors: {errors:?}");
    }

    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&html_result.nodes);
    if !r3_result.errors.is_empty() {
        let errors: Vec<_> = r3_result.errors.iter().map(|e| e.msg.clone()).collect();
        panic!("Transform errors: {errors:?}");
    }

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

/// Emit a `FunctionExpr` to JavaScript (mirrors the integration-test helper).
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

#[test]
fn svg_animate_to_binding_emits_validate_attribute() {
    // <animate [attr.to]> -> ɵɵattribute("to", ctx.x, ɵɵvalidateAttribute)
    let js = compile_template_to_js("<svg><animate [attr.to]=\"x\"></animate></svg>", "TestCmp");
    assert!(
        js.contains("ɵɵvalidateAttribute"),
        "expected ɵɵvalidateAttribute for <animate [attr.to]>, got:\n{js}"
    );
}

#[test]
fn svg_animate_from_binding_emits_validate_attribute() {
    let js = compile_template_to_js("<svg><animate [attr.from]=\"x\"></animate></svg>", "TestCmp");
    assert!(
        js.contains("ɵɵvalidateAttribute"),
        "expected ɵɵvalidateAttribute for <animate [attr.from]>, got:\n{js}"
    );
}

#[test]
fn svg_animate_values_binding_emits_validate_attribute() {
    let js =
        compile_template_to_js("<svg><animate [attr.values]=\"x\"></animate></svg>", "TestCmp");
    assert!(
        js.contains("ɵɵvalidateAttribute"),
        "expected ɵɵvalidateAttribute for <animate [attr.values]>, got:\n{js}"
    );
}

#[test]
fn svg_namespaced_animate_to_binding_emits_validate_attribute() {
    // Explicitly namespaced `<svg:animate>` (stored as `:svg:animate`) must
    // resolve identically once the namespace prefix is stripped at lookup.
    let js = compile_template_to_js("<svg:animate [attr.to]=\"x\"></svg:animate>", "TestCmp");
    assert!(
        js.contains("ɵɵvalidateAttribute"),
        "expected ɵɵvalidateAttribute for <svg:animate [attr.to]>, got:\n{js}"
    );
}

#[test]
fn svg_set_to_binding_emits_validate_attribute() {
    let js = compile_template_to_js("<svg><set [attr.to]=\"x\"></set></svg>", "TestCmp");
    assert!(
        js.contains("ɵɵvalidateAttribute"),
        "expected ɵɵvalidateAttribute for <set [attr.to]>, got:\n{js}"
    );
}

#[test]
fn plain_attr_binding_does_not_emit_validate_attribute() {
    // Control: an ordinary attribute binding must NOT gain a validator, proving
    // the harness compiles distinct templates and the schema change is targeted.
    let js = compile_template_to_js("<div [attr.title]=\"x\"></div>", "TestCmp");
    assert!(
        !js.contains("ɵɵvalidateAttribute"),
        "unexpected ɵɵvalidateAttribute for <div [attr.title]>, got:\n{js}"
    );
    assert!(js.contains("ɵɵattribute"), "expected ɵɵattribute for <div [attr.title]>, got:\n{js}");
}

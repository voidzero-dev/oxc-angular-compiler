//! `TransformResult::template_updates` carries each component's template
//! function compiled on its own (`compile_template_to_js`) for HMR. The
//! transform only does that extra compile when HMR is on.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, compile_template_to_js, transform_angular_file};

const TEMPLATE: &str = "<div>{{ name }}</div>@if (show) {<span>x</span>}";

fn source() -> String {
    format!(
        "import {{Component}} from '@angular/core';\n\
         @Component({{selector: 'a-c', template: `{TEMPLATE}`}})\n\
         export class A {{ name = 'a'; show = true; }}\n\
         @Component({{selector: 'b-c', template: ''}})\n\
         export class B {{}}\n"
    )
}

#[test]
fn hmr_reports_each_template_compiled_on_its_own() {
    let allocator = Allocator::default();
    let source = source();
    let options = TransformOptions { hmr: true, ..TransformOptions::default() };
    let result = transform_angular_file(&allocator, "test.ts", &source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

    let mut keys: Vec<&str> = result.template_updates.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["test.ts@A", "test.ts@B"]);

    let template_allocator = Allocator::default();
    let expected = compile_template_to_js(&template_allocator, TEMPLATE, "A", "test.ts").unwrap();
    assert_eq!(result.template_updates["test.ts@A"], expected);
    let expected = compile_template_to_js(&template_allocator, "", "B", "test.ts").unwrap();
    assert_eq!(result.template_updates["test.ts@B"], expected);
}

#[test]
fn no_template_updates_without_hmr() {
    let allocator = Allocator::default();
    let source = source();
    let result = transform_angular_file(&allocator, "test.ts", &source, None, None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(result.template_updates.is_empty());
    assert!(result.code.contains("A_Template"), "{}", result.code);
}

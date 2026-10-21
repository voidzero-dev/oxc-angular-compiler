//! Integration tests for the Angular build optimizer.

use oxc_allocator::Allocator;
use oxc_angular_compiler::optimizer::{OptimizeOptions, optimize};

#[test]
fn test_optimize_angular_component() {
    let allocator = Allocator::default();

    let code = r#"
import * as i0 from "@angular/core";
let MyComponent = class MyComponent {};
MyComponent.ɵcmp = /* @__PURE__ */ i0.ɵɵdefineComponent({
    type: MyComponent,
    selectors: [["my-component"]],
    template: function MyComponent_Template(rf, ctx) {}
});
MyComponent.ɵfac = function MyComponent_Factory(t) { return new (t || MyComponent)(); };
(function () {
    (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(MyComponent, [{
        type: i0.Component,
        args: [{ selector: 'my-component', template: '<h1>Hello</h1>' }]
    }], null, null);
})();
"#;

    let options = OptimizeOptions::all();
    let result = optimize(&allocator, code, "my-component.js", options);

    // Check that metadata was elided (replaced with void 0)
    assert!(!result.code.contains("ɵsetClassMetadata"), "Expected metadata call to be removed");

    // Check that the metadata call was replaced with void 0
    assert!(result.code.contains("void 0"), "Expected metadata call to be replaced with void 0");

    // Check that the class and static members are preserved
    assert!(result.code.contains("MyComponent"), "Expected MyComponent class to be preserved");
    assert!(result.code.contains("ɵcmp"), "Expected ɵcmp static member to be preserved");
    assert!(result.code.contains("ɵfac"), "Expected ɵfac static member to be preserved");
}

#[test]
fn test_optimize_with_wrap_static_members() {
    let allocator = Allocator::default();

    // This pattern matches what Angular compiles to without the ngDevMode IIFE wrapper
    let code = r#"
let MyDirective = class MyDirective {};
MyDirective.ɵdir = defineDirective({
    type: MyDirective,
    selectors: [["myDirective"]]
});
MyDirective.ɵfac = function MyDirective_Factory(t) { return new (t || MyDirective)(); };
"#;

    let options = OptimizeOptions { wrap_static_members: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, code, "directive.js", options);

    // Check that the class is wrapped in a pure IIFE
    assert!(
        result.code.contains("/* @__PURE__ */ (() =>"),
        "Expected class to be wrapped in pure IIFE"
    );
    assert!(result.code.contains("return MyDirective;"), "Expected return statement for class");
}

#[test]
fn test_optimize_typescript_enum() {
    let allocator = Allocator::default();

    let code = r#"
var ChangeDetectionStrategy;
(function(ChangeDetectionStrategy) {
    ChangeDetectionStrategy[ChangeDetectionStrategy["OnPush"] = 0] = "OnPush";
    ChangeDetectionStrategy[ChangeDetectionStrategy["Default"] = 1] = "Default";
})(ChangeDetectionStrategy || (ChangeDetectionStrategy = {}));
"#;

    let options = OptimizeOptions { adjust_enums: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, code, "enums.js", options);

    // Check that the enum is converted to a pure IIFE
    assert!(result.code.contains("/* @__PURE__ */"), "Expected enum to be marked as pure");
    assert!(
        result.code.contains("var ChangeDetectionStrategy = "),
        "Expected combined declaration"
    );
    assert!(
        result.code.contains("return ChangeDetectionStrategy;"),
        "Expected return statement in optimized enum"
    );
}

#[test]
fn test_optimize_mark_pure() {
    let allocator = Allocator::default();

    let code = r"
const injector = createInjector();
const instance = new MyClass();
";

    let options = OptimizeOptions { mark_pure: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, code, "calls.js", options);

    // Check that calls are marked as pure
    assert!(
        result.code.contains("/* @__PURE__ */ createInjector"),
        "Expected createInjector to be marked as pure"
    );
    assert!(
        result.code.contains("/* @__PURE__ */ new MyClass"),
        "Expected new MyClass to be marked as pure"
    );
}

#[test]
fn test_optimize_preserves_non_metadata() {
    let allocator = Allocator::default();

    let code = r#"
import * as i0 from "@angular/core";
export function helper() { return 42; }
export const CONFIG = { debug: false };
let MyService = class MyService {
    constructor() {
        this.value = 'test';
    }
};
MyService.ɵprov = i0.ɵɵdefineInjectable({ token: MyService, factory: MyService.ɵfac });
MyService.ɵfac = function MyService_Factory(t) { return new (t || MyService)(); };
"#;

    let options = OptimizeOptions::all();
    let result = optimize(&allocator, code, "service.js", options);

    // Check that non-metadata code is preserved
    assert!(
        result.code.contains("export function helper()"),
        "Expected helper function to be preserved"
    );
    assert!(
        result.code.contains("export const CONFIG"),
        "Expected CONFIG constant to be preserved"
    );
    assert!(
        result.code.contains("this.value = 'test'"),
        "Expected class constructor to be preserved"
    );
}

#[test]
fn test_optimize_disabled_transformations() {
    let allocator = Allocator::default();

    let code = r"
i0.ɵsetClassMetadata(MyComponent, [], null, null);
const x = createInjector();
";

    // Disable all transformations
    let options = OptimizeOptions {
        sourcemap: false,
        elide_metadata: false,
        wrap_static_members: false,
        mark_pure: false,
        adjust_enums: false,
    };

    let result = optimize(&allocator, code, "test.js", options);

    // Code should be unchanged
    assert!(
        result.code.contains("ɵsetClassMetadata"),
        "Expected metadata to be preserved when elide_metadata is false"
    );
    assert!(
        !result.code.contains("/* @__PURE__ */"),
        "Expected no pure annotations when mark_pure is false"
    );
}

/// With `sourcemap`, the optimizer returns a map from the optimized code back
/// to the input; code after an elided metadata block maps to its original line.
#[test]
fn test_optimize_sourcemap() {
    let code = r#"import * as i0 from "@angular/core";
class Foo {}
(function () {
    (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(Foo, [{
        type: i0.Injectable
    }], null, null);
})();
export { Foo };
"#;
    let allocator = Allocator::default();
    assert!(
        optimize(&allocator, code, "foo.mjs", OptimizeOptions::all()).map.is_none(),
        "no map unless requested"
    );

    let options = OptimizeOptions { sourcemap: true, ..OptimizeOptions::all() };
    let result = optimize(&allocator, code, "foo.mjs", options);
    assert!(!result.code.contains("ɵsetClassMetadata"), "{}", result.code);
    let map = result.map.expect("sourcemap requested");
    let map = oxc_sourcemap::SourceMap::from_json_string(&map).unwrap();
    assert_eq!(map.get_sources().collect::<Vec<_>>(), ["foo.mjs"]);

    let line_of = |text: &str| text.lines().position(|l| l.starts_with("export")).unwrap() as u32;
    let (src_line, out_line) = (line_of(code), line_of(&result.code));
    assert_ne!(src_line, out_line, "elision should remove lines:\n{}", result.code);
    assert!(
        map.get_tokens().any(|t| t.get_dst_line() == out_line
            && t.get_dst_col() == 0
            && t.get_src_line() == src_line
            && t.get_src_col() == 0),
        "`export` line {out_line} should map to source line {src_line}:\n{}",
        result.code
    );
}

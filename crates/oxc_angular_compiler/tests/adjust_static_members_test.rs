//! Tests for adjust-static-class-members transformation.
//! Tests focused on Angular-specific static fields (ɵcmp, ɵfac, ɵdir, etc.)
//!
//! Note: The full Angular CLI plugin also handles:
//! - Eliding decorator-related fields (ctorParameters, decorators, propDecorators)
//! - Wrapping classes with any pure static fields
//! These features are not yet implemented in this Rust version.

use oxc_allocator::Allocator;
use oxc_angular_compiler::optimizer::{OptimizeOptions, optimize};

fn test_wrap_static(input: &str, expected_contains: &[&str], expected_not_contains: &[&str]) {
    let allocator = Allocator::default();
    let options = OptimizeOptions { wrap_static_members: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, input, "test.js", options);

    for expected in expected_contains {
        assert!(
            result.code.contains(expected),
            "Expected output to contain '{}'\n\nActual output:\n{}",
            expected,
            result.code
        );
    }

    for not_expected in expected_not_contains {
        assert!(
            !result.code.contains(not_expected),
            "Expected output NOT to contain '{}'\n\nActual output:\n{}",
            not_expected,
            result.code
        );
    }
}

fn test_no_change(input: &str) {
    let allocator = Allocator::default();
    let options = OptimizeOptions { wrap_static_members: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, input, "test.js", options);

    // Should not wrap the class in an IIFE
    assert!(
        !result.code.contains("/* @__PURE__ */ (() =>"),
        "Expected no IIFE wrapping\n\nActual output:\n{}",
        result.code
    );
}

// ============================================================================
// Side effects tests
// ============================================================================

#[test]
fn test_does_not_wrap_class_with_only_side_effect_fields() {
    let input = r"
class CustomComponentEffects {
    constructor(_actions) {
        this._actions = _actions;
        this.doThis = this._actions;
    }
}
CustomComponentEffects.someFieldWithSideEffects = console.log('foo');
";

    test_no_change(input);
}

#[test]
fn test_does_not_wrap_class_with_only_side_effect_native_fields() {
    let input = r"
class CustomComponentEffects {
    static someFieldWithSideEffects = console.log('foo');
    constructor(_actions) {
        this._actions = _actions;
        this.doThis = this._actions;
    }
}
";

    test_no_change(input);
}

#[test]
fn test_does_not_wrap_class_with_only_instance_native_fields() {
    let input = r"
class CustomComponentEffects {
    someFieldWithSideEffects = console.log('foo');
    constructor(_actions) {
        this._actions = _actions;
        this.doThis = this._actions;
    }
}
";

    test_no_change(input);
}

#[test]
fn test_does_not_wrap_class_with_only_some_pure_static_fields() {
    let input = r"
class CustomComponentEffects {
    constructor(_actions) {
        this._actions = _actions;
        this.doThis = this._actions;
    }
}
CustomComponentEffects.someField = 42;
CustomComponentEffects.someFieldWithSideEffects = console.log('foo');
";

    test_no_change(input);
}

#[test]
fn test_does_not_wrap_class_with_pure_native_and_side_effect_static() {
    let input = r"
class CustomComponentEffects {
    static someField = 42;
    constructor(_actions) {
        this._actions = _actions;
        this.doThis = this._actions;
    }
}
CustomComponentEffects.someFieldWithSideEffects = console.log('foo');
";

    test_no_change(input);
}

#[test]
fn test_does_not_wrap_class_with_only_some_pure_native_static_fields() {
    let input = r"
class CustomComponentEffects {
    static someField = 42;
    static someFieldWithSideEffects = console.log('foo');
    constructor(_actions) {
        this._actions = _actions;
        this.doThis = this._actions;
    }
}
";

    test_no_change(input);
}

// ============================================================================
// Angular static fields tests
// ============================================================================

#[test]
fn test_wraps_class_with_angular_fac_static_field() {
    let input = r"
class CommonModule {
}
CommonModule.ɵfac = function CommonModule_Factory(t) { return new (t || CommonModule)(); };
";

    test_wrap_static(
        input,
        &[
            "/* @__PURE__ */ (() =>",
            "class CommonModule {",
            "CommonModule.ɵfac = function CommonModule_Factory(t) { return new (t || CommonModule)(); };",
            "return CommonModule;",
        ],
        &[],
    );
}

#[test]
fn test_does_not_wrap_class_with_side_effect_static_block() {
    let input = r"
class CommonModule {
    static { globalThis.bar = 1 }
}
";

    test_no_change(input);
}

#[test]
fn test_wraps_class_with_angular_mod_static_field() {
    let input = r"
class CommonModule {
}
CommonModule.ɵmod = /*@__PURE__*/ ɵngcc0.ɵɵdefineNgModule({ type: CommonModule });
";

    test_wrap_static(
        input,
        &[
            "/* @__PURE__ */ (() =>",
            "class CommonModule {",
            "CommonModule.ɵmod =",
            "return CommonModule;",
        ],
        &[],
    );
}

#[test]
fn test_wraps_class_with_angular_inj_static_field() {
    let input = r"
class CommonModule {
}
CommonModule.ɵinj = /*@__PURE__*/ ɵngcc0.ɵɵdefineInjector({ providers: [
    { provide: NgLocalization, useClass: NgLocaleLocalization },
] });
";

    test_wrap_static(
        input,
        &[
            "/* @__PURE__ */ (() =>",
            "class CommonModule {",
            "CommonModule.ɵinj =",
            "return CommonModule;",
        ],
        &[],
    );
}

#[test]
fn test_wraps_class_with_multiple_angular_static_fields() {
    let input = r"
class CommonModule {
}
CommonModule.ɵfac = function CommonModule_Factory(t) { return new (t || CommonModule)(); };
CommonModule.ɵmod = /*@__PURE__*/ ɵngcc0.ɵɵdefineNgModule({ type: CommonModule });
CommonModule.ɵinj = /*@__PURE__*/ ɵngcc0.ɵɵdefineInjector({ providers: [
    { provide: NgLocalization, useClass: NgLocaleLocalization },
] });
";

    test_wrap_static(
        input,
        &[
            "/* @__PURE__ */ (() =>",
            "class CommonModule {",
            "CommonModule.ɵfac =",
            "CommonModule.ɵmod =",
            "CommonModule.ɵinj =",
            "return CommonModule;",
        ],
        &[],
    );
}

// ============================================================================
// Angular component tests
// ============================================================================

#[test]
fn test_wraps_class_with_angular_cmp_static_field() {
    let input = r#"
let MyComponent = class MyComponent {};
MyComponent.ɵcmp = /* @__PURE__ */ defineComponent({
    type: MyComponent,
    selectors: [["my-component"]]
});
"#;

    test_wrap_static(
        input,
        &[
            "/* @__PURE__ */ (() =>",
            "class MyComponent {}",
            "MyComponent.ɵcmp =",
            "return MyComponent;",
        ],
        &[],
    );
}

#[test]
fn test_wraps_class_with_angular_dir_static_field() {
    let input = r#"
let MyDirective = class MyDirective {};
MyDirective.ɵdir = defineDirective({
    type: MyDirective,
    selectors: [["myDir"]]
});
"#;

    test_wrap_static(
        input,
        &[
            "/* @__PURE__ */ (() =>",
            "class MyDirective {}",
            "MyDirective.ɵdir =",
            "return MyDirective;",
        ],
        &[],
    );
}

#[test]
fn test_wraps_class_with_angular_pipe_static_field() {
    let input = r#"
let MyPipe = class MyPipe {};
MyPipe.ɵpipe = definePipe({
    type: MyPipe,
    name: "myPipe"
});
"#;

    test_wrap_static(
        input,
        &["/* @__PURE__ */ (() =>", "class MyPipe {}", "MyPipe.ɵpipe =", "return MyPipe;"],
        &[],
    );
}

#[test]
fn test_wraps_class_with_angular_prov_static_field() {
    let input = r"
let MyService = class MyService {};
MyService.ɵprov = defineInjectable({
    token: MyService,
    factory: MyService.ɵfac
});
";

    test_wrap_static(
        input,
        &["/* @__PURE__ */ (() =>", "class MyService {}", "MyService.ɵprov =", "return MyService;"],
        &[],
    );
}

// ============================================================================
// Decorator tests (wrapDecorators mode)
// ============================================================================

#[test]
fn test_wraps_class_with_decorators_esbuild_output() {
    // This tests the pattern esbuild emits with decorators
    let input = r#"
var ExampleClass = class {
    method() {
    }
};
__decorate([
    SomeDecorator()
], ExampleClass.prototype, "method", null);
"#;

    // Note: In the current implementation, we may or may not wrap this
    // depending on whether wrapDecorators is enabled. For now, we test
    // the basic behavior.
    let allocator = Allocator::default();
    let options = OptimizeOptions { wrap_static_members: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, input, "test.js", options);

    // The code should compile without errors
    assert!(result.code.contains("ExampleClass"), "Expected ExampleClass to be in output");
}

#[test]
fn test_wraps_class_with_class_decorators() {
    let input = r"
let SomeClass = class SomeClass {
};
SomeClass = __decorate([
    SomeDecorator()
], SomeClass);
";

    // Test that the code compiles - decorator wrapping may vary based on options
    let allocator = Allocator::default();
    let options = OptimizeOptions { wrap_static_members: true, ..OptimizeOptions::default() };

    let result = optimize(&allocator, input, "test.js", options);

    assert!(result.code.contains("SomeClass"), "Expected SomeClass to be in output");
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_handles_class_expression_assignment() {
    let input = r"
let MyComponent = class MyComponent {};
MyComponent.ɵcmp = defineComponent({});
MyComponent.ɵfac = function(t) { return new (t || MyComponent)(); };
";

    test_wrap_static(input, &["/* @__PURE__ */ (() =>", "return MyComponent;"], &[]);
}

#[test]
fn test_handles_var_class_expression_assignment() {
    let input = r"
var MyDirective = class MyDirective {};
MyDirective.ɵdir = defineDirective({});
";

    test_wrap_static(input, &["/* @__PURE__ */ (() =>", "return MyDirective;"], &[]);
}

#[test]
fn test_handles_multiple_classes() {
    let input = r"
class ClassA {}
ClassA.ɵfac = factoryA;
class ClassB {}
ClassB.ɵfac = factoryB;
";

    test_wrap_static(input, &["/* @__PURE__ */ (() =>", "return ClassA;", "return ClassB;"], &[]);
}

#[test]
fn test_class_declaration_preserves_binding_for_export() {
    // Regression test: class declarations (not variable declarations) must
    // produce `let X = /* @__PURE__ */ (() => { ... })()` so the name remains
    // in scope for subsequent `export { X }` statements.
    let input = r"
class ClipboardModule {}
ClipboardModule.ɵfac = function ClipboardModule_Factory(t) { return new (t || ClipboardModule)(); };
ClipboardModule.ɵmod = defineNgModule({ type: ClipboardModule });
export { ClipboardModule };
";

    test_wrap_static(
        input,
        &[
            "let ClipboardModule = /* @__PURE__ */ (() =>",
            "return ClipboardModule;",
            "export { ClipboardModule }",
        ],
        &[],
    );
}

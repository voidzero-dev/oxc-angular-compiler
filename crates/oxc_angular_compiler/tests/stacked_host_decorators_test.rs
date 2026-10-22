//! A member can carry several `@HostListener`s or `@HostBinding`s
//! (`@HostListener('click') @HostListener('keydown.enter') activate() {}`); each
//! one is a host listener or binding, as in ngtsc. The expected code is
//! `@angular/compiler-cli` 21.0.1's `hostBindings` for the same class.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

#[test]
fn every_stacked_host_listener_and_binding_is_compiled() {
    let source = r"
import { Component, HostBinding, HostListener } from '@angular/core';
@Component({ selector: 'c-cmp', template: '' })
export class CCmp {
  @HostListener('click') @HostListener('keydown.enter') activate() {}
  @HostListener('focus', ['$event']) @HostListener('blur', ['$event']) onFocus(e: Event) {}
  @HostBinding('class.on') @HostBinding('attr.aria-pressed') on = true;
}
";
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "test.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();

    let listeners = concat!(
        r#"i0.ɵɵlistener("click",functionCCmp_click_HostBindingHandler(){returnctx.activate();})"#,
        r#"("keydown.enter",functionCCmp_keydown_enter_HostBindingHandler(){returnctx.activate();})"#,
        r#"("focus",functionCCmp_focus_HostBindingHandler($event){returnctx.onFocus($event);})"#,
        r#"("blur",functionCCmp_blur_HostBindingHandler($event){returnctx.onFocus($event);});"#,
    );
    assert!(code.contains(listeners), "Expected the four listeners in:\n{}", result.code);
    assert!(
        code.contains(r#"i0.ɵɵattribute("aria-pressed",ctx.on);i0.ɵɵclassProp("on",ctx.on);"#),
        "Expected both bindings in:\n{}",
        result.code
    );
    assert!(code.contains("hostVars:3"), "Got:\n{}", result.code);
}

#[test]
fn an_unreadable_argument_on_a_later_stacked_decorator_is_reported() {
    // A lone surrogate (`'\uD800'`) can't be read. Like on the first decorator, it is
    // reported instead of compiling a fallback (the member name, no listener, no args).
    for (member, expected) in [
        (
            r"@HostBinding('title') @HostBinding('\uD800') v = 'a';",
            "@HostBinding's argument must be a string Value could not be determined statically.",
        ),
        (
            r"@HostListener('click') @HostListener('\uD800') a() {}",
            "@HostListener's event name argument must be a string Value could not be \
             determined statically.",
        ),
        (
            r"@HostListener('click') @HostListener('x', ['\uD800']) a(x: any) {}",
            "Failed to resolve @HostListener.args at position 0 to a string Value could not be \
             determined statically.",
        ),
    ] {
        let source = format!(
            "import {{ Directive, HostBinding, HostListener }} from '@angular/core';\n\
             @Directive({{ selector: '[d]' }})\n\
             export class D {{\n  {member}\n}}"
        );
        let allocator = Allocator::default();
        let result = transform_angular_file(
            &allocator,
            "test.ts",
            &source,
            Some(&TransformOptions::default()),
            None,
        );
        let messages: Vec<String> = result.diagnostics.iter().map(|d| format!("{d}")).collect();
        assert_eq!(messages, [expected], "`{member}`");
    }
}

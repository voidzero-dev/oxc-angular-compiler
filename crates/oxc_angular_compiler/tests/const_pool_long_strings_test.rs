//! Long strings (50+ chars) go through ngtsc's constant pool like this: the first use stays
//! inline, and a second use in the same file turns them into one shared `const`, which the
//! first use then reads too. Component styles go through it. ngtsc has one pool per file, so
//! the classes of a file share the constants they pool.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    ComponentHmrUpdateInput, TransformOptions, compile_component_hmr_update, transform_angular_file,
};

/// 76 chars, over the 50-char threshold.
const LONG: &str = ".a-long-selector-name .another-long-class { color: red; font-size: 12px; }";
const LONG_SCOPED: &str = ".a-long-selector-name[_ngcontent-%COMP%]   .another-long-class[_ngcontent-%COMP%] { color: red; font-size: 12px; }";
const OTHER: &str = ".b-long-selector-name .another-long-class { color: blue; font-size: 14px; }";
const OTHER_SCOPED: &str = ".b-long-selector-name[_ngcontent-%COMP%]   .another-long-class[_ngcontent-%COMP%] { color: blue; font-size: 14px; }";

fn transform(source: &str) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, None, None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result.code
}

/// `code` without whitespace, so the emitter's line wrapping doesn't matter.
fn squash(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn component(class_name: &str, metadata: &str) -> String {
    format!(
        "@Component({{selector: '{}', template: '', {metadata} standalone: true}})\n\
         export class {class_name} {{}}\n",
        class_name.to_lowercase()
    )
}

fn declaration(name: &str, value: &str) -> String {
    squash(&format!("const {name} = {value:?};"))
}

#[test]
fn long_style_of_two_components_is_one_constant() {
    let source = format!(
        "import {{Component}} from '@angular/core';\n{}{}",
        component("A", &format!("styles: ['{LONG}'],")),
        component("B", &format!("styles: ['{LONG}'],")),
    );
    let code = transform(&source);
    let squashed = squash(&code);
    assert_eq!(squashed.matches(&declaration("_c0", LONG_SCOPED)).count(), 1, "{code}");
    assert_eq!(squashed.matches("styles:[_c0]").count(), 2, "{code}");
    assert!(!squashed.contains("_c1"), "{code}");
    // The first component reads the constant while its class is defined.
    assert!(code.find("const _c0").unwrap() < code.find("export class A").unwrap(), "{code}");
}

#[test]
fn long_style_of_two_unencapsulated_components_is_one_constant() {
    let source = format!(
        "import {{Component, ViewEncapsulation}} from '@angular/core';\n{}{}",
        component("A", &format!("styles: ['{LONG}'], encapsulation: ViewEncapsulation.None,")),
        component("B", &format!("styles: ['{LONG}'], encapsulation: ViewEncapsulation.None,")),
    );
    let code = squash(&transform(&source));
    assert_eq!(code.matches(&declaration("_c0", LONG)).count(), 1, "{code}");
    assert_eq!(code.matches("styles:[_c0],encapsulation:2").count(), 2, "{code}");
}

#[test]
fn long_style_used_twice_by_one_component_is_one_constant() {
    let source = format!(
        "import {{Component}} from '@angular/core';\n{}",
        component("A", &format!("styles: ['{LONG}', '{LONG}'],")),
    );
    let code = squash(&transform(&source));
    assert_eq!(code.matches(&declaration("_c0", LONG_SCOPED)).count(), 1, "{code}");
    assert!(code.contains("styles:[_c0,_c0]"), "{code}");
}

#[test]
fn long_style_used_once_stays_inline() {
    let source = format!(
        "import {{Component}} from '@angular/core';\n{}{}",
        component("A", &format!("styles: ['{LONG}'],")),
        component("B", "styles: ['.a { color: red }', '.a { color: red }'],"),
    );
    let code = squash(&transform(&source));
    assert!(code.contains(&squash(&format!("styles:[{LONG_SCOPED:?}]"))), "{code}");
    assert_eq!(code.matches(r#"".a[_ngcontent-%COMP%]"#).count(), 2, "{code}");
    assert!(!code.contains("_c0"), "{code}");
}

#[test]
fn long_styles_are_named_after_the_constants_pooled_before_them() {
    // ngtsc: `_c0` is B's pure function, then `_c1` is LONG (shared by B) and
    // `_c2` OTHER (shared by C), all after A's inline first uses.
    let source = format!(
        "import {{Component}} from '@angular/core';\n{}{}{}",
        component("A", &format!("styles: ['{OTHER}', '{LONG}'],")),
        "@Component({selector: 'b', template: '<div [title]=\"[v, 1]\"></div>', \
         styles: ['LONG'], standalone: true})\nexport class B { v = 1; }\n"
            .replace("LONG", LONG),
        component("C", &format!("styles: ['{OTHER}'],")),
    );
    let code = squash(&transform(&source));
    assert!(code.contains("const_c0=(a0)=>[a0,1];"), "{code}");
    assert!(code.contains(&declaration("_c1", LONG_SCOPED)), "{code}");
    assert!(code.contains(&declaration("_c2", OTHER_SCOPED)), "{code}");
    assert!(code.contains("styles:[_c2,_c1]"), "{code}");
    assert_eq!(code.matches("styles:[_c1]").count(), 1, "{code}");
    assert_eq!(code.matches("styles:[_c2]").count(), 1, "{code}");
}

#[test]
fn query_predicate_of_two_components_is_one_constant() {
    let source = "import {Component, ViewChild} from '@angular/core';
@Component({selector: 'a', template: '', standalone: true})
export class A { @ViewChild('x') a: any; }
@Component({selector: 'b', template: '', standalone: true})
export class B { @ViewChild('x') b: any; }
";
    let code = squash(&transform(source));
    assert_eq!(code.matches(r#"const_c0=["x"];"#).count(), 1, "{code}");
    assert_eq!(code.matches("i0.ɵɵviewQuery(_c0,5)").count(), 2, "{code}");
    assert!(!code.contains("_c1"), "{code}");
}

#[test]
fn hmr_update_sends_a_shared_long_style_as_text() {
    let source = format!(
        "import {{Component}} from '@angular/core';\n\
         @Component({{selector: 'a', template: '<b>x</b>', styles: ['{LONG}', '{LONG}']}})\n\
         export class A {{}}\n"
    );
    let allocator = Allocator::default();
    let input = ComponentHmrUpdateInput { class_name: "A", ..Default::default() };
    let options = TransformOptions { hmr: true, ..Default::default() };
    let update =
        compile_component_hmr_update(&allocator, "test.ts", &source, &input, Some(&options), None);
    assert!(update.diagnostics.is_empty(), "{:?}", update.diagnostics);
    assert_eq!(update.styles, Some(vec![LONG_SCOPED.to_string(), LONG_SCOPED.to_string()]));
}

#[test]
fn constants_after_an_underscore_class_skip_its_template_names() {
    // ngtsc numbers `_c` constants on their own: `_A_Template` and
    // `_A_Conditional_0_Template` take no index, and `_B_Template` no suffix.
    let source = format!(
        "import {{Component, ViewChild}} from '@angular/core';\n\
         @Component({{selector: 'a', template: '@if (v) {{<i></i>}}', styles: ['{LONG}'], standalone: true}})\n\
         export class _A {{ v = 1; }}\n\
         @Component({{selector: 'b', template: '@if (v) {{<i></i>}}', styles: ['{LONG}'], standalone: true}})\n\
         export class _B {{ v = 1; @ViewChild('x') q: any; }}\n"
    );
    let code = squash(&transform(&source));
    assert!(code.contains(r#"const_c0=["x"];"#), "{code}");
    assert!(code.contains(&declaration("_c1", LONG_SCOPED)), "{code}");
    assert_eq!(code.matches("styles:[_c1]").count(), 2, "{code}");
    assert!(code.contains("i0.ɵɵviewQuery(_c0,5)"), "{code}");
    assert!(!code.contains("_c2"), "{code}");
    assert!(code.contains("function_B_Template(rf,ctx)"), "{code}");
    assert!(code.contains("function_B_Conditional_0_Template(rf,ctx)"), "{code}");
}

#[test]
fn failed_component_hands_no_constants_on() {
    // ngtsc emits nothing for the file. Oxc still emits the next component,
    // which must declare its own constants rather than read the failed one's.
    let source = "import {Component, ViewChild} from '@angular/core';
@Component({selector: 'b', template: '<div i18n=\"desc */ bad\">Hi</div>', standalone: true})
export class B { @ViewChild('x') q: any; }
@Component({selector: 'c', template: '', standalone: true})
export class C { @ViewChild('x') q: any; @ViewChild('y') r: any; }
";
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, None, None);
    assert!(result.has_errors());
    let code = squash(&result.code);
    assert!(code.contains(r#"const_c0=["x"];"#), "{code}");
    assert!(code.contains(r#"const_c1=["y"];"#), "{code}");
    assert!(code.contains("i0.ɵɵviewQuery(_c0,5)(_c1,5)"), "{code}");
}

#[test]
fn literals_whose_strings_join_alike_stay_apart() {
    // `["a", "b,str:c,str:d"]` and `["a", "b", "c", "d"]` are different arrays, so B must not
    // read A's `attrs` constant.
    let code = squash(&transform(
        "import {Component} from '@angular/core';
@Component({selector: '[a=\"b,str:c,str:d\"]', template: 'x', standalone: true})
export class A {}
@Component({selector: '[a=b][c=d]', template: 'y', standalone: true})
export class B {}
",
    ));
    assert!(code.contains(r#"const_c0=["a","b,str:c,str:d"];"#), "{code}");
    assert!(code.contains(r#"const_c1=["a","b","c","d"];"#), "{code}");
    assert!(code.contains("type:B,selectors:[[\"\",\"a\",\"b\",\"c\",\"d\"]],attrs:_c1"), "{code}");
}

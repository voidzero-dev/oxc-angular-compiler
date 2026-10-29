//! The HMR fingerprint: a component file's source with the inline `template` /
//! `styles` values of its compiled components left out. Two saves with the
//! same fingerprint differ only inside those values, so the build tool sends
//! an HMR update instead of reloading the page.
//!
//! Ported from the vite plugin's `decorator-fields.test.ts` (the JS lexer the
//! fingerprint replaced). There, each case checked the range the lexer found;
//! here, `fields` gives the excluded text with its quotes or brackets, which
//! is the same range. Every source gets Angular's `Component` import, since
//! only Angular's own decorator is read now.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    ComponentHmrFingerprint, ComponentHmrUpdateInput, TransformOptions,
    compile_component_hmr_update, component_hmr_fingerprint, transform_angular_file,
};

const IMPORT: &str = "import { Component } from '@angular/core';\n";

/// No class or no range.
const NONE: [&str; 0] = [];

fn fingerprint_raw(source: &str, classes: &[&str]) -> ComponentHmrFingerprint {
    let allocator = Allocator::default();
    component_hmr_fingerprint(&allocator, "test.ts", source, classes)
}

/// The fingerprint of `IMPORT` + `body`.
fn fingerprint(body: &str, classes: &[&str]) -> ComponentHmrFingerprint {
    fingerprint_raw(&format!("{IMPORT}{body}"), classes)
}

/// Each excluded range of `IMPORT` + `body`, with the quote or bracket on each
/// side (what the JS lexer's `textOf` gave).
fn fields(body: &str, classes: &[&str]) -> Vec<String> {
    let source = format!("{IMPORT}{body}");
    fingerprint_raw(&source, classes)
        .excluded
        .iter()
        .map(|span| source[span.start as usize - 1..=span.end as usize].to_string())
        .collect()
}

fn components(body: &str, classes: &[&str]) -> Vec<String> {
    fingerprint(body, classes).components
}

/// A one-component file whose `@Component` holds `selector: 'a'` and `field`.
fn one(field: &str) -> Vec<String> {
    fields(&format!("@Component({{ selector: 'a', {field} }})\nexport class Foo {{}}"), &["Foo"])
}

fn same(a: &str, b: &str, classes: &[&str]) -> bool {
    fingerprint(a, classes).fingerprint == fingerprint(b, classes).fingerprint
}

// -----------------------------------------------------------------
// The HMR decision
// -----------------------------------------------------------------

const TWO: &str = "@Component({ selector: 'a', template: '<a/>', styles: ['.a{}'] })
export class A { x = 1 }
@Component({ selector: 'b', template: `<b/>`, styles: '.b{}' })
export class B {}
";

#[test]
fn an_edit_inside_template_or_styles_keeps_the_fingerprint() {
    let classes = &["A", "B"];
    for edited in [
        TWO.replace("<a/>", "<a>changed</a>"),
        TWO.replace(".a{}", ".a{color:red}"),
        TWO.replace("`<b/>`", "`<b>${'x'}</b>`"),
        TWO.replace("'.b{}'", "'.b{} .c{}'"),
        TWO.replace("['.a{}']", "['.a{}', '.more{}']"),
        TWO.replace("['.a{}']", "[]"),
        TWO.replace("<b/>", "<b>both</b>").replace(".a{}", ""),
    ] {
        assert!(same(TWO, &edited, classes), "{edited}");
    }
}

#[test]
fn an_edit_anywhere_else_changes_the_fingerprint() {
    let classes = &["A", "B"];
    for edited in [
        TWO.replace("x = 1", "x = 2"),
        TWO.replace("selector: 'a'", "selector: 'aa'"),
        TWO.replace("export class B {}", "export class B {}\n// trailing comment"),
        TWO.replace("template: '<a/>'", "templateUrl: './a.html'"),
        // The shape of a value is kept: a string and an array are not the same.
        TWO.replace("['.a{}']", "''"),
        // The quote style is kept too.
        TWO.replace("'<a/>'", "\"<a/>\""),
    ] {
        assert!(!same(TWO, &edited, classes), "{edited}");
    }
}

/// `templateUrl` / `styleUrl(s)` stay in the fingerprint: changing one
/// changes which files the component owns, which only a fresh transform
/// records, so the page reloads (as it did before the fingerprint).
#[test]
fn resource_urls_stay_in_the_fingerprint() {
    let source = "@Component({ templateUrl: './a.html', styleUrl: './a.css', styleUrls: ['./b.css'] })\nexport class A {}";
    assert_eq!(fields(source, &["A"]), NONE);
    assert!(!same(source, &source.replace("./a.html", "./b.html"), &["A"]));
    assert!(!same(source, &source.replace("./a.css", "./c.css"), &["A"]));
    assert!(!same(source, &source.replace("['./b.css']", "['./b.css', './c.css']"), &["A"]));
}

/// Only the classes the transform compiled are read: an edit to another
/// class's template reloads, since no HMR update would carry it.
#[test]
fn only_the_named_classes_are_read() {
    assert_eq!(components(TWO, &["A"]), ["A"]);
    assert!(same(TWO, &TWO.replace("<a/>", "<a>x</a>"), &["A"]));
    assert!(!same(TWO, &TWO.replace("<b/>", "<b>x</b>"), &["A"]));
    assert!(!same(TWO, &TWO.replace("<a/>", "<a>x</a>"), &[]));
    assert_eq!(fingerprint(TWO, &["Nope"]).excluded, []);
}

/// A source the parser rejects excludes nothing, so a save that leaves the
/// file broken (here an unescaped quote inside the template) reloads.
#[test]
fn a_syntax_error_excludes_nothing() {
    let broken = TWO.replace("<a/>", "<a>it's</a>");
    let result = fingerprint(&broken, &["A", "B"]);
    assert_eq!(result.excluded, []);
    assert_eq!(result.components, NONE);
    assert!(!same(TWO, &broken, &["A", "B"]));
    // Same as excluding nothing.
    assert_eq!(result.fingerprint, fingerprint(&broken, &[]).fingerprint);
}

/// The transform reports the same fingerprint, for the classes it compiled.
#[test]
fn transform_reports_the_same_fingerprint() {
    let source = format!("{IMPORT}{TWO}");
    let allocator = Allocator::default();
    let options = TransformOptions { hmr: true, ..TransformOptions::default() };
    let result = transform_angular_file(&allocator, "test.ts", &source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        result.hmr_fingerprint.as_deref(),
        Some(fingerprint_raw(&source, &["A", "B"]).fingerprint.as_str())
    );

    let off = transform_angular_file(&allocator, "test.ts", &source, None, None);
    assert_eq!(off.hmr_fingerprint, None);
}

/// A `jit: true` component is not compiled AOT and gets no HMR update, so the
/// transform's fingerprint keeps its template: editing it reloads.
#[test]
fn transform_keeps_a_jit_components_template() {
    let source = format!(
        "{IMPORT}@Component({{ selector: 'a', template: '<a/>' }})\nexport class A {{}}\n@Component({{ selector: 'b', template: '<b/>', jit: true }})\nexport class B {{}}\n"
    );
    let allocator = Allocator::default();
    let options = TransformOptions { hmr: true, ..TransformOptions::default() };
    let result = transform_angular_file(&allocator, "test.ts", &source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        result.hmr_fingerprint.as_deref(),
        Some(fingerprint_raw(&source, &["A"]).fingerprint.as_str())
    );
    let edited_jit = source.replace("<b/>", "<b>x</b>");
    assert_ne!(
        result.hmr_fingerprint.as_deref(),
        Some(fingerprint_raw(&edited_jit, &["A"]).fingerprint.as_str())
    );
}

/// A component declared in a function is compiled with HMR too, so an edit to
/// its template is an HMR update. Two classes of one name compile without
/// HMR, so an edit to either one's template reloads. The transform and
/// `compile_component_hmr_update` give the same fingerprint.
#[test]
fn reads_a_nested_component() {
    let source = format!(
        "{IMPORT}export function make() {{
  @Component({{ selector: 'a', template: '<a/>', styles: ['.a{{}}'] }})
  class A {{ x = 1 }}
  return A;
}}
function one() {{ @Component({{ template: '<b/>' }}) class B {{}} return B; }}
function two() {{ @Component({{ template: '<c/>' }}) class B {{}} return B; }}
"
    );
    let allocator = Allocator::default();
    let options = TransformOptions { hmr: true, ..TransformOptions::default() };
    let result = transform_angular_file(&allocator, "test.ts", &source, Some(&options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.template_updates.keys().collect::<Vec<_>>(), ["test.ts@A"]);
    let fingerprint = result.hmr_fingerprint.unwrap();
    assert_eq!(fingerprint_raw(&source, &["A"]).fingerprint, fingerprint);
    assert_eq!(fingerprint_raw(&source, &["A", "B"]).fingerprint, fingerprint);
    let input = ComponentHmrUpdateInput {
        class_name: "A",
        source_fingerprint: None,
        compiled_class_names: Some(&["A"]),
    };
    let update =
        compile_component_hmr_update(&allocator, "test.ts", &source, &input, Some(&options), None);
    assert_eq!(update.fingerprint.as_deref(), Some(fingerprint.as_str()));

    let of = |edited: String| fingerprint_raw(&edited, &["A"]).fingerprint;
    assert_eq!(of(source.replace("<a/>", "<a>x</a>")), fingerprint);
    assert_eq!(of(source.replace(".a{}", ".a{color:red}")), fingerprint);
    assert_ne!(of(source.replace("x = 1", "x = 2")), fingerprint);
    assert_ne!(of(source.replace("<b/>", "<b>x</b>")), fingerprint);
}

// -----------------------------------------------------------------
// Which decorators are read
// -----------------------------------------------------------------

/// The decorator is Angular's by its `@angular/core` import, under any alias
/// or through a namespace, like the transform finds it. Another module's
/// `@Component` or a lookalike spelling is not read.
#[test]
fn reads_angulars_component_by_its_import() {
    let source = "import * as ng from '@angular/core';
import { Component as Cmp } from '@angular/core';
import { Component } from './foreign';
@Cmp({ template: 'a' })
class A {}
@ng.Component({ template: 'b' })
class B {}
@Component({ template: 'c' })
class C {}
@CmpX({ template: 'd' })
class D {}
@Cmp.x({ template: 'e' })
class E {}
@ng . Component ({ template: 'f' })
class F {}
";
    let result = fingerprint_raw(source, &["A", "B", "C", "D", "E", "F"]);
    assert_eq!(result.components, ["A", "B", "F"]);
    assert_eq!(result.excluded.len(), 3);
}

#[test]
fn reads_nothing_without_a_component_decorator() {
    assert_eq!(components("export class Foo {}", &["Foo"]), NONE);
    // A decorator with no class is a syntax error.
    assert_eq!(components("@Component({ selector: 'x' })", &["Foo"]), NONE);
    // Not an object literal argument.
    assert_eq!(components("@Component(CONFIG)\nexport class Foo {}", &["Foo"]), NONE);
    assert_eq!(components("@Component\nexport class Foo {}", &["Foo"]), NONE);
}

#[test]
fn ignores_decorators_in_comments_and_strings() {
    for body in [
        "@Component({ selector: 'x', styles: ['.real{}'] })\n// @Component({ styles: ['.old{}'] })\nexport class FooComponent {}",
        "@Component({ selector: 'x', styles: ['.real{}'] })\n/* @Component({ styles: ['.old{}'] }) */\nexport class FooComponent {}",
        "// @Component({ styles: ['.old{}'] })\n@Component({ selector: 'x', styles: ['.real{}'] })\nexport class FooComponent {}",
        "const doc = 'see @Component({ styles: [\".str{}\"] }) for details';\n@Component({ selector: 'x', styles: ['.real{}'] })\nexport class FooComponent {}",
    ] {
        assert_eq!(fields(body, &["FooComponent"]), ["['.real{}']"], "{body}");
    }
}

#[test]
fn pairs_each_class_with_its_own_decorator() {
    let body = "@Component({ selector: 'a', styles: ['.a{}'] })
// @Component({ styles: ['.fake{}'] })
export class AComponent {}
@Component({ selector: 'b', styles: ['.b{}'] })
export class BComponent {}";
    let classes = &["AComponent", "BComponent"];
    assert_eq!(components(body, classes), ["AComponent", "BComponent"]);
    assert_eq!(fields(body, classes), ["['.a{}']", "['.b{}']"]);
    assert_eq!(fields(body, &["BComponent"]), ["['.b{}']"]);
}

#[test]
fn reads_every_class_declaration_form() {
    for (body, name) in [
        ("@Component({ template: 't' })\nclass Foo {}", "Foo"),
        ("@Component({ template: 't' })\nexport default class Foo {}", "Foo"),
        ("@Component({ template: 't' })\nexport abstract class Foo {}", "Foo"),
        ("@Component({ template: 't' })\n@Inject() @Other()\nexport class Foo {}", "Foo"),
        ("@Other()\n@Component({ template: 't' })\nexport class Foo {}", "Foo"),
        (
            "@Component({ template: 't' })\nexport class Foo<T extends Bar, U> extends Base<T> implements Baz<U> {}",
            "Foo",
        ),
        ("@Component({ template: 't' })\nclass $Foo {}", "$Foo"),
        ("@Component({ template: 't' })\nclass _Bar {}", "_Bar"),
        ("@Component({ template: 't' })\nclass Café {}", "Café"),
        ("@Component({ template: 't' })\nclass 组件 {}", "组件"),
    ] {
        assert_eq!(components(body, &[name]), [name], "{body}");
        assert_eq!(fields(body, &[name]), ["'t'"], "{body}");
    }
}

/// Two `@Component`s on one class: the compiler takes the first and leaves
/// the second on the class, so only the first one's fields are excluded and
/// an edit to the second reloads. (The JS lexer paired the class with the
/// second one, the decorator the compiler does not take.)
#[test]
fn reads_the_first_of_two_component_decorators() {
    let body =
        "@Component({ template: 'first' })\n@Component({ template: 'second' })\nclass Bar {}";
    assert_eq!(fields(body, &["Bar"]), ["'first'"]);
    assert!(same(body, &body.replace("first", "1st"), &["Bar"]));
    assert!(!same(body, &body.replace("second", "2nd"), &["Bar"]));
}

#[test]
fn coexists_with_other_class_decorators() {
    let body =
        "@SignalComponent({ template: 'sig' })\n@Component({ template: '<real/>' })\nclass Foo {}";
    assert_eq!(fields(body, &["Foo"]), ["'<real/>'"]);
}

// -----------------------------------------------------------------
// Which properties are the fields
// -----------------------------------------------------------------

#[test]
fn reads_template_and_styles_values() {
    assert_eq!(one("template: '<p/>'"), ["'<p/>'"]);
    assert_eq!(one("template: \"<p/>\""), ["\"<p/>\""]);
    assert_eq!(one("template: `<p/>`"), ["`<p/>`"]);
    assert_eq!(one("styles: ['.x{}', '.y{}']"), ["['.x{}', '.y{}']"]);
    assert_eq!(one("styles: '.x{}'"), ["'.x{}'"]);
    assert_eq!(one("styles: `.x{}`"), ["`.x{}`"]);
    assert_eq!(one("template: '<p/>', styles: ['.x{}']"), ["'<p/>'", "['.x{}']"]);
    // An empty value excludes nothing; its delimiters are kept either way.
    assert_eq!(one("template: ''"), NONE);
    assert_eq!(one("styles: []"), NONE);
    assert_eq!(one("selector2: 'b'"), NONE);
}

#[test]
fn does_not_read_another_field_as_template_or_styles() {
    assert_eq!(one("styleUrls: ['./a.css']"), NONE);
    assert_eq!(one("styleUrl: './a.css'"), NONE);
    assert_eq!(one("templateUrl: './foo.html'"), NONE);
    assert_eq!(one("'styleUrls': ['./a.css']"), NONE);
    assert_eq!(one("'templateUrl': './a.html'"), NONE);
    assert_eq!(one("templateUrl: './real.html', template: '<p/>'"), ["'<p/>'"]);
    // A template literal is a template, so `styles:` inside it is text.
    assert_eq!(one("template2: `<pre>{ styles: ['fake'] }</pre>`"), NONE);
}

#[test]
fn reads_only_top_level_properties() {
    assert_eq!(
        one("template: `<pre>const cfg = { styles: ['fake'] }</pre>`, styles: ['real']"),
        ["`<pre>const cfg = { styles: ['fake'] }</pre>`", "['real']"]
    );
    assert_eq!(
        one("template: `${doThing({ styles: ['fake'] })}`, styles: ['real']"),
        ["`${doThing({ styles: ['fake'] })}`", "['real']"]
    );
    assert_eq!(
        one("host: { '[styles]': 'expr', styles: 'irrelevant' }, styles: ['real']"),
        ["['real']"]
    );
    assert_eq!(one("data: { styles: ['.deep{}'] }"), NONE);
    assert_eq!(
        one("styles: ['/* template: \"fake\" */'], template: '<real/>'"),
        ["['/* template: \"fake\" */']", "'<real/>'"]
    );
}

#[test]
fn reads_quoted_and_escaped_keys() {
    assert_eq!(one("'styles': ['.x{}']"), ["['.x{}']"]);
    assert_eq!(one("\"styles\": ['.x{}']"), ["['.x{}']"]);
    assert_eq!(one("'template': '<p/>'"), ["'<p/>'"]);
    assert_eq!(one("style\\u0073: ['.x{}']"), ["['.x{}']"]);
    assert_eq!(one("style\\u{73}: ['.x{}']"), ["['.x{}']"]);
    assert_eq!(one("\\u0073tyles: ['.x{}']"), ["['.x{}']"]);
    assert_eq!(one("templat\\u0065: '<p/>'"), ["'<p/>'"]);
    assert_eq!(one("'style\\u0073': ['.x{}']"), ["['.x{}']"]);
    assert_eq!(one("style\\u0073Extra: ['.x{}']"), NONE);
    assert_eq!(one("style\\u0055rls: ['./a.css']"), NONE);
    assert_eq!(one("templat\\u0065Url: './a.html'"), NONE);
    // Cyrillic `е` in place of `e`: another name.
    assert_eq!(one("stylеs: ['.x{}']"), NONE);
    assert_eq!(one("sel\\u0065ctor2: 'a', styles: ['.x{}']"), ["['.x{}']"]);
    // Malformed escapes are syntax errors, so nothing is excluded.
    assert_eq!(one("style\\u00ZZs: ['.x{}']"), NONE);
    assert_eq!(one("style\\x73: ['.x{}']"), NONE);
}

#[test]
fn skips_keys_without_a_literal_value() {
    assert_eq!(one("styles: STYLES"), NONE);
    assert_eq!(one("template: '<p/>' && TEMPLATE, styles"), NONE);
    assert_eq!(one("style\\u0073"), NONE);
    assert_eq!(one("styles() { return ['.x{}'] }"), NONE);
    assert_eq!(one("get template() { return '<p/>' }"), NONE);
    // A computed key is not read, even a literal one.
    assert_eq!(one("[K]: ['.x{}']"), NONE);
    assert_eq!(one("['styles']: ['.x{}']"), NONE);
    assert_eq!(one("styles: css`.x{}`"), NONE);
    assert_eq!(one("styles: ('.x{}')"), NONE);
}

#[test]
fn reads_the_field_past_other_properties() {
    for field in [
        "[K]: 1, styles: ['.x{}']",
        "...BASE, styles: ['.x{}']",
        "selector2, styles: ['.x{}']",
        "selector2: styles, styles: ['.x{}']",
        "foo() { return 1 }, styles: ['.x{}']",
        "set styles(v) {}, styles: ['.x{}']",
        "styles: ['.x{}'],",
    ] {
        assert_eq!(one(field), ["['.x{}']"], "{field}");
    }
}

/// A value that only STARTS with a literal does not denote it. Excluding
/// that leading piece would keep the rest of the expression outside the
/// excluded range, and an edit to it would read as a template/styles edit.
#[test]
fn skips_a_literal_that_starts_a_larger_expression() {
    for field in [
        "styles: '.a{}' + EXTRA",
        "template: '<p/>' + '<q/>'",
        "template: '<p/>' + SUFFIX",
        "styles: ['.a{}'] + EXTRA",
        "styles: '.a{}'.replace('a', 'b')",
        "styles: ['.a{}'].concat(MORE)",
        "template: '<p/>'.replace('a', 'b')",
        "styles: '.a{}' as string",
        "styles: ['.a{}'] as const",
        "template: '<p/>' as string",
        "styles: '.a{}'!",
        "styles: '.a{}' satisfies string",
        "styles: '.a{}' /* why */ + EXTRA",
        "styles: cond ? '.a{}' : '.b{}'",
    ] {
        assert_eq!(one(field), NONE, "{field}");
    }
    for (field, expected) in [
        ("styles: ['.a{}']", "['.a{}']"),
        ("styles: ['.a{}'],", "['.a{}']"),
        ("styles: '.a{}' /* why */", "'.a{}'"),
        ("styles: '.a{}' // why\n", "'.a{}'"),
        ("template: '<p/>'", "'<p/>'"),
        ("template: '<p/>',", "'<p/>'"),
    ] {
        assert_eq!(one(field), [expected], "{field}");
    }
}

// -----------------------------------------------------------------
// Comments, strings and other tokens
// -----------------------------------------------------------------

#[test]
fn reads_past_comments_in_the_decorator() {
    for body in [
        "@Component({\n  // I'm setting the styles below\n  styles: ['real']\n})\nclass Foo {}",
        "@Component({\n  /* It's important: don't use these */\n  styles: ['real']\n})\nclass Foo {}",
        "@Component({\n  // styles: ['fake'],\n  styles: ['real']\n})\nclass Foo {}",
        "@Component({\n  /* styles: ['fake'] */\n  styles: ['real']\n})\nclass Foo {}",
        "@Component({\n  /*\n   * styles: ['fake-line-1']\n   * styles: ['fake-line-2']\n   */\n  styles: ['real']\n})\nclass Foo {}",
        "@Component({\r\n  // a comment with an apostrophe: I'm here\r\n  styles: ['real']\r\n})\r\nclass Foo {}",
    ] {
        assert_eq!(fields(body, &["Foo"]), ["['real']"], "{body}");
    }
    assert_eq!(
        fields(
            "@Component({\n  /* template: '<fake/>' */\n  template: '<real/>'\n})\nclass Foo {}",
            &["Foo"]
        ),
        ["'<real/>'"]
    );
    assert_eq!(
        fields(
            "@Component({\n  // styles: ['fake']\n  selector: 'app-foo'\n})\nclass Foo {}",
            &["Foo"]
        ),
        NONE
    );
    assert_eq!(
        components(
            "@Component({ styles: ['x'] })\n// I'm decorating this class\nexport class Foo {}",
            &["Foo"]
        ),
        ["Foo"]
    );
}

#[test]
fn reads_past_comments_within_a_field() {
    for field in [
        "styles /* why */: ['.x{}']",
        "styles // why\n: ['.x{}']",
        "styles: /* why */ ['.x{}']",
        "/* why */ styles: ['.x{}']",
        "styles /* a */ /* b */: ['.x{}']",
        "'styles' /* why */: ['.x{}']",
        "styles /* a: b, c ] [ */: ['.x{}']",
        "styles /* styles: './fake.css' */: ['.x{}']",
        "styles /* don't */: ['.x{}']",
    ] {
        assert_eq!(one(field), ["['.x{}']"], "{field}");
    }
    assert_eq!(one("template /* why */: '<p/>'"), ["'<p/>'"]);
    assert_eq!(one("template: /* why */ '<p/>'"), ["'<p/>'"]);
    // The whole array is excluded, comments inside it included.
    for field in [
        "styles: [/* why */ '.x{}']",
        "styles: [// why\n '.x{}']",
        "styles: ['.x{}' /* why */, '.y{}']",
        "styles: ['.x{}' /* why */]",
        "styles: ['.x{}' /* ] */]",
        "styles: [/* why */]",
    ] {
        assert_eq!(one(field), [&field["styles: ".len()..]], "{field}");
    }
}

#[test]
fn reads_strings_holding_delimiters_and_comment_markers() {
    assert_eq!(one("template: 'http://x', styles: ['real']"), ["'http://x'", "['real']"]);
    assert_eq!(
        one("template: '/* not a comment */', styles: ['real']"),
        ["'/* not a comment */'", "['real']"]
    );
    assert_eq!(
        one("template: 'styles: [\"fake\"]', styles: ['real']"),
        ["'styles: [\"fake\"]'", "['real']"]
    );
    assert_eq!(
        one("template: 'has { and ] literally', styles: ['real']"),
        ["'has { and ] literally'", "['real']"]
    );
    assert_eq!(one("selector2: 'foo(bar)', styles: ['real']"), ["['real']"]);
    assert_eq!(one("template: `cost $5 or $${price}`"), ["`cost $5 or $${price}`"]);
}

#[test]
fn reads_strings_with_escapes() {
    assert_eq!(one(r"template: 'it\'s'"), [r"'it\'s'"]);
    assert_eq!(one(r#"template: "say \"hi\"""#), [r#""say \"hi\"""#]);
    assert_eq!(one(r"template: `a \` b ${'`'} c`"), [r"`a \` b ${'`'} c`"]);
    assert_eq!(
        one(r"styles: ['.a::before { content: \'\\\'\' }']"),
        [r"['.a::before { content: \'\\\'\' }']"]
    );
    assert_eq!(one("template: 'a\\\nb'"), ["'a\\\nb'"]);
}

#[test]
fn reads_template_literals_with_nested_braces() {
    assert_eq!(
        one("template: `${ { a: { b: '}' } }.a.b }`, styles: ['.x{}']"),
        ["`${ { a: { b: '}' } }.a.b }`", "['.x{}']"]
    );
    assert_eq!(one("template: `${`inner ${`deep`}`}`"), ["`${`inner ${`deep`}`}`"]);
}

/// Regex literals, which the JS lexer could not tell from division: a `/`
/// inside one read as a comment or a quote as a string opener, so the
/// fields after it were never found (the page reloaded on every edit).
#[test]
fn reads_past_regex_literals() {
    assert_eq!(
        one("providers: [{ provide: P, useValue: /['\"`)}\\]]/ }], styles: ['.x{}']"),
        ["['.x{}']"]
    );
    assert_eq!(
        one("providers: [{ provide: P, useValue: /\\/\\/*/g }], template: '<p/>'"),
        ["'<p/>'"]
    );
    assert_eq!(
        one(
            "template: '<p/>', providers: [{ provide: P, useFactory: () => 1 / 2 / 3 }], styles: ['.x{}']"
        ),
        ["'<p/>'", "['.x{}']"]
    );
}

#[test]
fn ignores_decoys_elsewhere_in_the_source() {
    for body in [
        "/**\n * Usage example:\n *   @Component({ template: 'fake' })\n *   class Example {}\n */\n@Component({ template: '<real/>' })\nexport class Foo {}",
        "const docs = \"use @Component({ template: 'fake' }) to declare\"\n@Component({ template: '<real/>' })\nclass Foo {}",
        "`Use @Component({ template: 'fake' })`\n@Component({ template: '<real/>' })\nclass Foo {}",
        "const base = { selector: 'app' }\n@Component({ ...base, template: '<real/>' })\nclass Foo {}",
        "@Component({ template: '<real/>' })\nclass Foo {\n  Component(x: number) { return x; }\n  styles = ['.not-metadata{}'];\n}",
    ] {
        assert_eq!(fields(body, &["Foo"]), ["'<real/>'"], "{body}");
    }
    let body = "@Component({ template: '<a/>' })
class First {}
const helper = () => '@Component({...})'
@Component({ template: '<b/>' })
class Second {}";
    assert_eq!(components(body, &["First", "Second"]), ["First", "Second"]);
    assert_eq!(fields(body, &["First", "Second"]), ["'<a/>'", "'<b/>'"]);
}

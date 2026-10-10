//! A template that fails to parse is reported from `transform_angular_file` with
//! Angular's message and with a position: a label when the template text is part of the
//! file being transformed, and `file:line:column` in the help text for inline templates
//! and `templateUrl` files alike.

use std::collections::HashMap;

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    ResolvedResources, TransformOptions, compile_template_to_js, transform_angular_file,
};
use oxc_diagnostics::OxcDiagnostic;

const INCOMPLETE_IF: &str = "Incomplete block \"if\". If you meant to write the @ character, \
                             you should use the \"&#64;\" HTML entity instead.";

fn transform(source: &str, resources: Option<&ResolvedResources>) -> Vec<OxcDiagnostic> {
    let allocator = Allocator::default();
    let options = TransformOptions::default();
    transform_angular_file(&allocator, "/x/cut.ts", source, Some(&options), resources).diagnostics
}

/// The text each label covers in `source`.
fn labelled<'s>(diagnostic: &OxcDiagnostic, source: &'s str) -> Vec<&'s str> {
    diagnostic
        .labels
        .iter()
        .map(|label| {
            let start = label.offset() as usize;
            &source[start..start + label.len() as usize]
        })
        .collect()
}

/// The reported template: the `@if` parameters never close.
#[test]
fn incomplete_block_in_an_inline_template() {
    let source = "import { Component } from '@angular/core';

@Component({
  selector: 'x-cut',
  template: `
    <text>before</text>
    @if (on() {
      <text>inside</text>
    }
    <text>after</text>
  `,
})
export class Cut {
  on() { return true; }
}
";
    let diagnostics = transform(source, None);
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(diagnostic.message, INCOMPLETE_IF);
    // Angular's span runs from the `@if` to the end of what the unclosed `(` swallowed.
    assert_eq!(
        labelled(diagnostic, source),
        ["@if (on() {\n      <text>inside</text>\n    }\n    <text>after</text>\n  "]
    );
    assert_eq!(diagnostic.help.as_deref(), Some("/x/cut.ts:7:5"));
}

#[test]
fn incomplete_block_in_a_single_quoted_inline_template() {
    let source = "import { Component } from '@angular/core';
@Component({ selector: 'x-cut', template: '<p>a</p>@if (cond) text' })
export class Cut {}
";
    let diagnostics = transform(source, None);
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(diagnostic.message, INCOMPLETE_IF);
    assert_eq!(labelled(diagnostic, source), ["@if (cond) "]);
    assert_eq!(diagnostic.help.as_deref(), Some("/x/cut.ts:2:52"));
}

/// An escape makes the template text differ from the source text, so offsets in one do
/// not map to the other: no label, and a position within the template instead.
#[test]
fn incomplete_block_in_an_inline_template_written_with_escapes() {
    let source = "import { Component } from '@angular/core';
@Component({ selector: 'x-cut', template: '<p>a</p>\\n@if (cond) text' })
export class Cut {}
";
    let diagnostics = transform(source, None);
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(diagnostic.message, INCOMPLETE_IF);
    assert!(labelled(diagnostic, source).is_empty());
    assert_eq!(diagnostic.help.as_deref(), Some("line 2, column 1 of the template"));
}

#[test]
fn incomplete_block_in_a_template_url_file() {
    let source = "import { Component } from '@angular/core';
@Component({ selector: 'x-cut', templateUrl: './cut.html' })
export class Cut {}
";
    let mut templates = HashMap::new();
    templates.insert(
        "./cut.html".to_string(),
        "<text>before</text>\n  @if (on() {\n<text>after</text>\n".to_string(),
    );
    let resources = ResolvedResources { templates, styles: HashMap::new() };
    let diagnostics = transform(source, Some(&resources));
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(diagnostic.message, INCOMPLETE_IF);
    // The offsets belong to the HTML file, so there is no label on the component file.
    assert!(labelled(diagnostic, source).is_empty());
    assert_eq!(diagnostic.help.as_deref(), Some("./cut.html:2:3"));
}

/// `templateUrl` wins even when the inline `template` holds identical text:
/// the error belongs to the external file, not the inline literal.
#[test]
fn template_url_wins_when_inline_template_matches() {
    let template = "<p>a</p>@if (cond) text";
    let source = "import { Component } from '@angular/core';
@Component({
  selector: 'x-cut',
  template: '<p>a</p>@if (cond) text',
  templateUrl: './cut.html',
})
export class Cut {}
";
    let mut templates = HashMap::new();
    templates.insert("./cut.html".to_string(), template.to_string());
    let resources = ResolvedResources { templates, styles: HashMap::new() };
    let diagnostics = transform(source, Some(&resources));
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(diagnostic.message, INCOMPLETE_IF);
    assert!(labelled(diagnostic, source).is_empty());
    assert_eq!(diagnostic.help.as_deref(), Some("./cut.html:1:9"));
}

/// The template-only entry point renders diagnostics against the template itself.
#[test]
fn incomplete_block_through_compile_template_to_js() {
    let template = "<p>a</p>\n@for (item of items; track item {";
    let allocator = Allocator::default();
    let diagnostics = compile_template_to_js(&allocator, template, "Cut", "/x/cut.html")
        .err()
        .expect("the template should not compile");
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(
        diagnostic.message,
        "Incomplete block \"for\". If you meant to write the @ character, \
         you should use the \"&#64;\" HTML entity instead."
    );
    assert_eq!(labelled(diagnostic, template), ["@for (item of items; track item {"]);
    assert_eq!(diagnostic.help.as_deref(), Some("/x/cut.html:2:1"));
}

/// A lexer error is reported at the byte offset of the offending character, so
/// the label lands on it and the help text has the real `line:column`. The
/// location used to read the `(line, column)` pair as the byte offset, which
/// pointed the label at the start of the file.
#[test]
fn lexer_error_in_an_inline_template_points_at_the_character() {
    let source = "import { Component } from '@angular/core';

@Component({
  selector: 'x-cut',
  template: `
    <p>before</p>
    <!x
    <p>after</p>
  `,
})
export class Cut {}
";
    let diagnostics = transform(source, None);
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic, got {diagnostics:?}");
    };
    assert_eq!(diagnostic.message, "Unexpected character \"x\"");
    // Zero-length label at the `x`.
    let [label] = diagnostic.labels.as_slice() else {
        panic!("expected one label, got {diagnostic:?}");
    };
    assert_eq!(label.offset() as usize, source.find("<!x").unwrap() + 2);
    assert_eq!(label.len(), 0);
    assert_eq!(diagnostic.help.as_deref(), Some("/x/cut.ts:7:7"));
}

#[test]
fn literal_at_sign_and_well_formed_blocks_still_compile() {
    let source = "import { Component } from '@angular/core';
@Component({
  selector: 'x-ok',
  template: `<p>user@example.com</p>@if (on()) { <i>a</i> } @else { <b>b</b> }`,
})
export class Ok { on() { return true; } }
";
    assert!(transform(source, None).is_empty());
}

//! Angular's `enableTemplateSourceLocations` option: each view's create
//! block ends with `ɵɵattachSourceLocations(path, [[slot, offset, line,
//! column], ...])`, and the `rootDirs` option makes paths project-relative.
//!
//! Every expected string below is ngtsc 22.2.3 full-mode output for the same
//! file (`enableTemplateSourceLocations: true`, `rootDirs` set to the file's
//! directory), with whitespace removed. Ported from Angular's
//! `attach_source_locations.ts` phase and compiler-cli's
//! `getProjectRelativePath`.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    AngularVersion, CompilationMode, ResolvedResources, TransformOptions, transform_angular_file,
};

const ROOT: &str = "/w";

fn options() -> TransformOptions {
    TransformOptions {
        enable_template_source_locations: true,
        root_dirs: Some(vec![ROOT.to_string()]),
        ..Default::default()
    }
}

fn compile_with(
    file: &str,
    source: &str,
    options: &TransformOptions,
    resources: Option<&ResolvedResources>,
) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, file, source, Some(options), resources);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile(file: &str, source: &str) -> String {
    compile_with(file, source, &options(), None)
}

/// Each `ɵɵattachSourceLocations(...)` and `ɵsetClassDebugInfo(...)` call, in order.
fn calls(code: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(start) =
        ["attachSourceLocations(", "ɵsetClassDebugInfo("].iter().filter_map(|p| rest.find(p)).min()
    {
        // `ɵsetClassDebugInfo(Type, {...})` ends at its object, the other at `);`.
        let (close, keep) = if rest[start..].starts_with("attach") { (");", 1) } else { ("})", 2) };
        let end = start + rest[start..].find(close).unwrap() + keep;
        found.push(&rest[start..end]);
        rest = &rest[end..];
    }
    found
}

const VIEWS: &str = r#"import { Component } from '@angular/core';

@Component({
  selector: 'app-views',
  template: `<div class="x" #ref>
  <span>{{ name }}</span><input (input)="name = $any($event.target).value"/>
  @if (name) { <b>hi</b><i>x</i> } @else { <em>no</em> }
  @for (item of items; track item) { <li>{{ item }}</li> } @empty { <p>empty</p> }
  <ng-container><u>c</u></ng-container>
  <ng-template #t><s>t</s></ng-template>
  @switch (name) { @case ('a') { <a>a</a> } @default { <small>d</small> } }
  @defer { <strong>later</strong> } @placeholder { <code>p</code> }
  <svg><circle r="1"></circle></svg>
  <br>
</div>
@let x = 1;
<p>{{ x }}</p>`,
})
export class ViewsComponent {
  name = 'a';
  items = [1, 2];
}

@Component({ selector: 'app-two', template: '<h1>two</h1>' })
export class TwoComponent {}

@Component({ selector: 'app-text', template: 'only text {{ 1 }}' })
export class TextComponent {}
"#;

#[test]
fn one_call_per_view_with_slots_offsets_lines_and_columns() {
    let code = compile("/w/views.component.ts", VIEWS);
    assert_eq!(
        calls(&code),
        [
            r#"attachSourceLocations("views.component.ts",[[0,208,6,15],[2,217,6,24]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,236,6,43]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,287,7,37]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,318,7,68]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,391,9,18]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,447,10,33]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,469,10,55]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,501,11,11]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,541,11,51]])"#,
            r#"attachSourceLocations("views.component.ts",[[0,95,4,13],[2,118,5,2],[4,141,5,25],[11,349,8,16],[21,560,12,2],[22,565,12,7],[23,597,13,2],[24,621,16,0]])"#,
            r#"ɵsetClassDebugInfo(ViewsComponent,{className:"ViewsComponent",filePath:"views.component.ts",lineNumber:19})"#,
            r#"attachSourceLocations("views.component.ts",[[0,751,23,45]])"#,
            r#"ɵsetClassDebugInfo(TwoComponent,{className:"TwoComponent",filePath:"views.component.ts",lineNumber:25})"#,
            r#"ɵsetClassDebugInfo(TextComponent,{className:"TextComponent",filePath:"views.component.ts",lineNumber:28})"#,
        ]
    );
}

#[test]
fn the_call_ends_the_create_block_unchained() {
    let code = compile("/w/views.component.ts", VIEWS);
    // ngtsc: `ɵɵdomElementEnd(); ɵɵattachSourceLocations(...); } if (rf & 2) {`
    // (oxc writes the test as `(rf & 2)`, as for any template).
    assert!(
        code.contains(
            r#"i0.ɵɵdomElementEnd();i0.ɵɵattachSourceLocations("views.component.ts",[[0,95,4,13],"#
        ),
        "{code}"
    );
    assert!(code.contains("[24,621,16,0]]);}if("), "{code}");
    assert!(code.contains("[[0,751,23,45]]);}},"), "{code}");
}

#[test]
fn escaped_literals_count_in_the_raw_source() {
    // Offsets are UTF-16 units of the raw `.ts` text: `\n` does not start a
    // line, and an escaped `<` (`\x3C`) sits at its backslash.
    let source = r#"import { Component } from '@angular/core';
// héllo 😀
@Component({ selector: 'app-esc', template: 'é😀<i>x</i>\n<p>\ty</p>\x3Cb>z</b>\u00e9<q>\u{1F600}</q>\
<del>cont</del>' })
export class EscComponent {}

@Component({ selector: 'app-esc2', template: "<a>\"q\"</a>\r\n<b>\'</b>" })
export class Esc2Component {}

@Component({ selector: 'app-esc3', template: `<abbr>\`</abbr>\n  <cite>😀\u{41}</cite>` })
export class Esc3Component {}
"#;
    assert_eq!(
        calls(&compile("/w/esc.component.ts", source)),
        [
            r#"attachSourceLocations("esc.component.ts",[[1,103,2,48],[3,113,2,58],[5,123,2,68],[8,140,2,85],[10,158,3,0]])"#,
            r#"ɵsetClassDebugInfo(EscComponent,{className:"EscComponent",filePath:"esc.component.ts",lineNumber:5})"#,
            r#"attachSourceLocations("esc.component.ts",[[0,254,6,46],[2,270,6,62]])"#,
            r#"ɵsetClassDebugInfo(Esc2Component,{className:"Esc2Component",filePath:"esc.component.ts",lineNumber:8})"#,
            r#"attachSourceLocations("esc.component.ts",[[0,361,9,46],[2,380,9,65]])"#,
            r#"ɵsetClassDebugInfo(Esc3Component,{className:"Esc3Component",filePath:"esc.component.ts",lineNumber:11})"#,
        ]
    );
}

#[test]
fn crlf_source() {
    let source = "import { Component } from '@angular/core';\r\n\r\n@Component({\r\n  selector: 'app-crlf',\r\n  template: `<div>\r\n  <span>a</span>\r\n</div>`,\r\n})\r\nexport class CrlfComponent {}\r\n";
    assert_eq!(
        calls(&compile("/w/crlf.component.ts", source)),
        [
            r#"attachSourceLocations("crlf.component.ts",[[0,98,4,13],[1,107,5,2]])"#,
            r#"ɵsetClassDebugInfo(CrlfComponent,{className:"CrlfComponent",filePath:"crlf.component.ts",lineNumber:9})"#,
        ]
    );
}

#[test]
fn template_url_counts_in_the_html_file() {
    // `\r` moves no column, as in Angular's lexer cursor.
    let source = r"import { Component } from '@angular/core';

@Component({ selector: 'app-url', templateUrl: './tpl/url.component.html' })
export class UrlComponent {}

@Component({ selector: 'app-url2', templateUrl: 'tpl/two.html' })
export class Url2Component {}
";
    let mut resources = ResolvedResources::default();
    resources.templates.insert(
        "./tpl/url.component.html".to_string(),
        "<div>\r\n  <span>é😀</span>\r\n</div>\n<p>\r<b>x</b></p>\n".to_string(),
    );
    resources
        .templates
        .insert("tpl/two.html".to_string(), "<h2>bom</h2>\n  <h3>x</h3>".to_string());
    assert_eq!(
        calls(&compile_with("/w/url.component.ts", source, &options(), Some(&resources))),
        [
            r#"attachSourceLocations("tpl/url.component.html",[[0,0,0,0],[1,9,1,2],[3,34,3,0],[4,38,3,3]])"#,
            r#"ɵsetClassDebugInfo(UrlComponent,{className:"UrlComponent",filePath:"url.component.ts",lineNumber:4})"#,
            r#"attachSourceLocations("tpl/two.html",[[0,0,0,0],[2,15,1,2]])"#,
            r#"ɵsetClassDebugInfo(Url2Component,{className:"Url2Component",filePath:"url.component.ts",lineNumber:7})"#,
        ]
    );
}

#[test]
fn template_url_of_a_bare_file_name_is_relative_to_the_working_directory() {
    let source = "import { Component } from '@angular/core';
@Component({ selector: 'app-bare', templateUrl: './bare.component.html' })
export class BareComponent {}
";
    let mut resources = ResolvedResources::default();
    resources.templates.insert("./bare.component.html".to_string(), "<p>x</p>".to_string());
    let options = TransformOptions { root_dirs: Some(vec![".".to_string()]), ..options() };
    let code = compile_with("bare.component.ts", source, &options, Some(&resources));
    assert_eq!(calls(&code)[0], r#"attachSourceLocations("bare.component.html",[[0,0,0,0]])"#);
}

#[test]
fn a_template_not_written_as_one_literal_counts_from_its_start() {
    let source = r"import { Component } from '@angular/core';

const TPL = `
  <div><span>x</span></div>`;

@Component({ selector: 'app-ind', template: TPL })
export class IndComponent {}

@Component({ selector: 'app-cat', template: '<a>a</a>' + '\n<b>b</b>' })
export class CatComponent {}

@Component({ selector: 'app-paren', template: ('<p>p</p>') })
export class ParenComponent {}
";
    assert_eq!(
        calls(&compile("/w/ind.component.ts", source)),
        [
            r#"attachSourceLocations("ind.component.ts",[[0,3,1,2],[1,8,1,7]])"#,
            r#"ɵsetClassDebugInfo(IndComponent,{className:"IndComponent",filePath:"ind.component.ts",lineNumber:7})"#,
            r#"attachSourceLocations("ind.component.ts",[[0,0,0,0],[2,9,1,0]])"#,
            r#"ɵsetClassDebugInfo(CatComponent,{className:"CatComponent",filePath:"ind.component.ts",lineNumber:10})"#,
            r#"attachSourceLocations("ind.component.ts",[[0,0,0,0]])"#,
            r#"ɵsetClassDebugInfo(ParenComponent,{className:"ParenComponent",filePath:"ind.component.ts",lineNumber:13})"#,
        ]
    );
}

#[test]
fn i18n_elements() {
    let source = r#"import { Component } from '@angular/core';

@Component({
  selector: 'app-i18n',
  template: `<div i18n>Hello <b>world</b> and <i>{{ n }}</i></div>
<p i18n-title title="t">x</p>
<span i18n>{n, plural, =1 {<em>one</em>} other {many}}</span>`,
})
export class I18nComponent { n = 1; }
"#;
    assert_eq!(
        calls(&compile("/w/i18n.component.ts", source)),
        [
            r#"attachSourceLocations("i18n.component.ts",[[0,94,4,13],[2,110,4,29],[3,127,4,46],[4,148,5,0],[6,178,6,0]])"#,
            r#"ɵsetClassDebugInfo(I18nComponent,{className:"I18nComponent",filePath:"i18n.component.ts",lineNumber:9})"#,
        ]
    );
}

const IN_SRC: &str = "import { Component } from '@angular/core';

@Component({ selector: 'app-in', template: '<div>in</div>' })
export class InComponent {}
";

const IN_LIB: &str = "import { Component } from '@angular/core';

@Component({ selector: 'app-out', template: '<div>out</div>' })
export class OutComponent {}
";

#[test]
fn paths_are_relative_to_the_first_root_that_holds_the_file() {
    let roots = |dirs: &[&str]| TransformOptions {
        root_dirs: Some(dirs.iter().map(ToString::to_string).collect()),
        ..options()
    };
    // rootDirs: [src]. A file outside every root gets no source locations and
    // no `filePath`.
    let options = roots(&["/w/src"]);
    assert_eq!(
        calls(&compile_with("/w/src/in.component.ts", IN_SRC, &options, None)),
        [
            r#"attachSourceLocations("in.component.ts",[[0,88,2,44]])"#,
            r#"ɵsetClassDebugInfo(InComponent,{className:"InComponent",filePath:"in.component.ts",lineNumber:4})"#,
        ]
    );
    assert_eq!(
        calls(&compile_with("/w/lib/out.component.ts", IN_LIB, &options, None)),
        [r#"ɵsetClassDebugInfo(OutComponent,{className:"OutComponent"})"#]
    );
    // rootDirs: [lib, src].
    let options = roots(&["/w/lib", "/w/src"]);
    assert_eq!(
        calls(&compile_with("/w/lib/out.component.ts", IN_LIB, &options, None)),
        [
            r#"attachSourceLocations("out.component.ts",[[0,89,2,45]])"#,
            r#"ɵsetClassDebugInfo(OutComponent,{className:"OutComponent",filePath:"out.component.ts",lineNumber:4})"#,
        ]
    );
    // A root above the file keeps the directories in between.
    assert!(
        compile_with("/w/src/in.component.ts", IN_SRC, &roots(&["/w"]), None)
            .contains(r#"attachSourceLocations("src/in.component.ts","#)
    );
}

#[test]
fn off_by_default_and_without_a_root() {
    let default = TransformOptions::default();
    let code = compile_with("/w/views.component.ts", VIEWS, &default, None);
    assert!(!code.contains("attachSourceLocations"), "{code}");
    // With no root dirs, `filePath` stays the path given to the transform.
    assert!(code.contains(r#"filePath:"/w/views.component.ts""#), "{code}");

    // The option alone, with no root to make the path relative, emits nothing
    // and changes nothing.
    let no_root = TransformOptions { enable_template_source_locations: true, ..Default::default() };
    assert_eq!(compile_with("/w/views.component.ts", VIEWS, &no_root, None), code);

    // A root alone only changes `filePath`.
    let root_only =
        TransformOptions { root_dirs: Some(vec![ROOT.to_string()]), ..Default::default() };
    assert_eq!(
        compile_with("/w/views.component.ts", VIEWS, &root_only, None),
        code.replace(r#"filePath:"/w/views.component.ts""#, r#"filePath:"views.component.ts""#)
    );
}

#[test]
fn partial_mode_emits_none() {
    // The linker hard-codes `enableTemplateSourceLocations: false`.
    let options = TransformOptions { compilation_mode: CompilationMode::Partial, ..options() };
    let code = compile_with("/w/views.component.ts", VIEWS, &options, None);
    assert!(!code.contains("attachSourceLocations"), "{code}");
}

#[test]
fn needs_angular_19_1() {
    let with_version = |version| TransformOptions { angular_version: Some(version), ..options() };
    let old = compile_with(
        "/w/views.component.ts",
        VIEWS,
        &with_version(AngularVersion::new(19, 0, 5)),
        None,
    );
    assert!(!old.contains("attachSourceLocations"), "{old}");
    let new = compile_with(
        "/w/views.component.ts",
        VIEWS,
        &with_version(AngularVersion::new(19, 1, 0)),
        None,
    );
    assert!(new.contains("attachSourceLocations"), "{new}");
}

//! `extract_component_resource_urls` reads `templateUrl` / `styleUrl(s)` the
//! way the transform does, and the transform reports the stylesheet `<link>`s
//! of a template it was not given (`TransformResult::missing_link_urls`), so a
//! build tool can hand the compiler every resource ngtsc would load.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    ComponentResourceUrls, ResolvedResources, TransformOptions, TransformResult,
    compile_template_for_hmr, extract_component_resource_urls, transform_angular_file,
};

fn extract(path: &str, source: &str, options: &TransformOptions) -> ComponentResourceUrls {
    let allocator = Allocator::default();
    extract_component_resource_urls(&allocator, path, source, Some(options))
}

fn transform(
    path: &str,
    source: &str,
    options: &TransformOptions,
    resources: &ResolvedResources,
) -> TransformResult {
    let allocator = Allocator::default();
    transform_angular_file(&allocator, path, source, Some(options), Some(resources))
}

fn resources(templates: &[(&str, &str)], styles: &[(&str, &str)]) -> ResolvedResources {
    ResolvedResources {
        templates: templates.iter().map(|&(k, v)| (k.to_owned(), v.to_owned())).collect(),
        styles: styles.iter().map(|&(k, v)| (k.to_owned(), vec![v.to_owned()])).collect(),
    }
}

fn error_messages(result: &TransformResult) -> String {
    result.diagnostics.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The `styles:[...]` of the `ɵɵdefineComponent` call, whitespace removed.
fn defined_styles(code: &str) -> String {
    let compact = compact(code);
    let start = compact.find("styles:[").expect("styles in ɵcmp");
    let end = compact[start..].find("\"]").map_or(compact.len(), |i| start + i + 2);
    compact[start..end].to_string()
}

#[test]
fn reads_literal_and_same_file_const_urls() {
    let urls = extract(
        "/app/a.component.ts",
        r"
import { Component } from '@angular/core';
const DIR = './a';
const URLS = [`${DIR}.component.css`, './shared.css'];
@Component({ selector: 'a-cmp', templateUrl: `${DIR}.component.html`, styleUrls: URLS })
export class A {}
@Component({ selector: 'b-cmp', template: '', styleUrl: DIR + '.b.css' })
export class B {}
@Component({ selector: 'c-cmp', template: '', styleUrls: ['./c.css', ...URLS] })
export class C {}
",
        &TransformOptions::default(),
    );
    assert_eq!(urls.template_urls, vec!["./a.component.html"]);
    assert_eq!(
        urls.style_urls,
        vec![
            "./a.component.css",
            "./shared.css",
            "./a.b.css",
            "./c.css",
            "./a.component.css",
            "./shared.css",
        ]
    );
}

#[test]
fn skips_values_that_are_not_strings() {
    let urls = extract(
        "/app/a.component.ts",
        r"
import { Component } from '@angular/core';
declare const dynamic: string;
@Component({ selector: 'a-cmp', templateUrl: dynamic, styleUrls: ['./a.css', 1, dynamic] })
export class A {}
",
        &TransformOptions::default(),
    );
    assert_eq!(urls.template_urls, Vec::<String>::new());
    assert_eq!(urls.style_urls, vec!["./a.css"]);
}

/// The transform looks the URLs up under the strings the extractor returns.
#[test]
fn transform_finds_the_resources_under_the_extracted_urls() {
    let source = r"
import { Component } from '@angular/core';
const BASE = './a.component';
@Component({ selector: 'a-cmp', templateUrl: BASE + '.html', styleUrls: [...[BASE + '.css']] })
export class A {}
";
    let options = TransformOptions::default();
    let urls = extract("/app/a.component.ts", source, &options);
    assert_eq!(urls.template_urls, vec!["./a.component.html"]);
    assert_eq!(urls.style_urls, vec!["./a.component.css"]);
    let result = transform(
        "/app/a.component.ts",
        source,
        &options,
        &resources(
            &[("./a.component.html", "<p>TEMPLATE_MARKER</p>")],
            &[("./a.component.css", ".STYLE_MARKER { color: red; }")],
        ),
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert!(result.code.contains("TEMPLATE_MARKER"), "{}", result.code);
    assert!(result.code.contains("STYLE_MARKER"), "{}", result.code);
}

/// ngtsc loads a template's `<link rel="stylesheet">` like a `styleUrl`,
/// relative to the component file, and compiles it after the `styleUrls`.
#[test]
fn reports_and_compiles_template_stylesheet_links() {
    let source = r#"
import { Component } from '@angular/core';
@Component({
  selector: 'a-cmp',
  templateUrl: './sub/a.html',
  styleUrl: './a.css',
})
export class A {}
@Component({
  selector: 'b-cmp',
  template: '<link rel="stylesheet" href="./link.css"><link rel="stylesheet" href="https://example.com/x.css"><p>b</p>',
})
export class B {}
"#;
    let template = r#"<link rel="stylesheet" href="./link.css"><p>a</p>"#;
    let options = TransformOptions::default();

    let urls = extract("/app/a.component.ts", source, &options);
    assert_eq!(urls.template_urls, vec!["./sub/a.html"]);
    assert_eq!(urls.style_urls, vec!["./a.css"]);

    // Without the link: one COMPONENT_RESOURCE_NOT_FOUND per component, and
    // the hrefs, as written, in `missing_link_urls`. A URL with a scheme is not
    // a stylesheet the compiler loads.
    let without = resources(&[("./sub/a.html", template)], &[("./a.css", ".a { color: red; }")]);
    let result = transform("/app/a.component.ts", source, &options, &without);
    assert_eq!(result.missing_link_urls, vec!["./link.css", "./link.css"]);
    assert_eq!(result.diagnostics.len(), 2, "{}", error_messages(&result));
    assert!(error_messages(&result).contains("COMPONENT_RESOURCE_NOT_FOUND"));
    assert!(result.dependencies.contains(&"./link.css".to_string()), "{:?}", result.dependencies);

    // With it: compiled after the decorator's `styleUrl`, as ngtsc does.
    let with = resources(
        &[("./sub/a.html", template)],
        &[("./a.css", ".a { color: red; }"), ("./link.css", ".link { color: blue; }")],
    );
    let result = transform("/app/a.component.ts", source, &options, &with);
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert_eq!(result.missing_link_urls, Vec::<String>::new());
    let scope = "[_ngcontent-%COMP%]";
    assert_eq!(
        defined_styles(&result.code),
        compact(&format!(
            r#"styles: [".a{scope} {{ color: red; }}", ".link{scope} {{ color: blue; }}"]"#
        ))
    );
    let b_start = result.code.find("type:B").or_else(|| result.code.find("type: B")).unwrap();
    assert!(
        defined_styles(&result.code[b_start..])
            .contains(&compact(&format!(".link{scope} {{ color: blue; }}"))),
        "{}",
        result.code
    );
}

/// No resources passed: the compiler cannot tell a missing link from one the
/// caller never meant to load, so nothing is reported.
#[test]
fn links_are_not_reported_without_resources() {
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "/app/a.component.ts",
        r#"
import { Component } from '@angular/core';
@Component({ selector: 'a-cmp', template: '<link rel="stylesheet" href="./link.css">' })
export class A {}
"#,
        None,
        None,
    );
    assert!(!result.has_errors(), "{}", error_messages(&result));
    assert_eq!(result.missing_link_urls, Vec::<String>::new());
}

#[test]
fn hmr_template_compile_lists_stylesheet_links() {
    let allocator = Allocator::default();
    let output = compile_template_for_hmr(
        &allocator,
        r#"<link rel="stylesheet" href="./a.css"><link rel="stylesheet" href="../b.css"><p>x</p>"#,
        "A",
        "/app/a.component.ts",
        &TransformOptions::default(),
    )
    .expect("compiles");
    assert_eq!(output.style_urls, vec!["./a.css", "../b.css"]);
}

#[cfg(feature = "cross_file_elision")]
mod imported {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &std::path::Path, name: &str, content: &str) -> String {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    const SOURCE: &str = r"
import { Component } from '@angular/core';
import { TEMPLATE_URL, STYLE_URLS } from './urls';
@Component({ selector: 'a-cmp', templateUrl: TEMPLATE_URL, styleUrls: STYLE_URLS })
export class A {}
";

    fn setup() -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "app/urls.ts",
            "export const TEMPLATE_URL = './a.component.html';\n\
             export const STYLE_URLS = ['./a.component.css'];\n",
        );
        let path = write(dir.path(), "app/a.component.ts", SOURCE);
        (dir, path)
    }

    /// ngtsc evaluates `templateUrl` / `styleUrls` with the program-wide
    /// checker, so a URL imported from another file loads.
    #[test]
    fn imported_urls_resolve_with_resolve_imported_values() {
        let (_dir, path) = setup();
        let options = TransformOptions { resolve_imported_values: true, ..Default::default() };

        let urls = extract(&path, SOURCE, &options);
        assert_eq!(urls.template_urls, vec!["./a.component.html"]);
        assert_eq!(urls.style_urls, vec!["./a.component.css"]);

        let result = transform(
            &path,
            SOURCE,
            &options,
            &resources(
                &[("./a.component.html", "<p>IMPORTED_TEMPLATE</p>")],
                &[("./a.component.css", ".IMPORTED_STYLE { color: red; }")],
            ),
        );
        assert!(!result.has_errors(), "{}", error_messages(&result));
        assert!(result.code.contains("IMPORTED_TEMPLATE"), "{}", result.code);
        assert!(result.code.contains("IMPORTED_STYLE"), "{}", result.code);
    }

    /// Without the resolver the extractor and the transform agree that the
    /// imported URLs are unknown.
    #[test]
    fn imported_urls_stay_unknown_without_the_resolver() {
        let (_dir, path) = setup();
        let urls = extract(&path, SOURCE, &TransformOptions::default());
        assert_eq!(urls.template_urls, Vec::<String>::new());
        assert_eq!(urls.style_urls, Vec::<String>::new());
    }
}

//! `compile_component_hmr_update` compiles a component's HMR update with the
//! options of the full transform, so the template fields it installs are the
//! ones `transform_angular_file` puts in the ɵcmp.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    ComponentHmrUpdate, ComponentHmrUpdateInput, ResolvedResources, TransformOptions,
    compile_component_hmr_update, compile_template_for_hmr, component_hmr_fingerprint,
    transform_angular_file,
};

const PATH: &str = "/app/src/x.component.ts";

fn component(metadata: &str, template: &str) -> String {
    format!(
        "import {{ Component }} from '@angular/core';\n\
         @Component({{ selector: 'x-cmp', {metadata} template: {template:?} }})\n\
         export class X {{ name = 'x'; }}\n"
    )
}

fn transform(
    source: &str,
    options: &TransformOptions,
    resources: Option<&ResolvedResources>,
) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, PATH, source, Some(options), resources);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.code
}

fn update(
    source: &str,
    options: &TransformOptions,
    resources: Option<&ResolvedResources>,
) -> ComponentHmrUpdate {
    let allocator = Allocator::default();
    let input = ComponentHmrUpdateInput { class_name: "X", ..Default::default() };
    let update =
        compile_component_hmr_update(&allocator, PATH, source, &input, Some(options), resources);
    assert!(update.diagnostics.is_empty(), "{:?}", update.diagnostics);
    update
}

/// `options` with HMR on, as the dev server compiles.
fn hmr(options: TransformOptions) -> TransformOptions {
    TransformOptions { hmr: true, ..options }
}

/// `code` with the whitespace the emitter's line wrapping puts in left out.
/// String literals keep theirs escaped, so nothing a template decides is lost.
fn squash(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The update's template function is the one the full transform emits.
fn assert_template_matches_transform(update: &ComponentHmrUpdate, code: &str) {
    assert!(update.module.is_some());
    assert_ne!(update.template_js, "");
    assert!(
        squash(code).contains(&squash(&update.template_js)),
        "template\n{}\nis not in the transform output\n{code}",
        update.template_js
    );
}

#[test]
fn keeps_whitespace_the_decorator_preserves() {
    let source =
        component("preserveWhitespaces: true,", "<div>\n  <span> {{ name }} </span>\n</div>");
    let options = hmr(TransformOptions::default());
    let update = update(&source, &options, None);
    assert_template_matches_transform(&update, &transform(&source, &options, None));
    assert!(update.template_js.contains(r#"ɵɵtext(1,"\n  ")"#), "{}", update.template_js);
}

#[test]
fn names_i18n_messages_after_the_file() {
    let source = component("", "<p i18n>Hello {{ name }}</p>");
    let options = hmr(TransformOptions { i18n_use_external_ids: false, ..Default::default() });
    let update = update(&source, &options, None);
    let code = transform(&source, &options, None);
    assert_template_matches_transform(&update, &code);
    let consts = update.consts_js();
    assert!(squash(&code).contains(&squash(consts)), "consts\n{consts}\nare not in\n{code}");
    assert!(consts.contains("MSG_"), "{consts}");

    // The template-only compile has no file to name them after.
    let allocator = Allocator::default();
    let old =
        compile_template_for_hmr(&allocator, "<p i18n>Hello {{ name }}</p>", "X", PATH, &options)
            .unwrap();
    assert_ne!(old.consts_js.as_deref(), Some(consts));
}

/// Help text of the error diagnostics, which say where a template error is.
fn error_locations(diagnostics: &[oxc_diagnostics::OxcDiagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| d.severity == oxc_diagnostics::Severity::Error)
        .map(|d| d.help.as_deref().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn locates_template_errors_in_the_file_like_the_transform() {
    let options = hmr(TransformOptions::default());
    let input = ComponentHmrUpdateInput { class_name: "X", ..Default::default() };

    // An inline template is part of the source: `file:line:column` in it.
    let source = "import { Component } from '@angular/core';\n\
                  @Component({ selector: 'x-cmp', template: `<div>\n  <span>\n</div>` })\n\
                  export class X {}\n";
    let allocator = Allocator::default();
    let update =
        compile_component_hmr_update(&allocator, PATH, source, &input, Some(&options), None);
    let transformed = transform_angular_file(&allocator, PATH, source, Some(&options), None);
    assert!(update.module.is_none());
    let locations = error_locations(&update.diagnostics);
    assert_ne!(locations.len(), 0);
    assert!(locations.iter().all(|l| l == &format!("{PATH}:4:1")), "{locations:?}");
    assert_eq!(locations, error_locations(&transformed.diagnostics));

    // An external one: `templateUrl:line:column`.
    let source = "import { Component } from '@angular/core';\n\
                  @Component({ selector: 'x-cmp', templateUrl: './x.html' })\n\
                  export class X {}\n";
    let resources = ResolvedResources {
        templates: [("./x.html".into(), "<b>\n</i>".into())].into(),
        styles: std::collections::HashMap::default(),
    };
    let update = compile_component_hmr_update(
        &allocator,
        PATH,
        source,
        &input,
        Some(&options),
        Some(&resources),
    );
    let transformed =
        transform_angular_file(&allocator, PATH, source, Some(&options), Some(&resources));
    let locations = error_locations(&update.diagnostics);
    assert_eq!(locations, ["./x.html:2:1"]);
    assert_eq!(locations, error_locations(&transformed.diagnostics));

    // The template-only compile knows neither.
    let old = compile_template_for_hmr(&allocator, "<b>\n</i>", "X", PATH, &options).unwrap_err();
    assert_eq!(error_locations(&old), ["line 2, column 1 of the template"]);
}

#[test]
fn takes_template_and_styles_from_the_resources() {
    let source = "import { Component } from '@angular/core';\n\
                  @Component({ selector: 'x-cmp', templateUrl: './x.html', styleUrl: './x.css', styles: ['b { color: blue }'] })\n\
                  export class X {}\n";
    let resources = || ResolvedResources {
        templates: [(
            "./x.html".into(),
            "<link rel=\"stylesheet\" href=\"./link.css\"><b>x</b>".into(),
        )]
        .into(),
        styles: [
            ("./x.css".into(), vec!["b { color: red }".into()]),
            ("./link.css".into(), vec!["b { color: green }".into()]),
        ]
        .into(),
    };
    let options = hmr(TransformOptions::default());
    let resources = resources();
    let update = update(source, &options, Some(&resources));
    let code = transform(source, &options, Some(&resources));
    assert_template_matches_transform(&update, &code);
    assert_eq!(update.template_url.as_deref(), Some("./x.html"));
    assert_eq!(update.style_urls, ["./x.css"]);
    assert_eq!(update.template_style_urls, ["./link.css"]);
    assert_eq!(update.missing_style_urls, Vec::<String>::new());
    let styles = update.styles.as_deref().unwrap();
    assert_eq!(styles.len(), 3);
    for style in styles {
        assert!(
            squash(&code).contains(&squash(&format!("{style:?}"))),
            "{style}\nis not in\n{code}"
        );
    }
    assert!(styles[0].contains("blue") && styles[1].contains("red") && styles[2].contains("green"));
}

#[test]
fn reports_resources_it_was_not_given() {
    let source = "import { Component } from '@angular/core';\n\
                  @Component({ selector: 'x-cmp', templateUrl: './x.html', styleUrls: ['./x.css'] })\n\
                  export class X {}\n";
    let options = hmr(TransformOptions::default());
    let update = update(source, &options, None);
    assert!(update.module.is_none());
    assert_eq!(update.missing_template_url.as_deref(), Some("./x.html"));
    assert_eq!(update.missing_style_urls, ["./x.css"]);

    // With the template but not the stylesheet, nothing is known about the
    // styles, so the update keeps the live ones.
    let resources = ResolvedResources {
        templates: [("./x.html".into(), "<link rel=\"stylesheet\" href=\"l.css\"><b>x</b>".into())]
            .into(),
        styles: std::collections::HashMap::default(),
    };
    let update = self::update(source, &options, Some(&resources));
    assert_eq!(update.missing_style_urls, ["./x.css", "l.css"]);
    assert_eq!(update.styles, None);
    assert!(!update.module.as_deref().unwrap().contains("styles:"));
}

#[test]
fn clears_styles_only_for_the_source_the_component_was_compiled_from() {
    let source = component("", "<b>x</b>");
    let options = hmr(TransformOptions::default());
    let allocator = Allocator::default();
    let compiled = component_hmr_fingerprint(&allocator, PATH, &source, &["X"]).fingerprint;

    let run = |fingerprint: &str| {
        let allocator = Allocator::default();
        let input = ComponentHmrUpdateInput {
            class_name: "X",
            source_fingerprint: Some(fingerprint),
            compiled_class_names: Some(&["X"]),
        };
        compile_component_hmr_update(&allocator, PATH, &source, &input, Some(&options), None)
    };
    let update = run(&compiled);
    assert_eq!(update.fingerprint.as_deref(), Some(compiled.as_str()));
    assert_eq!(update.styles, Some(vec![]));
    assert!(update.module.as_deref().unwrap().contains("styles: []"));

    let update = run("another source");
    assert_eq!(update.styles, None);
    assert!(!update.module.as_deref().unwrap().contains("styles:"));
}

#[test]
fn sends_nothing_for_an_empty_template_or_another_class() {
    let options = hmr(TransformOptions::default());
    let update = update(&component("", ""), &options, None);
    assert!(update.module.is_none());

    let allocator = Allocator::default();
    let input = ComponentHmrUpdateInput { class_name: "Y", ..Default::default() };
    let source = component("", "<b>x</b>");
    let update =
        compile_component_hmr_update(&allocator, PATH, &source, &input, Some(&options), None);
    assert!(update.module.is_none());
    assert_eq!(update.diagnostics.len(), 0, "{:?}", update.diagnostics);
}

trait ConstsJs {
    fn consts_js(&self) -> &str;
}

impl ConstsJs for ComponentHmrUpdate {
    /// The `consts` the update module writes.
    fn consts_js(&self) -> &str {
        let module = self.module.as_deref().unwrap();
        let start = module.find("consts: ").unwrap() + "consts: ".len();
        let end = module[start..].find(",\n    ngContentSelectors").unwrap();
        &module[start..start + end]
    }
}

#[test]
fn numbers_constants_like_the_transform_after_selector_attrs_and_queries() {
    let source = "import { Component, ViewChild } from '@angular/core';\n\
                  @Component({ selector: 'button[x-cmp]', host: { '[class.on]': 'on' }, \
                  template: '<input #field [attr.a]=\"[1, name]\"><ng-content select=\"b\"></ng-content>' })\n\
                  export class X { name = 'x'; on = true; @ViewChild('field') field: unknown; }\n";
    let options = hmr(TransformOptions::default());
    let update = update(source, &options, None);
    let code = transform(source, &options, None);
    assert_template_matches_transform(&update, &code);
    let module = update.module.as_deref().unwrap();
    assert!(squash(&code).contains(&squash(update.consts_js())), "{module}");
    assert!(module.contains("ngContentSelectors: _c"), "{module}");
}

#[test]
fn leaves_deferred_imports_to_the_live_definition() {
    let source = "import { Component } from '@angular/core';\n\
                  import { Child } from './child';\n\
                  @Component({ selector: 'x-cmp', deferredImports: [Child], \
                  template: '@defer { <child-cmp /> }' })\n\
                  export class X {}\n";
    let options = hmr(TransformOptions::default());
    let update = update(source, &options, None);
    assert!(transform(source, &options, None).contains("import(\"./child\")"));
    let module = update.module.as_deref().unwrap();
    assert!(!module.contains("import("), "{module}");
    assert!(module.contains("ɵɵdefer("), "{module}");
}

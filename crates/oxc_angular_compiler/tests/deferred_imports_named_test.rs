//! `@Component.deferredImports` as an object of block names (Angular v22.2,
//! #654): `deferredImports: {blk: [Cmp]}` with `@defer (name blk)`. Expected
//! output is ngtsc 22.2.3's, and Angular's linker for linked output.

use oxc_allocator::Allocator;
use oxc_angular_compiler::linker::{LinkOptions, link_with_options};
use oxc_angular_compiler::{
    AngularVersion, CompilationMode, TransformOptions, TransformResult, transform_angular_file,
};

fn version(v: &str) -> AngularVersion {
    AngularVersion::parse(v).unwrap()
}

fn compile(source: &str, mode: CompilationMode, angular_version: Option<AngularVersion>) -> String {
    let result = try_compile(source, mode, angular_version);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

fn try_compile(
    source: &str,
    compilation_mode: CompilationMode,
    angular_version: Option<AngularVersion>,
) -> TransformResult {
    let allocator = Allocator::default();
    let options = TransformOptions { compilation_mode, angular_version, ..Default::default() };
    transform_angular_file(&allocator, "app.ts", source, Some(&options), None)
}

fn errors(source: &str, angular_version: Option<AngularVersion>) -> Vec<String> {
    let mut all = Vec::new();
    for mode in [CompilationMode::Full, CompilationMode::Partial] {
        let result = try_compile(source, mode, angular_version);
        let messages: Vec<String> = result.diagnostics.iter().map(ToString::to_string).collect();
        if mode == CompilationMode::Full {
            all = messages;
        } else {
            assert_eq!(all, messages, "full and partial report the same errors");
        }
    }
    all
}

fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn component(config: &str, template: &str) -> String {
    format!(
        "import {{ Component }} from '@angular/core';\n\
         import {{ A }} from './a';\n\
         import {{ B }} from './b';\n\
         @Component({{ selector: 'app', {config}, template: '{template}' }})\n\
         export class App {{}}\n"
    )
}

const LAZY_A: &str = "import(\"./a\").then((m)=>m.A)";
const LAZY_B: &str = "import(\"./b\").then((m)=>m.B)";

#[test]
fn each_named_block_gets_its_own_dependency_function() {
    let source = component(
        "deferredImports: { b1: [A], b2: [B] }",
        "@defer (name b1) { <cmp-a /> } @defer (name b2) { <cmp-b /> }",
    );
    let code = compact(&compile(&source, CompilationMode::Full, None));
    assert!(code.contains(&format!("constApp_Defer_1_DepsFn=()=>[{LAZY_A}];")), "{code}");
    assert!(code.contains(&format!("constApp_Defer_4_DepsFn=()=>[{LAZY_B}];")), "{code}");
    assert!(code.contains("ɵɵdefer(1,0,App_Defer_1_DepsFn);"), "{code}");
    assert!(code.contains("ɵɵdefer(4,3,App_Defer_4_DepsFn);"), "{code}");
    assert!(code.contains(&format!("ɵsetClassMetadataAsync(App,()=>[{LAZY_A},{LAZY_B}],(A,B)=>")));
    // `@defer` loads both, so their static imports go.
    assert!(!code.contains("from'./a'") && !code.contains("from'./b'"), "{code}");
}

#[test]
fn blocks_naming_the_same_classes_share_one_function() {
    let source = component(
        "deferredImports: { b1: [A], b2: [A] }",
        "@defer (name b1) { <cmp-a /> } @defer (name b2) { <cmp-a /> }",
    );
    let code = compact(&compile(&source, CompilationMode::Full, None));
    assert!(code.contains("ɵɵdefer(1,0,App_Defer_1_DepsFn);"), "{code}");
    assert!(code.contains("ɵɵdefer(4,3,App_Defer_1_DepsFn);"), "{code}");
    assert!(code.contains(&format!("ɵsetClassMetadataAsync(App,()=>[{LAZY_A}],(A)=>")), "{code}");
}

#[test]
fn name_can_come_before_or_after_the_triggers() {
    let source = component(
        "deferredImports: { b1: [A], b2: [B] }",
        "@defer (name b1; on viewport; prefetch on hover) { <cmp-a /> } @placeholder { <span>p</span> } \
         @defer (on idle; name b2) { <cmp-b /> }",
    );
    let code = compact(&compile(&source, CompilationMode::Full, None));
    assert!(
        code.contains("ɵɵdefer(2,0,App_Defer_2_DepsFn,null,1);i0.ɵɵdeferOnViewport(0,-1);"),
        "{code}"
    );
    assert!(code.contains("ɵɵdeferPrefetchOnHover(0,-1);"), "{code}");
    assert!(code.contains(&format!("constApp_Defer_5_DepsFn=()=>[{LAZY_B}];")), "{code}");
    assert!(code.contains("ɵɵdefer(5,4,App_Defer_5_DepsFn);i0.ɵɵdeferOnIdle();"), "{code}");
}

#[test]
fn nested_blocks_load_their_own_lists() {
    let source = component(
        "deferredImports: { outer: [A], inner: [B] }",
        "@defer (name outer) { <cmp-a /> @defer (name inner) { <cmp-b /> } }",
    );
    let code = compact(&compile(&source, CompilationMode::Full, None));
    assert!(code.contains(&format!("constApp_Defer_1_DepsFn=()=>[{LAZY_A}];")), "{code}");
    assert!(code.contains(&format!("constApp_Defer_0_Defer_2_DepsFn=()=>[{LAZY_B}];")), "{code}");
    assert!(code.contains("ɵɵdefer(2,1,App_Defer_0_Defer_2_DepsFn);"), "{code}");

    // Outer block first, as the linker numbers them.
    let code = compact(&compile(&source, CompilationMode::Partial, None));
    assert!(code.contains(&format!("deferBlockDependencies:[()=>[{LAZY_A}],()=>[{LAZY_B}]]")));
}

#[test]
fn named_blocks_leave_imports_eager() {
    // ngtsc defers no `imports` entry when blocks are named: `C` stays.
    let source = "import { Component } from '@angular/core';\n\
                  import { A } from './a';\n\
                  import { C } from './c';\n\
                  @Component({ selector: 'app', imports: [C], deferredImports: { blk: [A] }, \
                  template: '@defer (name blk) { <cmp-a /> <cmp-c /> }' })\n\
                  export class App {}\n";
    let code = compact(&compile(source, CompilationMode::Full, None));
    assert!(code.contains(&format!("constApp_Defer_1_DepsFn=()=>[{LAZY_A}];")), "{code}");
    assert!(code.contains("import{C}from'./c';"), "{code}");
    assert!(code.contains("ɵɵgetComponentDepsFactory(App,[C])"), "{code}");

    let code = compact(&compile(source, CompilationMode::Partial, None));
    assert!(code.contains(&format!("deferBlockDependencies:[()=>[{LAZY_A}]]")), "{code}");
    assert!(code.contains("import{C}from'./c';"), "{code}");
}

#[test]
fn partial_declaration_lists_each_blocks_function() {
    let source = component(
        "deferredImports: { b1: [A], b2: [A] }",
        "@defer (name b1) { <cmp-a /> } @defer (name b2) { <cmp-a /> }",
    );
    let code = compact(&compile(&source, CompilationMode::Partial, None));
    assert!(code.contains(&format!("deferBlockDependencies:[()=>[{LAZY_A}],()=>[{LAZY_A}]]")));
    assert!(code.contains("ɵɵngDeclareClassMetadataAsync"), "{code}");
    assert!(!code.contains("from'./a'"), "{code}");
}

#[test]
fn string_keys_and_forward_refs_are_read() {
    let source = "import { Component, forwardRef } from '@angular/core';\n\
                  import { A } from './a';\n\
                  @Component({ selector: 'app', deferredImports: { 'blk': [forwardRef(() => A)] }, \
                  template: '@defer (name blk) { <cmp-a /> }' })\n\
                  export class App {}\n";
    let code = compact(&compile(source, CompilationMode::Full, None));
    assert!(code.contains(&format!("constApp_Defer_1_DepsFn=()=>[{LAZY_A}];")), "{code}");
}

#[test]
fn import_used_elsewhere_in_the_file_stays() {
    let source = "import { Component } from '@angular/core';\n\
                  import { A, X } from './a';\n\
                  console.log(X);\n\
                  @Component({ selector: 'one', deferredImports: { blk: [A] }, providers: [B], \
                  template: '@defer (name blk) { <cmp-a /> }' })\n\
                  export class One {}\n\
                  import { B } from './b';\n\
                  @Component({ selector: 'two', deferredImports: { blk: [B] }, \
                  template: '@defer (name blk) { <cmp-b /> }' })\n\
                  export class Two {}\n";
    let code = compile(source, CompilationMode::Full, None);
    // Only `X` keeps `./a`; `providers` keeps `B`.
    assert!(code.contains("import { X } from \"./a\";"), "{code}");
    assert!(code.contains("import { B } from './b';"), "{code}");
    assert!(compact(&code).contains(LAZY_A) && compact(&code).contains(LAZY_B), "{code}");
}

#[test]
fn import_named_by_two_components_goes() {
    let source = "import { Component } from '@angular/core';\n\
                  import { A } from './a';\n\
                  @Component({ selector: 'one', deferredImports: { blk: [A] }, \
                  template: '@defer (name blk) { <cmp-a /> }' })\n\
                  export class One {}\n\
                  @Component({ selector: 'two', deferredImports: { x: [A], y: [A] }, \
                  template: '@defer (name x) { <cmp-a /> } @defer (name y) { <cmp-a /> }' })\n\
                  export class Two {}\n";
    let code = compile(source, CompilationMode::Full, None);
    assert!(!code.contains("from './a'"), "{code}");
}

#[test]
fn without_defer_blocks_the_import_stays() {
    let source = component("deferredImports: { blk: [A] }", "<cmp-a />");
    let code = compile(&source, CompilationMode::Full, None);
    assert!(code.contains("import { A } from './a';"), "{code}");
    assert!(!code.contains("import(\"./a\")"), "{code}");
}

#[test]
fn hmr_keeps_the_import() {
    let source = component("deferredImports: { blk: [A] }", "@defer (name blk) { <cmp-a /> }");
    let allocator = Allocator::default();
    let options = TransformOptions { hmr: true, ..Default::default() };
    let result = transform_angular_file(&allocator, "app.ts", &source, Some(&options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    assert!(result.code.contains("import { A } from './a';"), "{}", result.code);
}

#[test]
fn unnamed_block_is_an_error() {
    let source = component("deferredImports: { blk: [A] }", "@defer { <cmp-a /> }");
    assert_eq!(
        errors(&source, None),
        ["@defer block must specify a 'name' parameter (e.g. '@defer (name blockName)') when \
          'deferredImports' is defined."]
    );
    // An empty object defines block names too, and so does a nested block.
    let source = component("deferredImports: {}", "@defer { <cmp-a /> }");
    assert_eq!(errors(&source, None).len(), 1);
    let source =
        component("deferredImports: { blk: [A] }", "@defer (name blk) { @defer { <cmp-a /> } }");
    assert_eq!(errors(&source, None).len(), 1);
}

#[test]
fn unknown_block_name_is_an_error() {
    let source = component("deferredImports: { blk: [A] }", "@defer (name nope) { <cmp-a /> }");
    assert_eq!(
        errors(&source, None),
        ["The 'name' parameter references block 'nope' which is missing from \
          '@Component.deferredImports'."]
    );
    // Like ngtsc's local compilation, a spread or a value other than an
    // array literal gives no block.
    for config in
        ["deferredImports: { ...{ blk: [A] } }", "deferredImports: { blk: [A].concat([]) }"]
    {
        let source = component(config, "@defer (name blk) { <cmp-a /> }");
        assert_eq!(errors(&source, None).len(), 1, "{config}");
    }
}

#[test]
fn name_without_deferred_imports_is_an_error() {
    let source = component("imports: [A]", "@defer (name blk) { <cmp-a /> }");
    assert_eq!(
        errors(&source, None),
        ["The 'name' parameter can only be used when '@Component.deferredImports' is defined."]
    );
}

#[test]
fn bad_names_are_parse_errors() {
    for (template, error) in [
        ("@defer (name blk; name blk) {}", "Cannot specify multiple name parameters"),
        ("@defer (name \"blk\") {}", "Block names cannot be quoted in @defer blocks"),
        ("@defer (name 1x) {}", "Invalid block name \"1x\""),
        ("@defer (name) {}", "Unrecognized trigger"),
    ] {
        let source = component("deferredImports: { blk: [A] }", template);
        let errors = errors(&source, None);
        assert!(errors.iter().any(|e| e.contains(error)), "{template}: {errors:?}");
    }
}

#[test]
fn array_is_an_error_from_v22_2_0_next_5() {
    let source = component("deferredImports: [A]", "@defer { <cmp-a /> }");
    let error = "'deferredImports' must be an object mapping block names to dependency arrays.";
    for v in
        [None, Some(version("22.2.0-next.5")), Some(version("22.2.0")), Some(version("23.0.0"))]
    {
        assert_eq!(errors(&source, v), [error], "{v:?}");
    }
    // v22.2.0-next.4 took both forms.
    assert_eq!(errors(&source, Some(version("22.2.0-next.4"))), Vec::<String>::new());
    let named = component("deferredImports: { blk: [A] }", "@defer (name blk) { <cmp-a /> }");
    assert_eq!(errors(&named, Some(version("22.2.0-next.4"))), Vec::<String>::new());
}

#[test]
fn before_v22_2_name_is_an_unknown_trigger() {
    let source = component("deferredImports: { blk: [A] }", "@defer (name blk) { <cmp-a /> }");
    for v in ["22.1.0", "22.2.0-next.3", "21.2.0"] {
        assert_eq!(errors(&source, Some(version(v))), ["Unrecognized trigger"], "{v}");
    }
    let source = component("deferredImports: { blk: [A] }", "@defer { <cmp-a /> }");
    assert_eq!(
        errors(&source, Some(version("22.1.0"))),
        ["'deferredImports' must be an array of components, directives, or pipes."]
    );
}

/// #654 (i): before v22.2, the array's classes load in every block, and the
/// partial declaration carries them for the linker.
#[test]
fn array_before_v22_2_loads_its_classes_in_every_block() {
    let source = component("deferredImports: [A, B]", "@defer { <cmp-a /> } @defer { <cmp-b /> }");
    let code = compact(&compile(&source, CompilationMode::Partial, Some(version("22.1.0"))));
    let both = format!("()=>[{LAZY_A},{LAZY_B}]");
    assert!(code.contains(&format!("deferBlockDependencies:[{both},{both}]")), "{code}");
    assert!(!code.contains("from'./a'") && !code.contains("from'./b'"), "{code}");

    let code = compact(&compile(&source, CompilationMode::Full, Some(version("22.1.0"))));
    assert!(code.contains(&format!("ɵɵdefer(1,0,{both});")), "{code}");
    assert!(!code.contains("from'./a'") && !code.contains("from'./b'"), "{code}");
}

/// ngtsc 22.2.3's partial output for a named block, linked as Angular's
/// linker does.
#[test]
fn linker_links_named_blocks() {
    let code = r#"import { Component } from '@angular/core';
import * as i0 from "@angular/core";
export class App {
    static { this.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "22.2.2", type: App, isStandalone: true, selector: "app", ngImport: i0, template: '@defer (name outer) { <cmp-a /> @defer (name inner) { <cmp-b /> } }', isInline: true, deferBlockDependencies: [() => [import("./a").then(m => m.A)], () => [import("./b").then(m => m.B)]] }); }
}
"#;
    let allocator = Allocator::default();
    let options = LinkOptions { angular_version: Some(version("22.2.3")), ..Default::default() };
    let result = link_with_options(&allocator, code, "app.mjs", &options);
    let linked = compact(&result.code);
    assert!(!linked.contains("ngDeclareComponent"), "{}", result.code);
    assert!(linked.contains("constApp_Defer_1_DepsFn=()=>[import(\"./a\")"), "{}", result.code);
    assert!(linked.contains("constApp_Defer_0_Defer_2_DepsFn=()=>[import(\"./b\")"));
    assert!(linked.contains("ɵɵdefer(2,1,App_Defer_0_Defer_2_DepsFn)"), "{}", result.code);
    assert!(linked.contains("ɵɵdefer(1,0,App_Defer_1_DepsFn)"), "{}", result.code);
}

#[test]
fn version_gates() {
    let named = |v: &str| AngularVersion::parse(v).unwrap().supports_named_deferred_imports();
    let required = |v: &str| AngularVersion::parse(v).unwrap().requires_named_deferred_imports();
    assert!(!named("22.2.0-next.3") && named("22.2.0-next.4") && named("22.2.0-rc.0"));
    assert!(!named("22.1.9") && named("22.2.0") && named("22.3.0") && named("23.0.0"));
    assert!(!required("22.2.0-next.4") && required("22.2.0-next.5") && required("22.2.1"));
    assert!(!required("21.2.0") && required("23.0.0-next.0"));
}

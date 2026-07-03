//! Tests for the partial-declaration NgModule + Injector emitters
//! (`ɵɵngDeclareNgModule` + `ɵɵngDeclareInjector`).

use oxc_allocator::{Allocator, Box, Vec};
use oxc_angular_compiler::{
    CompilationMode, TransformOptions, compile_declare_injector_from_metadata,
    compile_declare_ng_module_from_metadata,
    injector::R3InjectorMetadataBuilder,
    link,
    ng_module::{R3NgModuleMetadataBuilder, R3Reference, R3SelectorScopeMode},
    output::ast::{OutputExpression, ReadVarExpr},
    output::emitter::JsEmitter,
    transform_angular_file,
};
use oxc_str::Ident;

fn read_var<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        &allocator,
    ))
}

fn emit(expr: &OutputExpression<'_>) -> String {
    JsEmitter::new().emit_expression(expr)
}

#[test]
fn empty_module_emits_only_type() {
    let allocator = Allocator::default();
    let meta = R3NgModuleMetadataBuilder::new(&allocator)
        .r#type(R3Reference::value_only(read_var(&allocator, "EmptyModule")))
        .selector_scope_mode(R3SelectorScopeMode::Omit)
        .build()
        .unwrap();
    let expr = compile_declare_ng_module_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("type:EmptyModule"), "expected type:EmptyModule, got: {js}");
    assert!(!js.contains("bootstrap"), "bootstrap should be omitted when empty, got: {js}");
    assert!(!js.contains("declarations"), "declarations should be omitted when empty, got: {js}");
    assert!(!js.contains("imports"), "imports should be omitted when empty, got: {js}");
    assert!(!js.contains("exports"), "exports should be omitted when empty, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn module_with_declarations_and_imports() {
    let allocator = Allocator::default();
    let mut decls = Vec::new_in(&&allocator);
    decls.push(R3Reference::value_only(read_var(&allocator, "MyComp")));
    decls.push(R3Reference::value_only(read_var(&allocator, "MyDir")));

    let mut imports = Vec::new_in(&&allocator);
    imports.push(R3Reference::value_only(read_var(&allocator, "CommonModule")));

    let meta = R3NgModuleMetadataBuilder::new(&allocator)
        .r#type(R3Reference::value_only(read_var(&allocator, "FeatureModule")))
        .selector_scope_mode(R3SelectorScopeMode::Omit)
        .add_declaration(R3Reference::value_only(read_var(&allocator, "MyComp")))
        .add_declaration(R3Reference::value_only(read_var(&allocator, "MyDir")))
        .add_import(R3Reference::value_only(read_var(&allocator, "CommonModule")))
        .build()
        .unwrap();
    let expr = compile_declare_ng_module_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("declarations:[MyComp,MyDir]"), "expected plain array, got: {js}");
    assert!(js.contains("imports:[CommonModule]"), "expected imports array, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn module_with_forward_decls_wraps_lists_in_arrow() {
    // contains_forward_decls applies to all list fields at once — single
    // lazy arrow per list, not per element. Mirrors refsToArray in
    // upstream render3/util.ts:90-93.
    let allocator = Allocator::default();
    let meta = R3NgModuleMetadataBuilder::new(&allocator)
        .r#type(R3Reference::value_only(read_var(&allocator, "ForwardModule")))
        .selector_scope_mode(R3SelectorScopeMode::Omit)
        .contains_forward_decls(true)
        .add_declaration(R3Reference::value_only(read_var(&allocator, "LaterComp")))
        .add_export(R3Reference::value_only(read_var(&allocator, "LaterComp")))
        .build()
        .unwrap();
    let expr = compile_declare_ng_module_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    // Normalize whitespace and search — emitter wraps lines and adjusts
    // spacing around `=>` depending on context.
    let compact: String = js.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains("declarations:()=>[LaterComp]"),
        "expected lazy-arrow declarations, got: {js}"
    );
    assert!(compact.contains("exports:()=>[LaterComp]"), "expected lazy-arrow exports, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn module_with_bootstrap_and_schemas() {
    let allocator = Allocator::default();
    let meta = R3NgModuleMetadataBuilder::new(&allocator)
        .r#type(R3Reference::value_only(read_var(&allocator, "AppModule")))
        .selector_scope_mode(R3SelectorScopeMode::Omit)
        .add_bootstrap(R3Reference::value_only(read_var(&allocator, "AppComponent")))
        .add_schema(R3Reference::value_only(read_var(&allocator, "CUSTOM_ELEMENTS_SCHEMA")))
        .build()
        .unwrap();
    let expr = compile_declare_ng_module_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("bootstrap:[AppComponent]"), "expected bootstrap, got: {js}");
    assert!(js.contains("schemas:[CUSTOM_ELEMENTS_SCHEMA]"), "expected schemas, got: {js}");
}

#[test]
fn injector_with_providers_only() {
    let allocator = Allocator::default();
    let providers = read_var(&allocator, "MY_PROVIDERS");
    let meta = R3InjectorMetadataBuilder::new(&allocator)
        .name(Ident::from("MyModule"))
        .r#type(read_var(&allocator, "MyModule"))
        .providers(providers)
        .build()
        .unwrap();
    let expr = compile_declare_injector_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("providers:MY_PROVIDERS"), "expected providers expr, got: {js}");
    assert!(!js.contains("imports"), "imports should be omitted, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn injector_without_providers_omits_field() {
    // Upstream omits the `providers` field entirely when no providers are
    // present (see hello_world GOLDEN_PARTIAL.js where MyModule's ɵinj
    // has no providers field). Emit-mode parity: match upstream byte
    // shape so library output stays compact.
    let allocator = Allocator::default();
    let meta = R3InjectorMetadataBuilder::new(&allocator)
        .name(Ident::from("EmptyModule"))
        .r#type(read_var(&allocator, "EmptyModule"))
        .build()
        .unwrap();
    let expr = compile_declare_injector_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(!js.contains("providers"), "providers field should be omitted entirely, got: {js}");
}

#[test]
fn injector_with_imports_array() {
    let allocator = Allocator::default();
    let meta = R3InjectorMetadataBuilder::new(&allocator)
        .name(Ident::from("FeatureModule"))
        .r#type(read_var(&allocator, "FeatureModule"))
        .add_import(read_var(&allocator, "CommonModule"))
        .add_import(read_var(&allocator, "HttpClientModule"))
        .build()
        .unwrap();
    let expr = compile_declare_injector_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(
        js.contains("imports:[CommonModule,HttpClientModule]"),
        "expected imports array, got: {js}"
    );
}

#[test]
fn injector_raw_imports_preserved_over_per_element_imports() {
    // raw_imports preserves call expressions like StoreModule.forRoot(...);
    // same precedence as full-mode injector emit.
    let allocator = Allocator::default();
    let raw_imports = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from("EXTERNAL_IMPORTS_ARRAY"), source_span: None },
        &&allocator,
    ));
    let meta = R3InjectorMetadataBuilder::new(&allocator)
        .name(Ident::from("FeatureModule"))
        .r#type(read_var(&allocator, "FeatureModule"))
        .add_import(read_var(&allocator, "ShouldBeIgnored"))
        .raw_imports(raw_imports)
        .build()
        .unwrap();
    let expr = compile_declare_injector_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("imports:EXTERNAL_IMPORTS_ARRAY"), "expected raw imports, got: {js}");
    assert!(!js.contains("ShouldBeIgnored"), "per-element imports should be ignored, got: {js}");
}

#[test]
fn partial_ng_module_e2e_via_transform_angular_file() {
    let allocator = Allocator::default();
    let source = "import { NgModule } from '@angular/core';

@NgModule({})
export class FeatureModule {}
";

    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "feature.module.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let code = &result.code;

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareNgModule"),
        "expected ɵɵngDeclareNgModule, got:\n{code}"
    );
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareInjector"),
        "expected ɵɵngDeclareInjector, got:\n{code}"
    );
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareFactory"),
        "expected ɵɵngDeclareFactory, got:\n{code}"
    );
    assert!(
        !code.contains("\u{0275}\u{0275}defineNgModule"),
        "ɵɵdefineNgModule should not appear in partial mode, got:\n{code}"
    );
    assert!(
        !code.contains("\u{0275}\u{0275}defineInjector"),
        "ɵɵdefineInjector should not appear in partial mode, got:\n{code}"
    );
    // Partial mode banishes setNgModuleScope — match upstream
    // ng_module/handler.ts:971.
    assert!(
        !code.contains("setNgModuleScope"),
        "setNgModuleScope is banned in partial mode, got:\n{code}"
    );

    // Round-trip through linker — all three declarations should expand.
    let linked = link(&allocator, code, "feature.module.mjs");
    assert!(linked.linked, "linker should accept the partial output. emitted:\n{code}");
    assert!(
        linked.code.contains("\u{0275}\u{0275}defineNgModule")
            && linked.code.contains("\u{0275}\u{0275}defineInjector"),
        "linked output should contain both ɵɵdefineNgModule and ɵɵdefineInjector, got:\n{}",
        linked.code
    );
    assert!(
        !linked.code.contains("\u{0275}\u{0275}ngDeclare"),
        "linked output should have no ngDeclare* calls left, got:\n{}",
        linked.code
    );
}

#[test]
fn full_mode_ng_module_unchanged() {
    let allocator = Allocator::default();
    let source = "import { NgModule } from '@angular/core';

@NgModule({})
export class FeatureModule {}
";

    let result = transform_angular_file(&allocator, "feature.module.ts", source, None, None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    assert!(
        result.code.contains("\u{0275}\u{0275}defineNgModule"),
        "default-mode (Full) should still emit ɵɵdefineNgModule, got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("\u{0275}\u{0275}ngDeclareNgModule"),
        "Full mode must not leak ngDeclare calls, got:\n{}",
        result.code
    );
}

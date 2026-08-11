//! Tests for the partial-declaration Injectable emitter
//! (`ɵɵngDeclareInjectable`).
//!
//! Each test pins the emitted shape for one provider variant
//! (default / useClass / useExisting / useValue / useFactory / forwardRef
//! wrapping / providedIn variants), plus a round-trip through our linker.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_angular_compiler::compile_declare_injectable_from_metadata;
use oxc_angular_compiler::factory::R3DependencyMetadata;
use oxc_angular_compiler::injectable::R3InjectableMetadataBuilder;
use oxc_angular_compiler::output::ast::{OutputExpression, ReadVarExpr};
use oxc_angular_compiler::output::emitter::JsEmitter;
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
fn simple_root_injectable() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("MyService"))
        .r#type(read_var(&allocator, "MyService"))
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);
    insta::assert_snapshot!(emit(&expr));
}

#[test]
fn injectable_with_use_class() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("AbstractService"))
        .r#type(read_var(&allocator, "AbstractService"))
        .use_class(read_var(&allocator, "ConcreteService"), false, None)
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(
        js.contains("useClass:ConcreteService"),
        "expected useClass:ConcreteService, got: {js}"
    );
    assert!(!js.contains("forwardRef"), "non-forward useClass should not wrap, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn injectable_with_use_class_forward_ref_wraps() {
    // forwardRef wrapping mirrors upstream generateForwardRef
    // (packages/compiler/src/render3/util.ts:174).
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("AbstractService"))
        .r#type(read_var(&allocator, "AbstractService"))
        .use_class(read_var(&allocator, "ConcreteService"), true, None)
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(
        js.contains("useClass:i0.forwardRef(function") && js.contains("return ConcreteService"),
        "forward-ref useClass should wrap in i0.forwardRef(function() {{ return X; }}), got: {js}"
    );
    insta::assert_snapshot!(js);
}

#[test]
fn injectable_with_use_existing() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("AliasToken"))
        .r#type(read_var(&allocator, "AliasToken"))
        .use_existing(read_var(&allocator, "RealToken"), false)
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("useExisting:RealToken"), "expected useExisting:RealToken, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn injectable_with_use_value() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("CONFIG"))
        .r#type(read_var(&allocator, "CONFIG"))
        .use_value(read_var(&allocator, "configObject"))
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("useValue:configObject"), "expected useValue:configObject, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn injectable_with_use_factory_and_deps() {
    let allocator = Allocator::default();
    let mut deps = Vec::new_in(&&allocator);
    deps.push(R3DependencyMetadata::simple(read_var(&allocator, "Dep1")));
    let mut optional_dep = R3DependencyMetadata::simple(read_var(&allocator, "Dep2"));
    optional_dep.optional = true;
    deps.push(optional_dep);

    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("MyService"))
        .r#type(read_var(&allocator, "MyService"))
        .use_factory(read_var(&allocator, "createService"), Some(deps))
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("useFactory:createService"), "useFactory should be raw, got: {js}");
    assert!(js.contains("deps:[{token:Dep1}"), "first dep should be plain, got: {js}");
    // The emitter may wrap long object literals — check the fields are
    // present independently rather than as a single substring.
    assert!(js.contains("token:Dep2"), "second dep should reference Dep2, got: {js}");
    assert!(js.contains("optional:true"), "second dep should carry optional:true, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn provided_in_omitted_when_none() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("ScopedService"))
        .r#type(read_var(&allocator, "ScopedService"))
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(!js.contains("providedIn"), "providedIn should be omitted when None, got: {js}");
}

#[test]
fn provided_in_platform() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("PlatformService"))
        .r#type(read_var(&allocator, "PlatformService"))
        .provided_in_platform()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"providedIn:"platform""#), "expected providedIn:\"platform\", got: {js}");
}

#[test]
fn provided_in_any() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("AnyService"))
        .r#type(read_var(&allocator, "AnyService"))
        .provided_in_any()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"providedIn:"any""#), "expected providedIn:\"any\", got: {js}");
}

#[test]
fn provided_in_module() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("ModuleScopedService"))
        .r#type(read_var(&allocator, "ModuleScopedService"))
        .provided_in_module(read_var(&allocator, "FeatureModule"))
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(
        js.contains("providedIn:FeatureModule"),
        "expected providedIn:FeatureModule, got: {js}"
    );
}

#[test]
fn injectable_field_order_matches_upstream() {
    let allocator = Allocator::default();
    let meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("MyService"))
        .r#type(read_var(&allocator, "MyService"))
        .provided_in_root()
        .build()
        .unwrap();
    let expr = compile_declare_injectable_from_metadata(&allocator, &meta);
    let js = emit(&expr);

    let min_idx = js.find("minVersion").unwrap();
    let ver_idx = js.find("version:").unwrap();
    let ng_idx = js.find("ngImport").unwrap();
    let type_idx = js.find("type:").unwrap();
    let prov_idx = js.find("providedIn").unwrap();
    assert!(
        min_idx < ver_idx && ver_idx < ng_idx && ng_idx < type_idx && type_idx < prov_idx,
        "field order should be minVersion, version, ngImport, type, providedIn (matches partial/injectable.ts:48-59), got: {js}"
    );
}

/// Pair the partial Injectable with its partial Factory and round-trip both
/// through our linker — the strongest in-tree correctness check we have.
#[test]
fn round_trip_partial_injectable_and_factory_through_linker() {
    use oxc_allocator::Vec as OxcVec;
    use oxc_angular_compiler::factory::{
        FactoryTarget, R3ConstructorFactoryMetadata, R3FactoryDeps, R3FactoryMetadata,
    };
    use oxc_angular_compiler::{compile_declare_factory_function, link};

    let allocator = Allocator::default();
    let mut ctor_deps = OxcVec::new_in(&&allocator);
    ctor_deps.push(R3DependencyMetadata::simple(read_var(&allocator, "HttpClient")));

    let injectable_meta = R3InjectableMetadataBuilder::new()
        .name(Ident::from("ApiService"))
        .r#type(read_var(&allocator, "ApiService"))
        .deps(Some(ctor_deps))
        .provided_in_root()
        .build()
        .unwrap();
    let prov_expr = compile_declare_injectable_from_metadata(&allocator, &injectable_meta);

    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: Ident::from("ApiService"),
        type_expr: read_var(&allocator, "ApiService"),
        type_decl: read_var(&allocator, "ApiService"),
        type_argument_count: 0,
        deps: {
            let mut v = OxcVec::new_in(&&allocator);
            v.push(R3DependencyMetadata::simple(read_var(&allocator, "HttpClient")));
            R3FactoryDeps::Valid(v)
        },
        target: FactoryTarget::Injectable,
    });
    let fac_expr = compile_declare_factory_function(&allocator, &factory_meta);

    let source = format!(
        "import * as i0 from \"@angular/core\";\nexport class ApiService {{}}\nApiService.\u{0275}fac = {};\nApiService.\u{0275}prov = {};",
        emit(&fac_expr),
        emit(&prov_expr)
    );

    let linked = link(&allocator, &source, "service.mjs");
    assert!(linked.linked, "linker should have processed both declarations");
    assert!(
        !linked.code.contains("ɵɵngDeclareFactory")
            && !linked.code.contains("ɵɵngDeclareInjectable"),
        "linked output should not retain any ngDeclare calls, got:\n{}",
        linked.code
    );
    assert!(
        linked.code.contains("ɵɵdefineInjectable"),
        "linked output should contain ɵɵdefineInjectable, got:\n{}",
        linked.code
    );
    assert!(
        linked.code.contains("HttpClient"),
        "linked output should retain constructor dep, got:\n{}",
        linked.code
    );
}

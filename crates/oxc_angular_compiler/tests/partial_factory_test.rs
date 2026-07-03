//! Tests for the partial-declaration factory emitter
//! (`ɵɵngDeclareFactory`).
//!
//! These tests pin the emitted shape against insta snapshots. Cross-validation
//! against upstream golden files lives elsewhere (and isn't done yet).

use oxc_allocator::{Allocator, Box, Vec};
use oxc_angular_compiler::compile_declare_factory_function;
use oxc_angular_compiler::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata,
};
use oxc_angular_compiler::output::ast::{OutputExpression, ReadVarExpr};
use oxc_angular_compiler::output::emitter::JsEmitter;
use oxc_str::Ident;

fn read_var<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        &allocator,
    ))
}

fn make_meta<'a>(
    allocator: &'a Allocator,
    class_name: &'static str,
    deps: R3FactoryDeps<'a>,
    target: FactoryTarget,
) -> R3FactoryMetadata<'a> {
    R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: Ident::from(class_name),
        type_expr: read_var(allocator, class_name),
        type_decl: read_var(allocator, class_name),
        type_argument_count: 0,
        deps,
        target,
    })
}

fn emit(expr: &OutputExpression<'_>) -> String {
    let emitter = JsEmitter::new();
    emitter.emit_expression(expr)
}

#[test]
fn simple_injectable_factory_with_one_dep() {
    let allocator = Allocator::default();
    let mut deps = Vec::new_in(&&allocator);
    deps.push(R3DependencyMetadata::simple(read_var(&allocator, "HttpClient")));

    let meta =
        make_meta(&allocator, "MyService", R3FactoryDeps::Valid(deps), FactoryTarget::Injectable);
    let expr = compile_declare_factory_function(&allocator, &meta);

    insta::assert_snapshot!(emit(&expr));
}

#[test]
fn factory_with_no_deps_emits_null() {
    let allocator = Allocator::default();
    let meta = make_meta(&allocator, "MyService", R3FactoryDeps::None, FactoryTarget::Injectable);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("deps:null"), "deps should be null, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn factory_with_invalid_deps_emits_string_invalid() {
    let allocator = Allocator::default();
    let meta =
        make_meta(&allocator, "MyService", R3FactoryDeps::Invalid, FactoryTarget::Injectable);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"deps:"invalid""#), "deps should be \"invalid\", got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn factory_with_type_only_invalid_dep_coerces_to_invalid() {
    // Matches full-mode behavior at factory/compiler.rs:143 — any constructor
    // param resolving to a type-only import poisons the whole factory.
    let allocator = Allocator::default();
    let mut deps = Vec::new_in(&&allocator);
    let mut poisoned = R3DependencyMetadata::simple(read_var(&allocator, "TypeOnlyToken"));
    poisoned.type_only_invalid = true;
    deps.push(poisoned);

    let meta =
        make_meta(&allocator, "MyService", R3FactoryDeps::Valid(deps), FactoryTarget::Injectable);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"deps:"invalid""#), "deps should coerce to \"invalid\", got: {js}");
}

#[test]
fn factory_with_flag_deps_emits_only_set_flags() {
    let allocator = Allocator::default();
    let mut deps = Vec::new_in(&&allocator);
    let mut dep = R3DependencyMetadata::simple(read_var(&allocator, "ParentService"));
    dep.optional = true;
    dep.skip_self = true;
    deps.push(dep);

    let meta = make_meta(
        &allocator,
        "ChildService",
        R3FactoryDeps::Valid(deps),
        FactoryTarget::Injectable,
    );
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    // Set flags appear.
    assert!(js.contains("optional:true"), "expected optional:true, got: {js}");
    assert!(js.contains("skipSelf:true"), "expected skipSelf:true, got: {js}");
    // Unset flags omitted.
    assert!(!js.contains("host:"), "host should be omitted, got: {js}");
    // self is a substring of skipSelf — check that no entry KEY "self:" appears.
    // (We check for the standalone " self:" or "{self:" patterns.)
    assert!(!js.contains("{self:") && !js.contains(",self:"), "self should be omitted, got: {js}");
    assert!(!js.contains("attribute:"), "attribute should be omitted, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn factory_target_pipe() {
    let allocator = Allocator::default();
    let meta = make_meta(&allocator, "TitlePipe", R3FactoryDeps::None, FactoryTarget::Pipe);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("ɵɵFactoryTarget.Pipe"), "expected Pipe target, got: {js}");
}

#[test]
fn factory_target_directive() {
    let allocator = Allocator::default();
    let meta = make_meta(&allocator, "MyDir", R3FactoryDeps::None, FactoryTarget::Directive);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("ɵɵFactoryTarget.Directive"), "expected Directive target, got: {js}");
}

#[test]
fn factory_target_component() {
    let allocator = Allocator::default();
    let meta = make_meta(&allocator, "MyCmp", R3FactoryDeps::None, FactoryTarget::Component);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("ɵɵFactoryTarget.Component"), "expected Component target, got: {js}");
}

#[test]
fn factory_target_ng_module() {
    let allocator = Allocator::default();
    let meta = make_meta(&allocator, "MyMod", R3FactoryDeps::None, FactoryTarget::NgModule);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains("ɵɵFactoryTarget.NgModule"), "expected NgModule target, got: {js}");
}

/// Round-trip through our own linker: the partial declaration we emit should
/// expand back into a full ɵfac function. This is the strongest in-tree
/// correctness signal — if the linker doesn't recognize our output, no
/// downstream consumer will either.
#[test]
fn round_trip_through_linker_to_full_factory() {
    use oxc_angular_compiler::link;

    let allocator = Allocator::default();
    let mut deps = Vec::new_in(&&allocator);
    deps.push(R3DependencyMetadata::simple(read_var(&allocator, "HttpClient")));

    let meta =
        make_meta(&allocator, "MyService", R3FactoryDeps::Valid(deps), FactoryTarget::Injectable);
    let expr = compile_declare_factory_function(&allocator, &meta);

    let source = format!(
        "import * as i0 from \"@angular/core\";\nexport class MyService {{}}\nMyService.\u{0275}fac = {};",
        emit(&expr)
    );

    let linked = link(&allocator, &source, "service.mjs");
    assert!(linked.linked, "linker should have processed the declaration");
    // After linking, the partial-declare call is gone…
    assert!(
        !linked.code.contains("ɵɵngDeclareFactory"),
        "linked output should not retain ɵɵngDeclareFactory, got:\n{}",
        linked.code
    );
    // …and the full factory function shape is present.
    assert!(
        linked.code.contains("MyService_Factory") || linked.code.contains("function"),
        "linked output should contain a factory function, got:\n{}",
        linked.code
    );
    assert!(
        linked.code.contains("HttpClient"),
        "linked factory should still reference HttpClient, got:\n{}",
        linked.code
    );
}

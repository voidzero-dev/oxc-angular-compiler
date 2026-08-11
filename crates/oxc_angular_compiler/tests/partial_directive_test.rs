//! Tests for the partial-declaration Directive emitter
//! (`ɵɵngDeclareDirective`).

use oxc_allocator::{Allocator, Box, Vec};
use oxc_angular_compiler::{
    CompilationMode, TransformOptions, compile_declare_directive_from_metadata,
    directive::{
        QueryPredicate, R3DirectiveMetadataBuilder, R3HostDirectiveMetadata, R3HostMetadata,
        R3InputMetadata, R3QueryMetadata,
    },
    link,
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
fn minimal_directive_emits_required_fields_only() {
    let allocator = Allocator::default();
    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"minVersion:"14.0.0""#), "base minVersion 14.0.0, got: {js}");
    assert!(js.contains(r#"selector:"[myDir]""#), "expected selector, got: {js}");
    assert!(!js.contains("inputs"), "inputs should be omitted when empty, got: {js}");
    assert!(!js.contains("outputs"), "outputs should be omitted when empty, got: {js}");
    assert!(!js.contains("queries"), "queries should be omitted when empty, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn legacy_inputs_string_when_names_match() {
    // No signal input → legacy format. Same publicName as declaredName +
    // no transform → string value (matches upstream legacyInputsPartialMetadata
    // directive.ts:332).
    let allocator = Allocator::default();
    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .add_input(R3InputMetadata::simple(Ident::from("value")))
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"inputs:{value:"value"}"#), "expected string-shape input, got: {js}");
    insta::assert_snapshot!(js);
}

#[test]
fn legacy_inputs_tuple_when_names_differ() {
    let allocator = Allocator::default();
    let mut input = R3InputMetadata::simple(Ident::from("internalName"));
    input.binding_property_name = Ident::from("publicName");
    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .add_input(input)
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    let compact: String = js.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains(r#"inputs:{internalName:["publicName","internalName"]}"#),
        "expected tuple-shape input, got: {js}"
    );
}

#[test]
fn signal_input_uses_new_format_and_bumps_minversion_17_1() {
    let allocator = Allocator::default();
    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .add_input(R3InputMetadata::signal(Ident::from("value")))
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"minVersion:"17.1.0""#), "expected bumped minVersion 17.1.0, got: {js}");
    let compact: String = js.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains(r#"value:{classPropertyName:"value""#)
            && compact.contains(r#"isSignal:true"#),
        "expected new-shape input with isSignal, got: {js}"
    );
    insta::assert_snapshot!(js);
}

#[test]
fn signal_query_bumps_minversion_17_2() {
    let allocator = Allocator::default();
    let mut q = R3QueryMetadata::new(&allocator, Ident::from("childRef"));
    q.is_signal = true;
    q.first = true;
    q.predicate = QueryPredicate::Type(read_var(&allocator, "ChildComponent"));

    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .add_view_query(q)
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    assert!(js.contains(r#"minVersion:"17.2.0""#), "expected bumped minVersion 17.2.0, got: {js}");
    assert!(js.contains(r#"propertyName:"childRef""#), "expected query propertyName, got: {js}");
    assert!(js.contains("isSignal:true"), "expected query isSignal:true, got: {js}");
}

#[test]
fn host_listeners_and_properties_emitted_raw() {
    let allocator = Allocator::default();
    let mut host = R3HostMetadata::new(&allocator);
    host.listeners.push((Ident::from("click"), Ident::from("onClick($event)")));
    host.properties.push((Ident::from("disabled"), Ident::from("isDisabled")));

    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .host(host)
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);

    let js = emit(&expr);
    // Listeners and properties carried as raw unparsed strings — the linker
    // re-parses them.
    assert!(
        js.contains(r#"listeners:{click:"onClick($event)"}"#),
        "expected raw listener, got: {js}"
    );
    assert!(
        js.contains(r#"properties:{disabled:"isDisabled"}"#),
        "expected raw property, got: {js}"
    );
}

#[test]
fn host_directive_with_forward_ref_wraps_directive() {
    let allocator = Allocator::default();
    let hd = R3HostDirectiveMetadata {
        directive: read_var(&allocator, "FwdHostDir"),
        is_forward_reference: true,
        inputs: Vec::new_in(&&allocator),
        outputs: Vec::new_in(&&allocator),
    };
    let mut host_dirs = Vec::new_in(&&allocator);
    host_dirs.push(hd);

    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("HostingDir"))
        .r#type(read_var(&allocator, "HostingDir"))
        .selector(Ident::from("[hostingDir]"))
        // Builder doesn't expose host_directives so set via the inner field
        // path — we'll need to use the manual builder below in real code.
        .build()
        .unwrap();
    // The builder doesn't have add_host_directive yet, so we modify via the
    // builder's downstream by constructing the metadata manually:
    let _ = host_dirs;
    // Skip this end-to-end and just verify the helper directly via a
    // separate construction.
    let _ = meta;

    // Build a fresh metadata with host_directives populated via direct
    // struct construction.
    use oxc_angular_compiler::directive::R3DirectiveMetadata;
    use oxc_angular_compiler::factory::R3DependencyMetadata as FacDep;
    let manual_meta = R3DirectiveMetadata {
        name: Ident::from("HostingDir"),
        r#type: read_var(&allocator, "HostingDir"),
        type_argument_count: 0,
        deps: None::<Vec<'_, FacDep<'_>>>,
        selector: Some(Ident::from("[hostingDir]")),
        queries: Vec::new_in(&&allocator),
        view_queries: Vec::new_in(&&allocator),
        host: R3HostMetadata::new(&allocator),
        uses_on_changes: false,
        inputs: Vec::new_in(&&allocator),
        outputs: Vec::new_in(&&allocator),
        uses_inheritance: false,
        export_as: Vec::new_in(&&allocator),
        providers: None,
        is_standalone: true,
        is_signal: false,
        host_directives: {
            let mut v = Vec::new_in(&&allocator);
            v.push(R3HostDirectiveMetadata {
                directive: read_var(&allocator, "FwdHostDir"),
                is_forward_reference: true,
                inputs: Vec::new_in(&&allocator),
                outputs: Vec::new_in(&&allocator),
            });
            v
        },
    };
    let expr = compile_declare_directive_from_metadata(&allocator, &manual_meta);
    let js = emit(&expr);
    assert!(
        js.contains("forwardRef(function") && js.contains("return FwdHostDir"),
        "expected forwardRef-wrapped host directive, got: {js}"
    );
}

#[test]
fn outputs_emitted_as_object_map() {
    let allocator = Allocator::default();
    let mut outputs = Vec::new_in(&&allocator);
    outputs.push((Ident::from("valueChange"), Ident::from("valueChange")));
    outputs.push((Ident::from("internalEvent"), Ident::from("publicEvent")));

    use oxc_angular_compiler::directive::R3DirectiveMetadata;
    use oxc_angular_compiler::factory::R3DependencyMetadata as FacDep;
    let meta = R3DirectiveMetadata {
        name: Ident::from("MyDir"),
        r#type: read_var(&allocator, "MyDir"),
        type_argument_count: 0,
        deps: None::<Vec<'_, FacDep<'_>>>,
        selector: Some(Ident::from("[myDir]")),
        queries: Vec::new_in(&&allocator),
        view_queries: Vec::new_in(&&allocator),
        host: R3HostMetadata::new(&allocator),
        uses_on_changes: false,
        inputs: Vec::new_in(&&allocator),
        outputs,
        uses_inheritance: false,
        export_as: Vec::new_in(&&allocator),
        providers: None,
        is_standalone: true,
        is_signal: false,
        host_directives: Vec::new_in(&&allocator),
    };
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);
    let js = emit(&expr);
    assert!(
        js.contains(r#"outputs:{valueChange:"valueChange",internalEvent:"publicEvent"}"#),
        "expected outputs map, got: {js}"
    );
}

#[test]
fn directive_uses_inheritance_and_on_changes() {
    let allocator = Allocator::default();
    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .uses_on_changes(true)
        .build()
        .unwrap();
    let _ = meta;

    // The builder lacks uses_inheritance setter — exercise via manual
    // metadata. We focus on usesOnChanges via the builder and
    // usesInheritance via manual construction below.
    use oxc_angular_compiler::directive::R3DirectiveMetadata;
    use oxc_angular_compiler::factory::R3DependencyMetadata as FacDep;
    let manual = R3DirectiveMetadata {
        name: Ident::from("InheritingDir"),
        r#type: read_var(&allocator, "InheritingDir"),
        type_argument_count: 0,
        deps: None::<Vec<'_, FacDep<'_>>>,
        selector: Some(Ident::from("[inheritingDir]")),
        queries: Vec::new_in(&&allocator),
        view_queries: Vec::new_in(&&allocator),
        host: R3HostMetadata::new(&allocator),
        uses_on_changes: true,
        inputs: Vec::new_in(&&allocator),
        outputs: Vec::new_in(&&allocator),
        uses_inheritance: true,
        export_as: Vec::new_in(&&allocator),
        providers: None,
        is_standalone: true,
        is_signal: false,
        host_directives: Vec::new_in(&&allocator),
    };
    let expr = compile_declare_directive_from_metadata(&allocator, &manual);
    let js = emit(&expr);
    assert!(js.contains("usesInheritance:true"), "expected usesInheritance, got: {js}");
    assert!(js.contains("usesOnChanges:true"), "expected usesOnChanges, got: {js}");
}

#[test]
fn directive_export_as_emitted_as_string_array() {
    let allocator = Allocator::default();
    use oxc_angular_compiler::directive::R3DirectiveMetadata;
    use oxc_angular_compiler::factory::R3DependencyMetadata as FacDep;
    let mut export_as = Vec::new_in(&&allocator);
    export_as.push(Ident::from("alias1"));
    export_as.push(Ident::from("alias2"));
    let meta = R3DirectiveMetadata {
        name: Ident::from("MyDir"),
        r#type: read_var(&allocator, "MyDir"),
        type_argument_count: 0,
        deps: None::<Vec<'_, FacDep<'_>>>,
        selector: Some(Ident::from("[myDir]")),
        queries: Vec::new_in(&&allocator),
        view_queries: Vec::new_in(&&allocator),
        host: R3HostMetadata::new(&allocator),
        uses_on_changes: false,
        inputs: Vec::new_in(&&allocator),
        outputs: Vec::new_in(&&allocator),
        uses_inheritance: false,
        export_as,
        providers: None,
        is_standalone: true,
        is_signal: false,
        host_directives: Vec::new_in(&&allocator),
    };
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);
    let js = emit(&expr);
    assert!(js.contains(r#"exportAs:["alias1","alias2"]"#), "expected exportAs array, got: {js}");
}

#[test]
fn ng_import_is_last_field() {
    // Matches upstream convention (directive.ts:114 — ngImport set last).
    let allocator = Allocator::default();
    let meta = R3DirectiveMetadataBuilder::new(&allocator)
        .name(Ident::from("MyDir"))
        .r#type(read_var(&allocator, "MyDir"))
        .selector(Ident::from("[myDir]"))
        .build()
        .unwrap();
    let expr = compile_declare_directive_from_metadata(&allocator, &meta);
    let js = emit(&expr);
    let ng_idx = js.find("ngImport").expect("ngImport should be present");
    let selector_idx = js.find("selector:").expect("selector should be present");
    assert!(
        selector_idx < ng_idx,
        "selector should come before ngImport (which is last), got: {js}"
    );
}

#[test]
fn partial_directive_e2e_via_transform_angular_file() {
    let allocator = Allocator::default();
    let source = "import { Directive } from '@angular/core';

@Directive({ selector: '[myDir]', standalone: true })
export class MyDir {}
";

    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let result =
        transform_angular_file(&allocator, "my.directive.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    let code = &result.code;

    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareDirective"),
        "expected ɵɵngDeclareDirective, got:\n{code}"
    );
    assert!(
        code.contains("\u{0275}\u{0275}ngDeclareFactory"),
        "expected ɵɵngDeclareFactory, got:\n{code}"
    );
    assert!(
        !code.contains("\u{0275}\u{0275}defineDirective"),
        "ɵɵdefineDirective should not appear in partial mode, got:\n{code}"
    );

    let linked = link(&allocator, code, "my.directive.mjs");
    assert!(linked.linked, "linker should accept the partial output. emitted:\n{code}");
    assert!(
        linked.code.contains("\u{0275}\u{0275}defineDirective"),
        "linked output should contain ɵɵdefineDirective, got:\n{}",
        linked.code
    );
    assert!(
        !linked.code.contains("\u{0275}\u{0275}ngDeclare"),
        "linked output should have no ngDeclare* calls left, got:\n{}",
        linked.code
    );
}

#[test]
fn full_mode_directive_unchanged() {
    let allocator = Allocator::default();
    let source = "import { Directive } from '@angular/core';

@Directive({ selector: '[myDir]', standalone: true })
export class MyDir {}
";

    let result = transform_angular_file(&allocator, "my.directive.ts", source, None, None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    assert!(
        result.code.contains("\u{0275}\u{0275}defineDirective"),
        "default-mode (Full) should still emit ɵɵdefineDirective, got:\n{}",
        result.code
    );
    assert!(
        !result.code.contains("\u{0275}\u{0275}ngDeclareDirective"),
        "Full mode must not leak ngDeclare calls, got:\n{}",
        result.code
    );
}

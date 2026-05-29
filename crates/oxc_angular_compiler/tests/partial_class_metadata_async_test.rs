//! Tests for the partial-declaration async ClassMetadata emitter
//! (`ɵɵngDeclareClassMetadataAsync`). Fires for components whose
//! templates use `@defer` blocks with deferrable imports.
//!
//! These tests exercise the emitter directly via the public partial API,
//! constructing an `R3ClassMetadata` + `R3DeferPerComponentDependency`
//! list by hand. That decouples the test from the full @defer-template
//! parsing pipeline while still validating the partial-declaration
//! shape.

use oxc_allocator::{Allocator, Box};
use oxc_angular_compiler::class_metadata::{R3ClassMetadata, R3DeferPerComponentDependency};
use oxc_angular_compiler::output::ast::{
    LiteralArrayExpr, LiteralMapEntry, LiteralMapExpr, OutputExpression, ReadVarExpr,
};
use oxc_angular_compiler::output::emitter::JsEmitter;
use oxc_angular_compiler::{
    compile_component_declare_class_metadata, compile_declare_class_metadata_async,
};
use oxc_str::Ident;

fn read_var<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        allocator,
    ))
}

fn emit(expr: &OutputExpression<'_>) -> String {
    JsEmitter::new().emit_expression(expr)
}

fn make_class_metadata<'a>(
    allocator: &'a Allocator,
    class_name: &'static str,
    decorator_name: &'static str,
) -> R3ClassMetadata<'a> {
    // decorators: [{ type: <decorator_name> }]
    let mut decorator_map_entries = oxc_allocator::Vec::new_in(allocator);
    decorator_map_entries.push(LiteralMapEntry::new(
        Ident::from("type"),
        read_var(allocator, decorator_name),
        false,
    ));
    let decorator_map = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: decorator_map_entries, source_span: None },
        allocator,
    ));
    let mut decorators_array = oxc_allocator::Vec::new_in(allocator);
    decorators_array.push(decorator_map);
    let decorators = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: decorators_array, source_span: None },
        allocator,
    ));

    R3ClassMetadata {
        r#type: read_var(allocator, class_name),
        decorators,
        ctor_parameters: None,
        prop_decorators: None,
    }
}

#[test]
fn async_class_metadata_emits_async_call_with_18_0_min_version() {
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");

    let deps = [R3DeferPerComponentDependency {
        param_name: Ident::from("LazyCmp"),
        export_name: Ident::from("LazyCmp"),
        import_path: Ident::from("./lazy"),
        is_default_import: false,
    }];

    let expr = compile_declare_class_metadata_async(&allocator, &cmp_meta, &deps);
    let js = emit(&expr);

    assert!(
        js.contains("\u{0275}\u{0275}ngDeclareClassMetadataAsync"),
        "expected ɵɵngDeclareClassMetadataAsync call, got: {js}"
    );
    assert!(
        js.contains(r#"minVersion:"18.0.0""#),
        "expected minVersion:\"18.0.0\" for async variant (defer support landed in 18), got: {js}"
    );
}

#[test]
fn async_class_metadata_emits_resolve_deferred_deps_and_resolve_metadata() {
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");

    let deps = [
        R3DeferPerComponentDependency {
            param_name: Ident::from("LazyA"),
            export_name: Ident::from("LazyA"),
            import_path: Ident::from("./a"),
            is_default_import: false,
        },
        R3DeferPerComponentDependency {
            param_name: Ident::from("LazyB"),
            export_name: Ident::from("LazyBExport"),
            import_path: Ident::from("./b"),
            is_default_import: false,
        },
    ];

    let expr = compile_declare_class_metadata_async(&allocator, &cmp_meta, &deps);
    let js = emit(&expr);

    // resolveDeferredDeps: arrow returning dynamic imports for each dep.
    assert!(js.contains("resolveDeferredDeps:"), "expected resolveDeferredDeps field, got: {js}");
    assert!(
        js.contains("import(\"./a\")") || js.contains("import('./a')"),
        "expected dynamic import to ./a, got: {js}"
    );
    assert!(
        js.contains("import(\"./b\")") || js.contains("import('./b')"),
        "expected dynamic import to ./b, got: {js}"
    );
    // Property reads use the *export* name, which lets the local param
    // name shadow the static import for tree-shaking.
    assert!(js.contains("m.LazyA"), "expected m.LazyA in resolver, got: {js}");
    assert!(js.contains("m.LazyBExport"), "expected m.LazyBExport in resolver, got: {js}");

    // resolveMetadata: arrow taking (LazyA, LazyB) — the *param* names.
    assert!(js.contains("resolveMetadata:"), "expected resolveMetadata field, got: {js}");
    assert!(
        js.contains("(LazyA,LazyB)") || js.contains("(LazyA, LazyB)"),
        "expected resolveMetadata params (LazyA, LazyB), got: {js}"
    );
}

#[test]
fn async_class_metadata_emits_null_for_missing_ctor_params() {
    // Upstream emits ctorParameters/propDecorators as `null` literals
    // when undefined (not omitted, unlike the sync variant). Mirrors
    // class_metadata.ts:56-58.
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");

    let deps = [R3DeferPerComponentDependency {
        param_name: Ident::from("LazyCmp"),
        export_name: Ident::from("LazyCmp"),
        import_path: Ident::from("./lazy"),
        is_default_import: false,
    }];

    let expr = compile_declare_class_metadata_async(&allocator, &cmp_meta, &deps);
    let js = emit(&expr);

    assert!(
        js.contains("ctorParameters:null"),
        "ctorParameters should be emitted as null literal when undefined, got: {js}"
    );
    assert!(
        js.contains("propDecorators:null"),
        "propDecorators should be emitted as null literal when undefined, got: {js}"
    );
}

#[test]
fn default_import_uses_m_default_in_resolver() {
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");

    let deps = [R3DeferPerComponentDependency {
        param_name: Ident::from("DefaultCmp"),
        export_name: Ident::from("anything"), // ignored when is_default_import
        import_path: Ident::from("./default"),
        is_default_import: true,
    }];

    let expr = compile_declare_class_metadata_async(&allocator, &cmp_meta, &deps);
    let js = emit(&expr);

    assert!(
        js.contains("m.default"),
        "default imports should resolve via m.default (not m.<export>), got: {js}"
    );
}

#[test]
fn dispatch_helper_falls_back_to_sync_when_no_deferred_deps() {
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");

    let empty: [R3DeferPerComponentDependency<'_>; 0] = [];
    let expr = compile_component_declare_class_metadata(&allocator, &cmp_meta, &empty);
    let js = emit(&expr);

    // Sync form — NOT async.
    assert!(
        js.contains("\u{0275}\u{0275}ngDeclareClassMetadata"),
        "expected sync ɵɵngDeclareClassMetadata, got: {js}"
    );
    assert!(
        !js.contains("Async"),
        "should not emit the async variant when no deferred deps, got: {js}"
    );
    // minVersion should be the sync constant (12.0.0), not 18.0.0.
    assert!(js.contains(r#"minVersion:"12.0.0""#), "expected sync minVersion 12.0.0, got: {js}");
}

#[test]
fn dispatch_helper_picks_async_when_deferred_deps_present() {
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");

    let deps = [R3DeferPerComponentDependency {
        param_name: Ident::from("LazyCmp"),
        export_name: Ident::from("LazyCmp"),
        import_path: Ident::from("./lazy"),
        is_default_import: false,
    }];

    let expr = compile_component_declare_class_metadata(&allocator, &cmp_meta, &deps);
    let js = emit(&expr);

    assert!(
        js.contains("\u{0275}\u{0275}ngDeclareClassMetadataAsync"),
        "expected async ɵɵngDeclareClassMetadataAsync when deps present, got: {js}"
    );
    assert!(js.contains(r#"minVersion:"18.0.0""#), "expected async minVersion 18.0.0, got: {js}");
}

#[test]
fn unit_field_order_matches_upstream() {
    // Upstream class_metadata.ts:60-71:
    //   minVersion, version, ngImport, type, resolveDeferredDeps, resolveMetadata
    let allocator = Allocator::default();
    let cmp_meta = make_class_metadata(&allocator, "MyCmp", "Component");
    let deps = [R3DeferPerComponentDependency {
        param_name: Ident::from("LazyCmp"),
        export_name: Ident::from("LazyCmp"),
        import_path: Ident::from("./lazy"),
        is_default_import: false,
    }];
    let expr = compile_declare_class_metadata_async(&allocator, &cmp_meta, &deps);
    let js = emit(&expr);

    let positions = [
        js.find("minVersion"),
        js.find("version:"),
        js.find("ngImport"),
        js.find("type:"),
        js.find("resolveDeferredDeps"),
        js.find("resolveMetadata"),
    ];
    for w in positions.windows(2) {
        let (a, b) = (w[0].expect("field missing"), w[1].expect("field missing"));
        assert!(a < b, "field order violation. Full output:\n{js}");
    }
}

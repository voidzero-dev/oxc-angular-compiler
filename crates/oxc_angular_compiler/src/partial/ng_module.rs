//! Partial-declaration emit for `ɵɵngDeclareNgModule`.
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/ng_module.ts:29`
//! (`compileDeclareNgModuleFromMetadata`).
//!
//! ```text
//! i0.ɵɵngDeclareNgModule({
//!   minVersion: "14.0.0",
//!   version: "0.0.0-PLACEHOLDER",
//!   ngImport: i0,
//!   type: MyModule,
//!   bootstrap?: [Foo, Bar] | () => [Foo, Bar],
//!   declarations?: <same>,
//!   imports?: <same>,
//!   exports?: <same>,
//!   schemas?: [Schema],
//!   id?: <Expr>
//! })
//! ```
//!
//! The `() => [...]` lazy-arrow variant is used for ALL list fields when
//! the module has `contains_forward_decls = true`. Mirrors upstream
//! `refsToArray` at `packages/compiler/src/render3/util.ts:90-93`.
//!
//! The paired `ɵfac` and `ɵinj` are emitted separately by
//! `super::factory` and `super::injector`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::factory::compile_declare_factory_function;
use super::{MIN_VERSION_NG_MODULE, PLACEHOLDER_VERSION};
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3FactoryDeps, R3FactoryMetadata,
};
use crate::ng_module::{R3NgModuleMetadata, R3Reference};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr,
    LiteralMapEntry, LiteralMapExpr, LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Emits the `ɵɵngDeclareNgModule` call.
pub fn compile_declare_ng_module_from_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3NgModuleMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_NG_MODULE));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(
        Ident::from("type"),
        meta.r#type.value.clone_in(allocator),
        false,
    ));

    let wrap = meta.contains_forward_decls;
    push_ref_list(allocator, &mut entries, "bootstrap", &meta.bootstrap, wrap);
    push_ref_list(allocator, &mut entries, "declarations", &meta.declarations, wrap);
    push_ref_list(allocator, &mut entries, "imports", &meta.imports, wrap);
    push_ref_list(allocator, &mut entries, "exports", &meta.exports, wrap);
    // Schemas never need forward-decl wrapping — schemas reference runtime
    // tokens by import. Match upstream ng_module.ts:85-87.
    if !meta.schemas.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("schemas"),
            refs_to_array(allocator, &meta.schemas, false),
            false,
        ));
    }
    if let Some(id) = &meta.id {
        entries.push(LiteralMapEntry::new(Ident::from("id"), id.clone_in(allocator), false));
    }

    invoke_declare(allocator, Identifiers::DECLARE_NG_MODULE, entries)
}

/// Builds the partial ɵfac factory paired with this NgModule.
pub fn compile_declare_factory_for_ng_module<'a>(
    allocator: &'a Allocator,
    meta: &R3NgModuleMetadata<'a>,
) -> OutputExpression<'a> {
    // R3NgModuleMetadata doesn't carry constructor deps — the OXC
    // NgModule analyzer doesn't extract them since NgModule classes are
    // virtually always parameterless. Emit `deps: []` to match upstream's
    // golden behavior (see compiler-cli/test/compliance/test_cases/
    // r3_view_compiler/hello_world/GOLDEN_PARTIAL.js — every ɵfac for
    // an NgModule carries `deps: []`). The linker then generates the
    // simple `new MyModule()` factory.
    //
    // An NgModule that extends another class is exotic enough that
    // accepting a suboptimal-but-correct factory there is fine.
    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: Ident::from("NgModuleFactory"),
        type_expr: meta.r#type.value.clone_in(allocator),
        type_decl: meta.r#type.value.clone_in(allocator),
        type_argument_count: 0,
        deps: R3FactoryDeps::Valid(Vec::new_in(allocator)),
        target: FactoryTarget::NgModule,
    });
    compile_declare_factory_function(allocator, &factory_meta)
}

// ---- ref list helpers ------------------------------------------------------

fn push_ref_list<'a>(
    allocator: &'a Allocator,
    entries: &mut Vec<'a, LiteralMapEntry<'a>>,
    key: &'static str,
    refs: &Vec<'a, R3Reference<'a>>,
    contains_forward_decls: bool,
) {
    if refs.is_empty() {
        return;
    }
    entries.push(LiteralMapEntry::new(
        Ident::from(key),
        refs_to_array(allocator, refs, contains_forward_decls),
        false,
    ));
}

/// Mirrors upstream `refsToArray` at
/// `packages/compiler/src/render3/util.ts:90-93`.
///
/// Plain: `[Foo, Bar]`. Lazy (when `contains_forward_decls`):
/// `() => [Foo, Bar]`. The arrow wrap is applied once per list, not once
/// per element — matches upstream behavior.
fn refs_to_array<'a>(
    allocator: &'a Allocator,
    refs: &Vec<'a, R3Reference<'a>>,
    contains_forward_decls: bool,
) -> OutputExpression<'a> {
    let mut elements: Vec<'a, OutputExpression<'a>> = Vec::with_capacity_in(refs.len(), allocator);
    for r in refs {
        elements.push(r.value.clone_in(allocator));
    }
    let array_expr = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: elements, source_span: None },
        allocator,
    ));

    if !contains_forward_decls {
        return array_expr;
    }

    OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::new_in(allocator),
            body: ArrowFunctionBody::Expression(Box::new_in(array_expr, allocator)),
            source_span: None,
        },
        allocator,
    ))
}

// ---- shared low-level helpers ----------------------------------------------

fn invoke_declare<'a>(
    allocator: &'a Allocator,
    name: &'static str,
    entries: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ));
    let mut args = Vec::new_in(allocator);
    args.push(map_expr);
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(namespaced_prop(allocator, "i0", name), allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

fn read_var<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        allocator,
    ))
}

fn namespaced_prop<'a>(
    allocator: &'a Allocator,
    receiver: &'static str,
    prop: &'static str,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(read_var(allocator, receiver), allocator),
            name: Ident::from(prop),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

fn string_literal<'a>(allocator: &'a Allocator, value: &'static str) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(value)), source_span: None },
        allocator,
    ))
}

fn string_entry<'a>(
    allocator: &'a Allocator,
    key: &'static str,
    value: &'static str,
) -> LiteralMapEntry<'a> {
    LiteralMapEntry::new(Ident::from(key), string_literal(allocator, value), false)
}

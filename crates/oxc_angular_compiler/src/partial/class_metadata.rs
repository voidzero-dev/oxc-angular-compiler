//! Partial-declaration emit for `ɵɵngDeclareClassMetadata` (sync form).
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/class_metadata.ts:33`
//! (`compileDeclareClassMetadata`).
//!
//! Shape:
//!
//! ```text
//! i0.ɵɵngDeclareClassMetadata({
//!   minVersion: "12.0.0",
//!   version: "0.0.0-PLACEHOLDER",
//!   ngImport: i0,
//!   type: <class>,
//!   decorators: <expr>,          // required
//!   ctorParameters?: <expr>,     // arrow returning array literal
//!   propDecorators?: <expr>
//! })
//! ```
//!
//! Notably this is the inverse of the full-mode emit: full mode wraps in
//! an IIFE + `(typeof ngDevMode === "undefined" || ngDevMode) &&` guard
//! so the call tree-shakes out of production. Partial mode emits a bare
//! call — the linker re-wraps when expanding to `ɵsetClassMetadata`.
//!
//! The async variant (`ɵɵngDeclareClassMetadataAsync`, upstream
//! `class_metadata.ts:46`) is also implemented below. It fires for
//! components whose templates use `@defer` blocks with deferrable
//! imports — the dispatch in `component/transform.rs` switches on the
//! presence of `R3DeferPerComponentDependency` entries.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::{MIN_VERSION_CLASS_METADATA, MIN_VERSION_CLASS_METADATA_ASYNC, PLACEHOLDER_VERSION};
use crate::class_metadata::{
    R3ClassMetadata, R3DeferPerComponentDependency, compile_component_metadata_async_resolver,
};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, FnParam, InvokeFunctionExpr, LiteralExpr,
    LiteralMapEntry, LiteralMapExpr, LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Emits the `i0.ɵɵngDeclareClassMetadata({...})` call.
pub fn compile_declare_class_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3ClassMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_CLASS_METADATA));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));
    entries.push(LiteralMapEntry::new(
        Ident::from("decorators"),
        meta.decorators.clone_in(allocator),
        false,
    ));
    if let Some(ctor) = &meta.ctor_parameters {
        entries.push(LiteralMapEntry::new(
            Ident::from("ctorParameters"),
            ctor.clone_in(allocator),
            false,
        ));
    }
    if let Some(props) = &meta.prop_decorators {
        entries.push(LiteralMapEntry::new(
            Ident::from("propDecorators"),
            props.clone_in(allocator),
            false,
        ));
    }

    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ));
    let mut args = Vec::new_in(&allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(
                namespaced_prop(allocator, "i0", Identifiers::DECLARE_CLASS_METADATA),
                &allocator,
            ),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}

/// Emits the `i0.ɵɵngDeclareClassMetadataAsync({...})` call for a
/// component whose template uses `@defer` blocks with deferrable
/// imports.
///
/// Ported from upstream
/// `packages/compiler/src/render3/partial/class_metadata.ts:46`
/// (`compileComponentDeclareClassMetadata`).
///
/// Shape (per upstream):
///
/// ```text
/// i0.ɵɵngDeclareClassMetadataAsync({
///   minVersion: "18.0.0",
///   version: "0.0.0-PLACEHOLDER",
///   ngImport: i0,
///   type: <class>,
///   resolveDeferredDeps: () => [import('./a').then(m => m.A), ...],
///   resolveMetadata: (A, B, ...) => ({
///     decorators: [...],
///     ctorParameters: <expr | null>,
///     propDecorators: <expr | null>
///   })
/// })
/// ```
///
/// Differs from the sync variant in two ways:
/// - `ctorParameters` and `propDecorators` are emitted as `null`
///   literals when undefined, not omitted (matches upstream `??
///   o.literal(null)` at class_metadata.ts:56-58).
/// - minVersion is `"18.0.0"` — the linker version that gained defer
///   support.
pub fn compile_declare_class_metadata_async<'a>(
    allocator: &'a Allocator,
    meta: &R3ClassMetadata<'a>,
    dependencies: &[R3DeferPerComponentDependency<'a>],
) -> OutputExpression<'a> {
    // resolveMetadata callback body: { decorators, ctorParameters, propDecorators }
    let mut callback_entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);
    callback_entries.push(LiteralMapEntry::new(
        Ident::from("decorators"),
        meta.decorators.clone_in(allocator),
        false,
    ));
    callback_entries.push(LiteralMapEntry::new(
        Ident::from("ctorParameters"),
        meta.ctor_parameters
            .as_ref()
            .map(|e| e.clone_in(allocator))
            .unwrap_or_else(|| null_lit(allocator)),
        false,
    ));
    callback_entries.push(LiteralMapEntry::new(
        Ident::from("propDecorators"),
        meta.prop_decorators
            .as_ref()
            .map(|e| e.clone_in(allocator))
            .unwrap_or_else(|| null_lit(allocator)),
        false,
    ));
    let callback_body = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: callback_entries, source_span: None },
        &allocator,
    ));

    // resolveMetadata callback params: one per deferred dep, using the
    // local param_name (which shadows the static import so bundlers can
    // tree-shake the eager binding).
    let mut params = Vec::new_in(&allocator);
    for dep in dependencies {
        params.push(FnParam { name: dep.param_name.clone() });
    }
    let resolve_metadata = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params,
            body: ArrowFunctionBody::Expression(Box::new_in(callback_body, &allocator)),
            source_span: None,
        },
        &allocator,
    ));

    // resolveDeferredDeps arrow — reused from the full-mode emitter; same
    // shape in both modes.
    let resolve_deferred_deps = compile_component_metadata_async_resolver(allocator, dependencies);

    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);
    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_CLASS_METADATA_ASYNC));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));
    entries.push(LiteralMapEntry::new(
        Ident::from("resolveDeferredDeps"),
        resolve_deferred_deps,
        false,
    ));
    entries.push(LiteralMapEntry::new(Ident::from("resolveMetadata"), resolve_metadata, false));

    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ));
    let mut args = Vec::new_in(&allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(
                namespaced_prop(allocator, "i0", Identifiers::DECLARE_CLASS_METADATA_ASYNC),
                &allocator,
            ),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}

/// Dispatch helper that mirrors the upstream
/// `compileComponentDeclareClassMetadata` decision (class_metadata.ts:50):
/// no deferred deps → sync form; one or more → async form.
pub fn compile_component_declare_class_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3ClassMetadata<'a>,
    dependencies: &[R3DeferPerComponentDependency<'a>],
) -> OutputExpression<'a> {
    if dependencies.is_empty() {
        compile_declare_class_metadata(allocator, meta)
    } else {
        compile_declare_class_metadata_async(allocator, meta, dependencies)
    }
}

// ---- helpers ---------------------------------------------------------------

fn read_var<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        &allocator,
    ))
}

fn namespaced_prop<'a>(
    allocator: &'a Allocator,
    receiver: &'static str,
    prop: &'static str,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(read_var(allocator, receiver), &allocator),
            name: Ident::from(prop),
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}

fn string_literal<'a>(allocator: &'a Allocator, value: &'static str) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(value)), source_span: None },
        &allocator,
    ))
}

fn string_entry<'a>(
    allocator: &'a Allocator,
    key: &'static str,
    value: &'static str,
) -> LiteralMapEntry<'a> {
    LiteralMapEntry::new(Ident::from(key), string_literal(allocator, value), false)
}

fn null_lit<'a>(allocator: &'a Allocator) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Null, source_span: None },
        &allocator,
    ))
}

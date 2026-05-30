//! Partial-declaration emit for `ɵɵngDeclareInjector`.
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/injector.ts:25`
//! (`compileDeclareInjectorFromMetadata`).
//!
//! ```text
//! i0.ɵɵngDeclareInjector({
//!   minVersion: "12.0.0",
//!   version: "0.0.0-PLACEHOLDER",
//!   ngImport: i0,
//!   type: MyModule,
//!   providers: <Expr | null>,
//!   imports?: [<Expr>]
//! })
//! ```

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::{MIN_VERSION_INJECTOR, PLACEHOLDER_VERSION};
use crate::injector::R3InjectorMetadata;
use crate::output::ast::{
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Emits the `ɵɵngDeclareInjector` call.
pub fn compile_declare_injector_from_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3InjectorMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_INJECTOR));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));

    // providers: upstream emits this slot only when defined. Omitting it
    // when our metadata has no providers matches the byte-shape of
    // upstream's hello_world golden (`{ minVersion: ..., ngImport: ...,
    // type: MyModule }` — no providers field). The linker treats the
    // missing field as "no providers" — identical runtime to
    // `providers: []`.
    if let Some(providers) = &meta.providers {
        entries.push(LiteralMapEntry::new(
            Ident::from("providers"),
            providers.clone_in(allocator),
            false,
        ));
    }

    // imports: omitted when empty. Prefer raw_imports (preserves calls
    // like `StoreModule.forRoot(...)` and spread elements) over the
    // per-element imports vec — same precedence as full-mode injector
    // emit (ng_module/definition.rs:283-293).
    if let Some(raw) = &meta.raw_imports {
        entries.push(LiteralMapEntry::new(Ident::from("imports"), raw.clone_in(allocator), false));
    } else if !meta.imports.is_empty() {
        let mut elements: Vec<'a, OutputExpression<'a>> =
            Vec::with_capacity_in(meta.imports.len(), allocator);
        for imp in &meta.imports {
            elements.push(imp.clone_in(allocator));
        }
        entries.push(LiteralMapEntry::new(
            Ident::from("imports"),
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: elements, source_span: None },
                allocator,
            )),
            false,
        ));
    }

    invoke_declare(allocator, Identifiers::DECLARE_INJECTOR, entries)
}

// ---- helpers ---------------------------------------------------------------

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

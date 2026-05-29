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
//! `class_metadata.ts:46`) is not yet implemented; it's only triggered
//! for components with `@defer` deferrable imports, which need
//! cross-file selector info OXC doesn't carry today.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::{MIN_VERSION_CLASS_METADATA, PLACEHOLDER_VERSION};
use crate::class_metadata::R3ClassMetadata;
use crate::output::ast::{
    InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue,
    OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Emits the `i0.ɵɵngDeclareClassMetadata({...})` call.
pub fn compile_declare_class_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3ClassMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

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
        allocator,
    ));
    let mut args = Vec::new_in(allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(
                namespaced_prop(allocator, "i0", Identifiers::DECLARE_CLASS_METADATA),
                allocator,
            ),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

// ---- helpers ---------------------------------------------------------------

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

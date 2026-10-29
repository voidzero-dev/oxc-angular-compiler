//! Partial-declaration emit for `ɵɵngDeclareService`.
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/service.ts`
//! (`compileDeclareServiceFromMetadata`).
//!
//! Emits the `ɵprov` static for a `@Service` class (Angular v22+) in library
//! form:
//!
//! ```text
//! i0.ɵɵngDeclareService({
//!   minVersion: "22.0.0",
//!   version: "0.0.0-PLACEHOLDER",
//!   ngImport: i0,
//!   type: MyService,
//!   autoProvided?: false,
//!   factory?: <Expr>
//! })
//! ```
//!
//! The ɵfac side is a `ɵɵngDeclareFactory` with `target: Service`, emitted by
//! `super::factory`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::{MIN_VERSION_SERVICE, PLACEHOLDER_VERSION};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue,
    OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;
use crate::service::R3ServiceMetadata;

/// Emits the `ɵɵngDeclareService` call for a `@Service`'s `ɵprov` static.
///
/// Unlike full mode, the user's `factory` is kept as written: the linker
/// wraps it in `() => factory()`.
pub fn compile_declare_service_from_metadata<'a>(
    allocator: &'a Allocator,
    core_namespace: &'a str,
    meta: &R3ServiceMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_SERVICE));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(
        Ident::from("ngImport"),
        read_var(allocator, core_namespace),
        false,
    ));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));

    // Only emitted when explicitly disabled, like full mode.
    if matches!(meta.auto_provided, Some(false)) {
        entries.push(LiteralMapEntry::new(
            Ident::from("autoProvided"),
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(false), source_span: None },
                &allocator,
            )),
            false,
        ));
    }

    if let Some(factory) = &meta.factory {
        entries.push(LiteralMapEntry::new(
            Ident::from("factory"),
            factory.clone_in(allocator),
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
                OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(read_var(allocator, core_namespace), &allocator),
                        name: Ident::from(Identifiers::DECLARE_SERVICE),
                        optional: false,
                        source_span: None,
                    },
                    &allocator,
                )),
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

fn read_var<'a>(allocator: &'a Allocator, name: &'a str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(name), source_span: None },
        &allocator,
    ))
}

fn string_entry<'a>(
    allocator: &'a Allocator,
    key: &'static str,
    value: &'static str,
) -> LiteralMapEntry<'a> {
    LiteralMapEntry::new(
        Ident::from(key),
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(Ident::from(value)), source_span: None },
            &allocator,
        )),
        false,
    )
}

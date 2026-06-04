//! Partial-declaration emit for `ɵɵngDeclareFactory`.
//!
//! Ported from upstream `packages/compiler/src/render3/partial/factory.ts:27`
//! (`compileDeclareFactoryFunction`).
//!
//! The factory is the smallest partial declaration — every other partial
//! emitter (`ɵɵngDeclareComponent`, `…Directive`, `…Pipe`, `…Injectable`,
//! `…NgModule`) pairs with a `ɵɵngDeclareFactory` call assigned to the
//! class's static `ɵfac` field. So this is the shared piece all subsequent
//! partial work depends on.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::{MIN_VERSION_FACTORY, PLACEHOLDER_VERSION};
use crate::factory::{FactoryTarget, R3FactoryDeps, R3FactoryMetadata};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Compiles a partial-declaration factory for a class.
///
/// Returns the expression `i0.ɵɵngDeclareFactory({ minVersion, version,
/// ngImport, type, deps, target })`. The caller assigns the result to the
/// class's static `ɵfac` field, exactly as it does for the full-mode
/// factory.
///
/// See: upstream `packages/compiler/src/render3/partial/factory.ts:27`.
pub fn compile_declare_factory_function<'a>(
    allocator: &'a Allocator,
    meta: &R3FactoryMetadata<'a>,
) -> OutputExpression<'a> {
    let base = meta.base();

    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_FACTORY));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(
        Ident::from("type"),
        base.type_expr.clone_in(allocator),
        false,
    ));
    entries.push(LiteralMapEntry::new(
        Ident::from("deps"),
        compile_dependencies(allocator, &base.deps),
        false,
    ));
    entries.push(LiteralMapEntry::new(
        Ident::from("target"),
        factory_target_expr(allocator, base.target),
        false,
    ));

    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ));

    let mut args = Vec::new_in(allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(
                namespaced_prop(allocator, "i0", Identifiers::DECLARE_FACTORY),
                allocator,
            ),
            args,
            // ngDeclare* calls are not @__PURE__ — upstream emits them as
            // plain statement calls. Tree-shaking is the linker's concern.
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Builds the `deps` field of the partial factory declaration.
///
/// Mirrors upstream `compileDependencies` at
/// `packages/compiler/src/render3/partial/util.ts:58`.
///
/// - `R3FactoryDeps::Invalid` → `"invalid"` (string literal — linker emits
///   `ɵɵinvalidFactory()`).
/// - `R3FactoryDeps::None` → `null` (linker emits the inherited-factory
///   pattern).
/// - `R3FactoryDeps::Valid(deps)` where any dep has `type_only_invalid` →
///   coerced to `"invalid"`, matching full-mode behavior (see #288 and
///   `factory/compiler.rs:143`).
/// - Otherwise → array literal of per-dep maps.
fn compile_dependencies<'a>(
    allocator: &'a Allocator,
    deps: &R3FactoryDeps<'a>,
) -> OutputExpression<'a> {
    match deps {
        R3FactoryDeps::Invalid => string_literal(allocator, "invalid"),
        R3FactoryDeps::None => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )),
        R3FactoryDeps::Valid(deps) if deps.iter().any(|d| d.type_only_invalid) => {
            string_literal(allocator, "invalid")
        }
        R3FactoryDeps::Valid(deps) => {
            let mut entries = Vec::with_capacity_in(deps.len(), allocator);
            for dep in deps {
                entries.push(compile_dependency(allocator, dep));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ))
        }
    }
}

/// Builds one entry of the `deps` array.
///
/// Shape (`R3DeclareDependencyMetadata`):
/// ```text
/// { token, attribute?, host?, optional?, self?, skipSelf? }
/// ```
/// Boolean flags are omitted when false. `token` is `null` if the input
/// dependency has no token (an invalid dep — the linker emits
/// `ɵɵinvalidFactoryDep` for it).
fn compile_dependency<'a>(
    allocator: &'a Allocator,
    dep: &crate::factory::R3DependencyMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    let token_expr = match &dep.token {
        Some(token) => token.clone_in(allocator),
        None => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )),
    };
    entries.push(LiteralMapEntry::new(Ident::from("token"), token_expr, false));

    if dep.attribute_name_type.is_some() {
        entries.push(bool_entry(allocator, "attribute"));
    }
    if dep.host {
        entries.push(bool_entry(allocator, "host"));
    }
    if dep.optional {
        entries.push(bool_entry(allocator, "optional"));
    }
    if dep.self_ {
        entries.push(bool_entry(allocator, "self"));
    }
    if dep.skip_self {
        entries.push(bool_entry(allocator, "skipSelf"));
    }

    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ))
}

/// Builds the `target` field: `i0.ɵɵFactoryTarget.<Variant>`.
fn factory_target_expr<'a>(
    allocator: &'a Allocator,
    target: FactoryTarget,
) -> OutputExpression<'a> {
    let variant = match target {
        FactoryTarget::Component => "Component",
        FactoryTarget::Directive => "Directive",
        FactoryTarget::Pipe => "Pipe",
        FactoryTarget::NgModule => "NgModule",
        FactoryTarget::Injectable => "Injectable",
        // `@Service` (Angular v22+) uses the same `ɵɵinject` token resolution as
        // `Injectable`, so its partial-declaration factory target is `Injectable`.
        FactoryTarget::Service => "Injectable",
    };

    let factory_target_ref = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(read_var(allocator, "i0"), allocator),
            name: Ident::from(Identifiers::FACTORY_TARGET),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(factory_target_ref, allocator),
            name: Ident::from(variant),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

// ---- small helpers --------------------------------------------------------

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

fn bool_entry<'a>(allocator: &'a Allocator, key: &'static str) -> LiteralMapEntry<'a> {
    LiteralMapEntry::new(
        Ident::from(key),
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(true), source_span: None },
            allocator,
        )),
        false,
    )
}

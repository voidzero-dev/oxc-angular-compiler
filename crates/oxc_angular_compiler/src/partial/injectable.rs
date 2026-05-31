//! Partial-declaration emit for `ɵɵngDeclareInjectable`.
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/injectable.ts:29`
//! (`compileDeclareInjectableFromMetadata`).
//!
//! Emits the `ɵprov` static for an `@Injectable` class in library form:
//!
//! ```text
//! i0.ɵɵngDeclareInjectable({
//!   minVersion: "12.0.0",
//!   version: "0.0.0-PLACEHOLDER",
//!   ngImport: i0,
//!   type: MyService,
//!   providedIn?: "root" | "platform" | "any" | <Expr>,
//!   useClass?: <Expr> | i0.forwardRef(function() { return X; }),
//!   useExisting?: <Expr> | i0.forwardRef(...),
//!   useValue?: <Expr>,
//!   useFactory?: <Expr>,
//!   deps?: [<dep map>]
//! })
//! ```
//!
//! The ɵfac side is emitted separately by `super::factory`. Both pair up on
//! the class to form a valid library-published Injectable.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::factory::compile_declare_factory_function;
use super::{MIN_VERSION_INJECTABLE, PLACEHOLDER_VERSION, wrap_forward_ref};
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata,
};
use crate::injectable::{InjectableProvider, ProvidedIn, R3InjectableMetadata};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Emits the `ɵɵngDeclareInjectable` call for an `@Injectable`'s `ɵprov`
/// static.
///
/// See: upstream `packages/compiler/src/render3/partial/injectable.ts:29`.
pub fn compile_declare_injectable_from_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3InjectableMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_INJECTABLE));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));

    // providedIn — only emit when set. ProvidedIn::None is the "omit" case.
    if let Some(expr) = provided_in_expr(allocator, &meta.provided_in) {
        entries.push(LiteralMapEntry::new(Ident::from("providedIn"), expr, false));
    }

    // Provider variants: at most one of useClass/useExisting/useValue/useFactory.
    match &meta.provider {
        InjectableProvider::UseClass { class_expr, is_forward_ref, .. } => {
            entries.push(LiteralMapEntry::new(
                Ident::from("useClass"),
                maybe_forward_ref(allocator, class_expr.clone_in(allocator), *is_forward_ref),
                false,
            ));
        }
        InjectableProvider::UseExisting { existing, is_forward_ref } => {
            entries.push(LiteralMapEntry::new(
                Ident::from("useExisting"),
                maybe_forward_ref(allocator, existing.clone_in(allocator), *is_forward_ref),
                false,
            ));
        }
        InjectableProvider::UseValue { value } => {
            entries.push(LiteralMapEntry::new(
                Ident::from("useValue"),
                value.clone_in(allocator),
                false,
            ));
        }
        InjectableProvider::UseFactory { factory, .. } => {
            // Factory expressions are already function-wrapped — no
            // forwardRef needed. See upstream comment at
            // partial/injectable.ts:70-72.
            entries.push(LiteralMapEntry::new(
                Ident::from("useFactory"),
                factory.clone_in(allocator),
                false,
            ));
        }
        InjectableProvider::Default => {}
    }

    // deps — only set when the provider supplies them (useClass-with-deps /
    // useFactory-with-deps). Constructor deps live in `meta.deps` and feed
    // the ɵfac factory, not this `deps:` field.
    if let Some(provider_deps) = provider_deps(&meta.provider) {
        entries.push(LiteralMapEntry::new(
            Ident::from("deps"),
            compile_dependencies_array(allocator, provider_deps),
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
                namespaced_prop(allocator, "i0", Identifiers::DECLARE_INJECTABLE),
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

/// Builds the partial ɵfac factory paired with this Injectable.
///
/// Convenience for callers that already have an `R3InjectableMetadata` — it
/// constructs the `R3FactoryMetadata` shape (target = Injectable, deps =
/// the constructor deps) and delegates to `compile_declare_factory_function`.
pub fn compile_declare_factory_for_injectable<'a>(
    allocator: &'a Allocator,
    meta: &R3InjectableMetadata<'a>,
) -> OutputExpression<'a> {
    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: meta.name,
        type_expr: meta.r#type.clone_in(allocator),
        type_decl: meta.r#type.clone_in(allocator),
        type_argument_count: meta.type_argument_count,
        deps: clone_constructor_deps(allocator, meta.deps.as_ref().map(|v| v.as_slice())),
        target: FactoryTarget::Injectable,
    });

    compile_declare_factory_function(allocator, &factory_meta)
}

// ---- helpers ---------------------------------------------------------------

fn provider_deps<'a, 'm>(
    provider: &'m InjectableProvider<'a>,
) -> Option<&'m [R3DependencyMetadata<'a>]> {
    match provider {
        InjectableProvider::UseClass { deps: Some(deps), .. }
        | InjectableProvider::UseFactory { deps: Some(deps), .. } => Some(deps.as_slice()),
        _ => None,
    }
}

fn clone_constructor_deps<'a>(
    allocator: &'a Allocator,
    deps: Option<&[R3DependencyMetadata<'a>]>,
) -> R3FactoryDeps<'a> {
    match deps {
        // No constructor in source. Match upstream's behavior for
        // non-inheriting parameterless classes (`deps: []` → no-arg
        // `new Class()` factory). This is the common case.
        //
        // For a service that extends another class (uncommon enough to
        // ignore), the optimal emit would be `R3FactoryDeps::None` so
        // the linker uses `ɵɵgetInheritedFactory`. R3InjectableMetadata
        // doesn't carry an inheritance flag, so we default to the
        // safe-and-correct empty form. Inheriting services still get a
        // working factory (plain `new`), just without the
        // inherited-factory optimization.
        None => R3FactoryDeps::Valid(Vec::new_in(allocator)),
        Some(deps) => {
            let mut out = Vec::with_capacity_in(deps.len(), allocator);
            for dep in deps {
                out.push(R3DependencyMetadata {
                    token: dep.token.as_ref().map(|t| t.clone_in(allocator)),
                    attribute_name_type: dep
                        .attribute_name_type
                        .as_ref()
                        .map(|a| a.clone_in(allocator)),
                    host: dep.host,
                    optional: dep.optional,
                    self_: dep.self_,
                    skip_self: dep.skip_self,
                    type_only_invalid: dep.type_only_invalid,
                });
            }
            R3FactoryDeps::Valid(out)
        }
    }
}

fn maybe_forward_ref<'a>(
    allocator: &'a Allocator,
    expr: OutputExpression<'a>,
    is_forward_ref: bool,
) -> OutputExpression<'a> {
    if is_forward_ref { wrap_forward_ref(allocator, expr) } else { expr }
}

fn provided_in_expr<'a>(
    allocator: &'a Allocator,
    provided_in: &ProvidedIn<'a>,
) -> Option<OutputExpression<'a>> {
    match provided_in {
        ProvidedIn::None => None,
        ProvidedIn::Root => Some(string_literal(allocator, "root")),
        ProvidedIn::Platform => Some(string_literal(allocator, "platform")),
        ProvidedIn::Any => Some(string_literal(allocator, "any")),
        ProvidedIn::Module(expr) => Some(expr.clone_in(allocator)),
    }
}

fn compile_dependencies_array<'a>(
    allocator: &'a Allocator,
    deps: &[R3DependencyMetadata<'a>],
) -> OutputExpression<'a> {
    let mut entries = Vec::with_capacity_in(deps.len(), allocator);
    for dep in deps {
        entries.push(compile_one_dependency(allocator, dep));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

/// Per-dep map shape. Same as the factory partial's compile_dependency, but
/// kept locally rather than reusing across modules — easier to evolve
/// independently if upstream ever diverges.
fn compile_one_dependency<'a>(
    allocator: &'a Allocator,
    dep: &R3DependencyMetadata<'a>,
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

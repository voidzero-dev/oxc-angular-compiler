//! Partial-declaration emit for `ɵɵngDeclarePipe`.
//!
//! Ported from upstream `packages/compiler/src/render3/partial/pipe.ts:28`
//! (`compileDeclarePipeFromMetadata`).
//!
//! Pipe is the smallest of the structural-decorator partial shapes:
//!
//! ```text
//! i0.ɵɵngDeclarePipe({
//!   minVersion: "14.0.0",
//!   version: "0.0.0-PLACEHOLDER",
//!   ngImport: i0,
//!   type: MyPipe,
//!   isStandalone?: false,   // emitted only when not standalone
//!   name: "myPipe",
//!   pure?: false            // emitted only when not pure
//! })
//! ```
//!
//! The paired `ɵfac` is emitted separately by `super::factory`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::factory::compile_declare_factory_function;
use super::{MIN_VERSION_PIPE, PLACEHOLDER_VERSION};
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata,
};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue,
    OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::pipe::R3PipeMetadata;
use crate::r3::Identifiers;

/// Emits the `ɵɵngDeclarePipe` call for an `@Pipe`'s `ɵpipe` static.
///
/// See: upstream `packages/compiler/src/render3/partial/pipe.ts:28`.
pub fn compile_declare_pipe_from_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3PipeMetadata<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    entries.push(string_entry(allocator, "minVersion", MIN_VERSION_PIPE));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));

    // isStandalone: upstream only emits when defined. OXC's metadata is a
    // plain bool, so emit only when false — the linker defaults to true,
    // matching the v19+ runtime. (Skipping `isStandalone: true` produces a
    // smaller payload that the linker handles identically.)
    if !meta.is_standalone {
        entries.push(LiteralMapEntry::new(
            Ident::from("isStandalone"),
            bool_literal(allocator, false),
            false,
        ));
    }

    // name: required. Prefer pipe_name (template-visible name) over class
    // name. Matches upstream pipe.ts:57 — `meta.pipeName ?? meta.name`.
    let pipe_name = meta.pipe_name.as_ref().unwrap_or(&meta.name);
    entries.push(LiteralMapEntry::new(
        Ident::from("name"),
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(pipe_name.clone()), source_span: None },
            allocator,
        )),
        false,
    ));

    // pure: only when false. Linker default is true.
    if !meta.pure {
        entries.push(LiteralMapEntry::new(
            Ident::from("pure"),
            bool_literal(allocator, false),
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
                namespaced_prop(allocator, "i0", Identifiers::DECLARE_PIPE),
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

/// Builds the partial ɵfac factory paired with this Pipe.
///
/// Mirrors `compile_declare_factory_for_injectable` in
/// `super::injectable` — constructs the `R3FactoryMetadata` shape with
/// `target = Pipe` and delegates to `compile_declare_factory_function`.
pub fn compile_declare_factory_for_pipe<'a>(
    allocator: &'a Allocator,
    meta: &R3PipeMetadata<'a>,
) -> OutputExpression<'a> {
    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: meta.name.clone(),
        type_expr: meta.r#type.clone_in(allocator),
        type_decl: meta.r#type.clone_in(allocator),
        type_argument_count: meta.type_argument_count,
        deps: clone_constructor_deps(allocator, meta.deps.as_ref()),
        target: FactoryTarget::Pipe,
    });

    compile_declare_factory_function(allocator, &factory_meta)
}

// ---- helpers ---------------------------------------------------------------

fn clone_constructor_deps<'a>(
    allocator: &'a Allocator,
    deps: Option<&Vec<'a, crate::pipe::R3DependencyMetadata<'a>>>,
) -> R3FactoryDeps<'a> {
    match deps {
        // No constructor in source. Match upstream's behavior for
        // non-inheriting parameterless classes (`deps: []` → no-arg
        // `new Class()` factory). This is the common case.
        //
        // For a Pipe that DOES extend another class (rare), the optimal
        // emit would be `R3FactoryDeps::None` so the linker calls
        // `ɵɵgetInheritedFactory`. OXC's Pipe metadata doesn't track
        // inheritance today, so we default to the safe-and-correct empty
        // form. An inheriting pipe still gets a working factory (plain
        // `new`), just without the inherited-factory optimization.
        None => R3FactoryDeps::Valid(Vec::new_in(allocator)),
        Some(deps) => {
            let mut out = Vec::with_capacity_in(deps.len(), allocator);
            for dep in deps {
                // pipe::R3DependencyMetadata and factory::R3DependencyMetadata
                // are structurally similar but distinct types. Convert.
                out.push(R3DependencyMetadata {
                    token: dep.token.as_ref().map(|t| (**t).clone_in(allocator)),
                    attribute_name_type: dep
                        .attribute_name_type
                        .as_ref()
                        .map(|a| (**a).clone_in(allocator)),
                    host: dep.host,
                    optional: dep.optional,
                    self_: dep.self_,
                    skip_self: dep.skip_self,
                    // Pipe's R3DependencyMetadata predates the type-only-invalid
                    // tracking (#288); the field doesn't exist on this struct,
                    // so we default to false.
                    type_only_invalid: false,
                });
            }
            R3FactoryDeps::Valid(out)
        }
    }
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

fn bool_literal<'a>(allocator: &'a Allocator, value: bool) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Boolean(value), source_span: None },
        allocator,
    ))
}

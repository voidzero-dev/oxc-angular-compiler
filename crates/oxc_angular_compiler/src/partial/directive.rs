//! Partial-declaration emit for `ɵɵngDeclareDirective`.
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/directive.ts:26`
//! (`compileDeclareDirectiveFromMetadata`).
//!
//! Shape (each field optional unless marked required):
//!
//! ```text
//! i0.ɵɵngDeclareDirective({
//!   minVersion: <see below>,
//!   version: "0.0.0-PLACEHOLDER",
//!   type: <class>,
//!   isStandalone?: false,
//!   isSignal?: true,
//!   selector?: "css-selector",
//!   inputs?: <new-shape or legacy-shape>,
//!   outputs?: { classPropertyName: "bindingName", ... },
//!   host?: { listeners?, properties?, attributes?, styleAttribute?, classAttribute? },
//!   providers?: <expr>,
//!   queries?: [<query-map>],
//!   viewQueries?: [<query-map>],
//!   exportAs?: ["name", ...],
//!   usesInheritance?: true,
//!   usesOnChanges?: true,
//!   hostDirectives?: [{ directive, inputs?, outputs? }],
//!   ngImport: i0    // required, emitted LAST per upstream convention
//! })
//! ```
//!
//! `minVersion` bumps per upstream
//! `directive.ts:129-160`:
//!
//! - Base: `14.0.0`
//! - Any input has a `transformFunction`: `16.1.0`
//! - Any input is signal-based (new inputs shape required): `17.1.0`
//! - Any query (view or content) is signal-based: `17.2.0`

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::factory::compile_declare_factory_function;
use super::{PLACEHOLDER_VERSION, wrap_forward_ref};
use crate::directive::{
    QueryPredicate, R3DirectiveMetadata, R3HostDirectiveMetadata, R3HostMetadata, R3InputMetadata,
    R3QueryMetadata,
};
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata,
};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Emits the `ɵɵngDeclareDirective` call for a directive's `ɵdir` static.
pub fn compile_declare_directive_from_metadata<'a>(
    allocator: &'a Allocator,
    meta: &R3DirectiveMetadata<'a>,
) -> OutputExpression<'a> {
    let entries = create_directive_definition_map(allocator, meta);
    invoke_declare(allocator, Identifiers::DECLARE_DIRECTIVE, entries)
}

/// Builds the directive partial-declaration definition map.
///
/// Exposed so the component partial emitter (in the next slice) can
/// reuse it — upstream's `createComponentDefinitionMap` builds on top of
/// this. The component then adds its own fields and switches the
/// identifier to `ɵɵngDeclareComponent`.
pub(crate) fn create_directive_definition_map<'a>(
    allocator: &'a Allocator,
    meta: &R3DirectiveMetadata<'a>,
) -> Vec<'a, LiteralMapEntry<'a>> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);

    let min_version = compute_min_version(meta);
    entries.push(string_entry(allocator, "minVersion", min_version));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));
    entries.push(LiteralMapEntry::new(Ident::from("type"), meta.r#type.clone_in(allocator), false));

    // isStandalone: emit only when not standalone — matches Pipe/Component
    // convention. Linker defaults to true (v19+ runtime default).
    if !meta.is_standalone {
        entries.push(LiteralMapEntry::new(
            Ident::from("isStandalone"),
            bool_lit(allocator, false),
            false,
        ));
    }

    if meta.is_signal {
        entries.push(LiteralMapEntry::new(
            Ident::from("isSignal"),
            bool_lit(allocator, true),
            false,
        ));
    }

    if let Some(selector) = &meta.selector {
        entries.push(LiteralMapEntry::new(
            Ident::from("selector"),
            string_literal_owned(allocator, selector.clone()),
            false,
        ));
    }

    if !meta.inputs.is_empty() {
        let inputs_expr = if needs_new_input_partial_output(meta) {
            create_inputs_partial_metadata(allocator, &meta.inputs)
        } else {
            legacy_inputs_partial_metadata(allocator, &meta.inputs)
        };
        entries.push(LiteralMapEntry::new(Ident::from("inputs"), inputs_expr, false));
    }

    if !meta.outputs.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("outputs"),
            create_outputs_map(allocator, &meta.outputs),
            false,
        ));
    }

    if let Some(host_expr) = compile_host_metadata(allocator, &meta.host) {
        entries.push(LiteralMapEntry::new(Ident::from("host"), host_expr, false));
    }

    if let Some(providers) = &meta.providers {
        entries.push(LiteralMapEntry::new(
            Ident::from("providers"),
            providers.clone_in(allocator),
            false,
        ));
    }

    if !meta.queries.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("queries"),
            compile_queries_array(allocator, &meta.queries),
            false,
        ));
    }
    if !meta.view_queries.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("viewQueries"),
            compile_queries_array(allocator, &meta.view_queries),
            false,
        ));
    }

    if !meta.export_as.is_empty() {
        let mut elements: Vec<'a, OutputExpression<'a>> =
            Vec::with_capacity_in(meta.export_as.len(), &allocator);
        for name in &meta.export_as {
            elements.push(string_literal_owned(allocator, name.clone()));
        }
        entries.push(LiteralMapEntry::new(
            Ident::from("exportAs"),
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: elements, source_span: None },
                &allocator,
            )),
            false,
        ));
    }

    if meta.uses_inheritance {
        entries.push(LiteralMapEntry::new(
            Ident::from("usesInheritance"),
            bool_lit(allocator, true),
            false,
        ));
    }
    if meta.uses_on_changes {
        entries.push(LiteralMapEntry::new(
            Ident::from("usesOnChanges"),
            bool_lit(allocator, true),
            false,
        ));
    }

    if !meta.host_directives.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("hostDirectives"),
            create_host_directives_array(allocator, &meta.host_directives),
            false,
        ));
    }

    // ngImport is emitted LAST per upstream convention (directive.ts:114).
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));

    entries
}

/// Builds the partial ɵfac factory paired with a Directive.
pub fn compile_declare_factory_for_directive<'a>(
    allocator: &'a Allocator,
    meta: &R3DirectiveMetadata<'a>,
) -> OutputExpression<'a> {
    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: meta.name.clone(),
        type_expr: meta.r#type.clone_in(allocator),
        type_decl: meta.r#type.clone_in(allocator),
        type_argument_count: meta.type_argument_count,
        deps: clone_factory_deps(allocator, &meta.deps, meta.uses_inheritance),
        target: FactoryTarget::Directive,
    });
    compile_declare_factory_function(allocator, &factory_meta)
}

/// Public min-version calculator. Exposed so the component emitter (next
/// slice) can compute its own min-version on top of the directive base.
pub(crate) fn compute_min_version<'a>(meta: &R3DirectiveMetadata<'a>) -> &'static str {
    // Order matters: later bumps win.
    let mut min: &'static str = "14.0.0";

    if meta.inputs.iter().any(|i| i.transform_function.is_some()) {
        min = "16.1.0";
    }
    if needs_new_input_partial_output(meta) {
        min = "17.1.0";
    }
    if meta.queries.iter().any(|q| q.is_signal) || meta.view_queries.iter().any(|q| q.is_signal) {
        min = "17.2.0";
    }

    min
}

fn needs_new_input_partial_output<'a>(meta: &R3DirectiveMetadata<'a>) -> bool {
    meta.inputs.iter().any(|i| i.is_signal)
}

// ---- inputs ---------------------------------------------------------------

/// New input partial output (post-17.1). Each input is an object literal
/// `{ classPropertyName, publicName, isSignal, isRequired, transformFunction }`.
fn create_inputs_partial_metadata<'a>(
    allocator: &'a Allocator,
    inputs: &Vec<'a, R3InputMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(inputs.len(), &allocator);
    for input in inputs {
        let mut input_map: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);
        input_map.push(LiteralMapEntry::new(
            Ident::from("classPropertyName"),
            string_literal_owned(allocator, input.class_property_name.clone()),
            false,
        ));
        input_map.push(LiteralMapEntry::new(
            Ident::from("publicName"),
            string_literal_owned(allocator, input.binding_property_name.clone()),
            false,
        ));
        input_map.push(LiteralMapEntry::new(
            Ident::from("isSignal"),
            bool_lit(allocator, input.is_signal),
            false,
        ));
        input_map.push(LiteralMapEntry::new(
            Ident::from("isRequired"),
            bool_lit(allocator, input.required),
            false,
        ));
        // transformFunction is always emitted as expression or null. Matches
        // upstream NULL_EXPR fallback (directive.ts:291).
        let transform = match &input.transform_function {
            Some(expr) => expr.clone_in(allocator),
            None => OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Null, source_span: None },
                &allocator,
            )),
        };
        input_map.push(LiteralMapEntry::new(Ident::from("transformFunction"), transform, false));

        let key_str = input.class_property_name.as_str();
        let quoted = is_unsafe_object_key(key_str);
        entries.push(LiteralMapEntry::new(
            input.class_property_name.clone(),
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries: input_map, source_span: None },
                &allocator,
            )),
            quoted,
        ));
    }
    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ))
}

/// Legacy input partial output (pre-17.1). Each input is either a string
/// literal (matching publicName == declaredName) or a 2- or 3-tuple
/// `[publicName, declaredName, transformFn?]`.
fn legacy_inputs_partial_metadata<'a>(
    allocator: &'a Allocator,
    inputs: &Vec<'a, R3InputMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(inputs.len(), &allocator);
    for input in inputs {
        let declared = &input.class_property_name;
        let public = &input.binding_property_name;
        let value = if declared.as_str() != public.as_str() || input.transform_function.is_some() {
            let mut tuple: Vec<'a, OutputExpression<'a>> = Vec::new_in(&allocator);
            tuple.push(string_literal_owned(allocator, public.clone()));
            tuple.push(string_literal_owned(allocator, declared.clone()));
            if let Some(transform) = &input.transform_function {
                tuple.push(transform.clone_in(allocator));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: tuple, source_span: None },
                &allocator,
            ))
        } else {
            string_literal_owned(allocator, public.clone())
        };

        let key_str = declared.as_str();
        let quoted = is_unsafe_object_key(key_str);
        entries.push(LiteralMapEntry::new(declared.clone(), value, quoted));
    }
    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ))
}

// ---- outputs --------------------------------------------------------------

fn create_outputs_map<'a>(
    allocator: &'a Allocator,
    outputs: &Vec<'a, (Ident<'a>, Ident<'a>)>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> =
        Vec::with_capacity_in(outputs.len(), &allocator);
    for (class_name, binding_name) in outputs {
        let quoted = is_unsafe_object_key(class_name.as_str());
        entries.push(LiteralMapEntry::new(
            class_name.clone(),
            string_literal_owned(allocator, binding_name.clone()),
            quoted,
        ));
    }
    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ))
}

// ---- host -----------------------------------------------------------------

fn compile_host_metadata<'a>(
    allocator: &'a Allocator,
    host: &R3HostMetadata<'a>,
) -> Option<OutputExpression<'a>> {
    if !host.has_bindings() {
        return None;
    }
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);

    // Order matches upstream directive.ts:212-224: attributes, listeners,
    // properties, styleAttribute, classAttribute.
    if !host.attributes.is_empty() {
        let mut attr_entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);
        for (key, value) in &host.attributes {
            let quoted = is_unsafe_object_key(key.as_str());
            attr_entries.push(LiteralMapEntry::new(key.clone(), value.clone_in(allocator), quoted));
        }
        entries.push(LiteralMapEntry::new(
            Ident::from("attributes"),
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries: attr_entries, source_span: None },
                &allocator,
            )),
            false,
        ));
    }
    if !host.listeners.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("listeners"),
            ident_pairs_to_string_map(allocator, &host.listeners),
            false,
        ));
    }
    if !host.properties.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("properties"),
            ident_pairs_to_string_map(allocator, &host.properties),
            false,
        ));
    }
    if let Some(style_attr) = &host.style_attr {
        entries.push(LiteralMapEntry::new(
            Ident::from("styleAttribute"),
            string_literal_owned(allocator, style_attr.clone()),
            false,
        ));
    }
    if let Some(class_attr) = &host.class_attr {
        entries.push(LiteralMapEntry::new(
            Ident::from("classAttribute"),
            string_literal_owned(allocator, class_attr.clone()),
            false,
        ));
    }

    Some(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    )))
}

fn ident_pairs_to_string_map<'a>(
    allocator: &'a Allocator,
    pairs: &Vec<'a, (Ident<'a>, Ident<'a>)>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(pairs.len(), &allocator);
    for (key, value) in pairs {
        let quoted = is_unsafe_object_key(key.as_str());
        entries.push(LiteralMapEntry::new(
            key.clone(),
            string_literal_owned(allocator, value.clone()),
            quoted,
        ));
    }
    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ))
}

// ---- queries --------------------------------------------------------------

fn compile_queries_array<'a>(
    allocator: &'a Allocator,
    queries: &Vec<'a, R3QueryMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, OutputExpression<'a>> =
        Vec::with_capacity_in(queries.len(), &allocator);
    for q in queries {
        entries.push(compile_query(allocator, q));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        &allocator,
    ))
}

fn compile_query<'a>(allocator: &'a Allocator, q: &R3QueryMetadata<'a>) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);

    entries.push(LiteralMapEntry::new(
        Ident::from("propertyName"),
        string_literal_owned(allocator, q.property_name.clone()),
        false,
    ));
    if q.first {
        entries.push(LiteralMapEntry::new(Ident::from("first"), bool_lit(allocator, true), false));
    }
    // predicate: type expression OR string array of selectors. (Forward-ref
    // wrapping on a Type predicate is not tracked in the local metadata,
    // so we emit the expression verbatim. If we add forward-ref tracking
    // to QueryPredicate::Type, this is the place to wrap.)
    let predicate_expr = match &q.predicate {
        QueryPredicate::Type(expr) => expr.clone_in(allocator),
        QueryPredicate::Selectors(selectors) => {
            let mut elements: Vec<'a, OutputExpression<'a>> =
                Vec::with_capacity_in(selectors.len(), &allocator);
            for s in selectors {
                elements.push(string_literal_owned(allocator, s.clone()));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: elements, source_span: None },
                &allocator,
            ))
        }
    };
    entries.push(LiteralMapEntry::new(Ident::from("predicate"), predicate_expr, false));

    if !q.emit_distinct_changes_only {
        entries.push(LiteralMapEntry::new(
            Ident::from("emitDistinctChangesOnly"),
            bool_lit(allocator, false),
            false,
        ));
    }
    if q.descendants {
        entries.push(LiteralMapEntry::new(
            Ident::from("descendants"),
            bool_lit(allocator, true),
            false,
        ));
    }
    if let Some(read) = &q.read {
        entries.push(LiteralMapEntry::new(Ident::from("read"), read.clone_in(allocator), false));
    }
    if q.is_static {
        entries.push(LiteralMapEntry::new(Ident::from("static"), bool_lit(allocator, true), false));
    }
    if q.is_signal {
        entries.push(LiteralMapEntry::new(
            Ident::from("isSignal"),
            bool_lit(allocator, true),
            false,
        ));
    }

    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ))
}

// ---- host directives ------------------------------------------------------

fn create_host_directives_array<'a>(
    allocator: &'a Allocator,
    host_directives: &Vec<'a, R3HostDirectiveMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, OutputExpression<'a>> =
        Vec::with_capacity_in(host_directives.len(), &allocator);
    for hd in host_directives {
        let mut hd_entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(&allocator);
        let directive_expr = if hd.is_forward_reference {
            wrap_forward_ref(allocator, hd.directive.clone_in(allocator))
        } else {
            hd.directive.clone_in(allocator)
        };
        hd_entries.push(LiteralMapEntry::new(Ident::from("directive"), directive_expr, false));

        if !hd.inputs.is_empty() {
            hd_entries.push(LiteralMapEntry::new(
                Ident::from("inputs"),
                host_directives_mapping_array(allocator, &hd.inputs),
                false,
            ));
        }
        if !hd.outputs.is_empty() {
            hd_entries.push(LiteralMapEntry::new(
                Ident::from("outputs"),
                host_directives_mapping_array(allocator, &hd.outputs),
                false,
            ));
        }
        entries.push(OutputExpression::LiteralMap(Box::new_in(
            LiteralMapExpr { entries: hd_entries, source_span: None },
            &allocator,
        )));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        &allocator,
    ))
}

/// Mirrors upstream `createHostDirectivesMappingArray` — flat alternating
/// `[publicName, alias, publicName, alias, ...]` string array.
fn host_directives_mapping_array<'a>(
    allocator: &'a Allocator,
    pairs: &Vec<'a, (Ident<'a>, Ident<'a>)>,
) -> OutputExpression<'a> {
    let mut elements: Vec<'a, OutputExpression<'a>> =
        Vec::with_capacity_in(pairs.len() * 2, &allocator);
    for (public_name, alias) in pairs {
        elements.push(string_literal_owned(allocator, public_name.clone()));
        elements.push(string_literal_owned(allocator, alias.clone()));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: elements, source_span: None },
        &allocator,
    ))
}

// ---- low-level helpers ----------------------------------------------------

fn clone_factory_deps<'a>(
    allocator: &'a Allocator,
    deps: &Option<Vec<'a, R3DependencyMetadata<'a>>>,
    uses_inheritance: bool,
) -> R3FactoryDeps<'a> {
    match deps {
        Some(deps) => {
            let mut out = Vec::with_capacity_in(deps.len(), &allocator);
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
        None => {
            // Match full-mode behavior in directive/definition.rs:163-173:
            // None + inheritance → inherited factory; None + no inheritance
            // → empty-deps factory.
            if uses_inheritance {
                R3FactoryDeps::None
            } else {
                R3FactoryDeps::Valid(Vec::new_in(&allocator))
            }
        }
    }
}

/// Object keys containing `.` or `-` need quoting. Mirrors upstream
/// `UNSAFE_OBJECT_KEY_NAME_REGEXP = /[-.]/` at `render3/view/util.ts`.
fn is_unsafe_object_key(key: &str) -> bool {
    key.contains('.') || key.contains('-')
}

fn invoke_declare<'a>(
    allocator: &'a Allocator,
    name: &'static str,
    entries: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        &allocator,
    ));
    let mut args = Vec::new_in(&allocator);
    args.push(map_expr);
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(namespaced_prop(allocator, "i0", name), &allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}

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

fn string_literal_owned<'a>(allocator: &'a Allocator, value: Ident<'a>) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(value), source_span: None },
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

fn bool_lit<'a>(allocator: &'a Allocator, value: bool) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Boolean(value), source_span: None },
        &allocator,
    ))
}

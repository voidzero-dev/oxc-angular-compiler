//! Partial-declaration emit for `ɵɵngDeclareComponent`.
//!
//! Ported from upstream
//! `packages/compiler/src/render3/partial/component.ts:66`
//! (`compileDeclareComponentFromMetadata`).
//!
//! The component partial extends the directive partial with these
//! component-specific fields (upstream component.ts:82-156, applied AFTER
//! the directive map is built):
//!
//! ```text
//! i0.ɵɵngDeclareComponent({
//!   ...all directive fields,
//!   template: "<verbatim html>",
//!   isInline?: true,
//!   styles?: ["..."],
//!   dependencies?: [{ kind, type, selector?, name? }],
//!   viewProviders?: <expr>,
//!   animations?: <expr>,
//!   changeDetection?: i0.ChangeDetectionStrategy.OnPush,
//!   encapsulation?: i0.ViewEncapsulation.None | ShadowDom,
//!   preserveWhitespaces?: true
//! })
//! ```
//!
//! `minVersion` is bumped to `17.0.0` when the template contains any
//! block syntax (`@if`/`@for`/`@switch`/`@defer`) per upstream
//! `component.ts:98-102`.
//!
//! Partial mode keeps the template as a **verbatim string** — no
//! parsing, no AST, no instruction emission. The linker re-parses on the
//! consumer side.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::factory::compile_declare_factory_function;
use super::{PLACEHOLDER_VERSION, wrap_forward_ref};
use crate::component::R3DependencyMetadata as ComponentDep;
use crate::component::{
    ChangeDetectionStrategy, ComponentMetadata, DeclarationListEmitMode, HostDirectiveMetadata,
    HostMetadata, TemplateDependency, TemplateDependencyKind, ViewEncapsulation,
};
use crate::directive::R3InputMetadata;
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata,
};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::pipe::R3DependencyMetadata as PipeDep;
use crate::r3::Identifiers;

/// Inputs the partial Component emitter needs that aren't carried on the
/// `ComponentMetadata` struct directly. Mirrors what the full-mode
/// pipeline computes separately and threads in.
pub struct PartialComponentInputs<'a> {
    /// The verbatim template source (inline literal or external file
    /// content). Partial mode emits this as a string literal — the linker
    /// re-parses it.
    pub template: &'a str,
    /// Whether the template came from an inline `template: '...'` literal
    /// (`true`) versus an external `templateUrl` (`false`).
    pub is_inline: bool,
}

/// Emits the `ɵɵngDeclareComponent` call for a component's `ɵcmp` static.
pub fn compile_declare_component_from_metadata<'a>(
    allocator: &'a Allocator,
    meta: &ComponentMetadata<'a>,
    inputs: &PartialComponentInputs<'a>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    let min_version = compute_min_version(meta, inputs.template);
    entries.push(string_entry(allocator, "minVersion", min_version));
    entries.push(string_entry(allocator, "version", PLACEHOLDER_VERSION));

    let type_expr = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: meta.class_name.clone(), source_span: None },
        allocator,
    ));
    entries.push(LiteralMapEntry::new(Ident::from("type"), type_expr, false));

    // isStandalone — emit only when not standalone. Matches the
    // Pipe/Directive convention; linker defaults to true.
    if !meta.standalone {
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
        let inputs_expr = if needs_new_input_partial_output(&meta.inputs) {
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

    // Host — Component carries Option<HostMetadata>. Upstream emits when
    // any host sub-field is populated; same here.
    if let Some(host) = &meta.host
        && let Some(host_expr) = compile_host_metadata(allocator, host)
    {
        entries.push(LiteralMapEntry::new(Ident::from("host"), host_expr, false));
    }

    if let Some(providers) = &meta.providers {
        entries.push(LiteralMapEntry::new(
            Ident::from("providers"),
            providers.clone_in(allocator),
            false,
        ));
    }

    if !meta.export_as.is_empty() {
        let mut elements: Vec<'a, OutputExpression<'a>> =
            Vec::with_capacity_in(meta.export_as.len(), allocator);
        for name in &meta.export_as {
            elements.push(string_literal_owned(allocator, name.clone()));
        }
        entries.push(LiteralMapEntry::new(
            Ident::from("exportAs"),
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: elements, source_span: None },
                allocator,
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
    if meta.lifecycle.uses_on_changes {
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

    // ngImport closes the directive map (component.ts:114). Component-
    // specific fields come AFTER ngImport — that's how upstream emits
    // them (createComponentDefinitionMap calls createDirectiveDefinitionMap
    // first, then appends).
    entries.push(LiteralMapEntry::new(Ident::from("ngImport"), read_var(allocator, "i0"), false));

    // ---- Component-specific fields ----

    entries.push(LiteralMapEntry::new(
        Ident::from("template"),
        string_literal_str(allocator, inputs.template),
        false,
    ));

    if inputs.is_inline {
        entries.push(LiteralMapEntry::new(
            Ident::from("isInline"),
            bool_lit(allocator, true),
            false,
        ));
    }

    if !meta.styles.is_empty() {
        let mut elements: Vec<'a, OutputExpression<'a>> =
            Vec::with_capacity_in(meta.styles.len(), allocator);
        for s in &meta.styles {
            elements.push(string_literal_owned(allocator, s.clone()));
        }
        entries.push(LiteralMapEntry::new(
            Ident::from("styles"),
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: elements, source_span: None },
                allocator,
            )),
            false,
        ));
    }

    if !meta.declarations.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("dependencies"),
            create_dependencies_array(
                allocator,
                &meta.declarations,
                meta.declaration_list_emit_mode,
            ),
            false,
        ));
    }

    if let Some(view_providers) = &meta.view_providers {
        entries.push(LiteralMapEntry::new(
            Ident::from("viewProviders"),
            view_providers.clone_in(allocator),
            false,
        ));
    }
    if let Some(animations) = &meta.animations {
        entries.push(LiteralMapEntry::new(
            Ident::from("animations"),
            animations.clone_in(allocator),
            false,
        ));
    }

    // changeDetection: emit only when != Default (Default is the runtime
    // default and matches upstream's "null" handling).
    if meta.change_detection != ChangeDetectionStrategy::default() {
        let variant = match meta.change_detection {
            ChangeDetectionStrategy::Default => "Default",
            ChangeDetectionStrategy::OnPush => "OnPush",
        };
        entries.push(LiteralMapEntry::new(
            Ident::from("changeDetection"),
            namespaced_enum_member(allocator, "ChangeDetectionStrategy", variant),
            false,
        ));
    }

    // encapsulation: emit only when != Emulated (the default).
    if meta.encapsulation != ViewEncapsulation::default() {
        let variant = match meta.encapsulation {
            ViewEncapsulation::Emulated => "Emulated",
            ViewEncapsulation::None => "None",
            ViewEncapsulation::ShadowDom => "ShadowDom",
        };
        entries.push(LiteralMapEntry::new(
            Ident::from("encapsulation"),
            namespaced_enum_member(allocator, "ViewEncapsulation", variant),
            false,
        ));
    }

    if meta.preserve_whitespaces {
        entries.push(LiteralMapEntry::new(
            Ident::from("preserveWhitespaces"),
            bool_lit(allocator, true),
            false,
        ));
    }

    invoke_declare(allocator, Identifiers::DECLARE_COMPONENT, entries)
}

/// Builds the partial ɵfac factory paired with a Component.
pub fn compile_declare_factory_for_component<'a>(
    allocator: &'a Allocator,
    meta: &ComponentMetadata<'a>,
) -> OutputExpression<'a> {
    let type_expr = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: meta.class_name.clone(), source_span: None },
        allocator,
    ));

    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: meta.class_name.clone(),
        type_expr: type_expr.clone_in(allocator),
        type_decl: type_expr,
        type_argument_count: 0,
        deps: clone_factory_deps(allocator, &meta.constructor_deps, meta.uses_inheritance),
        target: FactoryTarget::Component,
    });
    compile_declare_factory_function(allocator, &factory_meta)
}

// ---- min version ---------------------------------------------------------

fn compute_min_version<'a>(meta: &ComponentMetadata<'a>, template: &str) -> &'static str {
    let mut min: &'static str = "14.0.0";

    if meta.inputs.iter().any(|i| i.transform_function.is_some()) {
        min = "16.1.0";
    }
    // Component bump: control-flow blocks in template.
    if template_uses_blocks(template) {
        min = bump(min, "17.0.0");
    }
    if needs_new_input_partial_output(&meta.inputs) {
        min = bump(min, "17.1.0");
    }
    // Signal queries — components carry these on the same path as
    // directives, but the metadata struct doesn't include them
    // directly. The dispatch layer that calls us will know if any query
    // is signal-based; for now we approximate by checking inputs only.
    // If signal queries land on ComponentMetadata later, bump 17.2.0
    // here.

    min
}

/// Returns the highest (most-recent) version between `current` and
/// `candidate`. Simple lexicographic comparison works for the version
/// strings we use ("14.0.0" < "17.0.0" < "17.1.0" < "17.2.0").
fn bump(current: &'static str, candidate: &'static str) -> &'static str {
    if candidate > current { candidate } else { current }
}

/// Cheap check for template control-flow block syntax — matches upstream
/// `BlockPresenceVisitor` at component.ts:242-288 in intent (presence of
/// any `@if`, `@for`, `@switch`, or `@defer` block). False positives are
/// fine: the linker accepts any minVersion ≥ what it needs.
fn template_uses_blocks(template: &str) -> bool {
    template.contains("@if")
        || template.contains("@for")
        || template.contains("@switch")
        || template.contains("@defer")
}

// ---- inputs / outputs (duplicated from partial::directive — they take
//      slightly different metadata sources) ---------------------------------

fn needs_new_input_partial_output<'a>(inputs: &Vec<'a, R3InputMetadata<'a>>) -> bool {
    inputs.iter().any(|i| i.is_signal)
}

fn create_inputs_partial_metadata<'a>(
    allocator: &'a Allocator,
    inputs: &Vec<'a, R3InputMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(inputs.len(), allocator);
    for input in inputs {
        let mut input_map: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);
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
        let transform = match &input.transform_function {
            Some(expr) => expr.clone_in(allocator),
            None => null_lit(allocator),
        };
        input_map.push(LiteralMapEntry::new(Ident::from("transformFunction"), transform, false));

        let key = input.class_property_name.clone();
        let quoted = is_unsafe_object_key(key.as_str());
        entries.push(LiteralMapEntry::new(
            key,
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries: input_map, source_span: None },
                allocator,
            )),
            quoted,
        ));
    }
    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ))
}

fn legacy_inputs_partial_metadata<'a>(
    allocator: &'a Allocator,
    inputs: &Vec<'a, R3InputMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(inputs.len(), allocator);
    for input in inputs {
        let declared = &input.class_property_name;
        let public = &input.binding_property_name;
        let value = if declared.as_str() != public.as_str() || input.transform_function.is_some() {
            let mut tuple: Vec<'a, OutputExpression<'a>> = Vec::new_in(allocator);
            tuple.push(string_literal_owned(allocator, public.clone()));
            tuple.push(string_literal_owned(allocator, declared.clone()));
            if let Some(transform) = &input.transform_function {
                tuple.push(transform.clone_in(allocator));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: tuple, source_span: None },
                allocator,
            ))
        } else {
            string_literal_owned(allocator, public.clone())
        };
        let quoted = is_unsafe_object_key(declared.as_str());
        entries.push(LiteralMapEntry::new(declared.clone(), value, quoted));
    }
    OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ))
}

fn create_outputs_map<'a>(
    allocator: &'a Allocator,
    outputs: &Vec<'a, (Ident<'a>, Ident<'a>)>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(outputs.len(), allocator);
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
        allocator,
    ))
}

// ---- host ----------------------------------------------------------------

/// Component carries host bindings as raw `(key, value)` string pairs
/// where the key still includes any binding syntax (`[class.x]`,
/// `(click)`, …). The linker parses these into Ivy host instructions.
fn compile_host_metadata<'a>(
    allocator: &'a Allocator,
    host: &HostMetadata<'a>,
) -> Option<OutputExpression<'a>> {
    if host.properties.is_empty()
        && host.attributes.is_empty()
        && host.listeners.is_empty()
        && host.class_attr.is_none()
        && host.style_attr.is_none()
    {
        return None;
    }

    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);

    if !host.attributes.is_empty() {
        entries.push(LiteralMapEntry::new(
            Ident::from("attributes"),
            ident_pairs_to_string_map(allocator, &host.attributes),
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
        allocator,
    )))
}

fn ident_pairs_to_string_map<'a>(
    allocator: &'a Allocator,
    pairs: &Vec<'a, (Ident<'a>, Ident<'a>)>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, LiteralMapEntry<'a>> = Vec::with_capacity_in(pairs.len(), allocator);
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
        allocator,
    ))
}

// ---- host directives ------------------------------------------------------

fn create_host_directives_array<'a>(
    allocator: &'a Allocator,
    host_directives: &Vec<'a, HostDirectiveMetadata<'a>>,
) -> OutputExpression<'a> {
    let mut entries: Vec<'a, OutputExpression<'a>> =
        Vec::with_capacity_in(host_directives.len(), allocator);
    for hd in host_directives {
        let mut hd_entries: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);
        let directive_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: hd.directive.clone(), source_span: None },
            allocator,
        ));
        let directive_expr = if hd.is_forward_reference {
            wrap_forward_ref(allocator, directive_expr)
        } else {
            directive_expr
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
            allocator,
        )));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

fn host_directives_mapping_array<'a>(
    allocator: &'a Allocator,
    pairs: &Vec<'a, (Ident<'a>, Ident<'a>)>,
) -> OutputExpression<'a> {
    let mut elements: Vec<'a, OutputExpression<'a>> =
        Vec::with_capacity_in(pairs.len() * 2, allocator);
    for (public_name, alias) in pairs {
        elements.push(string_literal_owned(allocator, public_name.clone()));
        elements.push(string_literal_owned(allocator, alias.clone()));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: elements, source_span: None },
        allocator,
    ))
}

// ---- dependencies --------------------------------------------------------

fn create_dependencies_array<'a>(
    allocator: &'a Allocator,
    deps: &Vec<'a, TemplateDependency<'a>>,
    emit_mode: DeclarationListEmitMode,
) -> OutputExpression<'a> {
    let wrap_in_forward_ref = matches!(
        emit_mode,
        DeclarationListEmitMode::Closure | DeclarationListEmitMode::ClosureResolved
    );

    let mut entries: Vec<'a, OutputExpression<'a>> = Vec::with_capacity_in(deps.len(), allocator);
    for dep in deps {
        let mut dep_map: Vec<'a, LiteralMapEntry<'a>> = Vec::new_in(allocator);
        let kind = match dep.kind {
            TemplateDependencyKind::Directive => {
                if dep.is_component {
                    "component"
                } else {
                    "directive"
                }
            }
            TemplateDependencyKind::Pipe => "pipe",
            TemplateDependencyKind::NgModule => "ngmodule",
        };
        dep_map.push(LiteralMapEntry::new(
            Ident::from("kind"),
            string_literal_static(allocator, kind),
            false,
        ));

        let mut type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: dep.type_name.clone(), source_span: None },
            allocator,
        ));
        if wrap_in_forward_ref || dep.is_forward_reference {
            type_expr = wrap_forward_ref(allocator, type_expr);
        }
        dep_map.push(LiteralMapEntry::new(Ident::from("type"), type_expr, false));

        match dep.kind {
            TemplateDependencyKind::Directive => {
                if let Some(selector) = &dep.selector {
                    dep_map.push(LiteralMapEntry::new(
                        Ident::from("selector"),
                        string_literal_owned(allocator, selector.clone()),
                        false,
                    ));
                }
                if !dep.inputs.is_empty() {
                    dep_map.push(LiteralMapEntry::new(
                        Ident::from("inputs"),
                        ident_array_string_literals(allocator, &dep.inputs),
                        false,
                    ));
                }
                if !dep.outputs.is_empty() {
                    dep_map.push(LiteralMapEntry::new(
                        Ident::from("outputs"),
                        ident_array_string_literals(allocator, &dep.outputs),
                        false,
                    ));
                }
                if !dep.export_as.is_empty() {
                    dep_map.push(LiteralMapEntry::new(
                        Ident::from("exportAs"),
                        ident_array_string_literals(allocator, &dep.export_as),
                        false,
                    ));
                }
            }
            TemplateDependencyKind::Pipe => {
                let pipe_name = dep.pipe_name.as_ref().unwrap_or(&dep.type_name);
                dep_map.push(LiteralMapEntry::new(
                    Ident::from("name"),
                    string_literal_owned(allocator, pipe_name.clone()),
                    false,
                ));
            }
            TemplateDependencyKind::NgModule => {}
        }

        entries.push(OutputExpression::LiteralMap(Box::new_in(
            LiteralMapExpr { entries: dep_map, source_span: None },
            allocator,
        )));
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

fn ident_array_string_literals<'a>(
    allocator: &'a Allocator,
    names: &Vec<'a, Ident<'a>>,
) -> OutputExpression<'a> {
    let mut elements: Vec<'a, OutputExpression<'a>> = Vec::with_capacity_in(names.len(), allocator);
    for name in names {
        elements.push(string_literal_owned(allocator, name.clone()));
    }
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: elements, source_span: None },
        allocator,
    ))
}

// ---- factory --------------------------------------------------------------

/// Converts `component::dependency::R3DependencyMetadata` (which stores
/// tokens as bare `Ident`s) into the `factory::R3DependencyMetadata` shape
/// (which stores tokens as `OutputExpression`s) by wrapping each token in
/// a `ReadVar`.
fn clone_factory_deps<'a>(
    allocator: &'a Allocator,
    deps: &Option<Vec<'a, ComponentDep<'a>>>,
    uses_inheritance: bool,
) -> R3FactoryDeps<'a> {
    match deps {
        Some(deps) => {
            let mut out = Vec::with_capacity_in(deps.len(), allocator);
            for dep in deps {
                let token_expr = dep.token.as_ref().map(|t| {
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: t.clone(), source_span: None },
                        allocator,
                    ))
                });
                let attr_expr = dep.attribute_name.as_ref().map(|a| {
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::String(a.clone()), source_span: None },
                        allocator,
                    ))
                });
                out.push(R3DependencyMetadata {
                    token: token_expr,
                    attribute_name_type: attr_expr,
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
            if uses_inheritance {
                R3FactoryDeps::None
            } else {
                R3FactoryDeps::Valid(Vec::new_in(allocator))
            }
        }
    }
}

// Suppress unused warning for type imported only to disambiguate names in
// the public surface — keeps the API surface stable when we later need it.
#[allow(dead_code)]
fn _unused_imports_marker(_: PipeDep<'_>) {}

// ---- low-level helpers ----------------------------------------------------

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

/// Builds `i0.<enum_name>.<variant>`, e.g.
/// `i0.ChangeDetectionStrategy.OnPush`.
fn namespaced_enum_member<'a>(
    allocator: &'a Allocator,
    enum_name: &'static str,
    variant: &'static str,
) -> OutputExpression<'a> {
    let enum_ref = namespaced_prop(allocator, "i0", enum_name);
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(enum_ref, allocator),
            name: Ident::from(variant),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

fn string_literal_static<'a>(
    allocator: &'a Allocator,
    value: &'static str,
) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(value)), source_span: None },
        allocator,
    ))
}

fn string_literal_owned<'a>(allocator: &'a Allocator, value: Ident<'a>) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(value), source_span: None },
        allocator,
    ))
}

fn string_literal_str<'a>(allocator: &'a Allocator, value: &str) -> OutputExpression<'a> {
    let owned = allocator.alloc_str(value);
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(owned)), source_span: None },
        allocator,
    ))
}

fn string_entry<'a>(
    allocator: &'a Allocator,
    key: &'static str,
    value: &'static str,
) -> LiteralMapEntry<'a> {
    LiteralMapEntry::new(Ident::from(key), string_literal_static(allocator, value), false)
}

fn bool_lit<'a>(allocator: &'a Allocator, value: bool) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Boolean(value), source_span: None },
        allocator,
    ))
}

fn null_lit<'a>(allocator: &'a Allocator) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Null, source_span: None },
        allocator,
    ))
}

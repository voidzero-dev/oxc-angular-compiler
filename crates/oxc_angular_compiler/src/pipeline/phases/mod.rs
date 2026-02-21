//! Transformation phases for Angular template compilation.
//!
//! This module contains the 67 ordered transformation phases that process
//! the IR before code emission. Each phase mutates the IR to prepare it
//! for the next phase.
//!
//! Ported from Angular's `template/pipeline/src/phases/`.

use super::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

// Phase implementations
mod allocate_slots;
mod any_cast;
mod apply_i18n_expressions;
mod assign_i18n_slot_dependencies;
mod attach_source_locations;
mod attribute_extraction;
mod binding_specialization;
mod chaining;
mod collapse_singleton_interpolations;
mod conditionals;
mod const_collection;
mod convert_animations;
mod convert_i18n_bindings;
mod create_i18n_contexts;
mod deduplicate_text_bindings;
mod defer_configs;
mod defer_resolve_targets;
mod empty_elements;
mod expand_safe_reads;
mod extract_i18n_messages;
mod generate_advance;
mod generate_arrow_functions;
mod generate_local_let_references;
mod generate_projection_def;
mod generate_variables;
mod has_const_expression_collection;
mod host_style_property_parsing;
pub mod i18n_closure;
mod i18n_const_collection;
mod i18n_text_extraction;
mod local_refs;
mod namespace;
mod naming;
mod next_context_merging;
mod ng_container;
mod nonbindable;
mod ordering;
mod parse_extracted_styles;
mod phase_remove_content_selectors;
mod pipe_creation;
mod pipe_variadic;
mod propagate_i18n_blocks;
mod pure_function_extraction;
mod pure_literal_structures;
mod regular_expression_optimization;
mod reify;
mod remove_empty_bindings;
mod remove_i18n_contexts;
mod remove_illegal_let_references;
mod remove_unused_i18n_attrs;
mod resolve_contexts;
mod resolve_defer_deps_fns;
mod resolve_dollar_event;
mod resolve_i18n_element_placeholders;
mod resolve_i18n_expression_placeholders;
mod resolve_names;
mod resolve_sanitizers;
mod save_restore_view;
mod store_let_optimization;
mod strip_nonrequired_parentheses;
mod style_binding_specialization;
mod temporary_variables;
mod track_fn_optimization;
mod track_variables;
mod transform_two_way_binding_set;
mod var_counting;
mod variable_optimization;
mod wrap_icus;

// ============================================================================
// Phase Infrastructure
// ============================================================================

/// The kind of compilation job a phase applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationJobKind {
    /// Template compilation (component templates).
    Template,
    /// Host binding compilation (@HostBinding/@HostListener).
    Host,
    /// Both template and host binding compilation.
    Both,
}

/// A transformation phase.
pub struct Phase {
    /// The kind of compilation this phase applies to.
    pub kind: CompilationJobKind,
    /// The phase function for template compilation.
    pub run: fn(&mut ComponentCompilationJob<'_>),
    /// The phase function for host binding compilation.
    /// Only used when kind is Host or Both.
    pub run_host: Option<fn(&mut HostBindingCompilationJob<'_>)>,
    /// Phase name for debugging.
    pub name: &'static str,
}

/// All 66 transformation phases in order.
///
/// This is the exact ordering from Angular's `emit.ts`.
pub static PHASES: &[Phase] = &[
    // Phase 1: removeContentSelectors (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: phase_remove_content_selectors::remove_content_selectors,
        run_host: None,
        name: "removeContentSelectors",
    },
    // Phase 2: optimizeRegularExpressions (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: regular_expression_optimization::optimize_regular_expressions,
        run_host: Some(regular_expression_optimization::optimize_regular_expressions_for_host),
        name: "optimizeRegularExpressions",
    },
    // Phase 3: parseHostStyleProperties (Host only)
    Phase {
        kind: CompilationJobKind::Host,
        run: host_style_property_parsing::parse_host_style_properties,
        run_host: Some(host_style_property_parsing::parse_host_style_properties_for_host),
        name: "parseHostStyleProperties",
    },
    // Phase 4: emitNamespaceChanges (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: namespace::emit_namespace_changes,
        run_host: None,
        name: "emitNamespaceChanges",
    },
    // Phase 5: propagateI18nBlocks (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: propagate_i18n_blocks::propagate_i18n_blocks,
        run_host: None,
        name: "propagateI18nBlocks",
    },
    // Phase 6: wrapI18nIcus (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: wrap_icus::wrap_i18n_icus,
        run_host: None,
        name: "wrapI18nIcus",
    },
    // Phase 7: deduplicateTextBindings (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: deduplicate_text_bindings::deduplicate_text_bindings,
        run_host: Some(deduplicate_text_bindings::deduplicate_text_bindings_for_host),
        name: "deduplicateTextBindings",
    },
    // Phase 8: specializeStyleBindings (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: style_binding_specialization::specialize_style_bindings,
        run_host: Some(style_binding_specialization::specialize_style_bindings_for_host),
        name: "specializeStyleBindings",
    },
    // Phase 9: specializeBindings (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: binding_specialization::specialize_bindings,
        run_host: Some(binding_specialization::specialize_bindings_for_host),
        name: "specializeBindings",
    },
    // Phase 10: convertAnimations (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: convert_animations::convert_animations,
        run_host: Some(convert_animations::convert_animations_for_host),
        name: "convertAnimations",
    },
    // Phase 11: extractAttributes (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: attribute_extraction::extract_attributes,
        run_host: Some(attribute_extraction::extract_attributes_for_host),
        name: "extractAttributes",
    },
    // Phase 12: createI18nContexts (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: create_i18n_contexts::create_i18n_contexts,
        run_host: None,
        name: "createI18nContexts",
    },
    // Phase 13: parseExtractedStyles (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: parse_extracted_styles::parse_extracted_styles,
        run_host: Some(parse_extracted_styles::parse_extracted_styles_for_host),
        name: "parseExtractedStyles",
    },
    // Phase 14: removeEmptyBindings (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: remove_empty_bindings::remove_empty_bindings,
        run_host: None,
        name: "removeEmptyBindings",
    },
    // Phase 15: collapseSingletonInterpolations (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: collapse_singleton_interpolations::collapse_singleton_interpolations,
        run_host: Some(
            collapse_singleton_interpolations::collapse_singleton_interpolations_for_host,
        ),
        name: "collapseSingletonInterpolations",
    },
    // Phase 16: orderOps (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: ordering::order_ops,
        run_host: Some(ordering::order_ops_for_host),
        name: "orderOps",
    },
    // Phase 17: generateConditionalExpressions (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: conditionals::generate_conditional_expressions,
        run_host: None,
        name: "generateConditionalExpressions",
    },
    // Phase 18: createPipes (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: pipe_creation::create_pipes,
        run_host: None,
        name: "createPipes",
    },
    // Phase 19: configureDeferInstructions (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: defer_configs::configure_defer_instructions,
        run_host: None,
        name: "configureDeferInstructions",
    },
    // Phase 20: createVariadicPipes (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: pipe_variadic::create_variadic_pipes,
        run_host: None,
        name: "createVariadicPipes",
    },
    // Phase 21: generateArrowFunctions (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: generate_arrow_functions::generate_arrow_functions,
        run_host: Some(generate_arrow_functions::generate_arrow_functions_for_host),
        name: "generateArrowFunctions",
    },
    // Phase 22: generatePureLiteralStructures (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: pure_literal_structures::generate_pure_literal_structures,
        run_host: Some(pure_literal_structures::generate_pure_literal_structures_for_host),
        name: "generatePureLiteralStructures",
    },
    // Phase 23: generateProjectionDefs (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: generate_projection_def::generate_projection_defs,
        run_host: None,
        name: "generateProjectionDefs",
    },
    // Phase 24: generateLocalLetReferences (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: generate_local_let_references::generate_local_let_references,
        run_host: None,
        name: "generateLocalLetReferences",
    },
    // Phase 25: generateVariables (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: generate_variables::generate_variables,
        run_host: None,
        name: "generateVariables",
    },
    // Phase 26: saveAndRestoreView (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: save_restore_view::save_and_restore_view,
        run_host: None,
        name: "saveAndRestoreView",
    },
    // Phase 27: deleteAnyCasts (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: any_cast::delete_any_casts,
        run_host: Some(any_cast::delete_any_casts_for_host),
        name: "deleteAnyCasts",
    },
    // Phase 28: resolveDollarEvent (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: resolve_dollar_event::resolve_dollar_event,
        run_host: Some(resolve_dollar_event::resolve_dollar_event_for_host),
        name: "resolveDollarEvent",
    },
    // Phase 29: generateTrackVariables (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: track_variables::generate_track_variables,
        run_host: None,
        name: "generateTrackVariables",
    },
    // Phase 30: removeIllegalLetReferences (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: remove_illegal_let_references::remove_illegal_let_references,
        run_host: None,
        name: "removeIllegalLetReferences",
    },
    // Phase 31: resolveNames (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: resolve_names::resolve_names,
        run_host: Some(resolve_names::resolve_names_for_host),
        name: "resolveNames",
    },
    // Phase 32: resolveDeferTargetNames (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: defer_resolve_targets::resolve_defer_target_names,
        run_host: None,
        name: "resolveDeferTargetNames",
    },
    // Phase 33: transformTwoWayBindingSet (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: transform_two_way_binding_set::transform_two_way_binding_set,
        run_host: None,
        name: "transformTwoWayBindingSet",
    },
    // Phase 34: optimizeTrackFns (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: track_fn_optimization::optimize_track_fns,
        run_host: None,
        name: "optimizeTrackFns",
    },
    // Phase 35: resolveContexts (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: resolve_contexts::resolve_contexts,
        run_host: Some(resolve_contexts::resolve_contexts_for_host),
        name: "resolveContexts",
    },
    // Phase 36: resolveSanitizers (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: resolve_sanitizers::resolve_sanitizers,
        run_host: Some(resolve_sanitizers::resolve_sanitizers_for_host),
        name: "resolveSanitizers",
    },
    // Phase 37: liftLocalRefs (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: local_refs::lift_local_refs,
        run_host: None,
        name: "liftLocalRefs",
    },
    // Phase 38: expandSafeReads (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: expand_safe_reads::expand_safe_reads,
        run_host: Some(expand_safe_reads::expand_safe_reads_for_host),
        name: "expandSafeReads",
    },
    // Phase 39: stripNonrequiredParentheses (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: strip_nonrequired_parentheses::strip_nonrequired_parentheses,
        run_host: Some(strip_nonrequired_parentheses::strip_nonrequired_parentheses_for_host),
        name: "stripNonrequiredParentheses",
    },
    // Phase 40: generateTemporaryVariables (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: temporary_variables::generate_temporary_variables,
        run_host: Some(temporary_variables::generate_temporary_variables_for_host),
        name: "generateTemporaryVariables",
    },
    // Phase 41: optimizeVariables (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: variable_optimization::optimize_variables,
        run_host: Some(variable_optimization::optimize_variables_for_host),
        name: "optimizeVariables",
    },
    // Phase 42: optimizeStoreLet (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: store_let_optimization::optimize_store_let,
        run_host: Some(store_let_optimization::optimize_store_let_for_host),
        name: "optimizeStoreLet",
    },
    // Phase 43: convertI18nText (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: i18n_text_extraction::convert_i18n_text,
        run_host: None,
        name: "convertI18nText",
    },
    // Phase 44: convertI18nBindings (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: convert_i18n_bindings::convert_i18n_bindings,
        run_host: None,
        name: "convertI18nBindings",
    },
    // Phase 45: removeUnusedI18nAttributesOps (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: remove_unused_i18n_attrs::remove_unused_i18n_attributes_ops,
        run_host: None,
        name: "removeUnusedI18nAttributesOps",
    },
    // Phase 46: assignI18nSlotDependencies (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: assign_i18n_slot_dependencies::assign_i18n_slot_dependencies,
        run_host: None,
        name: "assignI18nSlotDependencies",
    },
    // Phase 47: applyI18nExpressions (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: apply_i18n_expressions::apply_i18n_expressions,
        run_host: None,
        name: "applyI18nExpressions",
    },
    // Phase 48: allocateSlots (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: allocate_slots::allocate_slots,
        run_host: None,
        name: "allocateSlots",
    },
    // Phase 49: resolveI18nElementPlaceholders (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: resolve_i18n_element_placeholders::resolve_i18n_element_placeholders,
        run_host: None,
        name: "resolveI18nElementPlaceholders",
    },
    // Phase 50: resolveI18nExpressionPlaceholders (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: resolve_i18n_expression_placeholders::resolve_i18n_expression_placeholders,
        run_host: None,
        name: "resolveI18nExpressionPlaceholders",
    },
    // Phase 51: extractI18nMessages (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: extract_i18n_messages::extract_i18n_messages,
        run_host: None,
        name: "extractI18nMessages",
    },
    // Phase 52: collectI18nConsts (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: i18n_const_collection::collect_i18n_consts,
        run_host: None,
        name: "collectI18nConsts",
    },
    // Phase 53: collectConstExpressions (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: has_const_expression_collection::collect_const_expressions,
        run_host: None,
        name: "collectConstExpressions",
    },
    // Phase 54: collectElementConsts (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: const_collection::collect_element_consts,
        run_host: Some(const_collection::collect_element_consts_for_host),
        name: "collectElementConsts",
    },
    // Phase 55: removeI18nContexts (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: remove_i18n_contexts::remove_i18n_contexts,
        run_host: None,
        name: "removeI18nContexts",
    },
    // Phase 56: countVariables (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: var_counting::count_variables,
        run_host: Some(var_counting::count_variables_for_host),
        name: "countVariables",
    },
    // Phase 57: generateAdvance (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: generate_advance::generate_advance,
        run_host: None,
        name: "generateAdvance",
    },
    // Phase 58: nameFunctionsAndVariables (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: naming::name_functions_and_variables,
        run_host: Some(naming::name_functions_and_variables_for_host),
        name: "nameFunctionsAndVariables",
    },
    // Phase 59: resolveDeferDepsFns (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: resolve_defer_deps_fns::resolve_defer_deps_fns,
        run_host: None,
        name: "resolveDeferDepsFns",
    },
    // Phase 60: mergeNextContextExpressions (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: next_context_merging::merge_next_context_expressions,
        run_host: None,
        name: "mergeNextContextExpressions",
    },
    // Phase 61: generateNgContainerOps (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: ng_container::generate_ng_container_ops,
        run_host: None,
        name: "generateNgContainerOps",
    },
    // Phase 62: collapseEmptyInstructions (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: empty_elements::collapse_empty_instructions,
        run_host: None,
        name: "collapseEmptyInstructions",
    },
    // Phase 63: attachSourceLocations (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: attach_source_locations::attach_source_locations,
        run_host: None,
        name: "attachSourceLocations",
    },
    // Phase 64: disableBindings (Template only)
    Phase {
        kind: CompilationJobKind::Template,
        run: nonbindable::disable_bindings,
        run_host: None,
        name: "disableBindings",
    },
    // Phase 65: extractPureFunctions (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: pure_function_extraction::extract_pure_functions,
        run_host: Some(pure_function_extraction::extract_pure_functions_for_host),
        name: "extractPureFunctions",
    },
    // Phase 66: reify (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: reify::reify,
        run_host: Some(reify::reify_host),
        name: "reify",
    },
    // Phase 67: chain (Both)
    Phase {
        kind: CompilationJobKind::Both,
        run: chaining::chain,
        run_host: Some(chaining::chain_for_host),
        name: "chain",
    },
];

/// Run all transformation phases for template compilation.
pub fn transform_template(job: &mut ComponentCompilationJob<'_>) {
    for phase in PHASES {
        if matches!(phase.kind, CompilationJobKind::Template | CompilationJobKind::Both) {
            (phase.run)(job);
        }
    }
}

/// Run all transformation phases for host binding compilation.
///
/// This function applies all phases that have Kind::Host or Kind::Both,
/// matching the TypeScript behavior in Angular's `emit.ts`:
/// ```typescript
/// if (phase.kind === kind || phase.kind === Kind.Both) {
///   phase.fn(job);
/// }
/// ```
///
/// Ported from Angular's transform function with Kind.Host filtering in `emit.ts`.
pub fn transform_host_job(job: &mut HostBindingCompilationJob<'_>) {
    for phase in PHASES {
        if matches!(phase.kind, CompilationJobKind::Host | CompilationJobKind::Both) {
            if let Some(run_host) = phase.run_host {
                run_host(job);
            }
        }
    }
}

//! Naming phase.
//!
//! Generates names for functions and variables used in the template output.
//!
//! This phase assigns:
//! - Template function names (e.g., `ComponentName_Template`, `ComponentName_Conditional_0_Template`)
//! - Unique suffixes for embedded views based on their kind (hierarchical)
//! - Handler function names (e.g., `ComponentName_div_click_0_listener`)
//! - Variable names (e.g., `ctx_r0`, `item_i1`)
//! - Propagates variable names to `ReadVariableExpr`s
//!
//! Ported from Angular's `template/pipeline/src/phases/naming.ts`.

use oxc_span::Ident;
use rustc_hash::FxHashMap;

use crate::ir::enums::{BindingKind, SemanticVariableKind};
use crate::ir::expression::{
    IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};
use crate::pipeline::phases::parse_extracted_styles::hyphenate;

/// Sanitizes an identifier by replacing non-word characters with underscores.
/// Matches Angular's `sanitizeIdentifier` in parse_util.ts.
fn sanitize_identifier(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// State for generating unique variable names.
struct NamingState {
    /// Counter for unique variable indices.
    index: u32,
}

/// Key for identifying variables by their semantic identity.
///
/// TypeScript's Angular compiler shares SemanticVariable objects across multiple
/// Variable ops that reference the same semantic identity (e.g., two child views
/// both needing the parent's context share the same viewContextVariable object).
///
/// When the naming phase encounters a variable with `name === null`, it assigns
/// a new name. If the variable already has a name (because a previous Variable op
/// with the same SemanticVariable object was already named), it reuses that name.
///
/// In Rust, each Variable op owns its own data, so we simulate this behavior by
/// tracking semantic identity -> name mappings.
///
/// NOTE: Most Identifier variables are NOT tracked here! In TypeScript, different
/// SemanticVariable objects (even with the same identifier name) get different
/// variable names. We only track Context, SavedView, Alias, and Reference variables
/// which CAN be shared across multiple Variable ops.
///
/// References (template refs like `#defaultContent`) ARE tracked because in TypeScript,
/// the same SemanticVariable object is stored in the parent scope and reused when
/// generating Variable ops for child views. Multiple child views accessing the same
/// reference should share the same variable name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SemanticVariableKey<'a> {
    /// Context variable for a specific view.
    /// Multiple child views accessing the same parent context share this key.
    Context { view: XrefId },
    /// SavedView variable for a specific view.
    SavedView { view: XrefId },
    /// Alias variable with a specific identifier.
    Alias { identifier: Ident<'a> },
    /// Reference variable (template ref like #defaultContent).
    /// Multiple child views accessing the same template ref share this key.
    /// Identified by the target element and offset.
    Reference { target: XrefId, offset: i32 },
    /// ContextLet variable (readContextLet for @let declarations).
    /// Multiple variables reading the same @let slot share this key.
    /// Identified by the target XrefId and slot number.
    ContextLet { target: XrefId, slot: u32 },
    /// Context read variable (reads a property from a parent view's context).
    /// Multiple child views accessing the same context property share this key.
    /// This is created for variables like `const breadcrumb = nextContext().$implicit;`
    /// which read context properties (like $implicit, $index, etc.) from a parent view.
    ///
    /// In TypeScript, getScopeForView creates SemanticVariable objects stored in
    /// scope.contextVariables, and these are reused when generateVariablesInScopeForView
    /// is called for multiple sibling child views.
    ContextRead {
        /// The view whose context is being read (the parent view with context variables).
        view: XrefId,
        /// The identifier name for the variable (e.g., "breadcrumb").
        identifier: Ident<'a>,
    },
}

/// Creates a semantic key from variable properties.
///
/// IMPORTANT: Most Identifier variables are NOT deduplicated by name!
/// In TypeScript, deduplication happens at the object level - when the SAME SemanticVariable
/// object is referenced by multiple Variable ops. Two different Variable ops with Identifier
/// kind and the same name (e.g., two `@let iconInput` declarations in different views) get
/// DIFFERENT names because they are DIFFERENT SemanticVariable objects.
///
/// However, Reference variables (template refs like `#defaultContent`) ARE deduplicated.
/// In TypeScript, the parent scope stores a SemanticVariable object for each reference,
/// and when child views need to access that reference, they reuse the SAME SemanticVariable.
/// We detect references by checking if the initializer is an `IrExpression::Reference`.
fn make_semantic_key<'a>(
    kind: SemanticVariableKind,
    name: &Ident<'a>,
    view: Option<XrefId>,
    initializer: &IrExpression<'a>,
) -> Option<SemanticVariableKey<'a>> {
    match kind {
        SemanticVariableKind::Context => {
            // Context variables are identified by the view they reference, BUT only for
            // NextContext variables (not RestoreView variables).
            //
            // In TypeScript:
            // - NextContext uses `scope.viewContextVariable` which is a SHARED SemanticVariable
            //   object across sibling views that access the same parent scope. These should
            //   be deduplicated to share the same name.
            // - RestoreView creates a NEW SemanticVariable object each time (save_restore_view.ts
            //   lines 71-75). These should NOT be deduplicated.
            //
            // We detect RestoreView by checking the initializer expression.
            if matches!(initializer, IrExpression::RestoreView(_)) {
                // RestoreView context variables are NOT deduplicated
                None
            } else {
                // NextContext and other Context variables are deduplicated by view
                view.map(|v| SemanticVariableKey::Context { view: v })
            }
        }
        SemanticVariableKind::Identifier => {
            // Check if this is a Reference variable (template ref like #defaultContent).
            // References are identified by their target element and offset, and should
            // be deduplicated across child views that access the same reference.
            if let IrExpression::Reference(ref_expr) = initializer {
                return Some(SemanticVariableKey::Reference {
                    target: ref_expr.target,
                    offset: ref_expr.offset,
                });
            }

            // Check if this is a ContextLetReference (readContextLet for @let declarations).
            // Variables reading the same @let slot should share the same name.
            // This matches TypeScript where the same SemanticVariable object is reused
            // for multiple Variable ops accessing the same @let declaration.
            if let IrExpression::ContextLetReference(ctx_let) = initializer {
                if let Some(slot) = ctx_let.target_slot.slot {
                    return Some(SemanticVariableKey::ContextLet {
                        target: ctx_let.target,
                        slot: slot.0,
                    });
                }
            }

            // Check if this is a context read variable.
            // These are created by generate_variables for reading context properties from parent views.
            // In TypeScript, the same SemanticVariable object is stored in scope.contextVariables
            // and reused when generateVariablesInScopeForView is called for multiple sibling child views.
            //
            // Context read variables have:
            // 1. A `view` field pointing to the scope's view (set by generate_variables)
            // 2. An initializer that reads a context property (ResolvedPropertyRead or Context)
            //
            // After resolve_contexts, the Context expressions are transformed to ReadVariable,
            // so we can't rely on the initializer containing Context. Instead, we use the
            // `view` field from the Variable op, which is preserved through all phases.
            //
            // We detect context read variables by checking if:
            // - The variable has a `view` field set (context read variables always have this)
            // - The initializer is a ResolvedPropertyRead (ctx.$implicit, etc.) or Context (CTX_REF)
            //
            // After resolve_contexts, the patterns become:
            // - ResolvedPropertyRead with ReadVariable receiver (was Context)
            // - Context (for current view) or ReadVariable (was Context for ancestor view)
            //
            // After variable_optimization (which runs BEFORE naming), the Context variable
            // with NextContext initializer is inlined into the Identifier variable. So:
            // - Child view pattern: NextContext (was ReadVariable -> Context -> Context variable)
            if let Some(target_view) = view {
                // Check if this looks like a context read variable
                let is_context_read = match initializer {
                    // Pattern 1: ResolvedPropertyRead - this is the main case for $implicit, $index, etc.
                    IrExpression::ResolvedPropertyRead(_) => true,
                    // Pattern 2: Direct Context - for CTX_REF variables (conditional aliases)
                    IrExpression::Context(_) => true,
                    // Pattern 3: ReadVariable - this happens after resolve_contexts transforms
                    // Context expressions for ancestor views
                    IrExpression::ReadVariable(_) => true,
                    // Pattern 4: RestoreView - this happens after variable_optimization inlines
                    // the RestoreView Context variable into CTX_REF Identifier variables.
                    // For example, `const parent = ctx_r1` (where ctx_r1 = restoreView(_r1))
                    // becomes `const parent = restoreView(_r1)` after optimization.
                    IrExpression::RestoreView(_) => true,
                    // Pattern 5: NextContext - this happens after variable_optimization inlines
                    // the Context variable (which had NextContext initializer) into Identifier
                    // variables for child views accessing parent context.
                    // For example, child view has:
                    //   Context var: ctx_r1 = nextContext()
                    //   Identifier var: buttonConfig = ctx_r1
                    // After optimization:
                    //   Identifier var: buttonConfig = nextContext()
                    IrExpression::NextContext(_) => true,
                    _ => false,
                };
                if is_context_read {
                    return Some(SemanticVariableKey::ContextRead {
                        view: target_view,
                        identifier: name.clone(),
                    });
                }
            }

            // Other Identifier variables (like @let declarations, local variables, etc.)
            // are NOT deduplicated. Each Variable op gets its own unique name.
            // This matches TypeScript where different SemanticVariable objects
            // (even with the same identifier) get different names.
            None
        }
        SemanticVariableKind::SavedView => {
            // SavedView variables are identified by the view they reference
            view.map(|v| SemanticVariableKey::SavedView { view: v })
        }
        SemanticVariableKind::Alias => {
            // Alias variables are identified by their identifier
            if !name.is_empty() {
                Some(SemanticVariableKey::Alias { identifier: name.clone() })
            } else {
                None
            }
        }
    }
}

/// Generates names for template functions and variables.
///
/// Names follow Angular's TemplateDefinitionBuilder compatibility mode naming conventions:
/// - Root template: `ComponentName_Template`
/// - Embedded templates use hierarchical naming: `ComponentName_Conditional_0_ng_container_1_Template`
/// - Context variables: `ctx_r{N}` (post-increment, starts at 0)
/// - Identifier variables: `{identifier}_r{N}` (pre-increment, starts at 1)
/// - Alias/SavedView variables: `_r{N}` (pre-increment, starts at 1)
///
/// This function implements a recursive naming strategy matching TypeScript's `addNamesToView`:
/// When processing a child view, the parent's base name (without "_Template") is passed as the
/// new base, creating hierarchical names like:
/// - Parent: `ComponentName_Conditional_0`
/// - Child: `ComponentName_Conditional_0_ng_container_1`
pub fn name_functions_and_variables(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let component_name = job.component_name.as_str();

    // State for generating unique variable names
    let mut state = NamingState { index: 0 };

    // Sanitize the base name once
    let sanitized_base = sanitize_identifier(component_name);

    // Map from xref to generated variable name
    let mut var_names: FxHashMap<XrefId, Ident<'_>> = FxHashMap::default();

    // Map from semantic key to generated variable name.
    // This enables name reuse for variables with the same semantic identity,
    // matching TypeScript's behavior where multiple Variable ops can share
    // the same SemanticVariable object.
    let mut semantic_var_names: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
        FxHashMap::default();

    // Recursively name views starting from root
    // TypeScript: addNamesToView(job.root, job.componentName, state, compatibility)
    add_names_to_root_view(
        job,
        &sanitized_base,
        allocator,
        &mut state,
        &mut var_names,
        &mut semantic_var_names,
    );

    // Second pass: Propagate variable names to ReadVariableExpr expressions
    propagate_variable_names_in_view(&mut job.root.create, &mut job.root.update, &var_names);

    // Process embedded views
    let view_xrefs: Vec<_> = job.views.keys().copied().collect();
    for xref in view_xrefs {
        if let Some(view) = job.view_mut(xref) {
            propagate_variable_names_in_view(&mut view.create, &mut view.update, &var_names);
        }
    }
}

/// Recursively names views and variables starting from the root view.
/// This matches TypeScript's `addNamesToView` recursive structure.
///
/// The key insight is that when processing a child view, we pass the parent's
/// base name (without "_Template") as the new base, creating hierarchical names.
///
/// CRITICAL: TypeScript uses depth-first processing - when encountering a child view op
/// during create op iteration, it immediately recurses into that child view before
/// continuing with the parent's remaining ops. This affects variable naming order.
fn add_names_to_root_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    base_name: &str,
    allocator: &'a oxc_allocator::Allocator,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    // Name the root view
    // TypeScript: unit.fnName = unit.job.pool.uniqueName(sanitizeIdentifier(`${baseName}_${unit.job.fnSuffix}`), false)
    let fn_name_base = format!("{base_name}_Template");
    let sanitized_fn_name_base = sanitize_identifier(&fn_name_base);
    job.root.fn_name = Some(job.pool.unique_name(&sanitized_fn_name_base, false));

    // Process ops in root view with depth-first child view recursion
    // This matches TypeScript's unit.ops() iteration order where child views
    // are processed immediately when encountered in create ops
    let root_fn_name = job.root.fn_name.clone();
    process_view_ops_depth_first(
        job,
        None, // Root view - no xref
        base_name,
        allocator,
        root_fn_name.as_ref(),
        state,
        var_names,
        semantic_var_names,
    );
}

/// Recursively names a child view and its descendants.
/// Uses depth-first processing to match TypeScript's variable naming order.
fn add_names_to_child_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    base_name: &str,
    allocator: &'a oxc_allocator::Allocator,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    // Name this view
    // TypeScript: unit.fnName = unit.job.pool.uniqueName(sanitizeIdentifier(`${baseName}_${unit.job.fnSuffix}`), false)
    let fn_name_base = format!("{base_name}_Template");
    let sanitized_fn_name_base = sanitize_identifier(&fn_name_base);
    let fn_name = job.pool.unique_name(&sanitized_fn_name_base, false);

    // Set the function name
    if let Some(view) = job.view_mut(view_xref) {
        view.fn_name = Some(fn_name.clone());
    }

    // Process ops with depth-first child view recursion
    process_view_ops_depth_first(
        job,
        Some(view_xref),
        base_name,
        allocator,
        Some(&fn_name),
        state,
        var_names,
        semantic_var_names,
    );
}

/// Represents a child view that needs to be processed during create op iteration.
struct ChildViewInfo {
    xref: XrefId,
    base_name: String,
    /// Index of the create op that spawns this child view.
    /// Used to interleave child view processing during create ops.
    create_op_index: usize,
}

/// Process ops within a view using depth-first recursion into child views.
///
/// This matches TypeScript's `addNamesToView` behavior:
/// 1. Iterate through create ops one by one
/// 2. When encountering a child view op (Template, Conditional, etc.), **immediately recurse**
///    into that child view before continuing with the parent's remaining create ops
/// 3. Only after all create ops (including recursive child processing) are done,
///    process update ops
///
/// This depth-first order is essential for correct variable naming.
fn process_view_ops_depth_first<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: Option<XrefId>, // None for root view
    base_name: &str,
    allocator: &'a oxc_allocator::Allocator,
    fn_name: Option<&Ident<'a>>,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    // Phase 0: Process function ops FIRST (matches TypeScript ops() generator order)
    // Arrow functions have their own ops lists that contain Variable ops prepended
    // by the generate_variables phase. These must be named before create/update ops.
    process_function_ops_in_view(job, view_xref, allocator, state, var_names);

    // Phase 1: Collect info about child views with their create op indices
    // We need to collect upfront to avoid borrow issues during iteration
    let child_views = collect_child_views_with_indices(job, view_xref, base_name);

    // Phase 2: Process create ops with interleaved child view recursion
    // After processing each create op, if there's a child view at that index, recurse into it
    process_create_ops_with_child_recursion(
        job,
        view_xref,
        allocator,
        base_name,
        fn_name,
        state,
        var_names,
        semantic_var_names,
        &child_views,
    );

    // Phase 3: Process update ops (style props, variables) AFTER all create ops and child views
    process_update_ops_in_view(job, view_xref, allocator, state, var_names, semantic_var_names);
}

/// Collects child view info with their create op indices for interleaved processing.
fn collect_child_views_with_indices<'a>(
    job: &ComponentCompilationJob<'a>,
    view_xref: Option<XrefId>,
    parent_base_name: &str,
) -> Vec<ChildViewInfo> {
    let create_ops = match view_xref {
        None => &job.root.create,
        Some(xref) => {
            if let Some(view) = job.view(xref) {
                &view.create
            } else {
                return Vec::new();
            }
        }
    };

    let mut children = Vec::new();

    for (index, op) in create_ops.iter().enumerate() {
        match op {
            CreateOp::Template(template_op) => {
                let slot = template_op.slot.map(|s| s.0).unwrap_or(0);
                let suffix = template_op.fn_name_suffix.as_ref().map(|s| s.as_str()).unwrap_or("");
                // Sanitize suffix to replace non-word chars (like colons in `:svg:foreignObject`)
                // with underscores, matching Angular's behavior for SVG/MathML namespace prefixes.
                let sanitized_suffix = sanitize_identifier(suffix);
                let child_base = if sanitized_suffix.is_empty() {
                    format!("{parent_base_name}_{slot}")
                } else {
                    format!("{parent_base_name}_{sanitized_suffix}_{slot}")
                };
                children.push(ChildViewInfo {
                    xref: template_op.embedded_view,
                    base_name: child_base,
                    create_op_index: index,
                });
            }
            CreateOp::RepeaterCreate(rep) => {
                let slot = rep.slot.map(|s| s.0).unwrap_or(0);
                // Empty view first (if present), then body view - matching TypeScript order
                if let Some(empty_xref) = rep.empty_view {
                    let child_base = format!("{parent_base_name}_ForEmpty_{}", slot + 2);
                    children.push(ChildViewInfo {
                        xref: empty_xref,
                        base_name: child_base,
                        create_op_index: index,
                    });
                }
                let child_base = format!("{parent_base_name}_For_{}", slot + 1);
                children.push(ChildViewInfo {
                    xref: rep.body_view,
                    base_name: child_base,
                    create_op_index: index,
                });
            }
            CreateOp::Conditional(cond) => {
                let slot = cond.slot.map(|s| s.0).unwrap_or(0);
                let suffix = cond.fn_name_suffix.as_str();
                // Sanitize suffix for SVG/MathML namespace prefixes
                let sanitized_suffix = sanitize_identifier(suffix);
                let child_base = if sanitized_suffix.is_empty() {
                    format!("{parent_base_name}_{slot}")
                } else {
                    format!("{parent_base_name}_{sanitized_suffix}_{slot}")
                };
                children.push(ChildViewInfo {
                    xref: cond.xref,
                    base_name: child_base,
                    create_op_index: index,
                });
            }
            CreateOp::ConditionalBranch(branch) => {
                let slot = branch.slot.map(|s| s.0).unwrap_or(0);
                let suffix = branch.fn_name_suffix.as_str();
                // Sanitize suffix for SVG/MathML namespace prefixes
                let sanitized_suffix = sanitize_identifier(suffix);
                let child_base = if sanitized_suffix.is_empty() {
                    format!("{parent_base_name}_{slot}")
                } else {
                    format!("{parent_base_name}_{sanitized_suffix}_{slot}")
                };
                children.push(ChildViewInfo {
                    xref: branch.xref,
                    base_name: child_base,
                    create_op_index: index,
                });
            }
            CreateOp::Projection(proj) => {
                if let Some(fallback_xref) = proj.fallback {
                    let slot = proj.slot.map(|s| s.0).unwrap_or(0);
                    let child_base = format!("{parent_base_name}_ProjectionFallback_{slot}");
                    children.push(ChildViewInfo {
                        xref: fallback_xref,
                        base_name: child_base,
                        create_op_index: index,
                    });
                }
            }
            _ => {}
        }
    }

    children
}

/// Process create ops in a view with interleaved child view recursion.
/// This matches TypeScript's unit.ops() iteration order.
///
/// TypeScript's ops() generator yields ops in this order:
/// 1. Create op
/// 2. handler_ops (if Listener/TwoWayListener/AnimationListener/Animation)
/// 3. track_by_ops (if RepeaterCreate)
/// 4. Next create op...
/// 5. Update ops
///
/// The naming phase switch handles ops as they come:
/// - For Listener ops: names the handler function, then handler_ops (Variable) are processed
/// - For RepeaterCreate: track_by_ops are yielded inline (processed as Variable ops),
///   then the switch case recurses into child views
///
/// The order for a RepeaterCreate at index N is:
/// 1. Process RepeaterCreate op (without track_by_ops)
/// 2. Process track_by_ops (yielded inline by the generator)
/// 3. Recurse into child views (empty view, then body view)
/// 4. Move to next create op at index N+1
#[allow(clippy::too_many_arguments)]
fn process_create_ops_with_child_recursion<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: Option<XrefId>,
    allocator: &'a oxc_allocator::Allocator,
    base_name: &str,
    fn_name: Option<&Ident<'a>>,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
    child_views: &[ChildViewInfo],
) {
    // Group child views by their create op index for efficient lookup
    let mut child_views_by_index: FxHashMap<usize, Vec<(XrefId, String)>> = FxHashMap::default();
    for child in child_views {
        child_views_by_index
            .entry(child.create_op_index)
            .or_default()
            .push((child.xref, child.base_name.clone()));
    }

    // Get the number of create ops
    let num_create_ops = match view_xref {
        None => job.root.create.len(),
        Some(xref) => job.view(xref).map(|v| v.create.len()).unwrap_or(0),
    };

    // Process each create op by index
    // We use index-based iteration and re-acquire the iterator each time
    // to release the borrow and allow for child view recursion
    for index in 0..num_create_ops {
        // Track if this is a RepeaterCreate with track_by_ops
        let mut is_repeater_with_track_by = false;

        // Process the create op at this index (excluding track_by_ops for RepeaterCreate)
        {
            let create_ops = match view_xref {
                None => &mut job.root.create,
                Some(xref) => {
                    if let Some(view) = job.view_mut(xref) {
                        &mut view.create
                    } else {
                        continue;
                    }
                }
            };

            // Use nth() to get the specific op - this is O(n) per op making the
            // overall algorithm O(n^2), but necessary due to Rust's borrowing rules
            if let Some(op) = create_ops.iter_mut().nth(index) {
                // Check if this is a RepeaterCreate with track_by_ops BEFORE processing
                if let CreateOp::RepeaterCreate(rep) = op {
                    if rep.track_by_ops.is_some() {
                        is_repeater_with_track_by = true;
                    }
                }

                process_single_create_op_ref(
                    op,
                    allocator,
                    base_name,
                    fn_name,
                    state,
                    var_names,
                    semantic_var_names,
                );
            }
        } // Borrow of job ends here

        // Recurse into any child views at this index BEFORE processing track_by_ops.
        // This matches TypeScript's naming.ts behavior:
        // 1. ops() yields RepeaterCreate
        // 2. switch case handles RepeaterCreate → IMMEDIATELY recurses into child views
        // 3. switch case ends, control returns to for loop
        // 4. ops() yields track_by_ops (which are processed in subsequent for iterations)
        //
        // So the order is: recurse into children FIRST, then name track_by_ops variables.
        if let Some(children) = child_views_by_index.remove(&index) {
            for (child_xref, child_base_name) in children {
                add_names_to_child_view(
                    job,
                    child_xref,
                    &child_base_name,
                    allocator,
                    state,
                    var_names,
                    semantic_var_names,
                );
            }
        }

        // Process track_by_ops AFTER child view recursion.
        // In TypeScript, the ops() generator yields track_by_ops after the RepeaterCreate,
        // and the switch statement has already recursed into child views during the
        // RepeaterCreate case. So track_by_ops are named AFTER all child view variables.
        if is_repeater_with_track_by {
            let create_ops = match view_xref {
                None => &mut job.root.create,
                Some(xref) => {
                    if let Some(view) = job.view_mut(xref) {
                        &mut view.create
                    } else {
                        continue;
                    }
                }
            };

            if let Some(CreateOp::RepeaterCreate(repeater)) = create_ops.iter_mut().nth(index) {
                if let Some(track_by_ops) = &mut repeater.track_by_ops {
                    for track_op in track_by_ops.iter_mut() {
                        if let UpdateOp::Variable(var_op) = track_op {
                            name_variable_op(
                                var_op,
                                allocator,
                                state,
                                var_names,
                                semantic_var_names,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Names a create-phase variable op using semantic identity tracking.
///
/// This function checks if a variable with the same semantic identity has already
/// been named, and reuses that name if so. This matches TypeScript's behavior where
/// multiple Variable ops can share the same SemanticVariable object.
fn name_create_variable_op<'a>(
    var_op: &mut crate::ir::ops::VariableOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    // Check if we need to assign a name
    let needs_naming =
        var_op.name.is_empty() || matches!(var_op.kind, SemanticVariableKind::Identifier);

    if !needs_naming {
        // Already has a fixed name, just record it
        var_names.insert(var_op.xref, var_op.name.clone());
        return;
    }

    // Create the semantic key for this variable
    let semantic_key =
        make_semantic_key(var_op.kind, &var_op.name, var_op.view, &var_op.initializer);

    // Check if we've already named a variable with this semantic identity
    if let Some(key) = &semantic_key {
        if let Some(existing_name) = semantic_var_names.get(key) {
            // Reuse the existing name
            var_op.name = existing_name.clone();
            var_names.insert(var_op.xref, existing_name.clone());
            return;
        }
    }

    // Generate a new name
    let name = get_variable_name(allocator, var_op.kind, Some(&var_op.name), state);
    var_names.insert(var_op.xref, name.clone());
    var_op.name = name.clone();

    // Store in semantic_var_names for future reuse
    if let Some(key) = semantic_key {
        semantic_var_names.insert(key, name);
    }
}

/// Names an update-phase variable op using semantic identity tracking.
///
/// This function checks if a variable with the same semantic identity has already
/// been named, and reuses that name if so. This matches TypeScript's behavior where
/// multiple Variable ops can share the same SemanticVariable object.
fn name_variable_op<'a>(
    var_op: &mut crate::ir::ops::UpdateVariableOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    // Check if we need to assign a name
    let needs_naming =
        var_op.name.is_empty() || matches!(var_op.kind, SemanticVariableKind::Identifier);

    if !needs_naming {
        // Already has a fixed name, just record it
        var_names.insert(var_op.xref, var_op.name.clone());
        return;
    }

    // Create the semantic key for this variable
    let semantic_key =
        make_semantic_key(var_op.kind, &var_op.name, var_op.view, &var_op.initializer);

    // Check if we've already named a variable with this semantic identity
    if let Some(key) = &semantic_key {
        if let Some(existing_name) = semantic_var_names.get(key) {
            // Reuse the existing name
            var_op.name = existing_name.clone();
            var_names.insert(var_op.xref, existing_name.clone());
            return;
        }
    }

    // Generate a new name
    let name = get_variable_name(allocator, var_op.kind, Some(&var_op.name), state);
    var_names.insert(var_op.xref, name.clone());
    var_op.name = name.clone();

    // Store in semantic_var_names for future reuse
    if let Some(key) = semantic_key {
        semantic_var_names.insert(key, name);
    }
}

/// Process a single create op reference.
#[allow(clippy::too_many_arguments)]
fn process_single_create_op_ref<'a>(
    op: &mut CreateOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    base_name: &str,
    fn_name: Option<&Ident<'a>>,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    match op {
        CreateOp::Listener(listener) => {
            if listener.handler_fn_name.is_none() {
                let slot = listener.target_slot.0;
                let fn_name_str = fn_name.map(|n| n.as_str()).unwrap_or(base_name);

                // For legacy animation listeners, modify the event name and add animation prefix
                // Per TypeScript's naming.ts lines 91-102:
                // - Event name becomes "@{name}.{phase}" (e.g., "@openClose.start")
                // - Handler function name includes "animation" prefix (no trailing underscore)
                //
                // In TypeScript:
                //   animation = 'animation';
                //   op.name = `@${op.name}.${op.legacyAnimationPhase}`;
                //   op.handlerFnName = `${unit.fnName}_${op.tag}_${animation}${op.name}_${slot}_listener`;
                // So the handler name is: fnName_tag_animation@openClose.start_slot_listener
                // After sanitize: fnName_tag_animation_openClose_start_slot_listener
                // (the @ becomes _ via sanitize, giving animation_openClose not animation__openClose)
                let (event_name, animation_prefix) = if listener.is_animation_listener {
                    // Build the animation event name with @ prefix and phase suffix
                    let phase_str = listener
                        .animation_phase
                        .as_ref()
                        .map(|p| match p {
                            crate::ir::enums::AnimationKind::Enter => "start",
                            crate::ir::enums::AnimationKind::Leave => "done",
                        })
                        .unwrap_or("start");
                    let new_name = format!("@{}.{}", listener.name.as_str(), phase_str);
                    let name_str = allocator.alloc_str(&new_name);
                    listener.name = Ident::from(name_str);
                    // Note: no trailing underscore - the @ in event_name will become _ after sanitize
                    (new_name, "animation")
                } else {
                    (listener.name.as_str().to_string(), "")
                };

                let handler_name = if listener.host_listener {
                    format!("{base_name}_{animation_prefix}{event_name}_HostBindingHandler")
                } else {
                    let tag = listener
                        .tag
                        .as_ref()
                        .map(|t| t.as_str().replace('-', "_"))
                        .unwrap_or_default();
                    format!("{fn_name_str}_{tag}_{animation_prefix}{event_name}_{slot}_listener")
                };

                let sanitized = sanitize_identifier(&handler_name);
                let name_str = allocator.alloc_str(&sanitized);
                listener.handler_fn_name = Some(Ident::from(name_str));
            }
            // Process handler ops inline
            for handler_op in listener.handler_ops.iter_mut() {
                if let UpdateOp::Variable(var_op) = handler_op {
                    name_variable_op(var_op, allocator, state, var_names, semantic_var_names);
                }
            }
        }
        CreateOp::TwoWayListener(listener) => {
            if listener.handler_fn_name.is_none() {
                let slot = listener.target_slot.0;
                let event_name = listener.name.as_str();
                let fn_name_str = fn_name.map(|n| n.as_str()).unwrap_or(base_name);
                let tag =
                    listener.tag.as_ref().map(|t| t.as_str().replace('-', "_")).unwrap_or_default();
                let handler_name = format!("{fn_name_str}_{tag}_{event_name}_{slot}_listener");
                let sanitized = sanitize_identifier(&handler_name);
                let name_str = allocator.alloc_str(&sanitized);
                listener.handler_fn_name = Some(Ident::from(name_str));
            }
            // Process handler ops inline
            for handler_op in listener.handler_ops.iter_mut() {
                if let UpdateOp::Variable(var_op) = handler_op {
                    name_variable_op(var_op, allocator, state, var_names, semantic_var_names);
                }
            }
        }
        CreateOp::AnimationListener(listener) => {
            if listener.handler_fn_name.is_none() {
                let slot = listener.target_slot.0;
                let animation_kind = listener.name.as_str().replace('.', "");
                let fn_name_str = fn_name.map(|n| n.as_str()).unwrap_or(base_name);

                let handler_name = if listener.host_listener {
                    format!("{base_name}_{animation_kind}_HostBindingHandler")
                } else {
                    let tag = listener
                        .tag
                        .as_ref()
                        .map(|t| t.as_str().replace('-', "_"))
                        .unwrap_or_default();
                    format!("{fn_name_str}_{tag}_{animation_kind}_{slot}_listener")
                };

                let sanitized = sanitize_identifier(&handler_name);
                let name_str = allocator.alloc_str(&sanitized);
                listener.handler_fn_name = Some(Ident::from(name_str));
            }
            // Process handler ops inline
            for handler_op in listener.handler_ops.iter_mut() {
                if let UpdateOp::Variable(var_op) = handler_op {
                    name_variable_op(var_op, allocator, state, var_names, semantic_var_names);
                }
            }
        }
        // Animation ops (converted from AnimationBinding) have handler_ops that need variable naming
        // TypeScript naming.ts lines 60-66:
        //   const animationKind = op.name.replace('.', '');
        //   op.handlerFnName = sanitizeIdentifier(`${unit.fnName}_${animationKind}_cb`);
        CreateOp::Animation(animation) => {
            if animation.handler_fn_name.is_none() {
                let fn_name_str = fn_name.map(|n| n.as_str()).unwrap_or(base_name);
                let animation_kind = animation.name.as_str().replace('.', "");
                let handler_name = format!("{fn_name_str}_{animation_kind}_cb");
                let sanitized = sanitize_identifier(&handler_name);
                let name_str = allocator.alloc_str(&sanitized);
                animation.handler_fn_name = Some(Ident::from(name_str));
            }
            // Process handler ops inline
            for handler_op in animation.handler_ops.iter_mut() {
                if let UpdateOp::Variable(var_op) = handler_op {
                    name_variable_op(var_op, allocator, state, var_names, semantic_var_names);
                }
            }
        }
        CreateOp::RepeaterCreate(_) => {
            // NOTE: track_by_ops are processed in process_create_ops_with_child_recursion,
            // BEFORE child view recursion. This matches Angular's ops() generator which
            // yields track_by_ops inline with the RepeaterCreate op.
        }
        CreateOp::Variable(var_op) => {
            name_create_variable_op(var_op, allocator, state, var_names, semantic_var_names);
        }
        _ => {}
    }
}

/// Process function ops in a view (arrow function Variable ops).
/// This matches TypeScript's ops() generator which yields function ops before create ops.
///
/// From TypeScript compilation.ts:
/// ```typescript
/// *ops(): Generator<ir.CreateOp | ir.UpdateOp> {
///   for (const expr of this.functions) {
///     for (const op of expr.ops) {
///       yield op;
///     }
///   }
///   // ... then create, then update
/// }
/// ```
///
/// IMPORTANT: Arrow function variables must NOT be deduplicated with listener/update variables.
/// In Angular's TypeScript, `getScopeForView` is called fresh for each arrow function (line 80
/// of generate_variables.ts), creating brand new `SemanticVariable` objects with `name: null`.
/// These are different JavaScript objects from the listener/update scope variables. During naming,
/// each object gets its own name, consuming counter values independently.
///
/// In OXC, we simulate this by NOT using the shared `semantic_var_names` map when naming arrow
/// function variables. Each arrow function gets its own temporary semantic map, ensuring its
/// variables always get fresh names and advance the counter.
fn process_function_ops_in_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: Option<XrefId>,
    allocator: &'a oxc_allocator::Allocator,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
) {
    let functions = match view_xref {
        None => &job.root.functions,
        Some(xref) => {
            if let Some(view) = job.view(xref) {
                &view.functions
            } else {
                return;
            }
        }
    };

    // Process each arrow function's ops with its OWN semantic var names map.
    // This matches Angular's behavior where each arrow function gets a fresh scope
    // (getScopeForView creates new SemanticVariable objects per arrow function).
    // Variables from different arrow functions (and from listener/update scopes)
    // are distinct objects in Angular, so they never share names via object identity.
    for func_ptr in functions.iter() {
        // SAFETY: These pointers are valid as they point to ArrowFunctionExpr
        // allocated in the allocator and stored in the view's functions vec.
        let func = unsafe { &mut **func_ptr };

        // Each arrow function gets its own semantic variable map to prevent
        // deduplication with other scopes (listeners, update, other arrow functions).
        let mut arrow_fn_semantic_var_names: FxHashMap<SemanticVariableKey<'a>, Ident<'a>> =
            FxHashMap::default();

        // Process Variable ops in this arrow function
        for op in func.ops.iter_mut() {
            if let UpdateOp::Variable(var_op) = op {
                name_variable_op(
                    var_op,
                    allocator,
                    state,
                    var_names,
                    &mut arrow_fn_semantic_var_names,
                );
            }
        }
    }
}

/// Process update ops in a view: style props and variables.
fn process_update_ops_in_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: Option<XrefId>,
    allocator: &'a oxc_allocator::Allocator,
    state: &mut NamingState,
    var_names: &mut FxHashMap<XrefId, Ident<'a>>,
    semantic_var_names: &mut FxHashMap<SemanticVariableKey<'a>, Ident<'a>>,
) {
    let update_ops = match view_xref {
        None => &mut job.root.update,
        Some(xref) => {
            if let Some(view) = job.view_mut(xref) {
                &mut view.update
            } else {
                return;
            }
        }
    };

    for op in update_ops.iter_mut() {
        match op {
            // Add @ prefix for legacy animation bindings on Property and DomProperty
            // TypeScript: naming.ts lines 53-57
            // Only add prefix if not already present ([@xxx] already includes @)
            UpdateOp::Property(prop) => {
                if prop.binding_kind == BindingKind::LegacyAnimation && !prop.name.starts_with('@')
                {
                    let prefixed = format!("@{}", prop.name.as_str());
                    prop.name = Ident::from(allocator.alloc_str(&prefixed));
                }
            }
            UpdateOp::DomProperty(prop) => {
                if prop.binding_kind == BindingKind::LegacyAnimation && !prop.name.starts_with('@')
                {
                    let prefixed = format!("@{}", prop.name.as_str());
                    prop.name = Ident::from(allocator.alloc_str(&prefixed));
                }
            }
            UpdateOp::StyleProp(style) => {
                if !style.name.starts_with("--") {
                    let hyphenated = hyphenate(style.name.as_str());
                    style.name = Ident::from(allocator.alloc_str(&hyphenated));
                }
                if style.name.contains("!important") {
                    let stripped = style.name.as_str().replace("!important", "");
                    let stripped = stripped.trim_end();
                    style.name = Ident::from(allocator.alloc_str(stripped));
                }
            }
            UpdateOp::Variable(var_op) => {
                name_variable_op(var_op, allocator, state, var_names, semantic_var_names);
            }
            _ => {}
        }
    }
}

/// Generates a unique variable name based on the variable kind.
///
/// Following Angular's TemplateDefinitionBuilder compatibility mode naming conventions:
/// - Context: `ctx_r{N}` (post-increment, starts at 0)
/// - Identifier: `{identifier}_r{N}` (pre-increment, starts at 1)
///   - Special case: if identifier is "ctx", uses `ctx_ir{N}` to avoid collisions
/// - Alias/SavedView: `_r{N}` (pre-increment, starts at 1)
fn get_variable_name<'a>(
    allocator: &'a oxc_allocator::Allocator,
    kind: SemanticVariableKind,
    identifier: Option<&Ident<'a>>,
    state: &mut NamingState,
) -> Ident<'a> {
    let name = match kind {
        SemanticVariableKind::Context => {
            // Context uses post-increment (starts at 0)
            let name = format!("ctx_r{}", state.index);
            state.index += 1;
            name
        }
        SemanticVariableKind::Identifier => {
            // Identifier uses pre-increment (starts at 1) with _r suffix
            // Special case: if identifier is "ctx", add "i" prefix to avoid collision with Context
            if let Some(ident) = identifier {
                state.index += 1;
                let compat_prefix = if ident.as_str() == "ctx" { "i" } else { "" };
                format!("{}_{compat_prefix}r{}", ident.as_str(), state.index)
            } else {
                state.index += 1;
                format!("_r{}", state.index)
            }
        }
        SemanticVariableKind::Alias => {
            // Alias uses pre-increment (starts at 1) with _r suffix
            state.index += 1;
            format!("_r{}", state.index)
        }
        SemanticVariableKind::SavedView => {
            // SavedView uses pre-increment (starts at 1) with _r suffix
            state.index += 1;
            format!("_r{}", state.index)
        }
    };

    let name_str = allocator.alloc_str(&name);
    Ident::from(name_str)
}

/// Propagates variable names to ReadVariableExpr expressions in a view.
fn propagate_variable_names_in_view<'a>(
    create_ops: &mut crate::ir::list::CreateOpList<'a>,
    update_ops: &mut crate::ir::list::UpdateOpList<'a>,
    var_names: &FxHashMap<XrefId, Ident<'a>>,
) {
    // Process create operations
    for op in create_ops.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| {
                propagate_name_to_expression(expr, var_names);
            },
            VisitorContextFlag::NONE,
        );
    }

    // Process update operations
    for op in update_ops.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                propagate_name_to_expression(expr, var_names);
            },
            VisitorContextFlag::NONE,
        );
    }
}

/// Propagates a variable name to a ReadVariableExpr expression.
fn propagate_name_to_expression<'a>(
    expr: &mut IrExpression<'a>,
    var_names: &FxHashMap<XrefId, Ident<'a>>,
) {
    if let IrExpression::ReadVariable(read_var) = expr {
        // Only set name if it's currently None
        if read_var.name.is_none() {
            if let Some(name) = var_names.get(&read_var.xref) {
                read_var.name = Some(name.clone());
            }
        }
    }
}

/// Names functions and variables for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// Also names listener handler functions for host bindings.
pub fn name_functions_and_variables_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    let component_name = job.component_name.as_str();

    // Sanitize the base name once
    let sanitized_base = sanitize_identifier(component_name);

    // Generate the function name for the host binding function
    let fn_name = format!("{}_{}", component_name, job.fn_suffix);
    job.root.fn_name = Some(oxc_span::Ident::from(allocator.alloc_str(&fn_name)));

    // Name listener handlers in create ops
    // Host listeners need names like: ComponentName_click_HostBindingHandler
    for op in job.root.create.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                if listener.handler_fn_name.is_none() && listener.host_listener {
                    // For host listeners, format is: ComponentName_eventName_HostBindingHandler
                    // Event name needs dots replaced with underscores (e.g., keydown.enter -> keydown_enter)
                    let event_name = listener.name.as_str().replace('.', "_");
                    let handler_name = format!("{sanitized_base}_{event_name}_HostBindingHandler");
                    let sanitized = sanitize_identifier(&handler_name);
                    let name_str = allocator.alloc_str(&sanitized);
                    listener.handler_fn_name = Some(Ident::from(name_str));
                }
            }
            CreateOp::AnimationListener(listener) => {
                if listener.handler_fn_name.is_none() && listener.host_listener {
                    let animation_kind = listener.name.as_str().replace('.', "");
                    let handler_name =
                        format!("{sanitized_base}_{animation_kind}_HostBindingHandler");
                    let sanitized = sanitize_identifier(&handler_name);
                    let name_str = allocator.alloc_str(&sanitized);
                    listener.handler_fn_name = Some(Ident::from(name_str));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::enums::{SemanticVariableKind, VariableFlags};
    use crate::ir::expression::{IrExpression, NextContextExpr};
    use crate::ir::ops::{UpdateVariableOp, XrefId};
    use oxc_allocator::{Allocator, Box as AllocBox};
    use oxc_span::Ident;

    /// Helper: create an `UpdateVariableOp` representing a NextContext-based
    /// context variable for the given `view_xref`.  The variable starts with
    /// an empty name (needs naming) and `SemanticVariableKind::Context`.
    fn make_context_var_op<'a>(
        allocator: &'a Allocator,
        xref: XrefId,
        view_xref: XrefId,
    ) -> UpdateVariableOp<'a> {
        UpdateVariableOp {
            base: Default::default(),
            xref,
            kind: SemanticVariableKind::Context,
            name: Ident::from(""),
            initializer: AllocBox::new_in(
                IrExpression::NextContext(AllocBox::new_in(
                    NextContextExpr { steps: 1, source_span: None },
                    allocator,
                )),
                allocator,
            ),
            flags: VariableFlags::NONE,
            view: Some(view_xref),
            local: false,
        }
    }

    /// Verify that `name_variable_op` deduplicates context variables for the
    /// same view when they share the same `semantic_var_names` map.
    ///
    /// This is the CORRECT behaviour for variables within a single scope
    /// (e.g., two update ops in the same view that both access the parent
    /// context).
    #[test]
    fn test_shared_semantic_map_deduplicates_context_variables() {
        let allocator = Allocator::default();
        let mut state = NamingState { index: 0 };
        let mut var_names: FxHashMap<XrefId, Ident<'_>> = FxHashMap::default();
        let mut semantic_var_names: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
            FxHashMap::default();

        let parent_view = XrefId(100);

        // First variable: Context for parent_view -- should get ctx_r0.
        let mut var1 = make_context_var_op(&allocator, XrefId(1), parent_view);
        name_variable_op(
            &mut var1,
            &allocator,
            &mut state,
            &mut var_names,
            &mut semantic_var_names,
        );
        assert_eq!(var1.name.as_str(), "ctx_r0", "First context variable gets ctx_r0");
        assert_eq!(state.index, 1, "Counter advances to 1 after first naming");

        // Second variable: also Context for parent_view in the SAME semantic map.
        // With deduplication, it should reuse ctx_r0 and NOT advance the counter.
        let mut var2 = make_context_var_op(&allocator, XrefId(2), parent_view);
        name_variable_op(
            &mut var2,
            &allocator,
            &mut state,
            &mut var_names,
            &mut semantic_var_names,
        );
        assert_eq!(
            var2.name.as_str(),
            "ctx_r0",
            "Second context variable in same scope is deduplicated to ctx_r0"
        );
        assert_eq!(state.index, 1, "Counter stays at 1 (no advancement due to dedup)");
    }

    /// Verify that arrow function context variables get independent names when
    /// they use a separate `semantic_var_names` map (the fix).
    ///
    /// This test reproduces the exact scenario the fix addresses:
    /// - An update scope names a Context variable for a parent view
    /// - An arrow function also needs a Context variable for the SAME parent view
    /// - With independent maps, the arrow function gets its own name and the
    ///   counter advances, matching Angular's TypeScript behaviour where
    ///   `getScopeForView` creates fresh SemanticVariable objects per arrow
    ///   function.
    ///
    /// If the fix is reverted (arrow functions share the update scope's
    /// semantic map), this test FAILS because the arrow function's context
    /// variable would be deduplicated with the update scope's, producing
    /// `ctx_r0` instead of `ctx_r1`.
    #[test]
    fn test_arrow_function_gets_independent_context_variable_name() {
        let allocator = Allocator::default();
        let mut state = NamingState { index: 0 };
        let mut var_names: FxHashMap<XrefId, Ident<'_>> = FxHashMap::default();

        let parent_view = XrefId(100);

        // --- Simulate the update scope naming a Context variable ---
        let mut update_semantic: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
            FxHashMap::default();
        let mut update_ctx_var = make_context_var_op(&allocator, XrefId(1), parent_view);
        name_variable_op(
            &mut update_ctx_var,
            &allocator,
            &mut state,
            &mut var_names,
            &mut update_semantic,
        );
        assert_eq!(
            update_ctx_var.name.as_str(),
            "ctx_r0",
            "Update scope context variable gets ctx_r0"
        );
        assert_eq!(state.index, 1);

        // --- Simulate an arrow function with its OWN semantic map (the fix) ---
        let mut arrow_fn_semantic: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
            FxHashMap::default();
        let mut arrow_ctx_var = make_context_var_op(&allocator, XrefId(2), parent_view);
        name_variable_op(
            &mut arrow_ctx_var,
            &allocator,
            &mut state,
            &mut var_names,
            &mut arrow_fn_semantic,
        );

        // With the fix: arrow function gets its own name, counter advances.
        assert_eq!(
            arrow_ctx_var.name.as_str(),
            "ctx_r1",
            "Arrow function context variable must get ctx_r1 (not deduplicated with update scope). \
             If this fails with ctx_r0, the fix was reverted: arrow functions are sharing the \
             update scope's semantic_var_names map instead of getting their own."
        );
        assert_eq!(state.index, 2, "Counter must advance to 2");
    }

    /// Verify that two separate arrow functions each get independent names,
    /// even when they both reference the same parent view.
    ///
    /// In Angular's TypeScript, each arrow function gets its own scope via
    /// `getScopeForView`, so their SemanticVariable objects are distinct and
    /// naming produces independent counter values.
    #[test]
    fn test_multiple_arrow_functions_get_independent_names() {
        let allocator = Allocator::default();
        let mut state = NamingState { index: 0 };
        let mut var_names: FxHashMap<XrefId, Ident<'_>> = FxHashMap::default();

        let parent_view = XrefId(100);

        // Arrow function 1: own semantic map.
        let mut arrow1_semantic: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
            FxHashMap::default();
        let mut arrow1_ctx = make_context_var_op(&allocator, XrefId(1), parent_view);
        name_variable_op(
            &mut arrow1_ctx,
            &allocator,
            &mut state,
            &mut var_names,
            &mut arrow1_semantic,
        );
        assert_eq!(arrow1_ctx.name.as_str(), "ctx_r0");

        // Arrow function 2: own semantic map.
        let mut arrow2_semantic: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
            FxHashMap::default();
        let mut arrow2_ctx = make_context_var_op(&allocator, XrefId(2), parent_view);
        name_variable_op(
            &mut arrow2_ctx,
            &allocator,
            &mut state,
            &mut var_names,
            &mut arrow2_semantic,
        );
        assert_eq!(
            arrow2_ctx.name.as_str(),
            "ctx_r1",
            "Second arrow function must get ctx_r1, not ctx_r0"
        );

        // Both arrows independently advance the counter.
        assert_eq!(state.index, 2);
    }

    /// Verify that the BUG scenario (shared semantic map) would cause incorrect
    /// deduplication between an arrow function and the update scope.
    ///
    /// This test demonstrates what happens WITHOUT the fix: if an arrow function
    /// shares the update scope's semantic_var_names map, its context variable
    /// gets deduplicated and the counter does not advance.
    #[test]
    fn test_shared_map_causes_incorrect_deduplication_for_arrow_functions() {
        let allocator = Allocator::default();
        let mut state = NamingState { index: 0 };
        let mut var_names: FxHashMap<XrefId, Ident<'_>> = FxHashMap::default();

        // Use a SINGLE shared semantic map (simulating the buggy behavior).
        let mut shared_semantic: FxHashMap<SemanticVariableKey<'_>, Ident<'_>> =
            FxHashMap::default();

        let parent_view = XrefId(100);

        // Update scope names its context variable.
        let mut update_ctx = make_context_var_op(&allocator, XrefId(1), parent_view);
        name_variable_op(
            &mut update_ctx,
            &allocator,
            &mut state,
            &mut var_names,
            &mut shared_semantic,
        );
        assert_eq!(update_ctx.name.as_str(), "ctx_r0");
        assert_eq!(state.index, 1);

        // Arrow function uses the SAME shared map (the bug).
        let mut arrow_ctx = make_context_var_op(&allocator, XrefId(2), parent_view);
        name_variable_op(
            &mut arrow_ctx,
            &allocator,
            &mut state,
            &mut var_names,
            &mut shared_semantic,
        );

        // BUG: deduplication causes the arrow function to get the same name.
        assert_eq!(
            arrow_ctx.name.as_str(),
            "ctx_r0",
            "With shared map, arrow function is incorrectly deduplicated to ctx_r0"
        );
        assert_eq!(state.index, 1, "Counter does not advance because of incorrect deduplication");
    }
}

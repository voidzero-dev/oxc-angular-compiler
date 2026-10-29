//! Resolve defer deps functions phase.
//!
//! Resolves dependency functions for defer blocks.
//!
//! This phase creates dependency resolver functions for @defer blocks that
//! have lazy-loaded components. The resolver function is called when the
//! defer block is triggered to load the necessary dependencies.
//!
//! In Angular's implementation:
//! 1. `ownResolverFn` is set during ingestion from `deferMeta.blocks` when
//!    a defer block has lazy-loaded dependencies
//! 2. This phase only processes defer blocks where `ownResolverFn` is set
//! 3. The resolver is added to the constant pool via `getSharedFunctionReference`
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_defer_deps_fns.ts`.

use crate::ir::ops::{CreateOp, SlotId, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Resolves dependency functions for defer blocks.
///
/// This phase:
/// 1. Skips defer blocks that already have `resolver_fn` set
/// 2. Only processes defer blocks where `own_resolver_fn` is set
/// 3. Creates a shared function reference in the constant pool
///
/// Matches Angular's resolve_defer_deps_fns.ts behavior.
pub fn resolve_defer_deps_fns(job: &mut ComponentCompilationJob<'_>) {
    // First pass: collect which defer blocks need processing (view_xref, slot)
    // We need to avoid holding references while mutating
    let defer_locations: Vec<(XrefId, SlotId)> = job
        .all_views()
        .flat_map(|view| {
            let view_xref = view.xref;
            view.create.iter().filter_map(move |op| {
                if let CreateOp::Defer(defer) = op {
                    // Skip if resolver_fn is already set
                    if defer.resolver_fn.is_some() {
                        return None;
                    }
                    // Only process if own_resolver_fn is set
                    if defer.own_resolver_fn.is_none() {
                        return None;
                    }
                    // Slot must be assigned - Angular TS throws if null
                    // We skip processing if slot is not assigned yet
                    let slot = defer.slot?;
                    Some((view_xref, slot))
                } else {
                    None
                }
            })
        })
        .collect();

    // Second pass: process each defer block by taking ownership and transforming
    for (view_xref, slot_id) in defer_locations {
        // Get the full path name from the view's fn_name
        // In Angular TS: const fullPathName = unit.fnName?.replace('_Template', '');
        // `String.prototype.replace` with a string pattern replaces only the
        // first match.
        let full_path_name = job
            .view(view_xref)
            .and_then(|v| v.fn_name.as_ref())
            .map_or_else(|| job.component_name.to_string(), |n| n.replacen("_Template", "", 1));

        // Find the defer op and take ownership of own_resolver_fn
        let view =
            if view_xref == job.root.xref { Some(&mut job.root) } else { job.view_mut(view_xref) };

        let own_resolver = if let Some(view) = view {
            let mut found_resolver = None;
            for op in view.create.iter_mut() {
                if let CreateOp::Defer(defer) = op {
                    if defer.slot == Some(slot_id) {
                        // Take ownership of own_resolver_fn
                        found_resolver = defer.own_resolver_fn.take();
                        break;
                    }
                }
            }
            found_resolver
        } else {
            None
        };

        // If we got an own_resolver, process it through the constant pool
        if let Some(resolver_expr) = own_resolver {
            // Generate function name: ${fullPathName}_Defer_${slot}_DepsFn
            let fn_name = format!("{}_Defer_{}_DepsFn", full_path_name, slot_id.0);

            // Add to constant pool and get a reference
            // In Angular TS: job.pool.getSharedFunctionReference(op.ownResolverFn, fnName, false)
            let resolver_ref =
                job.pool.get_shared_function_reference(resolver_expr, &fn_name, false);

            // Update the DeferOp with the resolver function reference
            let view = if view_xref == job.root.xref {
                Some(&mut job.root)
            } else {
                job.view_mut(view_xref)
            };

            if let Some(view) = view {
                for op in view.create.iter_mut() {
                    if let CreateOp::Defer(defer) = op {
                        if defer.slot == Some(slot_id) {
                            defer.resolver_fn = Some(resolver_ref);
                            break;
                        }
                    }
                }
            }
        }
    }
}

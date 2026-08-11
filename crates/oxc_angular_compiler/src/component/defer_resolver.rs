//! Build the dependency-resolver function for `@defer` blocks.
//!
//! In Angular's local compilation mode the compiler emits a single
//! per-component deferrable-dependencies function — a no-arg arrow that
//! returns an array of dynamic `import()` calls resolving each lazy
//! dependency. The runtime invokes it lazily, the first time a defer block
//! triggers, so the deferred chunks can be code-split by the bundler.
//!
//! Example shape:
//!
//! ```js
//! () => [
//!   import('./lazy').then(m => m.LazyCmp),
//!   import('./widget').then(m => m.HeavyWidget),
//!   import('./default-export').then(m => m.default),
//! ]
//! ```
//!
//! This module produces that expression from the component's
//! `@Component.deferredImports` array (mirrored as
//! [`ComponentMetadata::deferred_imports`]) combined with the file-level
//! import map. It is consumed by the main compile path in
//! [`super::transform::compile_component_full`].
//!
//! ## Why `deferredImports`, not `imports`?
//!
//! Angular's full compilation mode also derives a deferrable set from
//! `@Component.imports` by matching template selectors to imported
//! directives/pipes/components (see `resolveAllDeferredDependencies` in
//! `packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts`).
//! That requires cross-file metadata OXC doesn't have — selectors live on
//! the imported class, not on the import declaration. In local compilation
//! (which OXC is — single file in, single file out) Angular falls back to
//! the `deferredImports` field as the only safe source of deferrable
//! symbols, and OXC does the same.

use oxc_allocator::Allocator;

use crate::ast::r3::{R3DeferredBlock, R3Node, R3Visitor, visit_all};
use crate::class_metadata::{
    R3DeferPerComponentDependency, compile_component_metadata_async_resolver,
};
use crate::output::ast::OutputExpression;
use oxc_str::Ident;

use super::metadata::ComponentMetadata;
use super::transform::ImportMap;

/// Returns `true` if any `@defer` block exists anywhere in the template.
///
/// We need this so the compiler only generates a deferrable-dependencies
/// function when it would actually be wired into a `ɵɵdefer` call — otherwise
/// the resolver would be dead code.
pub fn template_has_defer_block<'a>(nodes: &[R3Node<'a>]) -> bool {
    struct DeferDetector {
        found: bool,
    }

    impl<'a> R3Visitor<'a> for DeferDetector {
        fn visit_deferred_block(&mut self, _block: &R3DeferredBlock<'a>) {
            // No need to recurse: a single hit is enough.
            self.found = true;
        }
    }

    let mut visitor = DeferDetector { found: false };
    visit_all(&mut visitor, nodes);
    visitor.found
}

/// Build the list of `R3DeferPerComponentDependency` describing the
/// `@Component.deferredImports` entries, used to emit `setClassMetadataAsync`
/// and the per-component `ɵɵdefer` resolver.
///
/// Each entry carries two names that serve distinct roles in the emitted
/// output:
///
/// - `param_name` — the local binding from the source file (e.g. `Heavy` for
///   `import { HeavyWidget as Heavy }`, `LazyCmp` for
///   `import LazyCmp from './lazy'`). Used as the parameter name on the
///   `setClassMetadataAsync` callback so the wrapped decorator metadata
///   literal's identifier references shadow the outer static import,
///   allowing bundlers to drop the eager declaration.
/// - `export_name` — the name under which the symbol is exported from its
///   source module (e.g. `HeavyWidget` or `default`). Used as the property
///   read in the dynamic-import resolver chain (`m.<export_name>`). For
///   default imports the resolver substitutes `m.default` based on
///   `is_default_import`, so the value here is informational in that case.
///
/// Angular collapses both into a single `symbolName` field set to the
/// exported name; that leaves the static `import { Foo as Bar }` pinned for
/// aliased deferrable imports because the callback parameter (`Foo`) doesn't
/// match the metadata body's reference (`Bar`). Splitting the fields here is
/// a deliberate improvement on Angular's emission.
///
/// Entries for unresolved symbols, namespace imports, and type-only imports
/// are skipped — they have no concrete runtime value to lazy-load and the
/// resolver would silently misfire.
pub(super) fn build_defer_per_component_deps<'a>(
    allocator: &'a Allocator,
    deferred_imports: &[Ident<'a>],
    import_map: &ImportMap<'a>,
) -> oxc_allocator::Vec<'a, R3DeferPerComponentDependency<'a>> {
    let mut deps = oxc_allocator::Vec::new_in(&allocator);
    for local_name in deferred_imports {
        let Some(info) = import_map.get(local_name) else { continue };
        if !info.is_named_import || info.is_type_only {
            continue;
        }
        let is_default_import = info.imported_name.as_deref() == Some("default");
        // `export_name` is `info.imported_name` when present (aliased named
        // imports or default imports), otherwise the local name (which
        // equals the exported name for plain `import { Foo }`).
        let export_name = info.imported_name.clone().unwrap_or_else(|| local_name.clone());
        deps.push(R3DeferPerComponentDependency {
            param_name: local_name.clone(),
            export_name,
            import_path: info.source_module.clone(),
            is_default_import,
        });
    }
    deps
}

/// Build the deferrable-dependencies resolver expression from the component's
/// `@Component.deferredImports` array.
///
/// For each local identifier in `metadata.deferred_imports` that resolves to
/// an entry in `import_map`, emit one
/// `import(<path>).then(m => m.<exportedName>)` entry. Returns `None` when the
/// component has no `deferredImports` declared (no resolver to wire) — in
/// that case callers leave `ɵɵdefer`'s resolver argument as `null`, matching
/// Angular's behavior for a defer block with no explicit deferrable
/// dependencies.
///
/// A deferred entry is silently skipped (with no compiler error) when:
/// - the symbol isn't in the file's import map (it might be a local binding;
///   Angular would have flagged this earlier, but we treat it as a no-op),
/// - the import is a namespace import (`import * as ns`) — there is no
///   single `m.X` to point at,
/// - the import is type-only (erased at runtime).
///
/// Unlike eager `imports`, we do NOT filter by source module: Angular's
/// `compileDeferResolverFunction` emits a dynamic `import(<path>)` for every
/// entry the user lists, including bare specifiers like `@angular/material`.
/// Filtering would silently drop intentional opt-ins.
///
/// Shares the emission code path with `setClassMetadataAsync` via
/// [`compile_component_metadata_async_resolver`] so both call sites stay in
/// lockstep — the resolver wired into `ɵɵdefer(...)` is byte-equivalent to
/// the one wired into `ɵsetClassMetadataAsync(...)`.
pub fn build_defer_resolver_expression<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    import_map: &ImportMap<'a>,
) -> Option<OutputExpression<'a>> {
    if metadata.deferred_imports.is_empty() {
        return None;
    }

    let deps = build_defer_per_component_deps(allocator, &metadata.deferred_imports, import_map);
    if deps.is_empty() {
        return None;
    }

    Some(compile_component_metadata_async_resolver(allocator, &deps))
}

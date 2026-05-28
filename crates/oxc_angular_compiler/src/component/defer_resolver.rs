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

use oxc_allocator::{Allocator, Box, Vec as OxcVec};
use oxc_str::Ident;

use crate::ast::r3::{R3DeferredBlock, R3Node, R3Visitor, visit_all};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, DynamicImportExpr, DynamicImportUrl, FnParam,
    InvokeFunctionExpr, LiteralArrayExpr, OutputExpression, ReadPropExpr, ReadVarExpr,
};

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
pub fn build_defer_resolver_expression<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    import_map: &ImportMap<'a>,
) -> Option<OutputExpression<'a>> {
    if metadata.deferred_imports.is_empty() {
        return None;
    }

    let mut entries = OxcVec::new_in(allocator);

    for local_name in &metadata.deferred_imports {
        let Some(info) = import_map.get(local_name) else { continue };

        // Namespace imports (`import * as ns from 'x'`) cannot be turned into
        // a single `m.X` reference, so skip them.
        if !info.is_named_import {
            continue;
        }

        // Type-only imports are erased at runtime — they have no concrete
        // value to lazy-load.
        if info.is_type_only {
            continue;
        }

        // Choose the property to read off the loaded module:
        // - Aliased named import (`import { Foo as Bar }`) → original `Foo`
        // - Default import (`import D from 'x'`) → `default`
        // - Plain named import (`import { Foo }`) → local name `Foo`
        let export_name: Ident<'a> =
            info.imported_name.clone().unwrap_or_else(|| local_name.clone());

        entries.push(build_import_then_expression(
            allocator,
            info.source_module.clone(),
            export_name,
        ));
    }

    if entries.is_empty() {
        return None;
    }

    // Wrap the array in a no-arg arrow function so the resolver is lazy.
    let array_expr = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ));
    let arrow = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: OxcVec::new_in(allocator),
            body: ArrowFunctionBody::Expression(Box::new_in(array_expr, allocator)),
            source_span: None,
        },
        allocator,
    ));
    Some(arrow)
}

/// Build `import(<source>).then(m => m.<export_name>)`.
fn build_import_then_expression<'a>(
    allocator: &'a Allocator,
    source: Ident<'a>,
    export_name: Ident<'a>,
) -> OutputExpression<'a> {
    let dynamic_import = OutputExpression::DynamicImport(Box::new_in(
        DynamicImportExpr {
            url: DynamicImportUrl::String(source),
            url_comment: None,
            source_span: None,
        },
        allocator,
    ));

    // `m => m.<export_name>`
    let m_ident = Ident::from("m");
    let callback_body = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: m_ident.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: export_name,
            optional: false,
            source_span: None,
        },
        allocator,
    ));
    let mut params = OxcVec::with_capacity_in(1, allocator);
    params.push(FnParam { name: m_ident });
    let callback = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params,
            body: ArrowFunctionBody::Expression(Box::new_in(callback_body, allocator)),
            source_span: None,
        },
        allocator,
    ));

    // `import(<source>).then(<callback>)`
    let then_callee = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(dynamic_import, allocator),
            name: Ident::from("then"),
            optional: false,
            source_span: None,
        },
        allocator,
    ));
    let mut args = OxcVec::with_capacity_in(1, allocator);
    args.push(callback);
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(then_callee, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

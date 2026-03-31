//! Generate local let references phase.
//!
//! Replaces `StoreLet` update ops with `Variable` ops that can be used
//! to reference the value within the same view.
//!
//! ## @let Declarations
//!
//! Angular's @let syntax allows declaring local template variables:
//! ```html
//! @let name = user.firstName;
//! <p>Hello, {{name}}!</p>
//! ```
//!
//! This phase transforms the IR so that @let values are stored in semantic
//! variables that can be referenced by the generated code.
//!
//! Ported from Angular's `template/pipeline/src/phases/generate_local_let_references.ts`.

use oxc_allocator::Box;
use rustc_hash::FxHashMap;

use crate::ir::enums::{SemanticVariableKind, VariableFlags};
use crate::ir::expression::{IrExpression, StoreLetExpr};
use crate::ir::ops::{CreateOp, UpdateOp, UpdateOpBase, UpdateVariableOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Replaces `StoreLet` ops with `Variable` ops containing `StoreLetExpr`.
///
/// This phase:
/// 1. Collects DeclareLet names from create ops (to look up names by xref)
/// 2. Iterates update ops, finding StoreLet operations
/// 3. Replaces each StoreLet with a Variable op containing the expression
pub fn generate_local_let_references(job: &mut ComponentCompilationJob<'_>) {
    // Collect view xrefs first to avoid borrow issues
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        // First pass: collect DeclareLet names and count StoreLet ops
        let (let_names, store_let_count) = {
            let view = match job.view(view_xref) {
                Some(v) => v,
                None => continue,
            };

            let mut names: FxHashMap<XrefId, oxc_span::Ident<'_>> = FxHashMap::default();
            for op in view.create.iter() {
                if let CreateOp::DeclareLet(let_decl) = op {
                    names.insert(let_decl.xref, let_decl.name.clone());
                }
            }

            let count = view.update.iter().filter(|op| matches!(op, UpdateOp::StoreLet(_))).count();

            (names, count)
        };

        if store_let_count == 0 {
            continue;
        }

        // Pre-allocate xrefs for all StoreLet ops we'll replace
        let mut xref_pool: Vec<XrefId> = Vec::with_capacity(store_let_count);
        for _ in 0..store_let_count {
            xref_pool.push(job.allocate_xref_id());
        }
        let mut xref_iter = xref_pool.into_iter();

        // Second pass: replace StoreLet with Variable in update ops
        let allocator = job.allocator;
        let view = match job.view_mut(view_xref) {
            Some(v) => v,
            None => continue,
        };

        let mut cursor = view.update.cursor_front();
        loop {
            let replacement = {
                let op = match cursor.current() {
                    Some(op) => op,
                    None => break,
                };

                if let UpdateOp::StoreLet(store_let) = op {
                    // Look up the declared name from the DeclareLet create op
                    let declared_name = let_names
                        .get(&store_let.target)
                        .cloned()
                        .unwrap_or_else(|| oxc_span::Ident::from(""));

                    // Create a new Variable op with StoreLetExpr as the initializer
                    let store_let_expr = IrExpression::StoreLet(Box::new_in(
                        StoreLetExpr {
                            target: store_let.target,
                            value: Box::new_in((*store_let.value).clone_in(allocator), allocator),
                            var_offset: None, // Assigned by var_counting phase
                            source_span: store_let.base.source_span.unwrap_or_default(),
                        },
                        allocator,
                    ));

                    // Use pre-allocated xref
                    let var_xref = xref_iter.next().unwrap_or(XrefId(0));

                    Some(UpdateOp::Variable(UpdateVariableOp {
                        base: UpdateOpBase {
                            source_span: store_let.base.source_span,
                            ..Default::default()
                        },
                        xref: var_xref,
                        kind: SemanticVariableKind::Identifier,
                        name: declared_name,
                        initializer: Box::new_in(store_let_expr, allocator),
                        flags: VariableFlags::NONE,
                        view: None,
                        local: true, // @let declarations are local to the view
                    }))
                } else {
                    None
                }
            };

            if let Some(new_op) = replacement {
                cursor.replace_current(new_op);
            }

            if !cursor.move_next() {
                break;
            }
        }
    }
}

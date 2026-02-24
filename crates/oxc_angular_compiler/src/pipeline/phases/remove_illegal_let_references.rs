//! Remove illegal let references phase.
//!
//! It's not allowed to access a `@let` declaration before it has been defined.
//! This is enforced by template type checking, but can trip assertions in the pipeline.
//!
//! To avoid confusing errors in JIT mode (where type checking isn't running),
//! this phase detects illegal forward references and replaces them with `undefined`.
//!
//! ## Illegal @let References
//!
//! Forward references to @let variables are illegal:
//! ```html
//! <!-- Error: Cannot reference 'x' before it's declared -->
//! <p>{{x}}</p>
//! @let x = 42;
//! ```
//!
//! Ported from Angular's `template/pipeline/src/phases/remove_illegal_let_references.ts`.

use oxc_allocator::Box;
use oxc_span::Atom;

use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, LiteralPrimitive, LiteralValue, ParseSpan,
};
use crate::ir::enums::SemanticVariableKind;
use crate::ir::expression::{IrExpression, VisitorContextFlag, transform_expressions_in_update_op};
use crate::ir::ops::{UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Removes illegal @let forward references by replacing them with `undefined`.
///
/// This phase runs after `generate_local_let_references` which converts
/// `StoreLet` ops to `Variable` ops with `StoreLetExpr` initializers.
///
/// For each such Variable op, we walk backwards through all preceding ops
/// and replace any `LexicalRead` with the same name with `undefined`.
pub fn remove_illegal_let_references(job: &mut ComponentCompilationJob<'_>) {
    // Collect view xrefs first to avoid borrow issues
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        // First pass: collect let variable names from Variable ops
        let let_names: Vec<Atom<'_>> = {
            let view = match job.view(view_xref) {
                Some(v) => v,
                None => continue,
            };

            let mut names = Vec::new();
            for op in view.update.iter() {
                if let UpdateOp::Variable(var) = op {
                    // Check if this is an Identifier variable with StoreLetExpr initializer
                    if var.kind == SemanticVariableKind::Identifier {
                        if let IrExpression::StoreLet(_) = var.initializer.as_ref() {
                            names.push(var.name.clone());
                        }
                    }
                }
            }
            names
        };

        if let_names.is_empty() {
            continue;
        }

        // Second pass: for each let name, walk through all ops and replace
        // LexicalRead with matching name that appears before the Variable op
        let allocator = job.allocator;
        let view = match job.view_mut(view_xref) {
            Some(v) => v,
            None => continue,
        };

        // We process by iterating and tracking which let names have been "declared".
        // A let name is "declared" AFTER we finish transforming its Variable op.
        // This matches Angular which walks backward from the declaration op itself,
        // replacing self-references (e.g. `@let x = x + 1`) with `undefined`.
        let mut declared_names: Vec<Atom<'_>> = Vec::new();

        for op in view.update.iter_mut() {
            // Check if this op declares a let variable — extract the name before
            // transforming so we can mark it as declared AFTER the transform.
            let newly_declared = if let UpdateOp::Variable(var) = &*op {
                if var.kind == SemanticVariableKind::Identifier {
                    if let IrExpression::StoreLet(_) = var.initializer.as_ref() {
                        Some(var.name.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Replace any LexicalRead with undeclared let names with undefined.
            // This runs BEFORE marking the current op's name as declared, so
            // self-references in the declaration op are also replaced.
            let let_names_ref = &let_names;
            let declared_ref = &declared_names;

            transform_expressions_in_update_op(
                op,
                &|expr, _flags| {
                    if let IrExpression::LexicalRead(lr) = expr {
                        // Check if this is a let name that hasn't been declared yet
                        if let_names_ref.contains(&lr.name) && !declared_ref.contains(&lr.name) {
                            // Replace with undefined literal
                            let undefined_expr = AngularExpression::LiteralPrimitive(Box::new_in(
                                LiteralPrimitive {
                                    span: ParseSpan::new(0, 0),
                                    source_span: AbsoluteSourceSpan::new(0, 0),
                                    value: LiteralValue::Undefined,
                                },
                                allocator,
                            ));
                            *expr = IrExpression::Ast(Box::new_in(undefined_expr, allocator));
                        }
                    }
                },
                VisitorContextFlag::NONE,
            );

            // Mark this name as declared AFTER transforming, so subsequent ops
            // can legally reference it, but the declaration op itself cannot.
            if let Some(name) = newly_declared {
                declared_names.push(name);
            }
        }
    }
}

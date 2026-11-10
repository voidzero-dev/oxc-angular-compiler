//! Convert animations phase.
//!
//! Converts animation bindings to runtime animation calls. Animation bindings
//! are special property bindings that trigger Angular animations on elements.
//!
//! This phase:
//! 1. Finds all `AnimationBinding` update operations
//! 2. For `AnimationBindingKind::Value`: converts to `Animation` CreateOp
//! 3. For `AnimationBindingKind::String`: creates `AnimationString` CreateOp
//! 4. Inserts the new CreateOp after the target element in the create list
//! 5. Removes the original AnimationBinding from the update list
//!
//! Both Animation and AnimationString are CreateOps that are inserted after
//! their target element in the create list, one at a time in update-list
//! order, matching Angular's architecture.
//! For host bindings they are pushed to the end of the create list instead.
//!
//! Ported from Angular's `template/pipeline/src/phases/convert_animations.ts`.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_diagnostics::OxcDiagnostic;
use oxc_str::Ident;

use crate::ast::expression::{AbsoluteSourceSpan, AngularExpression, EmptyExpr, ParseSpan};
use crate::ast::r3::SecurityContext;
use crate::ir::enums::{AnimationBindingKind, AnimationKind};
use crate::ir::expression::IrExpression;
use crate::ir::ops::{
    AnimationBindingOp, AnimationOp, AnimationStringOp, CreateOp, CreateOpBase, StatementOp,
    UpdateOp, UpdateOpBase,
};
use crate::output::ast::{OutputExpression, OutputStatement, ReturnStatement, WrappedIrExpr};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Determines the animation kind from the binding name.
/// "animate.enter" -> Enter, "animate.leave" -> Leave
fn get_animation_kind(name: &str) -> AnimationKind {
    if name == "animate.enter" { AnimationKind::Enter } else { AnimationKind::Leave }
}

/// Converts animation bindings to animation runtime calls.
///
/// This phase transforms `AnimationBinding` update operations into either
/// `Animation` or `AnimationString` CreateOps based on the binding kind:
/// - `AnimationBindingKind::Value` -> `AnimationOp` (CreateOp)
/// - `AnimationBindingKind::String` -> `AnimationStringOp` (CreateOp)
///
/// The bindings are taken in update-list order, and each new op goes right
/// after its target element, as Angular's `convertAnimations` does
/// (`OpList.insertAfter`). So the last binding of an element comes first: for
/// `<p animate.leave="out" [animate.enter]="e">` that is `ɵɵanimateEnter(cb)`
/// then `ɵɵanimateLeave("out")`.
pub fn convert_animations(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect view xrefs to avoid borrow issues
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        let mut missing_targets: Vec<Ident<'_>> = Vec::new();

        if let Some(view) = job.view_mut(view_xref) {
            // Take every AnimationBinding out of the update list, in order.
            let mut created = Vec::new();
            let mut cursor = view.update.cursor();
            cursor.move_next();
            while let Some(op) = cursor.current_mut() {
                if let UpdateOp::AnimationBinding(binding) = op {
                    let target = binding.target;
                    let name = binding.name;
                    created.push((target, name, animation_create_op(allocator, binding)));
                    cursor.remove_current();
                } else {
                    cursor.move_next();
                }
            }

            // Insert each one right after its target element in the create list.
            for (target, name, op) in created {
                let mut create_op = Some(op);
                let mut cursor = view.create.cursor();
                while cursor.move_next() {
                    let is_target = match cursor.current() {
                        Some(CreateOp::ElementStart(el)) => el.xref == target,
                        Some(CreateOp::Element(el)) => el.xref == target,
                        Some(CreateOp::Template(t)) => t.xref == target,
                        Some(CreateOp::ContainerStart(c)) => c.xref == target,
                        Some(CreateOp::Container(c)) => c.xref == target,
                        _ => false,
                    };
                    if is_target {
                        if let Some(op) = create_op.take() {
                            cursor.insert_after(op);
                        }
                        break;
                    }
                }

                if let Some(op) = create_op {
                    missing_targets.push(name);
                    view.create.push(op);
                }
            }
        }

        // Add diagnostics for missing targets after releasing the view borrow
        for name in missing_targets {
            job.diagnostics.push(OxcDiagnostic::warn(format!(
                "Animation target element not found for animation '{name}'"
            )));
        }
    }
}

/// Creates a placeholder expression for mem::replace.
fn create_placeholder_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> Box<'a, IrExpression<'a>> {
    let empty_expr = AngularExpression::Empty(Box::new_in(
        EmptyExpr { span: ParseSpan::new(0, 0), source_span: AbsoluteSourceSpan::new(0, 0) },
        &allocator,
    ));
    Box::new_in(IrExpression::Ast(Box::new_in(empty_expr, &allocator)), &allocator)
}

/// Converts animation bindings for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// Host bindings have no element target, so each new op is pushed to the end
/// of the create list in update-list order, as Angular's `convertAnimations` does
/// for a host job (`unit.create.push`).
pub fn convert_animations_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    let mut created = Vec::new();
    let mut cursor = job.root.update.cursor();
    cursor.move_next();
    while let Some(op) = cursor.current_mut() {
        if let UpdateOp::AnimationBinding(binding) = op {
            created.push(animation_create_op(allocator, binding));
            cursor.remove_current();
        } else {
            cursor.move_next();
        }
    }

    for op in created {
        job.root.create.push(op);
    }
}

/// Builds the create op for one animation binding.
///
/// - `AnimationBindingKind::String` -> `AnimationString` with the expression
/// - `AnimationBindingKind::Value` -> `Animation` whose handler returns the expression
///
/// Ported from `getAnimationOp` in Angular's `convert_animations.ts`.
fn animation_create_op<'a>(
    allocator: &'a oxc_allocator::Allocator,
    binding: &mut AnimationBindingOp<'a>,
) -> CreateOp<'a> {
    let source_span = binding.base.source_span;
    let animation_kind = get_animation_kind(binding.name.as_str());
    let expression =
        std::mem::replace(&mut binding.expression, create_placeholder_expression(allocator));

    match binding.kind {
        AnimationBindingKind::String => CreateOp::AnimationString(AnimationStringOp {
            base: CreateOpBase { source_span, ..Default::default() },
            target: binding.target,
            name: binding.name,
            animation_kind,
            expression,
        }),
        AnimationBindingKind::Value => {
            let wrapped_expr = OutputExpression::WrappedIrNode(Box::new_in(
                WrappedIrExpr { node: expression, source_span },
                &allocator,
            ));
            let return_stmt = OutputStatement::Return(Box::new_in(
                ReturnStatement { value: wrapped_expr, source_span: None },
                &allocator,
            ));
            let mut handler_ops = OxcVec::new_in(&allocator);
            handler_ops.push(UpdateOp::Statement(StatementOp {
                base: UpdateOpBase { source_span, ..Default::default() },
                statement: return_stmt,
            }));
            CreateOp::Animation(AnimationOp {
                base: CreateOpBase { source_span, ..Default::default() },
                target: binding.target,
                name: binding.name,
                animation_kind,
                handler_ops,
                handler_fn_name: None,
                i18n_message: None,
                security_context: SecurityContext::None,
                sanitizer: None,
            })
        }
    }
}

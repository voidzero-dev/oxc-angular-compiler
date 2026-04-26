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
//! their target element in the create list, matching Angular's architecture.
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
    AnimationOp, AnimationStringOp, CreateOp, CreateOpBase, StatementOp, UpdateOp, UpdateOpBase,
    XrefId,
};
use crate::output::ast::{OutputExpression, OutputStatement, ReturnStatement, WrappedIrExpr};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Determines the animation kind from the binding name.
/// "animate.enter" -> Enter, "animate.leave" -> Leave
fn get_animation_kind(name: &str) -> AnimationKind {
    if name == "animate.enter" { AnimationKind::Enter } else { AnimationKind::Leave }
}

/// Info needed to create Animation CreateOps.
struct AnimationInfo<'a> {
    target: XrefId,
    name: Ident<'a>,
    animation_kind: AnimationKind,
    handler_ops: OxcVec<'a, UpdateOp<'a>>,
    source_span: Option<oxc_span::Span>,
}

/// Info needed to create AnimationString CreateOps.
struct AnimationStringInfo<'a> {
    target: XrefId,
    name: Ident<'a>,
    animation_kind: AnimationKind,
    expression: Box<'a, IrExpression<'a>>,
    source_span: Option<oxc_span::Span>,
}

/// Converts animation bindings to animation runtime calls.
///
/// This phase transforms `AnimationBinding` update operations into either
/// `Animation` or `AnimationString` CreateOps based on the binding kind:
/// - `AnimationBindingKind::Value` -> `AnimationOp` (CreateOp)
/// - `AnimationBindingKind::String` -> `AnimationStringOp` (CreateOp)
///
/// Both are inserted into the create list after their target element.
pub fn convert_animations(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect view xrefs to avoid borrow issues
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        // First pass: collect all AnimationBindingOp pointers
        // We need to collect pointers first because we can't iterate and modify at the same time
        let binding_ptrs: Vec<std::ptr::NonNull<UpdateOp<'_>>> = {
            let view = match job.view_mut(view_xref) {
                Some(v) => v,
                None => continue,
            };

            let mut ptrs = Vec::new();
            for op in view.update.iter() {
                if matches!(op, UpdateOp::AnimationBinding(_)) {
                    ptrs.push(std::ptr::NonNull::from(op));
                }
            }
            ptrs
        };

        // Second pass: process each AnimationBindingOp
        let mut animations_to_create: Vec<AnimationInfo<'_>> = Vec::new();
        let mut strings_to_create: Vec<AnimationStringInfo<'_>> = Vec::new();

        for ptr in binding_ptrs {
            let view = match job.view_mut(view_xref) {
                Some(v) => v,
                None => continue,
            };

            // SAFETY: ptr was obtained from a valid reference in the list
            let op = unsafe { ptr.as_ref() };
            if let UpdateOp::AnimationBinding(binding) = op {
                let target = binding.target;
                let source_span = binding.base.source_span;
                let name = binding.name.clone();
                let kind = binding.kind;
                let animation_kind = get_animation_kind(name.as_str());

                // Extract expression by replacing with placeholder
                // SAFETY: ptr was obtained from a valid reference in the list
                let expression = unsafe {
                    let op_mut = &mut *(ptr.as_ptr() as *mut UpdateOp<'_>);
                    if let UpdateOp::AnimationBinding(binding_mut) = op_mut {
                        std::mem::replace(
                            &mut binding_mut.expression,
                            create_placeholder_expression(allocator),
                        )
                    } else {
                        continue;
                    }
                };

                // Remove from update list - both kinds become CreateOps
                // SAFETY: ptr was obtained from this list
                unsafe { view.update.remove(ptr) };

                match kind {
                    AnimationBindingKind::String => {
                        strings_to_create.push(AnimationStringInfo {
                            target,
                            name,
                            animation_kind,
                            expression,
                            source_span,
                        });
                    }
                    AnimationBindingKind::Value => {
                        // Create handler_ops with a return statement containing the expression
                        // This matches Angular's createAnimationOp which wraps expression in ReturnStatement
                        let mut handler_ops = OxcVec::new_in(allocator);

                        // Create a WrappedIrNode to hold the IR expression
                        let wrapped_expr = OutputExpression::WrappedIrNode(Box::new_in(
                            WrappedIrExpr { node: expression, source_span },
                            allocator,
                        ));

                        let return_stmt = OutputStatement::Return(Box::new_in(
                            ReturnStatement { value: wrapped_expr, source_span: None },
                            allocator,
                        ));

                        handler_ops.push(UpdateOp::Statement(StatementOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            statement: return_stmt,
                        }));

                        animations_to_create.push(AnimationInfo {
                            target,
                            name,
                            animation_kind,
                            handler_ops,
                            source_span,
                        });
                    }
                }
            }
        }

        // Third pass: insert Animation CreateOps into create list after their target elements
        if !animations_to_create.is_empty() || !strings_to_create.is_empty() {
            let mut missing_targets: Vec<Ident<'_>> = Vec::new();

            if let Some(view) = job.view_mut(view_xref) {
                // Process Animation ops (Value kind)
                for info in animations_to_create {
                    let mut create_op = Some(CreateOp::Animation(AnimationOp {
                        base: CreateOpBase { source_span: info.source_span, ..Default::default() },
                        target: info.target,
                        name: info.name.clone(),
                        animation_kind: info.animation_kind,
                        handler_ops: info.handler_ops,
                        handler_fn_name: None,
                        i18n_message: None,
                        security_context: SecurityContext::None,
                        sanitizer: None,
                    }));

                    // Find target element in create list and insert after it
                    let mut cursor = view.create.cursor();
                    while cursor.move_next() {
                        let is_target = match cursor.current() {
                            Some(CreateOp::ElementStart(el)) => el.xref == info.target,
                            Some(CreateOp::Element(el)) => el.xref == info.target,
                            Some(CreateOp::Template(t)) => t.xref == info.target,
                            Some(CreateOp::ContainerStart(c)) => c.xref == info.target,
                            Some(CreateOp::Container(c)) => c.xref == info.target,
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
                        missing_targets.push(info.name);
                        view.create.push(op);
                    }
                }

                // Process AnimationString ops (String kind)
                for info in strings_to_create {
                    let mut create_op = Some(CreateOp::AnimationString(AnimationStringOp {
                        base: CreateOpBase { source_span: info.source_span, ..Default::default() },
                        target: info.target,
                        name: info.name.clone(),
                        animation_kind: info.animation_kind,
                        expression: info.expression,
                    }));

                    // Find target element in create list and insert after it
                    let mut cursor = view.create.cursor();
                    while cursor.move_next() {
                        let is_target = match cursor.current() {
                            Some(CreateOp::ElementStart(el)) => el.xref == info.target,
                            Some(CreateOp::Element(el)) => el.xref == info.target,
                            Some(CreateOp::Template(t)) => t.xref == info.target,
                            Some(CreateOp::ContainerStart(c)) => c.xref == info.target,
                            Some(CreateOp::Container(c)) => c.xref == info.target,
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
                        missing_targets.push(info.name);
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
}

/// Creates a placeholder expression for mem::replace.
fn create_placeholder_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> Box<'a, IrExpression<'a>> {
    let empty_expr = AngularExpression::Empty(Box::new_in(
        EmptyExpr { span: ParseSpan::new(0, 0), source_span: AbsoluteSourceSpan::new(0, 0) },
        allocator,
    ));
    Box::new_in(IrExpression::Ast(Box::new_in(empty_expr, allocator)), allocator)
}

/// Converts animation bindings for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// Host bindings don't have element targets, so animation ops are pushed to create list.
pub fn convert_animations_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // First pass: collect all AnimationBindingOp pointers that need conversion.
    // Skip AnimationBindingKind::Value ops — these are [@trigger] host bindings that
    // should remain in the update list and be reified as ɵɵsyntheticHostProperty.
    // Only AnimationBindingKind::String ops (animate.enter/animate.leave) are converted.
    let binding_ptrs: Vec<std::ptr::NonNull<UpdateOp<'_>>> = {
        let mut ptrs = Vec::new();
        for op in job.root.update.iter() {
            if let UpdateOp::AnimationBinding(binding) = op {
                if matches!(binding.kind, AnimationBindingKind::String) {
                    ptrs.push(std::ptr::NonNull::from(op));
                }
            }
        }
        ptrs
    };

    // Second pass: process each AnimationBindingOp (String kind only)
    let mut strings_to_create: Vec<AnimationStringInfo<'_>> = Vec::new();

    for ptr in binding_ptrs {
        // SAFETY: ptr was obtained from a valid reference in the list
        let op = unsafe { ptr.as_ref() };
        if let UpdateOp::AnimationBinding(binding) = op {
            let target = binding.target;
            let source_span = binding.base.source_span;
            let name = binding.name.clone();
            let animation_kind = get_animation_kind(name.as_str());

            // Extract expression by replacing with placeholder
            // SAFETY: ptr was obtained from a valid reference in the list
            let expression = unsafe {
                let op_mut = &mut *(ptr.as_ptr() as *mut UpdateOp<'_>);
                if let UpdateOp::AnimationBinding(binding_mut) = op_mut {
                    std::mem::replace(
                        &mut binding_mut.expression,
                        create_placeholder_expression(allocator),
                    )
                } else {
                    continue;
                }
            };

            // Remove this op from update list
            // SAFETY: ptr was obtained from this list
            unsafe { job.root.update.remove(ptr) };

            strings_to_create.push(AnimationStringInfo {
                target,
                name,
                animation_kind,
                expression,
                source_span,
            });
        }
    }

    // Third pass: add AnimationString CreateOps to create list
    for info in strings_to_create {
        job.root.create.push(CreateOp::AnimationString(AnimationStringOp {
            base: CreateOpBase { source_span: info.source_span, ..Default::default() },
            target: info.target,
            name: info.name,
            animation_kind: info.animation_kind,
            expression: info.expression,
        }));
    }
}

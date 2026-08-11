//! Style binding specialization phase.
//!
//! Transforms special-case bindings with 'style' or 'class' in their names to specialized
//! operations. Must run before the main binding specialization pass.
//!
//! This phase handles:
//! - `BindingKind::ClassName` → `ClassPropOp`
//! - `BindingKind::StyleProperty` → `StylePropOp`
//! - `BindingKind::Property/Template` with name "style" → `StyleMapOp`
//! - `BindingKind::Property/Template` with name "class" → `ClassMapOp`
//!
//! Ported from Angular's `template/pipeline/src/phases/style_binding_specialization.ts`.

use oxc_allocator::Box;

use crate::ast::expression::{AbsoluteSourceSpan, AngularExpression, EmptyExpr, ParseSpan};
use crate::ir::enums::BindingKind;
use crate::ir::expression::IrExpression;
use crate::ir::ops::{ClassMapOp, ClassPropOp, StyleMapOp, StylePropOp, UpdateOp, UpdateOpBase};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Specializes style and class bindings to specific operations.
///
/// This must run before the main `binding_specialization` phase to ensure
/// that style/class bindings are properly converted before other bindings
/// are processed.
pub fn specialize_style_bindings(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process root view
    specialize_in_view(&mut job.root.update, &allocator);

    // Process embedded views
    for view in job.views.values_mut() {
        specialize_in_view(&mut view.update, &allocator);
    }
}

/// Creates a placeholder expression to replace one that's been moved.
fn create_placeholder_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> Box<'a, IrExpression<'a>> {
    let empty_expr = AngularExpression::Empty(Box::new_in(
        EmptyExpr { span: ParseSpan::new(0, 0), source_span: AbsoluteSourceSpan::new(0, 0) },
        &allocator,
    ));
    Box::new_in(IrExpression::Ast(Box::new_in(empty_expr, &allocator)), &allocator)
}

/// Specializes style/class bindings within a single view's update list.
fn specialize_in_view<'a>(
    update_ops: &mut crate::ir::list::UpdateOpList<'a>,
    allocator: &'a oxc_allocator::Allocator,
) {
    let mut cursor = update_ops.cursor();

    while cursor.move_next() {
        if let Some(UpdateOp::Binding(binding)) = cursor.current() {
            // Get the binding details we need for replacement
            let target = binding.target;
            let source_span = binding.base.source_span;
            let kind = binding.kind;
            let name = binding.name.clone();

            match kind {
                BindingKind::ClassName => {
                    // Angular throws if expression is Interpolation. This is an invalid state
                    // that should be caught by the parser. Skip transformation gracefully.
                    if let Some(UpdateOp::Binding(binding)) = cursor.current() {
                        if matches!(*binding.expression, IrExpression::Interpolation(_)) {
                            continue;
                        }
                    }

                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        // Move the expression out, replacing with a placeholder
                        // The old binding node becomes garbage in the arena after replace_current
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::ClassProp(ClassPropOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                BindingKind::StyleProperty => {
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let unit = binding.unit.clone();
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::StyleProp(StylePropOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            unit,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                BindingKind::Property | BindingKind::Template => {
                    // Check if name is "style" or "class"
                    if name.as_str() == "style" {
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let new_op = UpdateOp::StyleMap(StyleMapOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                expression,
                            });
                            cursor.replace_current(new_op);
                        }
                    } else if name.as_str() == "class" {
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let new_op = UpdateOp::ClassMap(ClassMapOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                expression,
                            });
                            cursor.replace_current(new_op);
                        }
                    }
                }
                _ => {
                    // Other binding kinds are handled in binding_specialization
                }
            }
        }
    }
}

/// Specializes style and class bindings for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn specialize_style_bindings_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    specialize_in_view(&mut job.root.update, &allocator);
}

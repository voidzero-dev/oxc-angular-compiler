//! Binding specialization phase.
//!
//! Converts generic `Binding` operations to specialized operations like `Property`,
//! `Attribute`, `TwoWayProperty`, `DomProperty`, and animation bindings.
//!
//! This phase runs after `style_binding_specialization`, which handles style and class
//! bindings specifically.
//!
//! Transformations:
//! - `BindingKind::Attribute` → `AttributeOp` (with namespace handling) or `AnimationBindingOp`
//! - `BindingKind::Animation` → `AnimationBindingOp`
//! - `BindingKind::Property` → `PropertyOp` (or `DomPropertyOp` for host bindings)
//! - `BindingKind::TwoWayProperty` → `TwoWayPropertyOp`
//! - `BindingKind::LegacyAnimation` → `PropertyOp`
//!
//! Special cases:
//! - `ngNonBindable` attribute: marks element and removes binding
//! - `animate.*` attributes: convert to animation bindings
//! - `field` property: convert to control binding
//!
//! Ported from Angular's `template/pipeline/src/phases/binding_specialization.ts`.

use oxc_allocator::Box;
use oxc_span::Atom;
use rustc_hash::FxHashMap;

use crate::ast::expression::{AbsoluteSourceSpan, AngularExpression, EmptyExpr, ParseSpan};
use crate::ir::enums::{AnimationBindingKind, BindingKind};
use crate::ir::expression::IrExpression;
use crate::ir::ops::{
    AnimationBindingOp, AttributeOp, ControlOp, CreateOp, DomPropertyOp, PropertyOp,
    TwoWayPropertyOp, UpdateOp, UpdateOpBase, XrefId,
};
use crate::pipeline::compilation::{
    ComponentCompilationJob, HostBindingCompilationJob, TemplateCompilationMode,
};

/// The prefix for ARIA attributes.
const ARIA_PREFIX: &str = "aria-";

/// Checks if an attribute name is an ARIA attribute.
///
/// This is a heuristic based on whether name begins with and is longer than `aria-`.
/// For example, "aria-label" and "aria-hidden" are ARIA attributes.
fn is_aria_attribute(name: &str) -> bool {
    name.starts_with(ARIA_PREFIX) && name.len() > ARIA_PREFIX.len()
}

/// Known XML/SVG namespace prefixes.
/// These are standard namespace prefixes that should be separated from the local name.
const KNOWN_NS_PREFIXES: &[&str] = &["xlink", "xml", "xmlns"];

/// Splits a namespaced name into (namespace, local_name).
///
/// Handles two formats:
/// - `:namespace:name` → (Some("namespace"), "name") - Angular's internal format
/// - `namespace:name` → (Some("namespace"), "name") - for known namespaces like xlink, xml, xmlns
/// - `name` → (None, "name") - no namespace
fn split_ns_name(name: &str) -> (Option<&str>, &str) {
    // Check Angular's internal format first: `:namespace:name`
    if name.starts_with(':') {
        if let Some(colon_index) = name[1..].find(':') {
            let namespace = &name[1..colon_index + 1];
            let local_name = &name[colon_index + 2..];
            return (Some(namespace), local_name);
        }
        // Malformed `:` prefix - fall through
    }

    // Check for known namespace prefixes: `namespace:name`
    if let Some(colon_index) = name.find(':') {
        let prefix = &name[..colon_index];
        if KNOWN_NS_PREFIXES.contains(&prefix) {
            let local_name = &name[colon_index + 1..];
            return (Some(prefix), local_name);
        }
    }

    // No namespace
    (None, name)
}

/// Specializes generic bindings to specific binding operations.
///
/// This is the main binding specialization phase that converts remaining
/// `BindingOp`s to their specific operation types like `PropertyOp`,
/// `AttributeOp`, etc.
pub fn specialize_bindings(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let mode = job.mode;

    // First pass: Build element map from create operations
    let mut elements: FxHashMap<XrefId, ElementInfo> = FxHashMap::default();

    // Process root view create ops
    for op in job.root.create.iter() {
        if let Some((xref, info)) = get_element_info(op) {
            elements.insert(xref, info);
        }
    }

    // Process embedded view create ops
    for view in job.views.values() {
        for op in view.create.iter() {
            if let Some((xref, info)) = get_element_info(op) {
                elements.insert(xref, info);
            }
        }
    }

    // Second pass: Specialize bindings and collect non-bindable xrefs
    let mut all_non_bindable: Vec<XrefId> = Vec::new();

    // Process root view
    let root_non_bindable = specialize_in_view(&mut job.root.update, allocator, &elements, mode);
    all_non_bindable.extend(root_non_bindable);

    // Process embedded views
    for view in job.views.values_mut() {
        let view_non_bindable = specialize_in_view(&mut view.update, allocator, &elements, mode);
        all_non_bindable.extend(view_non_bindable);
    }

    // Third pass: Set non_bindable flag on elements
    if !all_non_bindable.is_empty() {
        set_non_bindable_flags(&mut job.root.create, &all_non_bindable);
        for view in job.views.values_mut() {
            set_non_bindable_flags(&mut view.create, &all_non_bindable);
        }
    }
}

/// Sets the non_bindable flag on elements matching the given xrefs.
fn set_non_bindable_flags(create_ops: &mut crate::ir::list::CreateOpList<'_>, xrefs: &[XrefId]) {
    for op in create_ops.iter_mut() {
        match op {
            CreateOp::ElementStart(e) if xrefs.contains(&e.xref) => {
                e.non_bindable = true;
            }
            CreateOp::Element(e) if xrefs.contains(&e.xref) => {
                e.non_bindable = true;
            }
            CreateOp::ContainerStart(c) if xrefs.contains(&c.xref) => {
                c.non_bindable = true;
            }
            CreateOp::Container(c) if xrefs.contains(&c.xref) => {
                c.non_bindable = true;
            }
            _ => {}
        }
    }
}

/// Information about an element for binding specialization.
#[derive(Debug, Clone, Default)]
struct ElementInfo {}

/// Extracts element info from a create operation.
fn get_element_info(op: &CreateOp<'_>) -> Option<(XrefId, ElementInfo)> {
    match op {
        CreateOp::ElementStart(elem) => Some((elem.xref, ElementInfo {})),
        CreateOp::Element(elem) => Some((elem.xref, ElementInfo {})),
        CreateOp::ContainerStart(container) => Some((container.xref, ElementInfo {})),
        CreateOp::Container(container) => Some((container.xref, ElementInfo {})),
        CreateOp::Template(tmpl) => Some((tmpl.xref, ElementInfo {})),
        _ => None,
    }
}

/// Creates a placeholder expression.
fn create_placeholder_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> Box<'a, IrExpression<'a>> {
    let empty_expr = AngularExpression::Empty(Box::new_in(
        EmptyExpr { span: ParseSpan::new(0, 0), source_span: AbsoluteSourceSpan::new(0, 0) },
        allocator,
    ));
    Box::new_in(IrExpression::Ast(Box::new_in(empty_expr, allocator)), allocator)
}

/// Specializes bindings within a single view.
/// Returns a list of XrefIds for elements that should be marked as non-bindable.
fn specialize_in_view<'a>(
    update_ops: &mut crate::ir::list::UpdateOpList<'a>,
    allocator: &'a oxc_allocator::Allocator,
    _elements: &FxHashMap<XrefId, ElementInfo>,
    mode: TemplateCompilationMode,
) -> Vec<XrefId> {
    // Track ops to remove (ngNonBindable)
    let mut to_remove: Vec<std::ptr::NonNull<UpdateOp<'a>>> = Vec::new();
    // Track elements to mark as non-bindable
    let mut mark_non_bindable: Vec<XrefId> = Vec::new();

    // Process bindings
    let mut cursor = update_ops.cursor();

    while cursor.move_next() {
        if let Some(UpdateOp::Binding(binding)) = cursor.current() {
            let target = binding.target;
            let source_span = binding.base.source_span;
            let kind = binding.kind;
            let name = binding.name.clone();
            let security_context = binding.security_context;

            match kind {
                BindingKind::Attribute => {
                    if name.as_str() == "ngNonBindable" {
                        // Mark element as non-bindable and remove this op
                        if let Some(ptr) = cursor.current_ptr() {
                            to_remove.push(ptr);
                        }
                        mark_non_bindable.push(target);
                    } else if name.as_str().starts_with("animate.") {
                        // Convert to animation binding
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            // Note: Animation kind (Enter/Leave from @.type or @name) is not
                            // yet used. Currently all animation bindings are String type.
                            let new_op = UpdateOp::AnimationBinding(AnimationBindingOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                kind: AnimationBindingKind::String,
                            });
                            cursor.replace_current(new_op);
                        }
                    } else {
                        // Regular attribute binding - split namespace
                        let (namespace, local_name) = split_ns_name(name.as_str());
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let is_text_attribute = binding.is_text_attribute;
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let ns_atom = namespace.map(|ns| Atom::from(ns));
                            let local_atom = Atom::from(local_name);
                            let new_op = UpdateOp::Attribute(AttributeOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: local_atom,
                                expression,
                                namespace: ns_atom,
                                security_context,
                                sanitizer: None,
                                i18n_context: None,
                                i18n_message: binding.i18n_message,
                                is_text_attribute,
                                is_structural_template_attribute: false,
                            });
                            cursor.replace_current(new_op);
                        }
                    }
                }
                BindingKind::Animation => {
                    // Convert to animation binding with VALUE kind
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::AnimationBinding(AnimationBindingOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            kind: AnimationBindingKind::Value,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                binding_kind @ (BindingKind::Property | BindingKind::LegacyAnimation) => {
                    // In DomOnly mode, ARIA attributes should become attribute bindings
                    // since they are HTML attributes, not DOM properties.
                    // Per TypeScript's binding_specialization.ts lines 97-117.
                    if mode == TemplateCompilationMode::DomOnly && is_aria_attribute(name.as_str())
                    {
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let is_text_attribute = binding.is_text_attribute;
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let new_op = UpdateOp::Attribute(AttributeOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                namespace: None,
                                security_context,
                                sanitizer: None,
                                i18n_context: None,
                                i18n_message: binding.i18n_message,
                                is_text_attribute,
                                is_structural_template_attribute: false,
                            });
                            cursor.replace_current(new_op);
                        }
                    } else if name.as_str() == "formField" {
                        // Check for special "formField" property (control binding)
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let new_op = UpdateOp::Control(ControlOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                security_context,
                            });
                            cursor.replace_current(new_op);
                        }
                    } else {
                        // Regular property binding
                        // Note: In host binding mode, this would become DomPropertyOp
                        // For now, we convert to PropertyOp for template compilation
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let new_op = UpdateOp::Property(PropertyOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                is_host: false, // Template mode
                                security_context,
                                sanitizer: None,
                                is_structural: false,
                                i18n_context: None,
                                i18n_message: binding.i18n_message,
                                binding_kind,
                            });
                            cursor.replace_current(new_op);
                        }
                    }
                }
                BindingKind::TwoWayProperty => {
                    // Two-way property binding
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::TwoWayProperty(TwoWayPropertyOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            security_context,
                            sanitizer: None,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                BindingKind::I18n | BindingKind::ClassName | BindingKind::StyleProperty => {
                    // These should have been handled by style_binding_specialization
                    // or will be handled by i18n phases
                    // For now, leave them as-is
                }
                BindingKind::Template => {
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let is_text_attribute = binding.is_text_attribute;
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );

                        if is_text_attribute {
                            // Text attributes from structural directives (e.g., `ngFor` from `*ngFor="let item of items"`)
                            // should become AttributeOp with is_structural_template_attribute=true.
                            // This allows attribute_extraction to:
                            // 1. Extract them with BindingKind::Template for the consts array
                            // 2. Remove them from the update list (no ɵɵproperty instruction)
                            let new_op = UpdateOp::Attribute(AttributeOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                namespace: None,
                                security_context,
                                sanitizer: None,
                                i18n_context: None,
                                i18n_message: binding.i18n_message,
                                is_text_attribute: true,
                                is_structural_template_attribute: true,
                            });
                            cursor.replace_current(new_op);
                        } else {
                            // Dynamic template bindings (e.g., `ngForOf` from `*ngFor="let item of items"`)
                            // become PropertyOp which will emit ɵɵproperty instruction.
                            let new_op = UpdateOp::Property(PropertyOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                is_host: false,
                                security_context,
                                sanitizer: None,
                                is_structural: true, // Template binding
                                i18n_context: None,
                                i18n_message: binding.i18n_message,
                                binding_kind: BindingKind::Template,
                            });
                            cursor.replace_current(new_op);
                        }
                    }
                }
            }
        }
    }

    // Remove ngNonBindable operations
    for ptr in to_remove {
        unsafe {
            update_ops.remove(ptr);
        }
    }

    // Return xrefs of elements to mark as non_bindable
    mark_non_bindable
}

/// Specializes bindings for host binding compilation.
///
/// Similar to `specialize_bindings` but works with `HostBindingCompilationJob`.
/// The key difference is that property bindings become host properties (`is_host: true`).
pub fn specialize_bindings_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process bindings in the host binding unit
    let mut cursor = job.root.update.cursor();

    while cursor.move_next() {
        if let Some(UpdateOp::Binding(binding)) = cursor.current() {
            let target = binding.target;
            let source_span = binding.base.source_span;
            let kind = binding.kind;
            let name = binding.name.clone();
            let security_context = binding.security_context;

            match kind {
                BindingKind::Attribute => {
                    if name.as_str().starts_with("animate.") {
                        // Convert to animation binding
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let new_op = UpdateOp::AnimationBinding(AnimationBindingOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: binding.name.clone(),
                                expression,
                                kind: AnimationBindingKind::String,
                            });
                            cursor.replace_current(new_op);
                        }
                    } else {
                        // Regular attribute binding
                        let (namespace, local_name) = split_ns_name(name.as_str());
                        if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                            let is_text_attribute = binding.is_text_attribute;
                            let expression = std::mem::replace(
                                &mut binding.expression,
                                create_placeholder_expression(allocator),
                            );
                            let ns_atom = namespace.map(|ns| Atom::from(ns));
                            let local_atom = Atom::from(local_name);
                            let new_op = UpdateOp::Attribute(AttributeOp {
                                base: UpdateOpBase { source_span, ..Default::default() },
                                target,
                                name: local_atom,
                                expression,
                                namespace: ns_atom,
                                security_context,
                                sanitizer: None,
                                i18n_context: None,
                                i18n_message: binding.i18n_message,
                                is_text_attribute,
                                is_structural_template_attribute: false,
                            });
                            cursor.replace_current(new_op);
                        }
                    }
                }
                BindingKind::Animation => {
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::AnimationBinding(AnimationBindingOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            kind: AnimationBindingKind::Value,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                binding_kind @ (BindingKind::Property | BindingKind::LegacyAnimation) => {
                    // Host property bindings use DomPropertyOp, which emits ɵɵdomProperty
                    // This matches Angular's binding_specialization.ts for host bindings
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::DomProperty(DomPropertyOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            is_host: true,
                            security_context,
                            sanitizer: None,
                            binding_kind,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                BindingKind::TwoWayProperty => {
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::TwoWayProperty(TwoWayPropertyOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            security_context,
                            sanitizer: None,
                        });
                        cursor.replace_current(new_op);
                    }
                }
                BindingKind::I18n | BindingKind::ClassName | BindingKind::StyleProperty => {
                    // These should be handled by style_binding_specialization phase
                }
                BindingKind::Template => {
                    // Template bindings in host context become host DOM properties
                    if let Some(UpdateOp::Binding(binding)) = cursor.current_mut() {
                        let expression = std::mem::replace(
                            &mut binding.expression,
                            create_placeholder_expression(allocator),
                        );
                        let new_op = UpdateOp::DomProperty(DomPropertyOp {
                            base: UpdateOpBase { source_span, ..Default::default() },
                            target,
                            name: binding.name.clone(),
                            expression,
                            is_host: true,
                            security_context,
                            sanitizer: None,
                            binding_kind: BindingKind::Template,
                        });
                        cursor.replace_current(new_op);
                    }
                }
            }
        }
    }
}

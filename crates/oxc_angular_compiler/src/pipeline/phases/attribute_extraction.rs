//! Attribute extraction phase.
//!
//! This phase finds all extractable attribute and binding ops and creates
//! ExtractedAttributeOp operations for them. In cases where no instruction
//! needs to be generated for the attribute or binding, it is removed.
//!
//! The extracted attributes are used to build the consts array that is
//! passed to element creation instructions at runtime.
//!
//! Ported from Angular's `template/pipeline/src/phases/attribute_extraction.ts`.

use std::ptr::NonNull;

use rustc_hash::FxHashMap;

use crate::ast::r3::SecurityContext;
use crate::ir::enums::BindingKind;
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, CreateOpBase, ExtractedAttributeOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Extracts static attributes from elements into the consts array.
///
/// This phase processes various binding types and creates ExtractedAttribute
/// operations that will later be collected into the consts array.
pub fn extract_attributes(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process root view
    process_view_attributes(job, job.root.xref, allocator);

    // Process embedded views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs {
        process_view_attributes(job, view_xref, allocator);
    }
}

/// Process attributes in a single view.
///
/// This follows Angular's iteration order from `unit.ops()` which iterates:
/// 1. Create ops first (includes Listener, TwoWayListener)
/// 2. Update ops second (includes Property, TwoWayProperty, Attribute, Binding)
///
/// This ordering is important because when using `insertBefore` to add extracted
/// attributes before their target element, later insertions end up closer to the
/// element. So by processing create ops (listeners) first and update ops (properties)
/// second, we get the correct order: listeners before properties in the bindings array.
fn process_view_attributes<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) {
    // Build element map for this view (maps xref to element pointer)
    let element_map = build_element_map(job, view_xref);

    // Collect extracted attributes to add (target xref, extracted op)
    let mut extracted_attrs: Vec<(XrefId, ExtractedAttributeOp<'a>)> = Vec::new();

    // Collect pointers to ops that should be removed (extractable text attributes)
    let mut ops_to_remove: Vec<NonNull<UpdateOp<'a>>> = Vec::new();

    // IMPORTANT: Process create operations FIRST (for listeners)
    // This matches Angular's unit.ops() which iterates create ops before update ops.
    // See Angular's compilation.ts lines 197-218 for the ops() generator.
    {
        let view = if view_xref.0 == 0 {
            &job.root
        } else if let Some(v) = job.views.get(&view_xref) {
            v.as_ref()
        } else {
            return;
        };

        for op in view.create.iter() {
            match op {
                CreateOp::Listener(listener) => {
                    if !listener.is_animation_listener {
                        let extracted = ExtractedAttributeOp {
                            base: CreateOpBase::default(),
                            target: listener.target,
                            binding_kind: BindingKind::Property,
                            namespace: None,
                            name: listener.name.clone(),
                            value: None,
                            security_context: SecurityContext::None,
                            truthy_expression: false,
                            i18n_context: None,
                            i18n_message: None,
                            trusted_value_fn: None, // Listeners don't need trusted value functions
                        };
                        extracted_attrs.push((listener.target, extracted));
                    }
                }
                CreateOp::TwoWayListener(listener) => {
                    // Two-way listeners aren't supported in host bindings, but we handle
                    // them here for component templates.
                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase::default(),
                        target: listener.target,
                        binding_kind: BindingKind::Property,
                        namespace: None,
                        name: listener.name.clone(),
                        value: None,
                        security_context: SecurityContext::None,
                        truthy_expression: false,
                        i18n_context: None,
                        i18n_message: None,
                        trusted_value_fn: None,
                    };
                    extracted_attrs.push((listener.target, extracted));
                }
                _ => {}
            }
        }
    }

    // Process update operations SECOND (for properties, attributes, bindings)
    // This comes after create ops to match Angular's iteration order.
    {
        let view = if view_xref.0 == 0 {
            &job.root
        } else if let Some(v) = job.views.get(&view_xref) {
            v.as_ref()
        } else {
            return;
        };

        for op in view.update.iter() {
            match op {
                UpdateOp::Attribute(attr_op) => {
                    // Check if this attribute is extractable:
                    // - Text attributes (static attributes from template) are always extractable
                    // - Non-interpolation constant expressions are also extractable
                    // Ported from Angular's attribute_extraction.ts line 194:
                    //   let extractable = op.isTextAttribute || op.expression.isConstant();
                    let is_interpolation =
                        matches!(*attr_op.expression, IrExpression::Interpolation(_));
                    let extractable = attr_op.is_text_attribute
                        || (!is_interpolation && is_extractable_expression(&attr_op.expression));

                    if extractable {
                        // Extract the value from the expression
                        // The expression may have come from a Binding op that was converted
                        // by the binding_specialization phase
                        let value = extract_value_from_binding_expr(allocator, &attr_op.expression);

                        // Determine extracted binding kind:
                        // - Structural template attributes (e.g., ngFor from *ngFor="let item of items")
                        //   should use BindingKind::Template for directive matching
                        // - Regular attributes use BindingKind::Attribute
                        // Ported from Angular's attribute_extraction.ts line 204:
                        //   op.isStructuralTemplateAttribute ? ir.BindingKind.Template : ir.BindingKind.Attribute
                        let binding_kind = if attr_op.is_structural_template_attribute {
                            BindingKind::Template
                        } else {
                            BindingKind::Attribute
                        };

                        // Create extracted attribute
                        let extracted = ExtractedAttributeOp {
                            base: CreateOpBase::default(),
                            target: attr_op.target,
                            binding_kind,
                            namespace: attr_op.namespace.clone(),
                            name: attr_op.name.clone(),
                            value,
                            security_context: attr_op.security_context,
                            truthy_expression: false,
                            i18n_context: attr_op.i18n_context,
                            i18n_message: attr_op.i18n_message,
                            trusted_value_fn: None, // Set by resolve_sanitizers phase
                        };
                        extracted_attrs.push((attr_op.target, extracted));
                        // Mark this op for removal - extractable attributes don't need
                        // runtime updates since they're emitted in the consts array
                        ops_to_remove.push(NonNull::from(op));
                    }
                }
                UpdateOp::Property(prop_op) => {
                    // Skip animation bindings - they don't participate in directive matching
                    // and are handled separately via property() instruction at runtime.
                    if matches!(
                        prop_op.binding_kind,
                        BindingKind::Animation | BindingKind::LegacyAnimation
                    ) {
                        continue;
                    }

                    // Angular's attribute_extraction.ts (lines 31-40):
                    //   if (op.i18nMessage !== null && op.templateKind === null)
                    //     bindingKind = ir.BindingKind.I18n;
                    //
                    // The I18n binding kind applies only to interpolated attributes
                    // with i18n markers (e.g., heading="Join {{ name }}" i18n-heading).
                    // Pure property bindings ([attr]="expr" i18n-attr) keep Property
                    // kind because the runtime uses domProperty, not i18nAttributes.
                    let binding_kind = if prop_op.i18n_message.is_some()
                        && prop_op.binding_kind != BindingKind::Template
                        && matches!(*prop_op.expression, IrExpression::Interpolation(_))
                    {
                        BindingKind::I18n
                    } else {
                        prop_op.binding_kind
                    };

                    // Properties also generate extracted attributes for directive matching
                    // Note: Property ops are NOT removed - they still need runtime updates
                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase::default(),
                        target: prop_op.target,
                        binding_kind,
                        namespace: None,
                        name: prop_op.name.clone(),
                        value: None, // Property bindings don't copy the expression
                        security_context: prop_op.security_context,
                        truthy_expression: false,
                        i18n_context: None,
                        i18n_message: None,
                        trusted_value_fn: None, // Set by resolve_sanitizers phase
                    };
                    extracted_attrs.push((prop_op.target, extracted));
                }
                UpdateOp::TwoWayProperty(twp_op) => {
                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase::default(),
                        target: twp_op.target,
                        binding_kind: BindingKind::TwoWayProperty,
                        namespace: None,
                        name: twp_op.name.clone(),
                        value: None,
                        security_context: twp_op.security_context,
                        truthy_expression: false,
                        i18n_context: None,
                        i18n_message: None,
                        trusted_value_fn: None, // Set by resolve_sanitizers phase
                    };
                    extracted_attrs.push((twp_op.target, extracted));
                }
                // StyleProp and ClassProp bindings:
                // In Angular TypeScript, these are only extracted in compatibility mode
                // (TemplateDefinitionBuilder) when the expression is empty. We don't support
                // compatibility mode, so we skip extraction for these binding types.
                // The bindings themselves still work - they just don't generate extracted
                // attributes for directive matching purposes.
                UpdateOp::StyleProp(_) | UpdateOp::ClassProp(_) => {
                    // In compatibility mode with empty expressions, these would be extracted.
                    // Since we target modern Angular only, we skip this.
                }
                // Handle generic Binding ops (from ingest_control_flow_insertion_point)
                // These are created for content projection and should be extracted if they're
                // text attributes or have constant expressions.
                UpdateOp::Binding(binding_op) => {
                    // Animation bindings are NOT extractable - they're handled separately
                    // by the property() instruction at runtime.
                    if matches!(
                        binding_op.kind,
                        BindingKind::Animation | BindingKind::LegacyAnimation
                    ) {
                        continue;
                    }

                    // Check if this binding is extractable:
                    // - Text attributes (static attributes from template) are always extractable
                    // - Non-interpolation constant expressions are also extractable
                    let is_interpolation =
                        matches!(*binding_op.expression, IrExpression::Interpolation(_));
                    let extractable = binding_op.is_text_attribute
                        || (!is_interpolation && is_extractable_expression(&binding_op.expression));

                    if extractable {
                        // Create extracted attribute based on the binding kind
                        // Preserve Template kind for structural directive bindings
                        let extracted_kind = match binding_op.kind {
                            BindingKind::Attribute => BindingKind::Attribute,
                            BindingKind::Property => BindingKind::Property,
                            BindingKind::TwoWayProperty => BindingKind::TwoWayProperty,
                            BindingKind::Template => BindingKind::Template,
                            _ => BindingKind::Attribute, // Default to attribute for others
                        };

                        // Extract the value from the binding expression
                        // Convert AST literal primitives to OutputExpr format for const_collection
                        let value =
                            extract_value_from_binding_expr(allocator, &binding_op.expression);

                        let extracted = ExtractedAttributeOp {
                            base: CreateOpBase::default(),
                            target: binding_op.target,
                            binding_kind: extracted_kind,
                            namespace: None,
                            name: binding_op.name.clone(),
                            value,
                            security_context: binding_op.security_context,
                            truthy_expression: false,
                            i18n_context: None,
                            i18n_message: binding_op.i18n_message,
                            trusted_value_fn: None,
                        };
                        extracted_attrs.push((binding_op.target, extracted));
                        // Mark this op for removal - extractable text bindings don't need
                        // runtime updates since they're handled by the consts array
                        ops_to_remove.push(NonNull::from(op));
                    }
                }
                _ => {}
            }
        }
    }

    // Remove extractable attribute ops from the update list
    // These are text attributes that are fully handled by the consts array
    if !ops_to_remove.is_empty() {
        let view = if view_xref.0 == 0 {
            &mut job.root
        } else if let Some(v) = job.views.get_mut(&view_xref) {
            v.as_mut()
        } else {
            return;
        };

        for op_ptr in ops_to_remove {
            // SAFETY: op_ptr was obtained from iterating this list
            unsafe {
                view.update.remove(op_ptr);
            }
        }
    }

    // Insert extracted attributes before their target elements
    for (target, extracted) in extracted_attrs {
        if let Some(&element_ptr) = element_map.get(&target) {
            let create_op = CreateOp::ExtractedAttribute(extracted);
            if view_xref.0 == 0 {
                // SAFETY: element_ptr is a valid pointer obtained from iteration
                unsafe {
                    job.root.create.insert_before(element_ptr, create_op);
                }
            } else if let Some(view) = job.views.get_mut(&view_xref) {
                unsafe {
                    view.create.insert_before(element_ptr, create_op);
                }
            }
        } else {
            // Fallback: push to end if target element not found
            if view_xref.0 == 0 {
                job.root.create.push(CreateOp::ExtractedAttribute(extracted));
            } else if let Some(view) = job.views.get_mut(&view_xref) {
                view.create.push(CreateOp::ExtractedAttribute(extracted));
            }
        }
    }
}

/// Builds a map of element xrefs to their pointers in the create list.
fn build_element_map<'a>(
    job: &ComponentCompilationJob<'a>,
    view_xref: XrefId,
) -> FxHashMap<XrefId, NonNull<CreateOp<'a>>> {
    let mut map = FxHashMap::default();

    let view = if view_xref.0 == 0 {
        &job.root
    } else if let Some(v) = job.views.get(&view_xref) {
        v.as_ref()
    } else {
        return map;
    };

    for op in view.create.iter() {
        match op {
            CreateOp::ElementStart(el) => {
                map.insert(el.xref, NonNull::from(op));
            }
            CreateOp::Element(el) => {
                map.insert(el.xref, NonNull::from(op));
            }
            CreateOp::Template(tmpl) => {
                map.insert(tmpl.xref, NonNull::from(op));
            }
            // RepeaterCreate ops need to be in the map for control flow insertion point
            // attributes. The target of these attributes is the body_view xref.
            CreateOp::RepeaterCreate(rep) => {
                map.insert(rep.body_view, NonNull::from(op));
                // Also add the empty view if present
                if let Some(empty_view) = rep.empty_view {
                    map.insert(empty_view, NonNull::from(op));
                }
            }
            // Conditional ops (from @if/@switch) also need to be in the map.
            CreateOp::Conditional(cond) => {
                map.insert(cond.xref, NonNull::from(op));
            }
            CreateOp::ConditionalBranch(branch) => {
                map.insert(branch.xref, NonNull::from(op));
            }
            // Projection ops (ng-content) need to be in the map for their attributes
            // to be extracted. The attributes are created as BindingOps targeting the
            // projection's xref.
            CreateOp::Projection(proj) => {
                map.insert(proj.xref, NonNull::from(op));
            }
            _ => {}
        }
    }

    map
}

/// Checks if an expression is extractable (constant).
/// An expression is extractable if it's:
/// - Empty (text attribute without value)
/// - A constant literal (string, number, boolean, null)
fn is_extractable_expression(expr: &IrExpression<'_>) -> bool {
    use crate::ast::expression::AngularExpression;

    match expr {
        IrExpression::Empty(_) => true,
        IrExpression::Ast(ast_expr) => {
            // Check if it's a constant literal primitive
            matches!(ast_expr.as_ref(), AngularExpression::LiteralPrimitive(_))
        }
        _ => false,
    }
}

/// Extracts the value from a binding expression and converts it to OutputExpr format.
/// This is needed because binding expressions from control flow insertion points use
/// AST LiteralPrimitive, but const_collection expects OutputExpr Literal format.
fn extract_value_from_binding_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &IrExpression<'a>,
) -> Option<oxc_allocator::Box<'a, IrExpression<'a>>> {
    use crate::ast::expression::{AngularExpression, LiteralValue as AstLiteralValue};
    use crate::output::ast::{LiteralExpr, LiteralValue as OutputLiteralValue, OutputExpression};
    use oxc_allocator::Box;

    match expr {
        IrExpression::Ast(ast_expr) => {
            if let AngularExpression::LiteralPrimitive(lit) = ast_expr.as_ref() {
                // Convert AST literal value to output literal value
                let output_value = match &lit.value {
                    AstLiteralValue::String(s) => OutputLiteralValue::String(s.clone()),
                    AstLiteralValue::Number(n) => OutputLiteralValue::Number(*n),
                    AstLiteralValue::Boolean(b) => OutputLiteralValue::Boolean(*b),
                    AstLiteralValue::Null | AstLiteralValue::Undefined => OutputLiteralValue::Null,
                };

                let literal_expr = OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: output_value, source_span: None },
                    allocator,
                ));
                let value_expr = IrExpression::OutputExpr(Box::new_in(literal_expr, allocator));
                Some(Box::new_in(value_expr, allocator))
            } else {
                None
            }
        }
        IrExpression::OutputExpr(output_expr) => {
            // Already in the right format - clone it and wrap in IrExpression
            // This is needed for host attributes from decorators which are already OutputExpr literals
            let cloned = output_expr.clone_in(allocator);
            let value_expr = IrExpression::OutputExpr(Box::new_in(cloned, allocator));
            Some(Box::new_in(value_expr, allocator))
        }
        IrExpression::Empty(_) => {
            // Empty expression means no value
            None
        }
        _ => None,
    }
}

/// Extracts attributes for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// Ported from Angular's extractAttributes for host bindings (CompilationJobKind.Host).
///
/// Note: For host bindings, Angular says order doesn't matter because the attributes
/// apply to the host element, not to child elements. However, we still follow the
/// same create-before-update ordering for consistency with component templates.
pub fn extract_attributes_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect extracted attributes to add and ops to remove
    let mut extracted_attrs: Vec<ExtractedAttributeOp<'_>> = Vec::new();
    let mut ops_to_remove: Vec<std::ptr::NonNull<UpdateOp<'_>>> = Vec::new();

    // IMPORTANT: Process create operations FIRST (for listeners)
    // This matches Angular's unit.ops() which iterates create ops before update ops.
    // Although Angular says order doesn't matter for host bindings, we maintain
    // consistent ordering with component templates for predictability.
    for op in job.root.create.iter() {
        if let CreateOp::Listener(listener) = op {
            if !listener.is_animation_listener {
                let extracted = ExtractedAttributeOp {
                    base: CreateOpBase::default(),
                    target: listener.target,
                    binding_kind: BindingKind::Property,
                    namespace: None,
                    name: listener.name.clone(),
                    value: None,
                    security_context: SecurityContext::None,
                    truthy_expression: false,
                    i18n_context: None,
                    i18n_message: None,
                    trusted_value_fn: None,
                };
                extracted_attrs.push(extracted);
            }
        }
    }

    // Process update operations SECOND (for properties, attributes, bindings)
    for op in job.root.update.iter() {
        match op {
            UpdateOp::Attribute(attr_op) => {
                // Check if this attribute is extractable:
                // - Text attributes (static attributes) are always extractable
                // - Non-interpolation constant expressions are also extractable
                // Angular's extractAttributeOp (line 194): op.isTextAttribute || op.expression.isConstant()
                let is_interpolation =
                    matches!(*attr_op.expression, IrExpression::Interpolation(_));
                let extractable = attr_op.is_text_attribute
                    || (!is_interpolation && is_extractable_expression(&attr_op.expression));

                if extractable {
                    // Extract the value from the expression
                    let value = extract_value_from_binding_expr(allocator, &attr_op.expression);

                    // Create extracted attribute
                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase::default(),
                        target: attr_op.target,
                        binding_kind: BindingKind::Attribute,
                        namespace: attr_op.namespace.clone(),
                        name: attr_op.name.clone(),
                        value,
                        security_context: attr_op.security_context,
                        truthy_expression: false,
                        i18n_context: attr_op.i18n_context,
                        i18n_message: attr_op.i18n_message,
                        trusted_value_fn: None,
                    };
                    extracted_attrs.push(extracted);
                    // Mark this op for removal - extractable attributes don't need
                    // runtime updates since they're emitted in the consts array
                    ops_to_remove.push(std::ptr::NonNull::from(op));
                }
            }
            UpdateOp::DomProperty(dom_prop_op) => {
                // DomProperty bindings in host context also generate extracted attributes
                // for directive matching, similar to Property in templates.
                // Skip animation bindings.
                if matches!(
                    dom_prop_op.binding_kind,
                    BindingKind::Animation | BindingKind::LegacyAnimation
                ) {
                    continue;
                }

                let extracted = ExtractedAttributeOp {
                    base: CreateOpBase::default(),
                    target: dom_prop_op.target,
                    binding_kind: dom_prop_op.binding_kind,
                    namespace: None,
                    name: dom_prop_op.name.clone(),
                    value: None, // Property bindings don't copy the expression
                    security_context: dom_prop_op.security_context,
                    truthy_expression: false,
                    i18n_context: None,
                    i18n_message: None,
                    trusted_value_fn: None,
                };
                extracted_attrs.push(extracted);
                // Note: DomProperty ops are NOT removed - they still need runtime updates
            }
            // Handle generic Binding ops that weren't specialized yet
            UpdateOp::Binding(binding_op) => {
                // Animation bindings are NOT extractable
                if matches!(binding_op.kind, BindingKind::Animation | BindingKind::LegacyAnimation)
                {
                    continue;
                }

                // Check if this binding is extractable
                let is_interpolation =
                    matches!(*binding_op.expression, IrExpression::Interpolation(_));
                let extractable = binding_op.is_text_attribute
                    || (!is_interpolation && is_extractable_expression(&binding_op.expression));

                if extractable {
                    let value = extract_value_from_binding_expr(allocator, &binding_op.expression);

                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase::default(),
                        target: binding_op.target,
                        binding_kind: binding_op.kind,
                        namespace: None,
                        name: binding_op.name.clone(),
                        value,
                        security_context: binding_op.security_context,
                        truthy_expression: false,
                        i18n_context: None,
                        i18n_message: binding_op.i18n_message,
                        trusted_value_fn: None,
                    };
                    extracted_attrs.push(extracted);
                    // Mark this op for removal
                    ops_to_remove.push(std::ptr::NonNull::from(op));
                }
            }
            _ => {}
        }
    }

    // Remove extractable ops from the update list
    for op_ptr in ops_to_remove {
        // SAFETY: op_ptr was obtained from iterating this list
        unsafe {
            job.root.update.remove(op_ptr);
        }
    }

    // Add extracted attributes to the create list
    for extracted in extracted_attrs {
        job.root.create.push(CreateOp::ExtractedAttribute(extracted));
    }
}

//! Slot allocation phase.
//!
//! Assigns data slots for all operations which consume slots, and propagates the assigned
//! data slots of those operations to any expressions which reference them.
//!
//! This phase is also responsible for counting the number of slots used for each view (its `decls`)
//! and propagating that number into the `Template` operations which declare embedded views.
//!
//! Ported from Angular's `template/pipeline/src/phases/slot_allocation.ts`.

use rustc_hash::FxHashMap;

use crate::ir::ops::{CreateOp, I18nSlotHandle, SlotId, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Allocates slot indices for elements and templates in each view.
///
/// Slot indices are used by the Angular runtime to identify elements, templates,
/// and other entities during change detection and rendering.
pub fn allocate_slots(job: &mut ComponentCompilationJob<'_>) {
    // Map of all declarations in all views within the component which require an assigned slot index.
    // This map is global across all views since it's possible to reference a slot from one view
    // in an expression within another (e.g., local references work this way).
    let mut slot_map: FxHashMap<XrefId, SlotId> = FxHashMap::default();

    // First pass: Allocate slots for all create operations that consume slots.
    // Collect the decl counts for each view.
    let mut view_decls: FxHashMap<XrefId, u32> = FxHashMap::default();

    // Process root view first
    let root_xref = job.root.xref;
    let root_slot_count = allocate_slots_in_view(&mut job.root.create, &mut slot_map);
    view_decls.insert(root_xref, root_slot_count);
    job.root.decl_count = Some(root_slot_count);

    // Process all embedded views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs.iter().copied() {
        if let Some(view) = job.views.get_mut(&view_xref) {
            let slot_count = allocate_slots_in_view(&mut view.create, &mut slot_map);
            view_decls.insert(view_xref, slot_count);
            view.decl_count = Some(slot_count);
        }
    }

    // Second pass: Propagate decl counts to Template, Conditional, and Repeater operations.
    // These operations need to know the decl count of the views they create.
    propagate_decls_to_templates(job, &view_decls);

    // Third pass: Resolve defer slots from their TemplateOp xrefs.
    // This sets main_slot, loading_slot, placeholder_slot, error_slot on DeferOp.
    propagate_slots_to_defer_ops(job, &slot_map);

    // Fourth pass: Propagate slots to expressions that reference them (e.g., PipeBinding, Reference).
    // This is necessary because expressions store their own slot handles that need to be updated.
    propagate_slots_to_expressions(job, &slot_map);
}

/// Allocates slots within a single view's create operations.
/// Returns the total number of slots used.
fn allocate_slots_in_view<'a>(
    ops: &mut crate::ir::list::CreateOpList<'a>,
    slot_map: &mut FxHashMap<XrefId, SlotId>,
) -> u32 {
    let mut slot_count: u32 = 0;

    for op in ops.iter_mut() {
        // Get the number of slots this operation consumes and assign slots
        match op {
            CreateOp::ElementStart(elem) => {
                let num_slots = 1 + elem.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                elem.slot = Some(slot);
                slot_map.insert(elem.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::Element(elem) => {
                let num_slots = 1 + elem.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                elem.slot = Some(slot);
                slot_map.insert(elem.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::Template(tmpl) => {
                // Template uses 1 slot + local_refs.len() for each local reference
                let num_slots = 1 + tmpl.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                tmpl.slot = Some(slot);
                slot_map.insert(tmpl.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::ContainerStart(container) => {
                // Container uses 1 slot + local_refs.len() for each local reference
                let num_slots = 1 + container.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                container.slot = Some(slot);
                slot_map.insert(container.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::Container(container) => {
                // Container uses 1 slot + local_refs.len() for each local reference
                let num_slots = 1 + container.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                container.slot = Some(slot);
                slot_map.insert(container.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::Text(text) => {
                let slot = SlotId(slot_count);
                text.slot = Some(slot);
                slot_map.insert(text.xref, slot);
                slot_count += 1;
            }
            CreateOp::Pipe(pipe) => {
                let slot = SlotId(slot_count);
                pipe.slot = Some(slot);
                slot_map.insert(pipe.xref, slot);
                slot_count += 1;
            }
            CreateOp::Projection(proj) => {
                // Projection uses 1 slot normally, or 2 if it has a fallback
                let num_slots = if proj.fallback.is_some() { 2 } else { 1 };
                let slot = SlotId(slot_count);
                proj.slot = Some(slot);
                slot_map.insert(proj.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::Conditional(cond) => {
                // Conditional uses 1 slot + local_refs.len() for each local reference
                let num_slots = 1 + cond.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                cond.slot = Some(slot);
                slot_map.insert(cond.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::ConditionalBranch(branch) => {
                // ConditionalBranch uses 1 slot + local_refs.len() for each local reference
                let num_slots = 1 + branch.local_refs.len() as u32;
                let slot = SlotId(slot_count);
                branch.slot = Some(slot);
                slot_map.insert(branch.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::RepeaterCreate(rep) => {
                // Repeater uses 2 slots normally, or 3 if it has an empty view
                let num_slots = if rep.empty_view.is_some() { 3 } else { 2 };
                let slot = SlotId(slot_count);
                rep.slot = Some(slot);
                slot_map.insert(rep.xref, slot);
                slot_count += num_slots;
            }
            CreateOp::Defer(defer) => {
                // Defer uses 2 slots, matching TypeScript's numSlotsUsed: 2 in createDeferOp
                let slot = SlotId(slot_count);
                defer.slot = Some(slot);
                slot_map.insert(defer.xref, slot);
                slot_count += 2;
            }
            CreateOp::DeclareLet(let_op) => {
                let slot = SlotId(slot_count);
                let_op.slot = Some(slot);
                slot_map.insert(let_op.xref, slot);
                slot_count += 1;
            }
            CreateOp::I18nStart(i18n) => {
                let slot = SlotId(slot_count);
                i18n.slot = Some(slot);
                slot_map.insert(i18n.xref, slot);
                slot_count += 1;
            }
            CreateOp::I18n(i18n) => {
                let slot = SlotId(slot_count);
                i18n.slot = Some(slot);
                slot_map.insert(i18n.xref, slot);
                slot_count += 1;
            }
            CreateOp::I18nAttributes(i18n_attrs) => {
                // I18nAttributes uses I18nSlotHandle instead of Option<SlotId>
                let slot = SlotId(slot_count);
                i18n_attrs.handle = I18nSlotHandle::Single(slot);
                slot_map.insert(i18n_attrs.xref, slot);
                slot_count += 1;
            }
            // These operations don't consume slots
            _ => {}
        }
    }

    // Second pass: resolve listener target_slots
    for op in ops.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                if let Some(&slot) = slot_map.get(&listener.target) {
                    listener.target_slot = slot;
                }
            }
            CreateOp::TwoWayListener(listener) => {
                if let Some(&slot) = slot_map.get(&listener.target) {
                    listener.target_slot = slot;
                }
            }
            CreateOp::AnimationListener(listener) => {
                if let Some(&slot) = slot_map.get(&listener.target) {
                    listener.target_slot = slot;
                }
            }
            _ => {}
        }
    }

    slot_count
}

/// Propagates decl counts to Template, Conditional, and Repeater operations.
fn propagate_decls_to_templates(
    job: &mut ComponentCompilationJob<'_>,
    view_decls: &FxHashMap<XrefId, u32>,
) {
    // Process root view create ops
    propagate_decls_in_ops(&mut job.root.create, view_decls);

    // Process all embedded views
    for view in job.views.values_mut() {
        propagate_decls_in_ops(&mut view.create, view_decls);
    }
}

/// Propagates decl counts within a single view's operations.
fn propagate_decls_in_ops<'a>(
    ops: &mut crate::ir::list::CreateOpList<'a>,
    view_decls: &FxHashMap<XrefId, u32>,
) {
    for op in ops.iter_mut() {
        match op {
            CreateOp::Template(tmpl) => {
                // Template's embedded_view is the view it creates
                if let Some(&decls) = view_decls.get(&tmpl.embedded_view) {
                    tmpl.decl_count = Some(decls);
                }
            }
            CreateOp::Conditional(cond) => {
                // ConditionalOp.xref is the view xref for this branch
                // Look up the decls directly from view_decls
                if let Some(&decls) = view_decls.get(&cond.xref) {
                    cond.decls = Some(decls);
                }
            }
            CreateOp::ConditionalBranch(branch) => {
                // ConditionalBranch views are their own embedded views
                if let Some(&decls) = view_decls.get(&branch.xref) {
                    branch.decls = Some(decls);
                }
            }
            CreateOp::RepeaterCreate(rep) => {
                // RepeaterCreate references body_view for repeated content.
                if let Some(&decls) = view_decls.get(&rep.body_view) {
                    rep.decls = Some(decls);
                }
                // Also set empty_decl_count if there's an empty view
                if let Some(empty_view) = rep.empty_view {
                    if let Some(&decls) = view_decls.get(&empty_view) {
                        rep.empty_decl_count = Some(decls);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Propagates slots from TemplateOps to DeferOps.
///
/// DeferOps store xrefs to their TemplateOps (main_view, loading_view, etc.),
/// and this function resolves those xrefs to actual slot IDs.
fn propagate_slots_to_defer_ops(
    job: &mut ComponentCompilationJob<'_>,
    slot_map: &FxHashMap<XrefId, SlotId>,
) {
    // Process root view
    propagate_defer_slots_in_ops(&mut job.root.create, slot_map);

    // Process all embedded views
    for view in job.views.values_mut() {
        propagate_defer_slots_in_ops(&mut view.create, slot_map);
    }
}

/// Propagates defer slots within a single view's operations.
fn propagate_defer_slots_in_ops<'a>(
    ops: &mut crate::ir::list::CreateOpList<'a>,
    slot_map: &FxHashMap<XrefId, SlotId>,
) {
    for op in ops.iter_mut() {
        match op {
            CreateOp::Defer(defer) => {
                // Resolve main_slot from main_view (TemplateOp xref)
                if let Some(main_xref) = defer.main_view {
                    if let Some(&slot) = slot_map.get(&main_xref) {
                        defer.main_slot = Some(slot);
                    }
                }
                // Resolve loading_slot from loading_view (TemplateOp xref)
                if let Some(loading_xref) = defer.loading_view {
                    if let Some(&slot) = slot_map.get(&loading_xref) {
                        defer.loading_slot = Some(slot);
                    }
                }
                // Resolve placeholder_slot from placeholder_view (TemplateOp xref)
                if let Some(placeholder_xref) = defer.placeholder_view {
                    if let Some(&slot) = slot_map.get(&placeholder_xref) {
                        defer.placeholder_slot = Some(slot);
                    }
                }
                // Resolve error_slot from error_view (TemplateOp xref)
                if let Some(error_xref) = defer.error_view {
                    if let Some(&slot) = slot_map.get(&error_xref) {
                        defer.error_slot = Some(slot);
                    }
                }
            }
            CreateOp::DeferOn(defer_on) => {
                // Resolve target_slot from target_xref.
                // This is set by defer_resolve_targets phase and needs slot propagation.
                if let Some(target_xref) = defer_on.target_xref {
                    if let Some(&slot) = slot_map.get(&target_xref) {
                        defer_on.target_slot = Some(slot);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Propagates allocated slots to expressions that reference them.
///
/// This is needed because expressions like PipeBinding and Reference store their own slot
/// handles that need to be updated with the slots allocated during the first pass.
fn propagate_slots_to_expressions(
    job: &mut ComponentCompilationJob<'_>,
    slot_map: &FxHashMap<XrefId, SlotId>,
) {
    use crate::ir::expression::IrExpression;
    use crate::ir::ops::UpdateOp;

    // Helper function to update slots in an expression tree
    fn update_expression_slots(expr: &mut IrExpression<'_>, slot_map: &FxHashMap<XrefId, SlotId>) {
        match expr {
            IrExpression::PipeBinding(pipe) => {
                // Update the pipe's target slot with the allocated slot
                if let Some(&slot) = slot_map.get(&pipe.target) {
                    pipe.target_slot.slot = Some(slot);
                }
                // Recursively update arguments
                for arg in pipe.args.iter_mut() {
                    update_expression_slots(arg, slot_map);
                }
            }
            IrExpression::PipeBindingVariadic(pipe) => {
                if let Some(&slot) = slot_map.get(&pipe.target) {
                    pipe.target_slot.slot = Some(slot);
                }
                update_expression_slots(&mut pipe.args, slot_map);
            }
            IrExpression::Reference(ref_expr) => {
                // Reference expressions also need slot propagation
                if let Some(&slot) = slot_map.get(&ref_expr.target) {
                    ref_expr.target_slot.slot = Some(slot);
                }
            }
            IrExpression::ContextLetReference(ctx_let) => {
                if let Some(&slot) = slot_map.get(&ctx_let.target) {
                    ctx_let.target_slot.slot = Some(slot);
                }
            }
            IrExpression::PureFunction(pf) => {
                for arg in pf.args.iter_mut() {
                    update_expression_slots(arg, slot_map);
                }
            }
            IrExpression::SafePropertyRead(safe) => {
                update_expression_slots(&mut safe.receiver, slot_map);
            }
            IrExpression::SafeKeyedRead(safe) => {
                update_expression_slots(&mut safe.receiver, slot_map);
                update_expression_slots(&mut safe.index, slot_map);
            }
            IrExpression::SafeInvokeFunction(safe) => {
                update_expression_slots(&mut safe.receiver, slot_map);
                for arg in safe.args.iter_mut() {
                    update_expression_slots(arg, slot_map);
                }
            }
            IrExpression::SafeTernary(ternary) => {
                update_expression_slots(&mut ternary.guard, slot_map);
                update_expression_slots(&mut ternary.expr, slot_map);
            }
            IrExpression::Interpolation(interp) => {
                for expr in interp.expressions.iter_mut() {
                    update_expression_slots(expr, slot_map);
                }
            }
            IrExpression::SlotLiteral(slot_literal) => {
                // Update slot literal based on target_xref if available
                if let Some(xref) = slot_literal.target_xref {
                    if let Some(&slot) = slot_map.get(&xref) {
                        slot_literal.slot.slot = Some(slot);
                    }
                }
            }
            IrExpression::Ternary(ternary) => {
                // Recursively update slots in ternary children
                update_expression_slots(&mut ternary.condition, slot_map);
                update_expression_slots(&mut ternary.true_expr, slot_map);
                update_expression_slots(&mut ternary.false_expr, slot_map);
            }
            IrExpression::Ast(ast) => {
                // Walk nested expressions in AST nodes
                update_angular_expression_slots(ast.as_mut(), slot_map);
            }
            IrExpression::AssignTemporary(assign) => {
                // Recursively update nested expression in AssignTemporary
                update_expression_slots(&mut assign.expr, slot_map);
            }
            IrExpression::Binary(binary) => {
                update_expression_slots(&mut binary.lhs, slot_map);
                update_expression_slots(&mut binary.rhs, slot_map);
            }
            IrExpression::StoreLet(store) => {
                update_expression_slots(&mut store.value, slot_map);
            }
            IrExpression::ResetView(rv) => {
                update_expression_slots(&mut rv.expr, slot_map);
            }
            IrExpression::RestoreView(rv) => {
                if let crate::ir::expression::RestoreViewTarget::Dynamic(inner) = &mut rv.view {
                    update_expression_slots(inner, slot_map);
                }
            }
            IrExpression::ResolvedPropertyRead(resolved) => {
                update_expression_slots(&mut resolved.receiver, slot_map);
            }
            IrExpression::ResolvedBinary(resolved) => {
                update_expression_slots(&mut resolved.left, slot_map);
                update_expression_slots(&mut resolved.right, slot_map);
            }
            IrExpression::ResolvedCall(resolved) => {
                update_expression_slots(&mut resolved.receiver, slot_map);
                for arg in resolved.args.iter_mut() {
                    update_expression_slots(arg, slot_map);
                }
            }
            IrExpression::ResolvedKeyedRead(resolved) => {
                update_expression_slots(&mut resolved.receiver, slot_map);
                update_expression_slots(&mut resolved.key, slot_map);
            }
            IrExpression::ResolvedSafePropertyRead(resolved) => {
                update_expression_slots(&mut resolved.receiver, slot_map);
            }
            IrExpression::DerivedLiteralArray(arr) => {
                for entry in arr.entries.iter_mut() {
                    update_expression_slots(entry, slot_map);
                }
            }
            IrExpression::DerivedLiteralMap(map) => {
                for value in map.values.iter_mut() {
                    update_expression_slots(value, slot_map);
                }
            }
            IrExpression::LiteralArray(arr) => {
                for elem in arr.elements.iter_mut() {
                    update_expression_slots(elem, slot_map);
                }
            }
            IrExpression::LiteralMap(map) => {
                for value in map.values.iter_mut() {
                    update_expression_slots(value, slot_map);
                }
            }
            IrExpression::ConstCollected(cc) => {
                update_expression_slots(&mut cc.expr, slot_map);
            }
            IrExpression::TwoWayBindingSet(tbs) => {
                update_expression_slots(&mut tbs.target, slot_map);
                update_expression_slots(&mut tbs.value, slot_map);
            }
            IrExpression::Not(not) => {
                update_expression_slots(&mut not.expr, slot_map);
            }
            IrExpression::Unary(unary) => {
                update_expression_slots(&mut unary.expr, slot_map);
            }
            IrExpression::Typeof(type_of) => {
                update_expression_slots(&mut type_of.expr, slot_map);
            }
            IrExpression::Void(void) => {
                update_expression_slots(&mut void.expr, slot_map);
            }
            IrExpression::Parenthesized(paren) => {
                update_expression_slots(&mut paren.expr, slot_map);
            }
            IrExpression::ArrowFunction(arrow_fn) => {
                update_expression_slots(&mut arrow_fn.body, slot_map);
            }
            IrExpression::ResolvedTemplateLiteral(rtl) => {
                for expr in rtl.expressions.iter_mut() {
                    update_expression_slots(expr, slot_map);
                }
            }
            _ => {}
        }
    }

    // Helper function to update slots in nested AngularExpression nodes
    fn update_angular_expression_slots(
        expr: &mut crate::ast::expression::AngularExpression<'_>,
        slot_map: &FxHashMap<XrefId, SlotId>,
    ) {
        use crate::ast::expression::AngularExpression;
        match expr {
            AngularExpression::Conditional(cond) => {
                update_angular_expression_slots(&mut cond.condition, slot_map);
                update_angular_expression_slots(&mut cond.true_exp, slot_map);
                update_angular_expression_slots(&mut cond.false_exp, slot_map);
            }
            AngularExpression::Binary(bin) => {
                update_angular_expression_slots(&mut bin.left, slot_map);
                update_angular_expression_slots(&mut bin.right, slot_map);
            }
            _ => {}
        }
    }

    // Helper function to update slots in an update operation's expressions
    fn update_op_expression_slots(op: &mut UpdateOp<'_>, slot_map: &FxHashMap<XrefId, SlotId>) {
        match op {
            UpdateOp::Property(prop) => {
                update_expression_slots(&mut prop.expression, slot_map);
            }
            UpdateOp::Attribute(attr) => {
                update_expression_slots(&mut attr.expression, slot_map);
            }
            UpdateOp::StyleProp(style) => {
                update_expression_slots(&mut style.expression, slot_map);
            }
            UpdateOp::ClassProp(class) => {
                update_expression_slots(&mut class.expression, slot_map);
            }
            UpdateOp::StyleMap(style) => {
                update_expression_slots(&mut style.expression, slot_map);
            }
            UpdateOp::ClassMap(class) => {
                update_expression_slots(&mut class.expression, slot_map);
            }
            UpdateOp::InterpolateText(interp) => {
                update_expression_slots(&mut interp.interpolation, slot_map);
            }
            UpdateOp::Variable(var) => {
                update_expression_slots(&mut var.initializer, slot_map);
            }
            UpdateOp::Binding(binding) => {
                update_expression_slots(&mut binding.expression, slot_map);
            }
            UpdateOp::Repeater(rep) => {
                update_expression_slots(&mut rep.collection, slot_map);
            }
            UpdateOp::Conditional(cond) => {
                if let Some(ref mut test) = cond.test {
                    update_expression_slots(test, slot_map);
                }
                // Update slot handles in conditions (ConditionalCaseExpr)
                for condition in cond.conditions.iter_mut() {
                    if let Some(&slot) = slot_map.get(&condition.target) {
                        condition.target_slot.slot = Some(slot);
                    }
                    // Update slots in condition expressions
                    if let Some(ref mut expr) = condition.expr {
                        update_expression_slots(expr, slot_map);
                    }
                }
                // Update slots in processed expression (built by conditionals phase)
                if let Some(ref mut processed) = cond.processed {
                    update_expression_slots(processed, slot_map);
                }
                // Update slots in context_value expression
                if let Some(ref mut context_value) = cond.context_value {
                    update_expression_slots(context_value, slot_map);
                }
            }
            UpdateOp::DeferWhen(when) => {
                update_expression_slots(&mut when.condition, slot_map);
            }
            UpdateOp::StoreLet(store) => {
                update_expression_slots(&mut store.value, slot_map);
            }
            UpdateOp::DomProperty(prop) => {
                update_expression_slots(&mut prop.expression, slot_map);
            }
            UpdateOp::TwoWayProperty(prop) => {
                update_expression_slots(&mut prop.expression, slot_map);
            }
            UpdateOp::I18nExpression(expr) => {
                update_expression_slots(&mut expr.expression, slot_map);
                // Update the handle slot from the i18n_owner (I18nStart op)
                if let Some(&slot) = slot_map.get(&expr.i18n_owner) {
                    expr.handle = I18nSlotHandle::Single(slot);
                }
            }
            UpdateOp::I18nApply(apply) => {
                // Update the handle slot from the i18n_owner (I18nStart op)
                if let Some(&slot) = slot_map.get(&apply.i18n_owner) {
                    apply.handle = I18nSlotHandle::Single(slot);
                }
            }
            UpdateOp::AnimationBinding(anim) => {
                update_expression_slots(&mut anim.expression, slot_map);
            }
            UpdateOp::Statement(stmt) => {
                // Statement ops may contain WrappedIrNode with IR expressions
                update_slots_in_output_statement(&mut stmt.statement, slot_map);
            }
            UpdateOp::Control(ctrl) => {
                update_expression_slots(&mut ctrl.expression, slot_map);
            }
            _ => {}
        }
    }

    // Helper function to update slots in output statements
    fn update_slots_in_output_statement(
        stmt: &mut crate::output::ast::OutputStatement<'_>,
        slot_map: &FxHashMap<XrefId, SlotId>,
    ) {
        use crate::output::ast::OutputStatement;
        match stmt {
            OutputStatement::Expression(expr_stmt) => {
                update_slots_in_output_expression(&mut expr_stmt.expr, slot_map);
            }
            OutputStatement::Return(ret) => {
                update_slots_in_output_expression(&mut ret.value, slot_map);
            }
            OutputStatement::DeclareVar(decl) => {
                if let Some(ref mut value) = decl.value {
                    update_slots_in_output_expression(value, slot_map);
                }
            }
            OutputStatement::DeclareFunction(func) => {
                for stmt in func.statements.iter_mut() {
                    update_slots_in_output_statement(stmt, slot_map);
                }
            }
            OutputStatement::If(if_stmt) => {
                update_slots_in_output_expression(&mut if_stmt.condition, slot_map);
                for stmt in if_stmt.true_case.iter_mut() {
                    update_slots_in_output_statement(stmt, slot_map);
                }
                for stmt in if_stmt.false_case.iter_mut() {
                    update_slots_in_output_statement(stmt, slot_map);
                }
            }
        }
    }

    // Helper function to update slots in output expressions
    fn update_slots_in_output_expression(
        expr: &mut crate::output::ast::OutputExpression<'_>,
        slot_map: &FxHashMap<XrefId, SlotId>,
    ) {
        use crate::output::ast::OutputExpression;
        match expr {
            OutputExpression::WrappedIrNode(wrapped) => {
                update_expression_slots(&mut wrapped.node, slot_map);
            }
            OutputExpression::InvokeFunction(invoke) => {
                update_slots_in_output_expression(&mut invoke.fn_expr, slot_map);
                for arg in invoke.args.iter_mut() {
                    update_slots_in_output_expression(arg, slot_map);
                }
            }
            OutputExpression::Conditional(cond) => {
                update_slots_in_output_expression(&mut cond.condition, slot_map);
                update_slots_in_output_expression(&mut cond.true_case, slot_map);
                if let Some(ref mut false_case) = cond.false_case {
                    update_slots_in_output_expression(false_case, slot_map);
                }
            }
            OutputExpression::BinaryOperator(bin) => {
                update_slots_in_output_expression(&mut bin.lhs, slot_map);
                update_slots_in_output_expression(&mut bin.rhs, slot_map);
            }
            OutputExpression::UnaryOperator(u) => {
                update_slots_in_output_expression(&mut u.expr, slot_map);
            }
            OutputExpression::TaggedTemplateLiteral(tt) => {
                update_slots_in_output_expression(&mut tt.tag, slot_map);
            }
            OutputExpression::Instantiate(inst) => {
                update_slots_in_output_expression(&mut inst.class_expr, slot_map);
                for arg in inst.args.iter_mut() {
                    update_slots_in_output_expression(arg, slot_map);
                }
            }
            OutputExpression::LiteralArray(arr) => {
                for entry in arr.entries.iter_mut() {
                    update_slots_in_output_expression(entry, slot_map);
                }
            }
            OutputExpression::LiteralMap(map) => {
                for entry in map.entries.iter_mut() {
                    update_slots_in_output_expression(&mut entry.value, slot_map);
                }
            }
            OutputExpression::Not(n) => {
                update_slots_in_output_expression(&mut n.condition, slot_map);
            }
            OutputExpression::Typeof(t) => {
                update_slots_in_output_expression(&mut t.expr, slot_map);
            }
            OutputExpression::Void(v) => {
                update_slots_in_output_expression(&mut v.expr, slot_map);
            }
            OutputExpression::Parenthesized(p) => {
                update_slots_in_output_expression(&mut p.expr, slot_map);
            }
            OutputExpression::Comma(c) => {
                for expr in c.parts.iter_mut() {
                    update_slots_in_output_expression(expr, slot_map);
                }
            }
            OutputExpression::ArrowFunction(af) => match &mut af.body {
                crate::output::ast::ArrowFunctionBody::Expression(expr) => {
                    update_slots_in_output_expression(expr, slot_map);
                }
                crate::output::ast::ArrowFunctionBody::Statements(stmts) => {
                    for stmt in stmts.iter_mut() {
                        update_slots_in_output_statement(stmt, slot_map);
                    }
                }
            },
            OutputExpression::ReadProp(rp) => {
                update_slots_in_output_expression(&mut rp.receiver, slot_map);
            }
            OutputExpression::ReadKey(rk) => {
                update_slots_in_output_expression(&mut rk.receiver, slot_map);
                update_slots_in_output_expression(&mut rk.index, slot_map);
            }
            OutputExpression::TemplateLiteral(tl) => {
                for expr in tl.expressions.iter_mut() {
                    update_slots_in_output_expression(expr, slot_map);
                }
            }
            OutputExpression::Function(f) => {
                for stmt in f.statements.iter_mut() {
                    update_slots_in_output_statement(stmt, slot_map);
                }
            }
            OutputExpression::DynamicImport(di) => {
                if let crate::output::ast::DynamicImportUrl::Expression(url_expr) = &mut di.url {
                    update_slots_in_output_expression(url_expr, slot_map);
                }
            }
            OutputExpression::SpreadElement(spread) => {
                update_slots_in_output_expression(&mut spread.expr, slot_map);
            }
            // These don't contain nested expressions that need slot updates
            OutputExpression::Literal(_)
            | OutputExpression::LocalizedString(_)
            | OutputExpression::External(_)
            | OutputExpression::ReadVar(_)
            | OutputExpression::RegularExpressionLiteral(_)
            | OutputExpression::WrappedNode(_)
            | OutputExpression::RawSource(_) => {}
        }
    }

    // Process root view's update operations
    for op in job.root.update.iter_mut() {
        update_op_expression_slots(op, slot_map);
    }

    // Process embedded views' update operations
    for view in job.views.values_mut() {
        for op in view.update.iter_mut() {
            update_op_expression_slots(op, slot_map);
        }
    }

    // Process listener handler_ops in root view
    for op in job.root.create.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                for handler_op in listener.handler_ops.iter_mut() {
                    update_op_expression_slots(handler_op, slot_map);
                }
            }
            CreateOp::TwoWayListener(listener) => {
                for handler_op in listener.handler_ops.iter_mut() {
                    update_op_expression_slots(handler_op, slot_map);
                }
            }
            CreateOp::AnimationListener(listener) => {
                for handler_op in listener.handler_ops.iter_mut() {
                    update_op_expression_slots(handler_op, slot_map);
                }
            }
            CreateOp::Animation(animation) => {
                for handler_op in animation.handler_ops.iter_mut() {
                    update_op_expression_slots(handler_op, slot_map);
                }
            }
            _ => {}
        }
    }

    // Process listener handler_ops in embedded views
    for view in job.views.values_mut() {
        for op in view.create.iter_mut() {
            match op {
                CreateOp::Listener(listener) => {
                    for handler_op in listener.handler_ops.iter_mut() {
                        update_op_expression_slots(handler_op, slot_map);
                    }
                }
                CreateOp::TwoWayListener(listener) => {
                    for handler_op in listener.handler_ops.iter_mut() {
                        update_op_expression_slots(handler_op, slot_map);
                    }
                }
                CreateOp::AnimationListener(listener) => {
                    for handler_op in listener.handler_ops.iter_mut() {
                        update_op_expression_slots(handler_op, slot_map);
                    }
                }
                CreateOp::Animation(animation) => {
                    for handler_op in animation.handler_ops.iter_mut() {
                        update_op_expression_slots(handler_op, slot_map);
                    }
                }
                _ => {}
            }
        }
    }

    // Process ArrowFunctionExpr ops and body in root view's functions
    for fn_ptr in job.root.functions.iter() {
        // SAFETY: The pointer is valid because it was created by generate_arrow_functions
        // and the allocator outlives this function call.
        let arrow_fn = unsafe { &mut **fn_ptr };
        for op in arrow_fn.ops.iter_mut() {
            update_op_expression_slots(op, slot_map);
        }
        update_expression_slots(&mut arrow_fn.body, slot_map);
    }

    // Process ArrowFunctionExpr ops and body in embedded views' functions
    for view in job.views.values_mut() {
        for fn_ptr in view.functions.iter() {
            // SAFETY: The pointer is valid because it was created by generate_arrow_functions
            // and the allocator outlives this function call.
            let arrow_fn = unsafe { &mut **fn_ptr };
            for op in arrow_fn.ops.iter_mut() {
                update_op_expression_slots(op, slot_map);
            }
            update_expression_slots(&mut arrow_fn.body, slot_map);
        }
    }
}

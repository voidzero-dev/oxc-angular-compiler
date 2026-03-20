//! Variable counting phase.
//!
//! Counts the number of variable slots used within each view, and stores that on the view itself,
//! as well as propagates it to the `ir.TemplateOp` for embedded views.
//!
//! Variable slots are used by the Angular runtime for temporary storage during template updates
//! (e.g., for interpolation results, pipe bindings, etc.).
//!
//! Ported from Angular's `template/pipeline/src/phases/var_counting.ts`.

use rustc_hash::FxHashMap;

use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Counts the number of variable slots needed for each view and assigns var_offset to expressions.
///
/// This is a three-pass algorithm matching Angular's TemplateDefinitionBuilder compatibility mode:
/// 1. Pass 1: Count vars used by top-level ops (Property, Attribute, etc.)
/// 2. Pass 2: Assign var_offset to non-PureFunction expressions (PipeBinding, StoreLet, etc.)
/// 3. Pass 3: Assign var_offset to PureFunction expressions only
///
/// Angular's TemplateDefinitionBuilder assigns variable offsets for everything but pure functions
/// first, and then assigns offsets to pure functions lazily. This ordering is critical for correct
/// runtime behavior. The var_offset assignment is essential: each expression that consumes vars
/// gets an offset that is the current var count BEFORE adding its consumption.
pub fn count_variables(job: &mut ComponentCompilationJob<'_>) {
    let mut view_vars: FxHashMap<XrefId, u32> = FxHashMap::default();

    // Process root view with three-pass algorithm
    let mut root_var_count =
        count_vars_in_view_base(&job.root.create, &job.root.update, &job.expressions);

    // Pass 2: Assign var_offset to non-PureFunction expressions in ALL ops (create + update)
    // TypeScript's unit.ops() iterates both create and update ops
    assign_var_offsets_in_create_ops(&mut job.root.create, &mut root_var_count, false);
    assign_var_offsets_in_update_ops(&mut job.root.update, &mut root_var_count, false);

    // Pass 3: Assign var_offset to PureFunction expressions only
    assign_var_offsets_in_create_ops(&mut job.root.create, &mut root_var_count, true);
    assign_var_offsets_in_update_ops(&mut job.root.update, &mut root_var_count, true);

    job.root.vars = Some(root_var_count);
    view_vars.insert(job.root.xref, root_var_count);

    // Process embedded views with three-pass algorithm
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs {
        let base_count = {
            let view = job.views.get(&view_xref);
            view.map(|v| count_vars_in_view_base(&v.create, &v.update, &job.expressions))
                .unwrap_or(0)
        };
        if let Some(view) = job.views.get_mut(&view_xref) {
            let mut var_count = base_count;

            // Pass 2: Assign var_offset to non-PureFunction expressions in ALL ops
            assign_var_offsets_in_create_ops(&mut view.create, &mut var_count, false);
            assign_var_offsets_in_update_ops(&mut view.update, &mut var_count, false);

            // Pass 3: Assign var_offset to PureFunction expressions only
            assign_var_offsets_in_create_ops(&mut view.create, &mut var_count, true);
            assign_var_offsets_in_update_ops(&mut view.update, &mut var_count, true);

            view.vars = Some(var_count);
            view_vars.insert(view_xref, var_count);
        }
    }

    // Propagate var counts to Template, RepeaterCreate, and Conditional operations
    propagate_vars_to_templates(job, &view_vars);
}

/// Counts base variable slots (create ops + update op slots) without expression vars.
fn count_vars_in_view_base<'a>(
    create_ops: &crate::ir::list::CreateOpList<'a>,
    update_ops: &crate::ir::list::UpdateOpList<'a>,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'a>,
) -> u32 {
    let mut var_count: u32 = 0;

    // Count variables from create operations
    for op in create_ops.iter() {
        var_count += vars_used_by_create_op(op);
    }

    // Count variables from update operations (top-level only)
    for op in update_ops.iter() {
        var_count += vars_used_by_update_op(op, expressions);
    }

    var_count
}

/// Assigns var_offset to expressions in a view's update ops.
///
/// When `pure_functions_only` is false, processes all var-consuming expressions EXCEPT PureFunctions.
/// When `pure_functions_only` is true, processes ONLY PureFunction expressions.
/// This two-phase approach matches Angular's TemplateDefinitionBuilder compatibility mode.
fn assign_var_offsets_in_update_ops<'a>(
    update_ops: &mut crate::ir::list::UpdateOpList<'a>,
    var_count: &mut u32,
    pure_functions_only: bool,
) {
    for op in update_ops.iter_mut() {
        assign_var_offsets_in_op(op, var_count, pure_functions_only);
    }
}

/// Assigns var_offset to expressions in a view's create ops.
///
/// When `pure_functions_only` is false, processes all var-consuming expressions EXCEPT PureFunctions.
/// When `pure_functions_only` is true, processes ONLY PureFunction expressions.
/// This two-phase approach matches Angular's TemplateDefinitionBuilder compatibility mode.
fn assign_var_offsets_in_create_ops<'a>(
    create_ops: &mut crate::ir::list::CreateOpList<'a>,
    var_count: &mut u32,
    pure_functions_only: bool,
) {
    for op in create_ops.iter_mut() {
        assign_var_offsets_in_create_op(op, var_count, pure_functions_only);
    }
}

/// Assigns var_offset to expressions inside a create operation.
///
/// This handles create ops that can contain expressions, primarily:
/// - RepeaterCreate: has `track` expression for trackBy function
fn assign_var_offsets_in_create_op(
    op: &mut CreateOp<'_>,
    var_count: &mut u32,
    pure_functions_only: bool,
) {
    match op {
        CreateOp::RepeaterCreate(rep) => {
            // Visit the track expression which can contain PureFunctions or pipes
            assign_var_offsets_in_expr(&mut rep.track, var_count, pure_functions_only);
        }
        // Most create operations don't have expressions that consume vars
        _ => {}
    }
}

/// Counts variables used by a create operation.
fn vars_used_by_create_op(op: &CreateOp<'_>) -> u32 {
    match op {
        CreateOp::RepeaterCreate(rep) => {
            // Repeaters need an extra variable slot if they have an empty view
            if rep.empty_view.is_some() { 1 } else { 0 }
        }
        // Most create operations don't use variable slots
        _ => 0,
    }
}

/// Counts variables used by an update operation.
///
/// This counts the slots used by the operation itself, not by nested expressions.
fn vars_used_by_update_op(
    op: &UpdateOp<'_>,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'_>,
) -> u32 {
    match op {
        UpdateOp::Property(prop) => {
            // Property bindings use 1 slot, plus 1 for each interpolation expression
            let mut slots = 1;
            if let Some(interp_len) = get_interpolation_length(&prop.expression, expressions) {
                slots += interp_len;
            }
            slots
        }
        UpdateOp::DomProperty(prop) => {
            // DomProperty bindings use 1 slot, plus 1 for each interpolation expression
            let mut slots = 1;
            if let Some(interp_len) = get_interpolation_length(&prop.expression, expressions) {
                slots += interp_len;
            }
            slots
        }
        UpdateOp::Attribute(attr) => {
            // Attribute bindings use 1 slot, plus 1 for each interpolation expression
            // (unless it's a singleton interpolation)
            let mut slots = 1;
            if let Some(interp_len) = get_interpolation_length(&attr.expression, expressions) {
                if !is_singleton_interpolation(&attr.expression, expressions) {
                    slots += interp_len;
                }
            }
            slots
        }
        UpdateOp::StyleProp(style) => {
            // Style bindings use 2 slots, plus 1 for each interpolation expression
            let mut slots = 2;
            if let Some(interp_len) = get_interpolation_length(&style.expression, expressions) {
                slots += interp_len;
            }
            slots
        }
        UpdateOp::ClassProp(cls) => {
            // Class bindings use 2 slots, plus 1 for each interpolation expression
            let mut slots = 2;
            if let Some(interp_len) = get_interpolation_length(&cls.expression, expressions) {
                slots += interp_len;
            }
            slots
        }
        UpdateOp::StyleMap(style) => {
            // StyleMap bindings use 2 slots, plus 1 for each interpolation expression
            let mut slots = 2;
            if let Some(interp_len) = get_interpolation_length(&style.expression, expressions) {
                slots += interp_len;
            }
            slots
        }
        UpdateOp::ClassMap(cls) => {
            // ClassMap bindings use 2 slots, plus 1 for each interpolation expression
            let mut slots = 2;
            if let Some(interp_len) = get_interpolation_length(&cls.expression, expressions) {
                slots += interp_len;
            }
            slots
        }
        UpdateOp::InterpolateText(interp) => {
            // Text interpolations use one slot per expression
            get_interpolation_length(&interp.interpolation, expressions).unwrap_or(1)
        }
        UpdateOp::TwoWayProperty(_) => {
            // Two-way properties use 1 variable slot
            1
        }
        UpdateOp::Control(_) => {
            // Control bindings use 2 slots (one for value, one for bound states)
            2
        }
        UpdateOp::Conditional(_) => 1,
        UpdateOp::StoreLet(_) => 1,
        UpdateOp::I18nExpression(_) => 1,
        UpdateOp::DeferWhen(_) => 1,
        // Handle Binding ops based on their kind
        // This is a fallback in case bindings haven't been specialized yet
        UpdateOp::Binding(binding) => {
            use crate::ir::enums::BindingKind;
            match binding.kind {
                // Class and style bindings use 2 slots
                BindingKind::ClassName | BindingKind::StyleProperty => {
                    let mut slots = 2;
                    if let Some(interp_len) =
                        get_interpolation_length(&binding.expression, expressions)
                    {
                        slots += interp_len;
                    }
                    slots
                }
                // Property bindings use 1 slot
                BindingKind::Property
                | BindingKind::Template
                | BindingKind::LegacyAnimation
                | BindingKind::Animation => {
                    let mut slots = 1;
                    if let Some(interp_len) =
                        get_interpolation_length(&binding.expression, expressions)
                    {
                        slots += interp_len;
                    }
                    slots
                }
                // Attribute bindings use 1 slot
                BindingKind::Attribute => {
                    let mut slots = 1;
                    if let Some(interp_len) =
                        get_interpolation_length(&binding.expression, expressions)
                    {
                        if !is_singleton_interpolation(&binding.expression, expressions) {
                            slots += interp_len;
                        }
                    }
                    slots
                }
                // TwoWayProperty uses 1 slot
                BindingKind::TwoWayProperty => 1,
                // I18n bindings are handled separately
                BindingKind::I18n => 0,
            }
        }
        // Animation bindings use 1 slot (for syntheticHostProperty)
        UpdateOp::AnimationBinding(_) => 1,
        // These operations don't consume variable slots
        _ => 0,
    }
}

/// Gets the number of expressions in an interpolation, if the expression is an interpolation.
fn get_interpolation_length(
    expr: &IrExpression<'_>,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'_>,
) -> Option<u32> {
    match expr {
        IrExpression::Interpolation(interp) => Some(interp.expressions.len() as u32),
        IrExpression::ExpressionRef(id) => {
            // Look up the expression in the store
            let stored = expressions.get(*id);
            if let crate::ast::expression::AngularExpression::Interpolation(interp) = stored {
                return Some(interp.expressions.len() as u32);
            }
            None
        }
        _ => None,
    }
}

/// Checks if an expression is a singleton interpolation (e.g., `{{value}}` with no surrounding text).
fn is_singleton_interpolation(
    expr: &IrExpression<'_>,
    expressions: &crate::pipeline::expression_store::ExpressionStore<'_>,
) -> bool {
    match expr {
        IrExpression::Interpolation(interp) => {
            if interp.expressions.len() != 1 || interp.strings.len() != 2 {
                return false;
            }
            interp.strings[0].is_empty() && interp.strings[1].is_empty()
        }
        IrExpression::ExpressionRef(id) => {
            // Look up the expression in the store
            let stored = expressions.get(*id);
            if let crate::ast::expression::AngularExpression::Interpolation(interp) = stored {
                if interp.expressions.len() != 1 || interp.strings.len() != 2 {
                    return false;
                }
                return interp.strings[0].is_empty() && interp.strings[1].is_empty();
            }
            false
        }
        _ => false,
    }
}

/// Assigns var_offset to expressions inside an update operation and counts their vars.
///
/// This traverses all expressions in the operation to find expressions that need var_offset
/// (PureFunction, PipeBinding, PipeBindingVariadic, StoreLet). For each such expression:
/// 1. Assign current var_count as var_offset
/// 2. Increment var_count by the number of vars consumed
///
/// When `pure_functions_only` is false, processes all var-consuming expressions EXCEPT PureFunctions.
/// When `pure_functions_only` is true, processes ONLY PureFunction expressions.
fn assign_var_offsets_in_op(op: &mut UpdateOp<'_>, var_count: &mut u32, pure_functions_only: bool) {
    match op {
        UpdateOp::Property(prop) => {
            assign_var_offsets_in_expr(&mut prop.expression, var_count, pure_functions_only);
        }
        UpdateOp::DomProperty(prop) => {
            assign_var_offsets_in_expr(&mut prop.expression, var_count, pure_functions_only);
        }
        UpdateOp::Attribute(attr) => {
            assign_var_offsets_in_expr(&mut attr.expression, var_count, pure_functions_only);
        }
        UpdateOp::StyleProp(style) => {
            assign_var_offsets_in_expr(&mut style.expression, var_count, pure_functions_only);
        }
        UpdateOp::ClassProp(cls) => {
            assign_var_offsets_in_expr(&mut cls.expression, var_count, pure_functions_only);
        }
        UpdateOp::StyleMap(style) => {
            assign_var_offsets_in_expr(&mut style.expression, var_count, pure_functions_only);
        }
        UpdateOp::ClassMap(cls) => {
            assign_var_offsets_in_expr(&mut cls.expression, var_count, pure_functions_only);
        }
        UpdateOp::InterpolateText(interp) => {
            assign_var_offsets_in_expr(&mut interp.interpolation, var_count, pure_functions_only);
        }
        UpdateOp::TwoWayProperty(prop) => {
            assign_var_offsets_in_expr(&mut prop.expression, var_count, pure_functions_only);
        }
        UpdateOp::Binding(binding) => {
            assign_var_offsets_in_expr(&mut binding.expression, var_count, pure_functions_only);
        }
        UpdateOp::StoreLet(store_let) => {
            assign_var_offsets_in_expr(&mut store_let.value, var_count, pure_functions_only);
        }
        UpdateOp::Conditional(cond) => {
            // Visit conditions[].expr like TypeScript's transformExpressionsInOp does
            for condition in cond.conditions.iter_mut() {
                if let Some(ref mut expr) = condition.expr {
                    assign_var_offsets_in_expr(expr, var_count, pure_functions_only);
                }
            }
            if let Some(ref mut processed) = cond.processed {
                assign_var_offsets_in_expr(processed, var_count, pure_functions_only);
            }
            if let Some(ref mut ctx_val) = cond.context_value {
                assign_var_offsets_in_expr(ctx_val, var_count, pure_functions_only);
            }
        }
        UpdateOp::I18nExpression(i18n) => {
            assign_var_offsets_in_expr(&mut i18n.expression, var_count, pure_functions_only);
        }
        UpdateOp::DeferWhen(defer) => {
            assign_var_offsets_in_expr(&mut defer.condition, var_count, pure_functions_only);
        }
        UpdateOp::Repeater(rep) => {
            assign_var_offsets_in_expr(&mut rep.collection, var_count, pure_functions_only);
        }
        UpdateOp::Control(ctrl) => {
            assign_var_offsets_in_expr(&mut ctrl.expression, var_count, pure_functions_only);
        }
        UpdateOp::AnimationBinding(anim) => {
            assign_var_offsets_in_expr(&mut anim.expression, var_count, pure_functions_only);
        }
        UpdateOp::Variable(var) => {
            assign_var_offsets_in_expr(&mut var.initializer, var_count, pure_functions_only);
        }
        UpdateOp::Statement(stmt) => {
            // Statement ops may contain WrappedIrNode with IR expressions
            assign_var_offsets_in_output_statement(
                &mut stmt.statement,
                var_count,
                pure_functions_only,
            );
        }
        // These operations don't have expressions
        UpdateOp::ListEnd(_) | UpdateOp::Advance(_) | UpdateOp::I18nApply(_) => {}
    }
}

/// Recursively assigns var_offset to expressions that need it.
///
/// IMPORTANT: Angular's visitor pattern processes children BEFORE the parent.
/// This is because `transformExpressionsInExpression` calls `transformInternalExpressions`
/// (which processes args) BEFORE calling `transform(expr, flags)` (the visitor callback).
/// We must match this order: process args FIRST, then assign var_offset to this expression.
///
/// The `pure_functions_only` parameter controls which expressions are processed:
/// - When `false`: process PipeBinding, PipeBindingVariadic, StoreLet (SKIP PureFunctions)
/// - When `true`: process ONLY PureFunction expressions
///
/// This matches Angular's TemplateDefinitionBuilder compatibility mode which assigns
/// variable offsets for everything but pure functions first, then assigns offsets to
/// pure functions in a separate pass.
fn assign_var_offsets_in_expr(
    expr: &mut IrExpression<'_>,
    var_count: &mut u32,
    pure_functions_only: bool,
) {
    match expr {
        IrExpression::PipeBinding(pipe) => {
            // Process args FIRST (matches Angular's transformInternalExpressions)
            for arg in pipe.args.iter_mut() {
                assign_var_offsets_in_expr(arg, var_count, pure_functions_only);
            }
            // Only assign var_offset when NOT in pure_functions_only mode
            if !pure_functions_only {
                pipe.var_offset = Some(*var_count);
                // Consume: 1 (for pipe instance) + number of args
                *var_count += 1 + pipe.args.len() as u32;
            }
        }
        IrExpression::PipeBindingVariadic(pipe) => {
            // Process args array FIRST
            assign_var_offsets_in_expr(&mut pipe.args, var_count, pure_functions_only);
            // Only assign var_offset when NOT in pure_functions_only mode
            if !pure_functions_only {
                pipe.var_offset = Some(*var_count);
                // Consume: 1 (for pipe instance) + num_args
                *var_count += 1 + pipe.num_args;
            }
        }
        IrExpression::PureFunction(pf) => {
            // Process body if present FIRST
            if let Some(ref mut body) = pf.body {
                assign_var_offsets_in_expr(body, var_count, pure_functions_only);
            }
            // Process fn_ref if present
            if let Some(ref mut fn_ref) = pf.fn_ref {
                assign_var_offsets_in_expr(fn_ref, var_count, pure_functions_only);
            }
            // Process args
            for arg in pf.args.iter_mut() {
                assign_var_offsets_in_expr(arg, var_count, pure_functions_only);
            }
            // Only assign var_offset when IN pure_functions_only mode
            if pure_functions_only {
                pf.var_offset = Some(*var_count);
                // Consume: 1 (for context) + number of args
                *var_count += 1 + pf.args.len() as u32;
            }
        }
        IrExpression::StoreLet(store_let) => {
            // Process value expression FIRST
            assign_var_offsets_in_expr(&mut store_let.value, var_count, pure_functions_only);
            // Only assign var_offset when NOT in pure_functions_only mode
            if !pure_functions_only {
                store_let.var_offset = Some(*var_count);
                // Consume 1 slot for the @let value
                *var_count += 1;
            }
        }
        // Recursively process nested expressions
        IrExpression::ConditionalCase(cond) => {
            if let Some(ref mut expr) = cond.expr {
                assign_var_offsets_in_expr(expr, var_count, pure_functions_only);
            }
        }
        IrExpression::SafeTernary(safe) => {
            assign_var_offsets_in_expr(&mut safe.guard, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut safe.expr, var_count, pure_functions_only);
        }
        IrExpression::Ternary(t) => {
            assign_var_offsets_in_expr(&mut t.condition, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut t.true_expr, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut t.false_expr, var_count, pure_functions_only);
        }
        IrExpression::Interpolation(interp) => {
            for inner in interp.expressions.iter_mut() {
                assign_var_offsets_in_expr(inner, var_count, pure_functions_only);
            }
        }
        IrExpression::SafePropertyRead(safe) => {
            assign_var_offsets_in_expr(&mut safe.receiver, var_count, pure_functions_only);
        }
        IrExpression::SafeKeyedRead(safe) => {
            assign_var_offsets_in_expr(&mut safe.receiver, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut safe.index, var_count, pure_functions_only);
        }
        IrExpression::SafeInvokeFunction(safe) => {
            assign_var_offsets_in_expr(&mut safe.receiver, var_count, pure_functions_only);
            for arg in safe.args.iter_mut() {
                assign_var_offsets_in_expr(arg, var_count, pure_functions_only);
            }
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter_mut() {
                assign_var_offsets_in_expr(entry, var_count, pure_functions_only);
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter_mut() {
                assign_var_offsets_in_expr(value, var_count, pure_functions_only);
            }
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter_mut() {
                assign_var_offsets_in_expr(elem, var_count, pure_functions_only);
            }
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter_mut() {
                assign_var_offsets_in_expr(value, var_count, pure_functions_only);
            }
        }
        // Binary expressions may have nested expressions that consume vars
        IrExpression::Binary(binary) => {
            assign_var_offsets_in_expr(&mut binary.lhs, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut binary.rhs, var_count, pure_functions_only);
        }
        IrExpression::ResolvedBinary(binary) => {
            assign_var_offsets_in_expr(&mut binary.left, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut binary.right, var_count, pure_functions_only);
        }
        // ResolvedCall has receiver and arguments
        IrExpression::ResolvedCall(call) => {
            assign_var_offsets_in_expr(&mut call.receiver, var_count, pure_functions_only);
            for arg in call.args.iter_mut() {
                assign_var_offsets_in_expr(arg, var_count, pure_functions_only);
            }
        }
        // ResolvedPropertyRead has a receiver that may contain var-consuming expressions
        IrExpression::ResolvedPropertyRead(rpr) => {
            assign_var_offsets_in_expr(&mut rpr.receiver, var_count, pure_functions_only);
        }
        // ResolvedKeyedRead has receiver and key
        IrExpression::ResolvedKeyedRead(rkr) => {
            assign_var_offsets_in_expr(&mut rkr.receiver, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut rkr.key, var_count, pure_functions_only);
        }
        // ResolvedSafePropertyRead has a receiver
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            assign_var_offsets_in_expr(&mut rspr.receiver, var_count, pure_functions_only);
        }
        // AssignTemporary has inner expression
        IrExpression::AssignTemporary(at) => {
            assign_var_offsets_in_expr(&mut at.expr, var_count, pure_functions_only);
        }
        // ResetView has inner expression
        IrExpression::ResetView(rv) => {
            assign_var_offsets_in_expr(&mut rv.expr, var_count, pure_functions_only);
        }
        // RestoreView - check if it has a dynamic expression
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref mut expr) = rv.view {
                assign_var_offsets_in_expr(expr, var_count, pure_functions_only);
            }
        }
        // TwoWayBindingSet has target and value expressions
        IrExpression::TwoWayBindingSet(twbs) => {
            assign_var_offsets_in_expr(&mut twbs.target, var_count, pure_functions_only);
            assign_var_offsets_in_expr(&mut twbs.value, var_count, pure_functions_only);
        }
        // ConstCollected has inner expression
        IrExpression::ConstCollected(cc) => {
            assign_var_offsets_in_expr(&mut cc.expr, var_count, pure_functions_only);
        }
        // These expressions don't contain nested expressions that need var_offset
        IrExpression::Ast(_)
        | IrExpression::Empty(_)
        | IrExpression::ConstReference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::TrackContext(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::ReadVariable(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::ContextLetReference(_) => {}
        IrExpression::Not(n) => {
            assign_var_offsets_in_expr(&mut n.expr, var_count, pure_functions_only);
        }
        IrExpression::Unary(u) => {
            assign_var_offsets_in_expr(&mut u.expr, var_count, pure_functions_only);
        }
        IrExpression::Typeof(t) => {
            assign_var_offsets_in_expr(&mut t.expr, var_count, pure_functions_only);
        }
        IrExpression::Void(v) => {
            assign_var_offsets_in_expr(&mut v.expr, var_count, pure_functions_only);
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter_mut() {
                assign_var_offsets_in_expr(e, var_count, pure_functions_only);
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            // Arrow functions consume a var slot and have their own var_offset
            if !pure_functions_only {
                arrow_fn.var_offset = Some(*var_count);
                *var_count += 1;
            }
            // Process the body expression
            assign_var_offsets_in_expr(&mut arrow_fn.body, var_count, pure_functions_only);
        }
        IrExpression::Parenthesized(paren) => {
            assign_var_offsets_in_expr(&mut paren.expr, var_count, pure_functions_only);
        }
    }
}

/// Assigns var_offset to expressions in output statements.
/// This handles Statement ops that contain WrappedIrNode with IR expressions.
fn assign_var_offsets_in_output_statement(
    stmt: &mut crate::output::ast::OutputStatement<'_>,
    var_count: &mut u32,
    pure_functions_only: bool,
) {
    use crate::output::ast::OutputStatement;
    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            assign_var_offsets_in_output_expression(
                &mut expr_stmt.expr,
                var_count,
                pure_functions_only,
            );
        }
        OutputStatement::Return(ret) => {
            assign_var_offsets_in_output_expression(&mut ret.value, var_count, pure_functions_only);
        }
        OutputStatement::DeclareVar(decl) => {
            if let Some(ref mut value) = decl.value {
                assign_var_offsets_in_output_expression(value, var_count, pure_functions_only);
            }
        }
        OutputStatement::DeclareFunction(func) => {
            for stmt in func.statements.iter_mut() {
                assign_var_offsets_in_output_statement(stmt, var_count, pure_functions_only);
            }
        }
        OutputStatement::If(if_stmt) => {
            assign_var_offsets_in_output_expression(
                &mut if_stmt.condition,
                var_count,
                pure_functions_only,
            );
            for stmt in if_stmt.true_case.iter_mut() {
                assign_var_offsets_in_output_statement(stmt, var_count, pure_functions_only);
            }
            for stmt in if_stmt.false_case.iter_mut() {
                assign_var_offsets_in_output_statement(stmt, var_count, pure_functions_only);
            }
        }
    }
}

/// Assigns var_offset to expressions in output expressions.
fn assign_var_offsets_in_output_expression(
    expr: &mut crate::output::ast::OutputExpression<'_>,
    var_count: &mut u32,
    pure_functions_only: bool,
) {
    use crate::output::ast::OutputExpression;
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            assign_var_offsets_in_expr(&mut wrapped.node, var_count, pure_functions_only);
        }
        OutputExpression::InvokeFunction(invoke) => {
            assign_var_offsets_in_output_expression(
                &mut invoke.fn_expr,
                var_count,
                pure_functions_only,
            );
            for arg in invoke.args.iter_mut() {
                assign_var_offsets_in_output_expression(arg, var_count, pure_functions_only);
            }
        }
        OutputExpression::Conditional(cond) => {
            assign_var_offsets_in_output_expression(
                &mut cond.condition,
                var_count,
                pure_functions_only,
            );
            assign_var_offsets_in_output_expression(
                &mut cond.true_case,
                var_count,
                pure_functions_only,
            );
            if let Some(ref mut false_case) = cond.false_case {
                assign_var_offsets_in_output_expression(false_case, var_count, pure_functions_only);
            }
        }
        OutputExpression::BinaryOperator(bin) => {
            assign_var_offsets_in_output_expression(&mut bin.lhs, var_count, pure_functions_only);
            assign_var_offsets_in_output_expression(&mut bin.rhs, var_count, pure_functions_only);
        }
        OutputExpression::UnaryOperator(u) => {
            assign_var_offsets_in_output_expression(&mut u.expr, var_count, pure_functions_only);
        }
        OutputExpression::TaggedTemplateLiteral(tt) => {
            assign_var_offsets_in_output_expression(&mut tt.tag, var_count, pure_functions_only);
        }
        OutputExpression::Instantiate(inst) => {
            assign_var_offsets_in_output_expression(
                &mut inst.class_expr,
                var_count,
                pure_functions_only,
            );
            for arg in inst.args.iter_mut() {
                assign_var_offsets_in_output_expression(arg, var_count, pure_functions_only);
            }
        }
        OutputExpression::LiteralArray(arr) => {
            for entry in arr.entries.iter_mut() {
                assign_var_offsets_in_output_expression(entry, var_count, pure_functions_only);
            }
        }
        OutputExpression::LiteralMap(map) => {
            for entry in map.entries.iter_mut() {
                assign_var_offsets_in_output_expression(
                    &mut entry.value,
                    var_count,
                    pure_functions_only,
                );
            }
        }
        OutputExpression::Not(n) => {
            assign_var_offsets_in_output_expression(
                &mut n.condition,
                var_count,
                pure_functions_only,
            );
        }
        OutputExpression::Typeof(t) => {
            assign_var_offsets_in_output_expression(&mut t.expr, var_count, pure_functions_only);
        }
        OutputExpression::Void(v) => {
            assign_var_offsets_in_output_expression(&mut v.expr, var_count, pure_functions_only);
        }
        OutputExpression::Parenthesized(p) => {
            assign_var_offsets_in_output_expression(&mut p.expr, var_count, pure_functions_only);
        }
        OutputExpression::Comma(c) => {
            for expr in c.parts.iter_mut() {
                assign_var_offsets_in_output_expression(expr, var_count, pure_functions_only);
            }
        }
        OutputExpression::ArrowFunction(af) => match &mut af.body {
            crate::output::ast::ArrowFunctionBody::Expression(expr) => {
                assign_var_offsets_in_output_expression(expr, var_count, pure_functions_only);
            }
            crate::output::ast::ArrowFunctionBody::Statements(stmts) => {
                for stmt in stmts.iter_mut() {
                    assign_var_offsets_in_output_statement(stmt, var_count, pure_functions_only);
                }
            }
        },
        OutputExpression::ReadProp(rp) => {
            assign_var_offsets_in_output_expression(
                &mut rp.receiver,
                var_count,
                pure_functions_only,
            );
        }
        OutputExpression::ReadKey(rk) => {
            assign_var_offsets_in_output_expression(
                &mut rk.receiver,
                var_count,
                pure_functions_only,
            );
            assign_var_offsets_in_output_expression(&mut rk.index, var_count, pure_functions_only);
        }
        OutputExpression::TemplateLiteral(tl) => {
            for expr in tl.expressions.iter_mut() {
                assign_var_offsets_in_output_expression(expr, var_count, pure_functions_only);
            }
        }
        OutputExpression::Function(f) => {
            for stmt in f.statements.iter_mut() {
                assign_var_offsets_in_output_statement(stmt, var_count, pure_functions_only);
            }
        }
        OutputExpression::DynamicImport(di) => {
            if let crate::output::ast::DynamicImportUrl::Expression(url_expr) = &mut di.url {
                assign_var_offsets_in_output_expression(url_expr, var_count, pure_functions_only);
            }
        }
        OutputExpression::SpreadElement(spread) => {
            assign_var_offsets_in_output_expression(
                &mut spread.expr,
                var_count,
                pure_functions_only,
            );
        }
        // These don't contain nested expressions that need var_offset
        OutputExpression::Literal(_)
        | OutputExpression::LocalizedString(_)
        | OutputExpression::External(_)
        | OutputExpression::ReadVar(_)
        | OutputExpression::RegularExpressionLiteral(_)
        | OutputExpression::WrappedNode(_)
        | OutputExpression::RawSource(_) => {}
    }
}

/// Propagates var counts to Template, RepeaterCreate, and Conditional operations.
fn propagate_vars_to_templates(
    job: &mut ComponentCompilationJob<'_>,
    view_vars: &FxHashMap<XrefId, u32>,
) {
    // Process root view's create operations
    propagate_vars_in_ops(&mut job.root.create, view_vars);

    // Process embedded views' create operations
    for view in job.views.values_mut() {
        propagate_vars_in_ops(&mut view.create, view_vars);
    }
}

/// Propagates var counts within a single view's operations.
fn propagate_vars_in_ops<'a>(
    ops: &mut crate::ir::list::CreateOpList<'a>,
    view_vars: &FxHashMap<XrefId, u32>,
) {
    for op in ops.iter_mut() {
        match op {
            CreateOp::Template(tmpl) => {
                // Template's embedded_view xref references the view it creates
                if let Some(&vars) = view_vars.get(&tmpl.embedded_view) {
                    tmpl.vars = Some(vars);
                }
            }
            CreateOp::RepeaterCreate(rep) => {
                // RepeaterCreate references a body view for repeated content
                if let Some(&vars) = view_vars.get(&rep.body_view) {
                    rep.vars = Some(vars);
                }
                // Also set empty_var_count if there's an empty view
                if let Some(empty_view) = rep.empty_view {
                    if let Some(&vars) = view_vars.get(&empty_view) {
                        rep.empty_var_count = Some(vars);
                    }
                }
            }
            CreateOp::Conditional(cond) => {
                // ConditionalOp.xref is the view xref for this branch
                // Look up the vars directly from view_vars
                if let Some(&vars) = view_vars.get(&cond.xref) {
                    cond.vars = Some(vars);
                }
            }
            CreateOp::ConditionalBranch(branch) => {
                // ConditionalBranch views are their own embedded views
                if let Some(&vars) = view_vars.get(&branch.xref) {
                    branch.vars = Some(vars);
                }
            }
            _ => {}
        }
    }
}

/// Counts variables for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// Uses the same three-pass algorithm as component compilation.
pub fn count_variables_for_host(job: &mut HostBindingCompilationJob<'_>) {
    // Count vars in root unit (Pass 1)
    let base_count = count_vars_in_view_base(&job.root.create, &job.root.update, &job.expressions);
    let mut var_count = base_count;

    // Pass 2: Assign var_offset to non-PureFunction expressions in ALL ops
    assign_var_offsets_in_create_ops(&mut job.root.create, &mut var_count, false);
    assign_var_offsets_in_update_ops(&mut job.root.update, &mut var_count, false);

    // Pass 3: Assign var_offset to PureFunction expressions only
    assign_var_offsets_in_create_ops(&mut job.root.create, &mut var_count, true);
    assign_var_offsets_in_update_ops(&mut job.root.update, &mut var_count, true);

    job.root.vars = Some(var_count);
}

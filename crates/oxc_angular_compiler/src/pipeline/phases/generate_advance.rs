//! Generate advance phase.
//!
//! Generates `ir.AdvanceOp`s in between `ir.UpdateOp`s that ensure the runtime's implicit slot
//! context will be advanced correctly.
//!
//! The Angular runtime maintains an implicit "current slot" pointer that must be at the correct
//! position before executing update operations. This phase inserts `ɵɵadvance(n)` calls to
//! move that pointer as needed.
//!
//! Ported from Angular's `template/pipeline/src/phases/generate_advance.ts`.

use oxc_diagnostics::OxcDiagnostic;
use rustc_hash::FxHashMap;

use crate::ir::expression::IrExpression;
use crate::ir::ops::{AdvanceOp, CreateOp, SlotId, UpdateOp, UpdateOpBase, XrefId};
use crate::output::ast::{OutputExpression, OutputStatement};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Generates advance() calls between update operations.
///
/// This phase inserts `AdvanceOp`s before update operations to ensure the runtime's
/// implicit slot context is at the correct position.
pub fn generate_advance(job: &mut ComponentCompilationJob<'_>) {
    let mut diagnostics = Vec::new();

    // Process root view
    let root_slot_map = build_slot_map(&job.root.create);
    generate_advance_in_view(&mut job.root.update, &root_slot_map, &mut diagnostics);

    // Process embedded views
    for view in job.views.values_mut() {
        let slot_map = build_slot_map(&view.create);
        generate_advance_in_view(&mut view.update, &slot_map, &mut diagnostics);
    }

    job.diagnostics.extend(diagnostics);
}

/// Builds a map from XrefId to slot number from create operations.
fn build_slot_map<'a>(create_ops: &crate::ir::list::CreateOpList<'a>) -> FxHashMap<XrefId, SlotId> {
    let mut slot_map: FxHashMap<XrefId, SlotId> = FxHashMap::default();

    for op in create_ops.iter() {
        // Check if the operation has an assigned slot
        let (xref, slot) = match op {
            CreateOp::ElementStart(elem) => (Some(elem.xref), elem.slot),
            CreateOp::Element(elem) => (Some(elem.xref), elem.slot),
            CreateOp::Template(tmpl) => (Some(tmpl.xref), tmpl.slot),
            CreateOp::ContainerStart(container) => (Some(container.xref), container.slot),
            CreateOp::Container(container) => (Some(container.xref), container.slot),
            CreateOp::Text(text) => (Some(text.xref), text.slot),
            CreateOp::Pipe(pipe) => (Some(pipe.xref), pipe.slot),
            CreateOp::Projection(proj) => (Some(proj.xref), proj.slot),
            CreateOp::Conditional(cond) => (Some(cond.xref), cond.slot),
            CreateOp::RepeaterCreate(rep) => (Some(rep.xref), rep.slot),
            CreateOp::Defer(defer) => (Some(defer.xref), defer.slot),
            CreateOp::DeclareLet(let_op) => (Some(let_op.xref), let_op.slot),
            CreateOp::I18nStart(i18n) => (Some(i18n.xref), i18n.slot),
            CreateOp::I18n(i18n) => (Some(i18n.xref), i18n.slot),
            _ => (None, None),
        };

        if let (Some(xref), Some(slot)) = (xref, slot) {
            slot_map.insert(xref, slot);
        }
    }

    slot_map
}

/// Generates advance operations in a view's update list.
fn generate_advance_in_view<'a>(
    update_ops: &mut crate::ir::list::UpdateOpList<'a>,
    slot_map: &FxHashMap<XrefId, SlotId>,
    diagnostics: &mut Vec<OxcDiagnostic>,
) {
    // Track the current slot context
    let mut slot_context: u32 = 0;

    // Use cursor to iterate and insert advance operations
    let mut cursor = update_ops.cursor();

    while cursor.move_next() {
        // Get the target XrefId if this operation depends on slot context
        let target =
            if let Some(op) = cursor.current() { get_slot_dependency_target(op) } else { None };

        if let Some(target_xref) = target {
            if let Some(&slot) = slot_map.get(&target_xref) {
                let target_slot = slot.0;

                // Check if we need to advance
                if slot_context != target_slot {
                    // Slot counter should never move backwards (indicates ingest phase bug)
                    if target_slot < slot_context {
                        diagnostics.push(OxcDiagnostic::warn(format!(
                            "Slot counter moved backwards from {} to {} - update operations may be out of order",
                            slot_context, target_slot
                        )));
                    }

                    let delta = target_slot.saturating_sub(slot_context);
                    if delta > 0 {
                        // Insert advance operation before current operation
                        let advance_op = UpdateOp::Advance(AdvanceOp {
                            base: UpdateOpBase::default(),
                            delta,
                            slot,
                        });
                        cursor.insert_before(advance_op);
                    }
                    slot_context = target_slot;
                }
            }
        }
    }
}

/// Gets the target XrefId if an update operation depends on slot context.
fn get_slot_dependency_target(op: &UpdateOp<'_>) -> Option<XrefId> {
    match op {
        UpdateOp::Property(prop) => Some(prop.target),
        UpdateOp::Attribute(attr) => Some(attr.target),
        UpdateOp::StyleProp(style) => Some(style.target),
        UpdateOp::ClassProp(class) => Some(class.target),
        UpdateOp::StyleMap(style) => Some(style.target),
        UpdateOp::ClassMap(class) => Some(class.target),
        UpdateOp::DomProperty(dom) => Some(dom.target),
        UpdateOp::InterpolateText(text) => Some(text.target),
        UpdateOp::TwoWayProperty(prop) => Some(prop.target),
        UpdateOp::Binding(binding) => Some(binding.target),
        UpdateOp::I18nExpression(i18n) => Some(i18n.target),
        UpdateOp::AnimationBinding(anim) => Some(anim.target),
        UpdateOp::Control(ctrl) => Some(ctrl.target),
        UpdateOp::Repeater(rep) => Some(rep.target),
        UpdateOp::Conditional(cond) => Some(cond.target),
        UpdateOp::StoreLet(store_let) => Some(store_let.target),
        UpdateOp::Variable(var) => get_slot_dependency_from_variable(var),
        // Statement ops may contain expressions with slot dependencies (e.g., StoreLet)
        // This happens after optimize_variables converts Variable ops to Statement ops
        UpdateOp::Statement(stmt) => get_slot_dependency_from_statement(&stmt.statement),
        UpdateOp::DeferWhen(defer_when) => Some(defer_when.defer),
        // These operations don't depend on slot context
        UpdateOp::ListEnd(_) | UpdateOp::Advance(_) | UpdateOp::I18nApply(_) => None,
    }
}

/// Extracts slot dependency target from a Variable op's initializer.
///
/// Variable ops containing a `StoreLetExpr` depend on the slot context
/// of the `@let` declaration they're storing to. This matches the TypeScript
/// Angular compiler's behavior where `StoreLetExpr` implements `DependsOnSlotContextOpTrait`.
fn get_slot_dependency_from_variable(var: &crate::ir::ops::UpdateVariableOp<'_>) -> Option<XrefId> {
    // Check if the initializer is a StoreLetExpr - if so, use its target
    if let IrExpression::StoreLet(store_let) = var.initializer.as_ref() {
        return Some(store_let.target);
    }

    None
}

/// Extracts slot dependency target from a Statement op's output statement.
///
/// After `optimize_variables` runs, `Variable` ops containing `StoreLet` expressions
/// may be converted to `Statement` ops. This function walks through the `OutputStatement`
/// to find any expressions that depend on slot context (e.g., `StoreLetExpr` wrapped
/// in `WrappedIrNode`).
///
/// This matches TypeScript's `visitExpressionsInOp` behavior in `generate_advance.ts`.
fn get_slot_dependency_from_statement(stmt: &OutputStatement<'_>) -> Option<XrefId> {
    match stmt {
        OutputStatement::DeclareVar(decl) => {
            if let Some(ref value) = decl.value {
                get_slot_dependency_from_output_expr(value)
            } else {
                None
            }
        }
        OutputStatement::Expression(expr_stmt) => {
            get_slot_dependency_from_output_expr(&expr_stmt.expr)
        }
        OutputStatement::Return(ret) => get_slot_dependency_from_output_expr(&ret.value),
        OutputStatement::DeclareFunction(func) => {
            // Check statements inside the function
            for inner_stmt in func.statements.iter() {
                if let Some(target) = get_slot_dependency_from_statement(inner_stmt) {
                    return Some(target);
                }
            }
            None
        }
        OutputStatement::If(if_stmt) => {
            // Check condition
            if let Some(target) = get_slot_dependency_from_output_expr(&if_stmt.condition) {
                return Some(target);
            }
            // Check true case
            for inner_stmt in if_stmt.true_case.iter() {
                if let Some(target) = get_slot_dependency_from_statement(inner_stmt) {
                    return Some(target);
                }
            }
            // Check false case
            for inner_stmt in if_stmt.false_case.iter() {
                if let Some(target) = get_slot_dependency_from_statement(inner_stmt) {
                    return Some(target);
                }
            }
            None
        }
    }
}

/// Extracts slot dependency target from an output expression.
///
/// Recursively walks the expression tree to find any `WrappedIrNode` containing
/// an `IrExpression` that implements `DependsOnSlotContextTrait` (e.g., `StoreLet`).
fn get_slot_dependency_from_output_expr(expr: &OutputExpression<'_>) -> Option<XrefId> {
    match expr {
        // WrappedIrNode contains IR expressions that may have slot dependencies
        OutputExpression::WrappedIrNode(wrapped) => get_slot_dependency_from_ir_expr(&wrapped.node),
        // Recursively check compound expressions
        OutputExpression::BinaryOperator(bin) => get_slot_dependency_from_output_expr(&bin.lhs)
            .or_else(|| get_slot_dependency_from_output_expr(&bin.rhs)),
        OutputExpression::UnaryOperator(unary) => get_slot_dependency_from_output_expr(&unary.expr),
        OutputExpression::Conditional(cond) => {
            get_slot_dependency_from_output_expr(&cond.condition)
                .or_else(|| get_slot_dependency_from_output_expr(&cond.true_case))
                .or_else(|| {
                    cond.false_case.as_ref().and_then(|fc| get_slot_dependency_from_output_expr(fc))
                })
        }
        OutputExpression::Not(not) => get_slot_dependency_from_output_expr(&not.condition),
        OutputExpression::Typeof(typeof_expr) => {
            get_slot_dependency_from_output_expr(&typeof_expr.expr)
        }
        OutputExpression::Void(void) => get_slot_dependency_from_output_expr(&void.expr),
        OutputExpression::Parenthesized(paren) => get_slot_dependency_from_output_expr(&paren.expr),
        OutputExpression::Comma(comma) => {
            for part in comma.parts.iter() {
                if let Some(target) = get_slot_dependency_from_output_expr(part) {
                    return Some(target);
                }
            }
            None
        }
        OutputExpression::InvokeFunction(invoke) => {
            // Check callee
            if let Some(target) = get_slot_dependency_from_output_expr(&invoke.fn_expr) {
                return Some(target);
            }
            // Check arguments
            for arg in invoke.args.iter() {
                if let Some(target) = get_slot_dependency_from_output_expr(arg) {
                    return Some(target);
                }
            }
            None
        }
        OutputExpression::ReadProp(prop) => get_slot_dependency_from_output_expr(&prop.receiver),
        OutputExpression::ReadKey(key) => get_slot_dependency_from_output_expr(&key.receiver)
            .or_else(|| get_slot_dependency_from_output_expr(&key.index)),
        OutputExpression::LiteralArray(arr) => {
            for entry in arr.entries.iter() {
                if let Some(target) = get_slot_dependency_from_output_expr(entry) {
                    return Some(target);
                }
            }
            None
        }
        OutputExpression::LiteralMap(map) => {
            for entry in map.entries.iter() {
                if let Some(target) = get_slot_dependency_from_output_expr(&entry.value) {
                    return Some(target);
                }
            }
            None
        }
        OutputExpression::SpreadElement(spread) => {
            get_slot_dependency_from_output_expr(&spread.expr)
        }
        // Simple expressions that don't contain nested expressions
        OutputExpression::Literal(_)
        | OutputExpression::ReadVar(_)
        | OutputExpression::External(_)
        | OutputExpression::WrappedNode(_)
        | OutputExpression::RegularExpressionLiteral(_)
        | OutputExpression::TemplateLiteral(_)
        | OutputExpression::TaggedTemplateLiteral(_)
        | OutputExpression::Function(_)
        | OutputExpression::ArrowFunction(_)
        | OutputExpression::Instantiate(_)
        | OutputExpression::DynamicImport(_)
        | OutputExpression::LocalizedString(_)
        | OutputExpression::RawSource(_) => None,
    }
}

/// Extracts slot dependency target from an IR expression.
///
/// This checks if the expression is a `StoreLet` or contains nested expressions
/// that might have slot dependencies.
fn get_slot_dependency_from_ir_expr(expr: &IrExpression<'_>) -> Option<XrefId> {
    match expr {
        // StoreLet implements DependsOnSlotContextTrait
        IrExpression::StoreLet(store_let) => Some(store_let.target),
        // SlotLiteral also depends on slot context (use target_xref if available)
        IrExpression::SlotLiteral(slot) => slot.target_xref,
        // Recursively check nested expressions
        IrExpression::PureFunction(pf) => {
            for arg in pf.args.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(arg) {
                    return Some(target);
                }
            }
            if let Some(ref body) = pf.body { get_slot_dependency_from_ir_expr(body) } else { None }
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(arg) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::PipeBindingVariadic(pb) => {
            // args is a single expression (usually an array), check it directly
            get_slot_dependency_from_ir_expr(&pb.args)
        }
        IrExpression::Binary(bin) => get_slot_dependency_from_ir_expr(&bin.lhs)
            .or_else(|| get_slot_dependency_from_ir_expr(&bin.rhs)),
        IrExpression::ResolvedBinary(bin) => get_slot_dependency_from_ir_expr(&bin.left)
            .or_else(|| get_slot_dependency_from_ir_expr(&bin.right)),
        IrExpression::Ternary(tern) => get_slot_dependency_from_ir_expr(&tern.condition)
            .or_else(|| get_slot_dependency_from_ir_expr(&tern.true_expr))
            .or_else(|| get_slot_dependency_from_ir_expr(&tern.false_expr)),
        IrExpression::SafeTernary(st) => get_slot_dependency_from_ir_expr(&st.guard)
            .or_else(|| get_slot_dependency_from_ir_expr(&st.expr)),
        IrExpression::Not(not) => get_slot_dependency_from_ir_expr(&not.expr),
        IrExpression::Unary(unary) => get_slot_dependency_from_ir_expr(&unary.expr),
        IrExpression::Typeof(typeof_expr) => get_slot_dependency_from_ir_expr(&typeof_expr.expr),
        IrExpression::Void(void) => get_slot_dependency_from_ir_expr(&void.expr),
        IrExpression::Parenthesized(paren) => get_slot_dependency_from_ir_expr(&paren.expr),
        IrExpression::ConstCollected(cc) => get_slot_dependency_from_ir_expr(&cc.expr),
        IrExpression::AssignTemporary(at) => get_slot_dependency_from_ir_expr(&at.expr),
        IrExpression::ResetView(rv) => get_slot_dependency_from_ir_expr(&rv.expr),
        IrExpression::TwoWayBindingSet(tbs) => get_slot_dependency_from_ir_expr(&tbs.target)
            .or_else(|| get_slot_dependency_from_ir_expr(&tbs.value)),
        IrExpression::Interpolation(interp) => {
            for expr in interp.expressions.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(expr) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::SafePropertyRead(spr) => get_slot_dependency_from_ir_expr(&spr.receiver),
        IrExpression::SafeKeyedRead(skr) => get_slot_dependency_from_ir_expr(&skr.receiver)
            .or_else(|| get_slot_dependency_from_ir_expr(&skr.index)),
        IrExpression::SafeInvokeFunction(sif) => {
            if let Some(target) = get_slot_dependency_from_ir_expr(&sif.receiver) {
                return Some(target);
            }
            for arg in sif.args.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(arg) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::ResolvedPropertyRead(rpr) => get_slot_dependency_from_ir_expr(&rpr.receiver),
        IrExpression::ResolvedKeyedRead(rkr) => get_slot_dependency_from_ir_expr(&rkr.receiver)
            .or_else(|| get_slot_dependency_from_ir_expr(&rkr.key)),
        IrExpression::ResolvedCall(rc) => {
            if let Some(target) = get_slot_dependency_from_ir_expr(&rc.receiver) {
                return Some(target);
            }
            for arg in rc.args.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(arg) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            get_slot_dependency_from_ir_expr(&rspr.receiver)
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(entry) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(value) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(elem) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(value) {
                    return Some(target);
                }
            }
            None
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for expr in rtl.expressions.iter() {
                if let Some(target) = get_slot_dependency_from_ir_expr(expr) {
                    return Some(target);
                }
            }
            None
        }
        // Expressions that don't contain slot dependencies
        IrExpression::LexicalRead(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::RestoreView(_)
        | IrExpression::Reference(_)
        | IrExpression::ReadVariable(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::ConditionalCase(_)
        | IrExpression::ConstReference(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::ArrowFunction(_) => None,
    }
}

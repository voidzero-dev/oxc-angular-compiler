//! Variable optimization phase.
//!
//! Optimizes variable usage in the IR by:
//! 1. Inlining AlwaysInline variables unconditionally
//! 2. Removing unused variable declarations
//! 3. Inlining single-use variables where safe (respecting fences)
//! 4. Converting unused side-effectful variables to statements
//!
//! Ported from Angular's `template/pipeline/src/phases/variable_optimization.ts`.

use std::collections::{HashMap, HashSet};
use std::ptr::NonNull;

use oxc_allocator::{Box as OxcBox, Vec as OxcVec};

use crate::ir::enums::VariableFlags;
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, StatementOp, UpdateOp, UpdateOpBase, UpdateVariableOp, XrefId};
use crate::output::ast::{
    ExpressionStatement, OutputExpression, OutputStatement, ReturnStatement, WrappedIrExpr,
};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Fence flags for expressions indicating how they can be optimized.
/// Ported from Angular's variable_optimization.ts Fence enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Fence(u8);

impl Fence {
    /// Empty flag (no fence exists)
    const NONE: Self = Self(0b000);
    /// Expression reads from the "current view" context
    const VIEW_CONTEXT_READ: Self = Self(0b001);
    /// Expression writes to the "current view" context
    const VIEW_CONTEXT_WRITE: Self = Self(0b010);
    /// Call is required for its side-effects
    const SIDE_EFFECTFUL: Self = Self(0b100);

    /// Check if this fence contains a specific flag
    fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }
}

impl std::ops::BitOr for Fence {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Fence {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Get the fence flags for an IR expression.
/// Ported from Angular's `fencesForIrExpression` function.
fn fences_for_expression(expr: &IrExpression<'_>) -> Fence {
    match expr {
        // NextContext reads and writes view context
        IrExpression::NextContext(_) => Fence::VIEW_CONTEXT_READ | Fence::VIEW_CONTEXT_WRITE,
        // RestoreView reads/writes view context and is side-effectful
        IrExpression::RestoreView(_) => {
            Fence::VIEW_CONTEXT_READ | Fence::VIEW_CONTEXT_WRITE | Fence::SIDE_EFFECTFUL
        }
        // StoreLet is side-effectful
        IrExpression::StoreLet(_) => Fence::SIDE_EFFECTFUL,
        // Reference and ContextLetReference read view context
        IrExpression::Reference(_) | IrExpression::ContextLetReference(_) => {
            Fence::VIEW_CONTEXT_READ
        }
        _ => Fence::NONE,
    }
}

/// Recursively collect fence flags from an expression and all sub-expressions.
fn collect_fences(expr: &IrExpression<'_>) -> Fence {
    let mut fences = fences_for_expression(expr);

    // Recurse into sub-expressions
    match expr {
        IrExpression::PureFunction(pf) => {
            if let Some(ref body) = pf.body {
                fences |= collect_fences(body);
            }
            if let Some(ref fn_ref) = pf.fn_ref {
                fences |= collect_fences(fn_ref);
            }
            for arg in pf.args.iter() {
                fences |= collect_fences(arg);
            }
        }
        IrExpression::Interpolation(interp) => {
            for e in interp.expressions.iter() {
                fences |= collect_fences(e);
            }
        }
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref inner) = rv.view {
                fences |= collect_fences(inner);
            }
        }
        IrExpression::ResetView(rv) => {
            fences |= collect_fences(&rv.expr);
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter() {
                fences |= collect_fences(arg);
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            fences |= collect_fences(&pbv.args);
        }
        IrExpression::SafePropertyRead(spr) => {
            fences |= collect_fences(&spr.receiver);
        }
        IrExpression::SafeKeyedRead(skr) => {
            fences |= collect_fences(&skr.receiver);
            fences |= collect_fences(&skr.index);
        }
        IrExpression::SafeInvokeFunction(sif) => {
            fences |= collect_fences(&sif.receiver);
            for arg in sif.args.iter() {
                fences |= collect_fences(arg);
            }
        }
        IrExpression::SafeTernary(st) => {
            fences |= collect_fences(&st.guard);
            fences |= collect_fences(&st.expr);
        }
        IrExpression::Ternary(t) => {
            fences |= collect_fences(&t.condition);
            fences |= collect_fences(&t.true_expr);
            fences |= collect_fences(&t.false_expr);
        }
        IrExpression::AssignTemporary(at) => {
            fences |= collect_fences(&at.expr);
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref e) = cc.expr {
                fences |= collect_fences(e);
            }
        }
        IrExpression::ConstCollected(cc) => {
            fences |= collect_fences(&cc.expr);
        }
        IrExpression::TwoWayBindingSet(twb) => {
            fences |= collect_fences(&twb.target);
            fences |= collect_fences(&twb.value);
        }
        IrExpression::StoreLet(sl) => {
            fences |= collect_fences(&sl.value);
        }
        IrExpression::Binary(binary) => {
            fences |= collect_fences(&binary.lhs);
            fences |= collect_fences(&binary.rhs);
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            fences |= collect_fences(&rpr.receiver);
        }
        IrExpression::ResolvedBinary(rb) => {
            fences |= collect_fences(&rb.left);
            fences |= collect_fences(&rb.right);
        }
        IrExpression::ResolvedCall(rc) => {
            fences |= collect_fences(&rc.receiver);
            for arg in rc.args.iter() {
                fences |= collect_fences(arg);
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            fences |= collect_fences(&rkr.receiver);
            fences |= collect_fences(&rkr.key);
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            fences |= collect_fences(&rspr.receiver);
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter() {
                fences |= collect_fences(entry);
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter() {
                fences |= collect_fences(value);
            }
        }
        // Leaf expressions - no sub-expressions to recurse into
        // Note: Reference, NextContext, and ContextLetReference are handled by fences_for_expression
        // but don't have sub-expressions to recurse into
        IrExpression::LexicalRead(_)
        | IrExpression::ReadVariable(_)
        | IrExpression::Context(_)
        | IrExpression::Reference(_)
        | IrExpression::NextContext(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::LiteralArray(_)
        | IrExpression::LiteralMap(_) => {}
        IrExpression::Not(n) => {
            fences |= collect_fences(&n.expr);
        }
        IrExpression::Unary(u) => {
            fences |= collect_fences(&u.expr);
        }
        IrExpression::Typeof(t) => {
            fences |= collect_fences(&t.expr);
        }
        IrExpression::Void(v) => {
            fences |= collect_fences(&v.expr);
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter() {
                fences |= collect_fences(e);
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            fences |= collect_fences(&arrow_fn.body);
        }
        IrExpression::Parenthesized(paren) => {
            fences |= collect_fences(&paren.expr);
        }
    }

    fences
}

/// Collect fences for an update operation.
fn collect_op_fences(op: &UpdateOp<'_>) -> Fence {
    match op {
        UpdateOp::Variable(var) => collect_fences(&var.initializer),
        UpdateOp::Property(prop) => collect_fences(&prop.expression),
        UpdateOp::StyleProp(style) => collect_fences(&style.expression),
        UpdateOp::ClassProp(class) => collect_fences(&class.expression),
        UpdateOp::StyleMap(style) => collect_fences(&style.expression),
        UpdateOp::ClassMap(class) => collect_fences(&class.expression),
        UpdateOp::Attribute(attr) => collect_fences(&attr.expression),
        UpdateOp::DomProperty(dom) => collect_fences(&dom.expression),
        UpdateOp::TwoWayProperty(two_way) => collect_fences(&two_way.expression),
        UpdateOp::Binding(binding) => collect_fences(&binding.expression),
        UpdateOp::InterpolateText(text) => collect_fences(&text.interpolation),
        UpdateOp::StoreLet(store) => collect_fences(&store.value),
        UpdateOp::Conditional(cond) => {
            let mut fences = Fence::NONE;
            if let Some(ref test) = cond.test {
                fences |= collect_fences(test);
            }
            for condition in cond.conditions.iter() {
                if let Some(ref expr) = condition.expr {
                    fences |= collect_fences(expr);
                }
            }
            if let Some(ref processed) = cond.processed {
                fences |= collect_fences(processed);
            }
            if let Some(ref ctx_val) = cond.context_value {
                fences |= collect_fences(ctx_val);
            }
            fences
        }
        UpdateOp::Repeater(rep) => collect_fences(&rep.collection),
        UpdateOp::AnimationBinding(anim) => collect_fences(&anim.expression),
        UpdateOp::Control(ctrl) => collect_fences(&ctrl.expression),
        UpdateOp::I18nExpression(i18n) => collect_fences(&i18n.expression),
        UpdateOp::DeferWhen(defer_when) => collect_fences(&defer_when.condition),
        UpdateOp::Statement(stmt) => collect_fences_in_output_statement(&stmt.statement),
        _ => Fence::NONE,
    }
}

/// Collect fences from an OutputStatement.
fn collect_fences_in_output_statement(stmt: &OutputStatement<'_>) -> Fence {
    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            collect_fences_in_output_expression(&expr_stmt.expr)
        }
        OutputStatement::If(if_stmt) => {
            let mut fences = collect_fences_in_output_expression(&if_stmt.condition);
            for s in if_stmt.true_case.iter() {
                fences |= collect_fences_in_output_statement(s);
            }
            for s in if_stmt.false_case.iter() {
                fences |= collect_fences_in_output_statement(s);
            }
            fences
        }
        OutputStatement::Return(ret) => collect_fences_in_output_expression(&ret.value),
        _ => Fence::NONE,
    }
}

/// Collect fences from an OutputExpression.
fn collect_fences_in_output_expression(expr: &OutputExpression<'_>) -> Fence {
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => collect_fences(&wrapped.node),
        OutputExpression::BinaryOperator(bin) => {
            collect_fences_in_output_expression(&bin.lhs)
                | collect_fences_in_output_expression(&bin.rhs)
        }
        OutputExpression::Conditional(cond) => {
            let mut fences = collect_fences_in_output_expression(&cond.condition);
            fences |= collect_fences_in_output_expression(&cond.true_case);
            if let Some(ref false_case) = cond.false_case {
                fences |= collect_fences_in_output_expression(false_case);
            }
            fences
        }
        OutputExpression::InvokeFunction(call) => {
            let mut fences = collect_fences_in_output_expression(&call.fn_expr);
            for arg in call.args.iter() {
                fences |= collect_fences_in_output_expression(arg);
            }
            fences
        }
        OutputExpression::Not(unary) => collect_fences_in_output_expression(&unary.condition),
        OutputExpression::ReadProp(prop) => collect_fences_in_output_expression(&prop.receiver),
        OutputExpression::ReadKey(keyed) => {
            collect_fences_in_output_expression(&keyed.receiver)
                | collect_fences_in_output_expression(&keyed.index)
        }
        _ => Fence::NONE,
    }
}

/// Collect fences for a create operation.
fn collect_create_op_fences(op: &CreateOp<'_>) -> Fence {
    match op {
        CreateOp::Variable(var) => collect_fences(&var.initializer),
        CreateOp::Listener(listener) => {
            let mut fences = Fence::NONE;
            for handler_op in listener.handler_ops.iter() {
                fences |= collect_op_fences(handler_op);
            }
            if let Some(ref handler_expr) = listener.handler_expression {
                fences |= collect_fences(handler_expr);
            }
            fences
        }
        CreateOp::TwoWayListener(listener) => {
            let mut fences = Fence::NONE;
            for handler_op in listener.handler_ops.iter() {
                fences |= collect_op_fences(handler_op);
            }
            fences
        }
        CreateOp::AnimationListener(listener) => {
            let mut fences = Fence::NONE;
            for handler_op in listener.handler_ops.iter() {
                fences |= collect_op_fences(handler_op);
            }
            fences
        }
        CreateOp::Animation(animation) => {
            let mut fences = Fence::NONE;
            for handler_op in animation.handler_ops.iter() {
                fences |= collect_op_fences(handler_op);
            }
            fences
        }
        // DeclareLet doesn't have expressions at the create phase
        CreateOp::DeclareLet(_) => Fence::NONE,
        _ => Fence::NONE,
    }
}

/// Optimizes variable declarations and usage.
///
/// This phase:
/// 1. Inlines AlwaysInline variables unconditionally (for aliases like ctx)
/// 2. Removes unused variables (converting side-effectful ones to statements)
/// 3. Inlines single-use variables where safe (respecting fences)
/// 4. Optimizes variables in listener handler_ops (separately for each listener)
///
/// Following Angular's variable_optimization.ts, listener handler_ops are optimized
/// as separate scope - variables unused within a handler are removed from that handler.
///
/// Note: SavedView variables (from `getCurrentView()`) can be removed if unused
/// because `GetCurrentViewExpr` has no side effects.
pub fn optimize_variables(job: &mut ComponentCompilationJob<'_>) {
    // Step 1: Inline AlwaysInline variables unconditionally
    // Per TypeScript's variable_optimization.ts lines 33-46
    inline_always_inline_variables(job);

    // Step 2: Inline Identifier variables whose initializer is just `ctx`.
    // Per TypeScript's allowConservativeInlining (lines 527-535):
    // ```typescript
    // case ir.SemanticVariableKind.Identifier:
    //   if (decl.initializer instanceof o.ReadVarExpr && decl.initializer.name === 'ctx') {
    //     return true;  // Allow inlining for conditional aliases
    //   }
    //   return false;
    // ```
    // This handles conditional aliases like `@if (icon(); as icon)` where the child view
    // creates an Identifier variable `icon` with initializer `ctx`. TypeScript inlines
    // these to use `ctx` directly: `ɵɵclassMap(ctx)` instead of `const icon = ctx; ɵɵclassMap(icon)`.
    inline_identifier_ctx_variables(job);

    // Step 3: Remove unused variables and inline single-use variables
    // Iterate until no more variables can be removed.
    // This handles cases where variable A is only used by variable B,
    // and B is unused - we need to remove B first, then A becomes unused.
    //
    // IMPORTANT: This must run BEFORE Context inlining so that unused Identifier
    // variables are removed first, reducing the usage count of Context variables.
    loop {
        let removed = optimize_variables_once(job);
        if !removed {
            break;
        }
    }

    // Step 4: Inline Context variables (nextContext()) into other Variable ops.
    // Per TypeScript's allowConservativeInlining (lines 536-538):
    // "Context can only be inlined into other variables."
    // This is safe because we only inline into Variable ops, not into arbitrary ops.
    //
    // This runs AFTER unused variable removal so that Context variables have
    // minimal usage counts (ideally 1 for single-use inlining).
    inline_context_variables_into_variable_ops(job);

    // Step 5: Run unused variable removal again after context inlining.
    // Context inlining may reduce usage counts of Context variables to 0, making them
    // eligible for removal. Without this, unused Context variables would be converted
    // to statements (standalone nextContext() calls) instead of being removed entirely.
    //
    // Example: If we have ctx_a = nextContext() followed by ctx_b = nextContext(),
    // and ctx_b is inlined into another variable, ctx_b's usage drops to 0. At this point,
    // ctx_a may also become removable if nothing depends on its context write.
    loop {
        let removed = optimize_variables_once(job);
        if !removed {
            break;
        }
    }

    // Step 6: Optimize arrow function ops (per Angular's variable_optimization.ts lines 53-56)
    // for (const expr of unit.functions) {
    //   optimizeVariablesInOpList(expr.ops, job.compatibility, null);
    //   optimizeSaveRestoreView(expr.ops);
    // }
    optimize_arrow_function_ops(job);

    // Step 7: Optimize listener handler_ops separately (per Angular's variable_optimization.ts lines 58-70)
    optimize_listener_handler_ops(job);
}

/// Inlines variables that should always be inlined.
///
/// Per TypeScript's variable_optimization.ts (lines 121-150), only variables marked
/// with the AlwaysInline flag are inlined in this phase. This includes aliases for
/// computed @for loop variables like `$first`, `$last`, etc.
///
/// Per TypeScript's variable_optimization.ts (lines 36-47), we also need to process
/// listener handler_ops and repeater trackByOps separately, as they form their own scope.
fn inline_always_inline_variables(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect view xrefs to process
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        // Process update ops
        inline_always_inline_in_op_list_update(job, view_xref, allocator);
        // Process create ops
        inline_always_inline_in_op_list_create(job, view_xref, allocator);
        // Process listener handler_ops and repeater trackByOps
        // Per TypeScript's variable_optimization.ts lines 36-47:
        // for (const op of unit.create) {
        //   if (op.kind === ir.OpKind.Listener || ...) {
        //     inlineAlwaysInlineVariables(op.handlerOps);
        //   } else if (op.kind === ir.OpKind.RepeaterCreate && op.trackByOps !== null) {
        //     inlineAlwaysInlineVariables(op.trackByOps);
        //   }
        // }
        inline_always_inline_in_listener_handler_ops(job, view_xref, allocator);
    }
}

/// Inline AlwaysInline variables in listener handler_ops and repeater trackByOps.
///
/// Per TypeScript's variable_optimization.ts (lines 36-47), these ops form their own scope
/// and need to be processed separately with their own AlwaysInline variables.
fn inline_always_inline_in_listener_handler_ops<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) {
    let view = if view_xref == job.root.xref {
        &mut job.root
    } else if let Some(v) = job.views.get_mut(&view_xref) {
        v.as_mut()
    } else {
        return;
    };

    for op in view.create.iter_mut() {
        match op {
            CreateOp::Listener(listener) => {
                inline_always_inline_in_handler_ops_and_expr(
                    &mut listener.handler_ops,
                    listener.handler_expression.as_mut(),
                    allocator,
                );
            }
            CreateOp::TwoWayListener(listener) => {
                inline_always_inline_in_handler_ops_and_expr(
                    &mut listener.handler_ops,
                    None, // TwoWayListener doesn't have handler_expression
                    allocator,
                );
            }
            CreateOp::AnimationListener(listener) => {
                inline_always_inline_in_handler_ops_and_expr(
                    &mut listener.handler_ops,
                    None, // AnimationListener doesn't have handler_expression
                    allocator,
                );
            }
            CreateOp::Animation(animation) => {
                inline_always_inline_in_handler_ops_and_expr(
                    &mut animation.handler_ops,
                    None, // Animation doesn't have handler_expression
                    allocator,
                );
            }
            CreateOp::RepeaterCreate(repeater) => {
                if let Some(track_by_ops) = &mut repeater.track_by_ops {
                    inline_always_inline_in_handler_ops_and_expr(track_by_ops, None, allocator);
                }
            }
            _ => {}
        }
    }
}

/// Inline AlwaysInline variables in handler_ops and handler_expression.
///
/// Per TypeScript's variable_optimization.ts (lines 121-150), this:
/// 1. Collects all AlwaysInline variables from the ops
/// 2. Inlines them into all expressions that reference them (including handler_expression)
/// 3. Removes the variable declarations
fn inline_always_inline_in_handler_ops_and_expr<'a>(
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    handler_expression: Option<&mut OxcBox<'a, IrExpression<'a>>>,
    allocator: &'a oxc_allocator::Allocator,
) {
    // First pass: collect AlwaysInline variables
    let mut always_inline_vars: HashMap<XrefId, IrExpression<'a>> = HashMap::new();
    for op in handler_ops.iter() {
        if let UpdateOp::Variable(var) = op {
            if var.flags.contains(VariableFlags::ALWAYS_INLINE) {
                always_inline_vars.insert(var.xref, var.initializer.clone_in(allocator));
            }
        }
    }

    if always_inline_vars.is_empty() {
        return;
    }

    // Second pass: inline the variables into all expressions in handler_ops
    for op in handler_ops.iter_mut() {
        transform_expressions_in_update_op(op, allocator, |expr| {
            if let IrExpression::ReadVariable(read_var) = expr {
                if let Some(initializer) = always_inline_vars.get(&read_var.xref) {
                    return Some(initializer.clone_in(allocator));
                }
            }
            None
        });
    }

    // Also inline into handler_expression (the return value expression)
    if let Some(expr) = handler_expression {
        inline_into_expression(expr.as_mut(), &always_inline_vars, allocator);
    }

    // Third pass: remove the inlined variable declarations
    // We need to filter out the AlwaysInline variables from the Vec
    let mut indices_to_remove: Vec<usize> = Vec::new();
    for (idx, op) in handler_ops.iter().enumerate() {
        if let UpdateOp::Variable(var) = op {
            if always_inline_vars.contains_key(&var.xref) {
                indices_to_remove.push(idx);
            }
        }
    }

    // Remove in reverse order to preserve indices
    for idx in indices_to_remove.into_iter().rev() {
        handler_ops.remove(idx);
    }
}

/// Recursively inline variables into an expression.
fn inline_into_expression<'a>(
    expr: &mut IrExpression<'a>,
    vars: &HashMap<XrefId, IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) {
    // Check if this expression should be inlined
    if let IrExpression::ReadVariable(read_var) = expr {
        if let Some(initializer) = vars.get(&read_var.xref) {
            *expr = initializer.clone_in(allocator);
            return;
        }
    }

    // Recursively process sub-expressions
    use crate::ir::expression::{VisitorContextFlag, transform_expressions_in_expression};
    transform_expressions_in_expression(
        expr,
        &|e, _| {
            if let IrExpression::ReadVariable(read_var) = e {
                if let Some(initializer) = vars.get(&read_var.xref) {
                    *e = initializer.clone_in(allocator);
                }
            }
        },
        VisitorContextFlag::NONE,
    );
}

/// Inline AlwaysInline variables in an update op list.
///
/// Per TypeScript's variable_optimization.ts (lines 121-150), only variables marked
/// with the AlwaysInline flag are inlined.
fn inline_always_inline_in_op_list_update<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) {
    // First pass: collect AlwaysInline variables and ctx alias variables
    let always_inline_vars: HashMap<XrefId, IrExpression<'a>> = {
        let view = if view_xref == job.root.xref {
            Some(&job.root)
        } else {
            job.views.get(&view_xref).map(|v| v.as_ref())
        };

        let Some(view) = view else {
            return;
        };

        let mut vars = HashMap::new();
        for op in view.update.iter() {
            if let UpdateOp::Variable(var) = op {
                // Per TypeScript's variable_optimization.ts (lines 121-150),
                // only inline variables with the AlwaysInline flag
                if var.flags.contains(VariableFlags::ALWAYS_INLINE) {
                    // Clone the initializer for inlining
                    vars.insert(var.xref, var.initializer.clone_in(allocator));
                }
            }
        }
        vars
    };

    if always_inline_vars.is_empty() {
        return;
    }

    // Second pass: inline the variables into all expressions
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        for op in view.update.iter_mut() {
            transform_expressions_in_update_op(op, allocator, |expr| {
                if let IrExpression::ReadVariable(read_var) = expr {
                    if let Some(initializer) = always_inline_vars.get(&read_var.xref) {
                        return Some(initializer.clone_in(allocator));
                    }
                }
                None
            });
        }
    }

    // Third pass: remove the inlined variable declarations
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        let mut ops_to_remove: Vec<NonNull<UpdateOp<'a>>> = Vec::new();
        for op in view.update.iter() {
            if let UpdateOp::Variable(var) = op {
                if always_inline_vars.contains_key(&var.xref) {
                    ops_to_remove.push(NonNull::from(op));
                }
            }
        }

        for op_ptr in ops_to_remove {
            // SAFETY: The pointer came from the list and we're removing it
            unsafe {
                view.update.remove(op_ptr);
            }
        }
    }
}

/// Inline AlwaysInline variables in a create op list.
///
/// Per TypeScript's variable_optimization.ts (lines 121-150), only variables marked
/// with the AlwaysInline flag are inlined.
fn inline_always_inline_in_op_list_create<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) {
    // First pass: collect AlwaysInline variables
    let always_inline_vars: HashMap<XrefId, IrExpression<'a>> = {
        let view = if view_xref == job.root.xref {
            Some(&job.root)
        } else {
            job.views.get(&view_xref).map(|v| v.as_ref())
        };

        let Some(view) = view else {
            return;
        };

        let mut vars = HashMap::new();
        for op in view.create.iter() {
            if let CreateOp::Variable(var) = op {
                // Per TypeScript's variable_optimization.ts (lines 121-150),
                // only inline variables with the AlwaysInline flag
                if var.flags.contains(VariableFlags::ALWAYS_INLINE) {
                    // Clone the initializer for inlining
                    vars.insert(var.xref, var.initializer.clone_in(allocator));
                }
            }
        }
        vars
    };

    if always_inline_vars.is_empty() {
        return;
    }

    // Second pass: inline the variables into all expressions
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        for op in view.create.iter_mut() {
            transform_expressions_in_create_op(op, allocator, |expr| {
                if let IrExpression::ReadVariable(read_var) = expr {
                    if let Some(initializer) = always_inline_vars.get(&read_var.xref) {
                        return Some(initializer.clone_in(allocator));
                    }
                }
                None
            });
        }
    }

    // Third pass: remove the inlined variable declarations
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        let mut ops_to_remove: Vec<NonNull<CreateOp<'a>>> = Vec::new();
        for op in view.create.iter() {
            if let CreateOp::Variable(var) = op {
                if always_inline_vars.contains_key(&var.xref) {
                    ops_to_remove.push(NonNull::from(op));
                }
            }
        }

        for op_ptr in ops_to_remove {
            // SAFETY: The pointer came from the list and we're removing it
            unsafe {
                view.create.remove(op_ptr);
            }
        }
    }
}

/// Inlines Identifier variables whose initializer is `LexicalRead("ctx")`.
///
/// Per TypeScript's allowConservativeInlining (variable_optimization.ts lines 527-535):
/// ```typescript
/// case ir.SemanticVariableKind.Identifier:
///   if (decl.initializer instanceof o.ReadVarExpr && decl.initializer.name === 'ctx') {
///     // Although TemplateDefinitionBuilder is cautious about inlining, we still want to do so
///     // when the variable is the context, to imitate its behavior with aliases in control flow
///     // blocks. This quirky behavior will become dead code once compatibility mode is no longer
///     // supported.
///     return true;
///   }
///   return false;
/// ```
///
/// This handles conditional aliases like `@if (icon(); as icon)` where the child view creates
/// an Identifier variable with initializer `ctx`. TypeScript allows these to be inlined into
/// any target operation (not just Variable ops), so `const icon = ctx; ɵɵclassMap(icon)` becomes
/// `ɵɵclassMap(ctx)`.
fn inline_identifier_ctx_variables(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect view xrefs to process
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        inline_identifier_ctx_vars_in_update_ops(job, view_xref, allocator);
    }
}

/// Inline Identifier variables with `ctx` initializer in update ops.
///
/// Per TypeScript's allowConservativeInlining (variable_optimization.ts lines 527-535):
/// ```typescript
/// case ir.SemanticVariableKind.Identifier:
///   if (decl.initializer instanceof o.ReadVarExpr && decl.initializer.name === 'ctx') {
///     return true;  // Allow inlining for conditional aliases
///   }
///   return false;
/// ```
///
/// TypeScript's `allowConservativeInlining` determines what TYPES of variables can be inlined
/// into any target operation. For Identifier variables with `ctx` initializer, inlining is
/// allowed into ANY target op (not just Variable ops). However, the actual inlining is still
/// gated by usage count - we only inline when count === 1 (per lines 232-252).
///
/// Additionally, we also inline when count === 0 (unused variables), effectively removing them.
/// This handles conditional aliases like `@if (data$ | async; as data)` where `data` is an
/// alias for `ctx` but may not be used in the template.
fn inline_identifier_ctx_vars_in_update_ops<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) {
    use crate::ir::enums::SemanticVariableKind;

    // First pass: collect Identifier variables with LexicalRead("ctx") or Context initializer
    // and count their usages in the view's update ops.
    let (ctx_vars, usage_counts): (HashMap<XrefId, IrExpression<'a>>, HashMap<XrefId, usize>) = {
        let view = if view_xref == job.root.xref {
            Some(&job.root)
        } else {
            job.views.get(&view_xref).map(|v| v.as_ref())
        };

        let Some(view) = view else {
            return;
        };

        let mut vars = HashMap::new();
        for op in view.update.iter() {
            if let UpdateOp::Variable(var) = op {
                // Only collect Identifier kind variables with ctx initializer
                // This can be either LexicalRead("ctx") or Context expression (for current view)
                if var.kind == SemanticVariableKind::Identifier {
                    let is_ctx_ref = match var.initializer.as_ref() {
                        IrExpression::LexicalRead(lexical) => lexical.name.as_str() == "ctx",
                        // Context for current view - resolve_contexts keeps this as Context
                        // and reify converts it to `ctx`
                        IrExpression::Context(ctx) => ctx.view == view_xref,
                        _ => false,
                    };

                    if is_ctx_ref {
                        vars.insert(var.xref, var.initializer.clone_in(allocator));
                    }
                }
            }
        }

        // Count usages of these variables in the view's update ops
        let mut counts: HashMap<XrefId, usize> = HashMap::new();
        for op in view.update.iter() {
            count_in_update_op(op, &mut counts);
        }

        (vars, counts)
    };

    if ctx_vars.is_empty() {
        return;
    }

    // Filter to only inline variables that are used at most once (count <= 1).
    // Per TypeScript's variable_optimization.ts (lines 232-245):
    // "We can inline variables that are used exactly once"
    // We also inline/remove variables with count === 0 (unused).
    let single_use_ctx_vars: HashMap<XrefId, IrExpression<'a>> = ctx_vars
        .into_iter()
        .filter(|(xref, _)| usage_counts.get(xref).copied().unwrap_or(0) <= 1)
        .collect();

    if single_use_ctx_vars.is_empty() {
        return;
    }

    // Second pass: inline the variables into all expressions (any target op is allowed).
    // Unlike Context variables (nextContext()), Identifier variables with `ctx` initializer
    // can be inlined into ANY target operation because `ctx` is just the function parameter.
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        for op in view.update.iter_mut() {
            transform_expressions_in_update_op(op, allocator, |expr| {
                if let IrExpression::ReadVariable(read_var) = expr {
                    if let Some(initializer) = single_use_ctx_vars.get(&read_var.xref) {
                        return Some(initializer.clone_in(allocator));
                    }
                }
                None
            });
        }
    }

    // Third pass: remove the inlined variable declarations
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        let mut ops_to_remove: Vec<NonNull<UpdateOp<'a>>> = Vec::new();
        for op in view.update.iter() {
            if let UpdateOp::Variable(var) = op {
                if single_use_ctx_vars.contains_key(&var.xref) {
                    ops_to_remove.push(NonNull::from(op));
                }
            }
        }

        for op_ptr in ops_to_remove {
            unsafe {
                view.update.remove(op_ptr);
            }
        }
    }
}

/// Inline Context variables (with NextContext initializers) into other Variable ops.
///
/// Per TypeScript's allowConservativeInlining (variable_optimization.ts lines 536-538):
/// ```typescript
/// case ir.SemanticVariableKind.Context:
///   // Context can only be inlined into other variables.
///   return target.kind === ir.OpKind.Variable;
/// ```
///
/// This is safe because:
/// 1. We only inline Context variables, which contain NextContext()
/// 2. We only inline into Variable ops, not into property bindings or listeners
/// 3. The target Variable's initializer contains a ReadVariable referencing the Context var
fn inline_context_variables_into_variable_ops(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect view xrefs
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        inline_context_vars_in_view_update_ops(job, view_xref, allocator);
    }
}

/// Inline Context variables in a view's update ops.
fn inline_context_vars_in_view_update_ops<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    allocator: &'a oxc_allocator::Allocator,
) {
    use crate::ir::enums::SemanticVariableKind;

    // First pass: collect Context variables (with NextContext initializers), their positions, and count usages
    // Also collect positions of all ops for intervening fence checking.
    let (context_vars, context_var_positions, op_fences): (
        HashMap<XrefId, IrExpression<'a>>,
        HashMap<XrefId, usize>,
        Vec<Fence>,
    ) = {
        let view = if view_xref == job.root.xref {
            Some(&job.root)
        } else {
            job.views.get(&view_xref).map(|v| v.as_ref())
        };

        let Some(view) = view else {
            return;
        };

        let mut vars = HashMap::new();
        let mut positions = HashMap::new();
        let mut fences = Vec::new();
        for (idx, op) in view.update.iter().enumerate() {
            fences.push(collect_op_fences(op));
            if let UpdateOp::Variable(var) = op {
                // Only collect Context kind variables with NextContext initializer
                if var.kind == SemanticVariableKind::Context {
                    if matches!(var.initializer.as_ref(), IrExpression::NextContext(_)) {
                        vars.insert(var.xref, var.initializer.clone_in(allocator));
                        positions.insert(var.xref, idx);
                    }
                }
            }
        }
        (vars, positions, fences)
    };

    if context_vars.is_empty() {
        return;
    }

    // Count usages of context variables, tracking which are used in Variable ops vs elsewhere
    // Also track the position of the using variable op
    let (usage_counts, var_op_usages, var_op_positions): (
        HashMap<XrefId, usize>,
        HashMap<XrefId, HashSet<XrefId>>,
        HashMap<XrefId, usize>,
    ) = {
        let view = if view_xref == job.root.xref {
            Some(&job.root)
        } else {
            job.views.get(&view_xref).map(|v| v.as_ref())
        };

        let Some(view) = view else {
            return;
        };

        let mut counts: HashMap<XrefId, usize> = HashMap::new();
        let mut var_usages: HashMap<XrefId, HashSet<XrefId>> = HashMap::new(); // context_xref -> set of using var xrefs
        let mut var_positions: HashMap<XrefId, usize> = HashMap::new(); // var_xref -> position

        for (idx, op) in view.update.iter().enumerate() {
            if let UpdateOp::Variable(var) = op {
                // Check if this variable uses any context variable
                visit_all_expressions(&var.initializer, &mut |expr| {
                    if let IrExpression::ReadVariable(read) = expr {
                        if context_vars.contains_key(&read.xref) {
                            *counts.entry(read.xref).or_insert(0) += 1;
                            var_usages.entry(read.xref).or_default().insert(var.xref);
                            var_positions.insert(var.xref, idx);
                        }
                    }
                });
            } else {
                // Check usages in non-Variable ops (these prevent inlining)
                visit_expressions_in_update_op(op, |expr| {
                    if let IrExpression::ReadVariable(read) = expr {
                        if context_vars.contains_key(&read.xref) {
                            *counts.entry(read.xref).or_insert(0) += 1;
                            // Don't add to var_usages - this is a non-Variable usage
                        }
                    }
                });
            }
        }

        (counts, var_usages, var_positions)
    };

    // Determine which context variables can be inlined:
    // - Used exactly once
    // - Used only in a Variable op (not in property bindings, etc.)
    // - No intervening ops with VIEW_CONTEXT_READ fence between definition and usage
    //   (because nextContext() changes the view context state, and intervening reference()
    //   calls depend on that state being in the correct order)
    let mut to_inline: HashSet<XrefId> = HashSet::new();
    for (ctx_xref, count) in &usage_counts {
        if *count == 1 {
            // Check if the single usage is in a Variable op
            if let Some(using_vars) = var_op_usages.get(ctx_xref) {
                if using_vars.len() == 1 {
                    let using_var_xref = using_vars.iter().next().unwrap();

                    // Check for intervening VIEW_CONTEXT_READ ops between context var and usage
                    let ctx_pos = context_var_positions.get(ctx_xref).copied().unwrap_or(0);
                    let usage_pos = var_op_positions.get(using_var_xref).copied().unwrap_or(0);

                    // Check if any ops between ctx_pos (exclusive) and usage_pos (exclusive)
                    // have VIEW_CONTEXT_READ fences
                    let has_intervening_context_read = (ctx_pos + 1..usage_pos).any(|i| {
                        op_fences
                            .get(i)
                            .copied()
                            .unwrap_or(Fence::NONE)
                            .contains(Fence::VIEW_CONTEXT_READ)
                    });

                    if !has_intervening_context_read {
                        to_inline.insert(*ctx_xref);
                    }
                }
            }
        }
    }

    if to_inline.is_empty() {
        return;
    }

    // Second pass: inline the context variables into the Variable ops that use them
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        for op in view.update.iter_mut() {
            if let UpdateOp::Variable(var) = op {
                // Only transform Variable ops
                transform_expression_in_place(&mut var.initializer, allocator, |expr| {
                    if let IrExpression::ReadVariable(read) = expr {
                        if to_inline.contains(&read.xref) {
                            if let Some(initializer) = context_vars.get(&read.xref) {
                                return Some(initializer.clone_in(allocator));
                            }
                        }
                    }
                    None
                });
            }
        }
    }

    // Third pass: remove the inlined context variable declarations
    {
        let view = if view_xref == job.root.xref {
            Some(&mut job.root)
        } else {
            job.views.get_mut(&view_xref).map(|v| v.as_mut())
        };

        let Some(view) = view else {
            return;
        };

        let mut ops_to_remove: Vec<NonNull<UpdateOp<'a>>> = Vec::new();
        for op in view.update.iter() {
            if let UpdateOp::Variable(var) = op {
                if to_inline.contains(&var.xref) {
                    ops_to_remove.push(NonNull::from(op));
                }
            }
        }

        for op_ptr in ops_to_remove {
            unsafe {
                view.update.remove(op_ptr);
            }
        }
    }
}

/// Transform an expression in place.
fn transform_expression_in_place<'a, F>(
    expr: &mut OxcBox<'a, IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
    mut transform: F,
) where
    F: FnMut(&IrExpression<'a>) -> Option<IrExpression<'a>>,
{
    let new_expr = transform_expression(expr.as_ref(), allocator, &mut transform);
    **expr = new_expr;
}

/// Performs one pass of variable optimization on view create/update lists.
/// Returns true if any variables were removed, converted, or inlined.
fn optimize_variables_once(job: &mut ComponentCompilationJob<'_>) -> bool {
    // Count variable usages across all views (create/update ops)
    let usage_counts = count_variable_usages(job);

    // Collect variables to remove or convert
    // Per Angular's variable_optimization.ts lines 185-230:
    // - Process ops in reverse order
    // - Track whether view context is used (contextIsUsed)
    // - Unused side-effectful variables are converted to statement ops
    // - Unused non-side-effectful variables are removed
    let actions = collect_removable_variables_with_context_tracking(job, &usage_counts);

    let has_removals = !actions.to_remove.is_empty() || !actions.to_convert.is_empty();

    if has_removals {
        // Convert side-effectful variables to statement ops
        convert_variables_to_statements(job, &actions.to_convert);
        // Remove unused variables from all views
        remove_unused_variables(job, &actions.to_remove);
    }

    has_removals
}

/// Visit all expressions in an UpdateOp.
fn visit_expressions_in_update_op<F>(op: &UpdateOp<'_>, mut visitor: F)
where
    F: FnMut(&IrExpression<'_>),
{
    match op {
        UpdateOp::Variable(var) => visit_all_expressions(&var.initializer, &mut visitor),
        UpdateOp::Property(prop) => visit_all_expressions(&prop.expression, &mut visitor),
        UpdateOp::StyleProp(style) => visit_all_expressions(&style.expression, &mut visitor),
        UpdateOp::ClassProp(class) => visit_all_expressions(&class.expression, &mut visitor),
        UpdateOp::StyleMap(style) => visit_all_expressions(&style.expression, &mut visitor),
        UpdateOp::ClassMap(class) => visit_all_expressions(&class.expression, &mut visitor),
        UpdateOp::Attribute(attr) => visit_all_expressions(&attr.expression, &mut visitor),
        UpdateOp::DomProperty(dom) => visit_all_expressions(&dom.expression, &mut visitor),
        UpdateOp::TwoWayProperty(two_way) => {
            visit_all_expressions(&two_way.expression, &mut visitor)
        }
        UpdateOp::Binding(binding) => visit_all_expressions(&binding.expression, &mut visitor),
        UpdateOp::InterpolateText(text) => visit_all_expressions(&text.interpolation, &mut visitor),
        UpdateOp::StoreLet(store) => visit_all_expressions(&store.value, &mut visitor),
        UpdateOp::Conditional(cond) => {
            if let Some(ref test) = cond.test {
                visit_all_expressions(test, &mut visitor);
            }
            for condition in cond.conditions.iter() {
                if let Some(ref expr) = condition.expr {
                    visit_all_expressions(expr, &mut visitor);
                }
            }
            if let Some(ref processed) = cond.processed {
                visit_all_expressions(processed, &mut visitor);
            }
            if let Some(ref ctx_val) = cond.context_value {
                visit_all_expressions(ctx_val, &mut visitor);
            }
        }
        UpdateOp::Repeater(rep) => visit_all_expressions(&rep.collection, &mut visitor),
        UpdateOp::AnimationBinding(anim) => visit_all_expressions(&anim.expression, &mut visitor),
        UpdateOp::Control(ctrl) => visit_all_expressions(&ctrl.expression, &mut visitor),
        UpdateOp::I18nExpression(i18n) => visit_all_expressions(&i18n.expression, &mut visitor),
        UpdateOp::DeferWhen(defer_when) => {
            visit_all_expressions(&defer_when.condition, &mut visitor)
        }
        UpdateOp::Statement(stmt) => {
            visit_ir_expressions_in_output_statement(&stmt.statement, &mut visitor);
        }
        _ => {}
    }
}

/// Recursively visit all expressions.
fn visit_all_expressions<F>(expr: &IrExpression<'_>, visitor: &mut F)
where
    F: FnMut(&IrExpression<'_>),
{
    visitor(expr);
    match expr {
        IrExpression::PureFunction(pf) => {
            if let Some(ref body) = pf.body {
                visit_all_expressions(body, visitor);
            }
            if let Some(ref fn_ref) = pf.fn_ref {
                visit_all_expressions(fn_ref, visitor);
            }
            for arg in pf.args.iter() {
                visit_all_expressions(arg, visitor);
            }
        }
        IrExpression::Interpolation(interp) => {
            for e in interp.expressions.iter() {
                visit_all_expressions(e, visitor);
            }
        }
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref inner) = rv.view {
                visit_all_expressions(inner, visitor);
            }
        }
        IrExpression::ResetView(rv) => {
            visit_all_expressions(&rv.expr, visitor);
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter() {
                visit_all_expressions(arg, visitor);
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            visit_all_expressions(&pbv.args, visitor);
        }
        IrExpression::SafePropertyRead(spr) => {
            visit_all_expressions(&spr.receiver, visitor);
        }
        IrExpression::SafeKeyedRead(skr) => {
            visit_all_expressions(&skr.receiver, visitor);
            visit_all_expressions(&skr.index, visitor);
        }
        IrExpression::SafeInvokeFunction(sif) => {
            visit_all_expressions(&sif.receiver, visitor);
            for arg in sif.args.iter() {
                visit_all_expressions(arg, visitor);
            }
        }
        IrExpression::SafeTernary(st) => {
            visit_all_expressions(&st.guard, visitor);
            visit_all_expressions(&st.expr, visitor);
        }
        IrExpression::Ternary(t) => {
            visit_all_expressions(&t.condition, visitor);
            visit_all_expressions(&t.true_expr, visitor);
            visit_all_expressions(&t.false_expr, visitor);
        }
        IrExpression::AssignTemporary(at) => {
            visit_all_expressions(&at.expr, visitor);
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref e) = cc.expr {
                visit_all_expressions(e, visitor);
            }
        }
        IrExpression::ConstCollected(cc) => {
            visit_all_expressions(&cc.expr, visitor);
        }
        IrExpression::TwoWayBindingSet(twb) => {
            visit_all_expressions(&twb.target, visitor);
            visit_all_expressions(&twb.value, visitor);
        }
        IrExpression::StoreLet(sl) => {
            visit_all_expressions(&sl.value, visitor);
        }
        IrExpression::Binary(binary) => {
            visit_all_expressions(&binary.lhs, visitor);
            visit_all_expressions(&binary.rhs, visitor);
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            visit_all_expressions(&rpr.receiver, visitor);
        }
        IrExpression::ResolvedBinary(rb) => {
            visit_all_expressions(&rb.left, visitor);
            visit_all_expressions(&rb.right, visitor);
        }
        IrExpression::ResolvedCall(rc) => {
            visit_all_expressions(&rc.receiver, visitor);
            for arg in rc.args.iter() {
                visit_all_expressions(arg, visitor);
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            visit_all_expressions(&rkr.receiver, visitor);
            visit_all_expressions(&rkr.key, visitor);
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            visit_all_expressions(&rspr.receiver, visitor);
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter() {
                visit_all_expressions(entry, visitor);
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter() {
                visit_all_expressions(value, visitor);
            }
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter() {
                visit_all_expressions(elem, visitor);
            }
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter() {
                visit_all_expressions(value, visitor);
            }
        }
        IrExpression::Not(n) => {
            visit_all_expressions(&n.expr, visitor);
        }
        IrExpression::Unary(u) => {
            visit_all_expressions(&u.expr, visitor);
        }
        IrExpression::Typeof(t) => {
            visit_all_expressions(&t.expr, visitor);
        }
        IrExpression::Void(v) => {
            visit_all_expressions(&v.expr, visitor);
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter() {
                visit_all_expressions(e, visitor);
            }
        }
        IrExpression::ArrowFunction(arrow_fn) => {
            visit_all_expressions(&arrow_fn.body, visitor);
        }
        IrExpression::Parenthesized(paren) => {
            visit_all_expressions(&paren.expr, visitor);
        }
        // Leaf expressions
        _ => {}
    }
}

/// Visit IR expressions inside an OutputStatement.
///
/// Statement ops are created by `convert_variables_to_statements` when unused
/// side-effectful variables (like StoreLet) are converted from Variable ops to
/// standalone expression statements. The IR expressions inside these statements
/// still contain ReadVariable references that must be counted for correct
/// context variable inlining decisions.
fn visit_ir_expressions_in_output_statement<F>(stmt: &OutputStatement<'_>, visitor: &mut F)
where
    F: FnMut(&IrExpression<'_>),
{
    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            visit_ir_expressions_in_output_expression(&expr_stmt.expr, visitor);
        }
        OutputStatement::If(if_stmt) => {
            visit_ir_expressions_in_output_expression(&if_stmt.condition, visitor);
            for s in if_stmt.true_case.iter() {
                visit_ir_expressions_in_output_statement(s, visitor);
            }
            for s in if_stmt.false_case.iter() {
                visit_ir_expressions_in_output_statement(s, visitor);
            }
        }
        OutputStatement::Return(ret) => {
            visit_ir_expressions_in_output_expression(&ret.value, visitor);
        }
        _ => {}
    }
}

/// Visit IR expressions inside an OutputExpression.
///
/// Traverses OutputExpression tree to find WrappedIrNode nodes which contain
/// actual IR expressions that may reference variables.
fn visit_ir_expressions_in_output_expression<F>(expr: &OutputExpression<'_>, visitor: &mut F)
where
    F: FnMut(&IrExpression<'_>),
{
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            visit_all_expressions(&wrapped.node, visitor);
        }
        OutputExpression::BinaryOperator(bin) => {
            visit_ir_expressions_in_output_expression(&bin.lhs, visitor);
            visit_ir_expressions_in_output_expression(&bin.rhs, visitor);
        }
        OutputExpression::Conditional(cond) => {
            visit_ir_expressions_in_output_expression(&cond.condition, visitor);
            visit_ir_expressions_in_output_expression(&cond.true_case, visitor);
            if let Some(ref false_case) = cond.false_case {
                visit_ir_expressions_in_output_expression(false_case, visitor);
            }
        }
        OutputExpression::InvokeFunction(call) => {
            visit_ir_expressions_in_output_expression(&call.fn_expr, visitor);
            for arg in call.args.iter() {
                visit_ir_expressions_in_output_expression(arg, visitor);
            }
        }
        OutputExpression::Not(unary) => {
            visit_ir_expressions_in_output_expression(&unary.condition, visitor);
        }
        OutputExpression::Instantiate(inst) => {
            visit_ir_expressions_in_output_expression(&inst.class_expr, visitor);
            for arg in inst.args.iter() {
                visit_ir_expressions_in_output_expression(arg, visitor);
            }
        }
        OutputExpression::ReadProp(prop) => {
            visit_ir_expressions_in_output_expression(&prop.receiver, visitor);
        }
        OutputExpression::ReadKey(keyed) => {
            visit_ir_expressions_in_output_expression(&keyed.receiver, visitor);
            visit_ir_expressions_in_output_expression(&keyed.index, visitor);
        }
        _ => {}
    }
}

/// Result of collecting variables that need to be processed.
struct VariableActions {
    /// Variables that should be completely removed (unused, not side-effectful).
    to_remove: HashSet<XrefId>,
    /// Variables that should be converted to statement ops (unused but side-effectful).
    to_convert: HashSet<XrefId>,
}

/// Collect removable variables from all views, using proper context tracking.
///
/// Per Angular's variable_optimization.ts lines 185-230:
/// - Process ops in reverse order
/// - Track whether view context is used (contextIsUsed)
/// - For unused variables:
///   - If side-effectful OR (ViewContextWrite AND contextIsUsed): convert to statement
///   - Otherwise: remove entirely
fn collect_removable_variables_with_context_tracking(
    job: &ComponentCompilationJob<'_>,
    usage_counts: &HashMap<XrefId, usize>,
) -> VariableActions {
    let mut actions = VariableActions { to_remove: HashSet::new(), to_convert: HashSet::new() };

    // Make a mutable copy of usage counts so we can decrement as we remove
    // Per TypeScript's variable_optimization.ts, when a variable is removed,
    // we call uncountVariableUsages to decrement the counts of variables it references.
    // This allows later processing in the same pass to see the updated counts.
    let mut usage_counts = usage_counts.clone();

    for view in job.all_views() {
        // Collect ops into vectors so we can iterate in reverse
        let update_ops: Vec<_> = view.update.iter().collect();
        let create_ops: Vec<_> = view.create.iter().collect();

        // Per TypeScript's variable_optimization.ts lines 49-50:
        // Create and update ops are optimized SEPARATELY with independent context tracking.
        // The view context is distinct between the create (rf & 1) and update (rf & 2) phases.

        // Process update ops with their own context tracking
        let mut context_is_used = false;
        for op in update_ops.iter().rev() {
            if let UpdateOp::Variable(var) = op {
                let usage_count = usage_counts.get(&var.xref).copied().unwrap_or(0);
                if usage_count == 0 {
                    let fences = collect_fences(&var.initializer);

                    // Per Angular's variable_optimization.ts lines 197-218:
                    // - If it has SideEffectful fence: convert to statement
                    // - If it has ViewContextWrite AND context is used later: convert to statement
                    // - Otherwise: remove entirely
                    //
                    // The `contextIsUsed` flag is set by `ViewContextRead` fences (from reference()
                    // calls). When a reference() appears after a nextContext(), the nextContext()
                    // must be kept as a standalone statement to ensure correct context positioning.
                    let should_keep_as_statement = fences.contains(Fence::SIDE_EFFECTFUL)
                        || (context_is_used && fences.contains(Fence::VIEW_CONTEXT_WRITE));

                    if should_keep_as_statement {
                        actions.to_convert.insert(var.xref);
                    } else {
                        // When removing a variable, decrement usage counts of variables it references
                        // Per TypeScript's uncountVariableUsages
                        uncount_variable_usages_in_expr(&var.initializer, &mut usage_counts);
                        actions.to_remove.insert(var.xref);
                    }
                    // Per TypeScript: skip context_is_used update for unused variables
                    // (they will be removed, so their fences shouldn't affect the tracking)
                    continue;
                }
            }
            // Track if this op reads from view context
            let op_fences = collect_op_fences(op);
            if op_fences.contains(Fence::VIEW_CONTEXT_READ) {
                context_is_used = true;
            }
        }

        // Process create ops with their own SEPARATE context tracking
        // Per TypeScript: optimizeVariablesInOpList is called separately for create and update
        let mut context_is_used = false;
        for op in create_ops.iter().rev() {
            if let CreateOp::Variable(var) = op {
                let usage_count = usage_counts.get(&var.xref).copied().unwrap_or(0);
                if usage_count == 0 {
                    let fences = collect_fences(&var.initializer);

                    // Per Angular's variable_optimization.ts lines 197-218:
                    // Same logic as update ops - keep if side-effectful or context is used.
                    let should_keep_as_statement = fences.contains(Fence::SIDE_EFFECTFUL)
                        || (context_is_used && fences.contains(Fence::VIEW_CONTEXT_WRITE));

                    if should_keep_as_statement {
                        actions.to_convert.insert(var.xref);
                    } else {
                        // When removing a variable, decrement usage counts of variables it references
                        uncount_variable_usages_in_expr(&var.initializer, &mut usage_counts);
                        actions.to_remove.insert(var.xref);
                    }
                    // Per TypeScript: skip context_is_used update for unused variables
                    continue;
                }
            }
            // Track if this op reads from view context
            let op_fences = collect_create_op_fences(op);
            if op_fences.contains(Fence::VIEW_CONTEXT_READ) {
                context_is_used = true;
            }
        }
    }

    actions
}

/// Decrement usage counts for variables referenced by an expression.
/// Per TypeScript's uncountVariableUsages function.
fn uncount_variable_usages_in_expr(
    expr: &IrExpression<'_>,
    usage_counts: &mut HashMap<XrefId, usize>,
) {
    visit_all_expressions(expr, &mut |e| {
        if let IrExpression::ReadVariable(read_var) = e {
            if let Some(count) = usage_counts.get_mut(&read_var.xref) {
                *count = count.saturating_sub(1);
            }
        }
    });
}

/// Optimize variables within arrow function ops.
///
/// Per Angular's variable_optimization.ts lines 53-56:
/// ```typescript
/// for (const expr of unit.functions) {
///   optimizeVariablesInOpList(expr.ops, job.compatibility, null);
///   optimizeSaveRestoreView(expr.ops);
/// }
/// ```
///
/// Arrow functions have their own ops list that needs optimization and
/// save/restore view optimization after variable optimization.
fn optimize_arrow_function_ops<'a>(job: &mut ComponentCompilationJob<'a>) {
    let allocator = job.allocator;

    // Collect view xrefs
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        let view = if view_xref == job.root.xref {
            &mut job.root
        } else if let Some(v) = job.views.get_mut(&view_xref) {
            v.as_mut()
        } else {
            continue;
        };

        // Process each arrow function's ops
        // view.functions contains pointers to ArrowFunctionExpr which have ops: Vec<UpdateOp>
        for func_ptr in view.functions.iter() {
            // SAFETY: These pointers are valid as they point to ArrowFunctionExpr
            // allocated in the allocator and stored in the view's functions vec.
            let func = unsafe { &mut **func_ptr };

            // Step 1: Optimize variables in the arrow function's ops
            // (No handler_expression for arrow functions)
            optimize_handler_ops(&mut func.ops, None, allocator);

            // Step 2: Apply save/restore view optimization
            optimize_save_restore_view(&mut func.ops, allocator);
        }
    }
}

/// Optimize variables within listener handler_ops.
///
/// This is done separately from the main optimization because listener handler_ops
/// form their own scope - variables are declared and used within the handler.
/// Per Angular's variable_optimization.ts, `optimizeVariablesInOpList(op.handlerOps)`
/// is called separately for each listener.
fn optimize_listener_handler_ops<'a>(job: &mut ComponentCompilationJob<'a>) {
    // Get allocator reference
    let allocator = job.allocator;

    // Collect view xrefs
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        let view = if view_xref == job.root.xref {
            &mut job.root
        } else if let Some(v) = job.views.get_mut(&view_xref) {
            v.as_mut()
        } else {
            continue;
        };

        // Process each create op to find listeners
        // Per Angular's variable_optimization.ts lines 58-70:
        // - Listener, Animation, AnimationListener, TwoWayListener are processed
        // - optimizeVariablesInOpList is called followed by optimizeSaveRestoreView
        for op in view.create.iter_mut() {
            match op {
                CreateOp::Listener(listener) => {
                    optimize_handler_ops(
                        &mut listener.handler_ops,
                        listener.handler_expression.as_ref().map(|e| e.as_ref()),
                        allocator,
                    );
                    optimize_save_restore_view(&mut listener.handler_ops, allocator);
                }
                CreateOp::TwoWayListener(listener) => {
                    optimize_handler_ops(&mut listener.handler_ops, None, allocator);
                    optimize_save_restore_view(&mut listener.handler_ops, allocator);
                }
                CreateOp::AnimationListener(listener) => {
                    optimize_handler_ops(&mut listener.handler_ops, None, allocator);
                    optimize_save_restore_view(&mut listener.handler_ops, allocator);
                }
                CreateOp::Animation(animation) => {
                    optimize_handler_ops(&mut animation.handler_ops, None, allocator);
                    // Note: We intentionally do NOT call optimize_save_restore_view on
                    // Animation handler_ops. Angular's ngtsc output keeps restoreView/resetView
                    // in animation callbacks even when the return value doesn't reference the
                    // view context (e.g., `return "animate-in"`). Skipping this optimization
                    // for Animation handlers matches the observed Angular output.
                }
                _ => {}
            }
        }
    }
}

/// Optimize variables within a single handler_ops Vec.
///
/// This implements the core optimization logic for a listener's handler,
/// following Angular's variable_optimization.ts:
/// 1. Build OpInfo (fences + variable usage) for each op
/// 2. Iterate backwards through operations
/// 3. Track whether any operation reads the view context (contextIsUsed)
/// 4. For unused variables:
///    - If SideEffectful OR (ViewContextWrite AND contextIsUsed): convert to statement
///    - Otherwise: remove entirely
/// 5. Repeat until no more changes
/// 6. Inline context variables into other variable ops (per allowConservativeInlining)
fn optimize_handler_ops<'a>(
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    handler_expression: Option<&IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) {
    // Step 1: Remove unused variables (loop until stable)
    loop {
        let changed = optimize_handler_ops_once(handler_ops, handler_expression, allocator);
        if !changed {
            break;
        }
    }

    // Step 2: Inline context variables (nextContext()) into other variable ops.
    // Per TypeScript's allowConservativeInlining (lines 536-538):
    // "Context can only be inlined into other variables."
    inline_context_vars_in_handler_ops(handler_ops, handler_expression, allocator);
}

/// After variables have been optimized in nested ops (e.g. handlers or functions), we may end up
/// with `saveView`/`restoreView` calls that aren't necessary since all the references to the view
/// were optimized away. This function removes the ops related to the view restoration.
///
/// Ported from Angular's `optimizeSaveRestoreView` in `variable_optimization.ts` (lines 575-596).
///
/// We can only optimize if we have exactly two ops:
/// 1. A call to `restoreView` (ExpressionStatement with RestoreViewExpr).
/// 2. A return statement with a `resetView` in it.
///
/// If these conditions are met:
/// - Remove the restoreView call (first op)
/// - Unwrap the resetView in the return statement (replace ResetView(expr) with just expr)
fn optimize_save_restore_view<'a>(
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) {
    // We need exactly 2 ops to optimize
    if handler_ops.len() != 2 {
        return;
    }

    // Check first op: must be a Statement with ExpressionStatement containing RestoreViewExpr
    let first_is_restore_view = matches!(
        &handler_ops[0],
        UpdateOp::Statement(stmt) if matches!(
            &stmt.statement,
            OutputStatement::Expression(expr_stmt) if matches!(
                &expr_stmt.expr,
                OutputExpression::WrappedIrNode(wrapped) if matches!(
                    wrapped.node.as_ref(),
                    IrExpression::RestoreView(_)
                )
            )
        )
    );

    if !first_is_restore_view {
        return;
    }

    // Check second op: must be a Statement with ReturnStatement containing ResetViewExpr
    let second_is_reset_view_return = matches!(
        &handler_ops[1],
        UpdateOp::Statement(stmt) if matches!(
            &stmt.statement,
            OutputStatement::Return(ret_stmt) if matches!(
                &ret_stmt.value,
                OutputExpression::WrappedIrNode(wrapped) if matches!(
                    wrapped.node.as_ref(),
                    IrExpression::ResetView(_)
                )
            )
        )
    );

    if !second_is_reset_view_return {
        return;
    }

    // Both conditions met - apply the optimization
    // 1. Remove the first op (restoreView call)
    // 2. Unwrap the ResetView in the return statement

    // Extract the inner expression from the ResetView in the return statement
    let inner_expr = if let UpdateOp::Statement(stmt) = &handler_ops[1] {
        if let OutputStatement::Return(ret_stmt) = &stmt.statement {
            if let OutputExpression::WrappedIrNode(wrapped) = &ret_stmt.value {
                if let IrExpression::ResetView(reset_view) = wrapped.node.as_ref() {
                    Some(reset_view.expr.clone_in(allocator))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let Some(inner_expr) = inner_expr else {
        return;
    };

    // Create the new return statement with the unwrapped expression
    let new_return_stmt = UpdateOp::Statement(StatementOp {
        base: UpdateOpBase::default(),
        statement: OutputStatement::Return(oxc_allocator::Box::new_in(
            ReturnStatement {
                value: OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
                    WrappedIrExpr {
                        node: oxc_allocator::Box::new_in(inner_expr, allocator),
                        source_span: None,
                    },
                    allocator,
                )),
                source_span: None,
            },
            allocator,
        )),
    });

    // Clear handler_ops and add only the new return statement
    handler_ops.clear();
    handler_ops.push(new_return_stmt);
}

/// Performs one pass of handler_ops optimization.
/// Returns true if any variables were removed or converted to statements.
fn optimize_handler_ops_once<'a>(
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    handler_expression: Option<&IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) -> bool {
    // Build OpInfo for each operation: fences and variable usage
    // We need indices to track operations
    let mut op_fences: Vec<Fence> = Vec::with_capacity(handler_ops.len());
    let mut var_decls: HashMap<XrefId, usize> = HashMap::new(); // xref -> index in handler_ops
    let mut var_usages: HashMap<XrefId, usize> = HashMap::new();

    // First pass: collect variable declarations and calculate fences
    for (idx, op) in handler_ops.iter().enumerate() {
        let fences = collect_op_fences(op);
        op_fences.push(fences);

        if let UpdateOp::Variable(var) = op {
            var_decls.insert(var.xref, idx);
            var_usages.insert(var.xref, 0);
        }
    }

    // Count usages in handler_ops
    for op in handler_ops.iter() {
        count_in_update_op(op, &mut var_usages);
    }

    // Count usages in handler_expression (the return value)
    if let Some(expr) = handler_expression {
        count_in_expression(expr, &mut var_usages);
    }

    // Process operations in reverse order (following Angular's approach)
    // Track whether we've seen an operation that reads from the view context.
    //
    // IMPORTANT: In Angular's TypeScript implementation (variable_optimization.ts line 211),
    // `contextIsUsed` is initialized to `false`. The return statement is part of handler_ops
    // as a `StatementOp`, so it gets processed FIRST during the reverse iteration (since it's
    // at the end of handler_ops). In Oxc, the return value is stored separately in
    // `handler_expression`, so we need to process it first to match Angular's behavior.
    //
    // Key insight: The return statement typically contains `ResetView(...)` which has NO fence
    // in Angular's `fencesForIrExpression`. Variable reads (ReadVariableExpr) also have no fence.
    // So the return statement usually has Fence::None, meaning it doesn't set contextIsUsed.
    //
    // We must "process" handler_expression first (simulating it being at the end of handler_ops)
    // to ensure correct fence tracking. This affects whether unused NextContext variables that
    // come before the return should be removed (contextIsUsed=false) or kept as statements
    // (contextIsUsed=true because a later Reference read from the context).
    let mut context_is_used = if let Some(expr) = handler_expression {
        // Check if handler_expression contains any VIEW_CONTEXT_READ fence
        // This simulates processing a StatementOp(Return(expr)) in the reverse iteration
        let fences = collect_fences(expr);
        fences.contains(Fence::VIEW_CONTEXT_READ)
    } else {
        false
    };

    // Collect actions to take (we can't mutate while iterating)
    #[derive(Debug, Clone)]
    enum Action {
        Remove,
        ConvertToStatement,
        Keep,
    }
    let mut actions: Vec<Action> = vec![Action::Keep; handler_ops.len()];

    // Iterate in reverse
    for idx in (0..handler_ops.len()).rev() {
        let op = &handler_ops[idx];
        let op_fence = op_fences[idx];

        if let UpdateOp::Variable(var) = op {
            let usage_count = var_usages.get(&var.xref).copied().unwrap_or(0);

            if usage_count == 0 {
                // This variable is unused and can potentially be removed
                // Check if we need to keep the initializer for side effects
                let should_keep_as_statement = op_fence.contains(Fence::SIDE_EFFECTFUL)
                    || (context_is_used && op_fence.contains(Fence::VIEW_CONTEXT_WRITE));

                if should_keep_as_statement {
                    // Convert to statement - keep the side effect but remove the variable
                    actions[idx] = Action::ConvertToStatement;
                } else {
                    // Safe to remove entirely.
                    // Per TypeScript's variable_optimization.ts (line 240):
                    // uncountVariableUsages(op, varUsages) is called before removing.
                    // This decrements usage counts for variables referenced in the
                    // removed variable's initializer, allowing cascading unused
                    // detection in this single backwards pass.
                    uncount_variable_usages_in_expr(&var.initializer, &mut var_usages);
                    actions[idx] = Action::Remove;
                }
                // Per TypeScript's variable_optimization.ts (line 223): after handling an unused
                // variable, we skip the contextIsUsed update. Variables being removed should not
                // affect the contextIsUsed tracking for earlier variables.
                continue;
            }
        }

        // Update context_is_used if this operation reads the view context
        if op_fence.contains(Fence::VIEW_CONTEXT_READ) {
            context_is_used = true;
        }
    }

    // Check if there are any changes to make
    let has_changes =
        actions.iter().any(|a| matches!(a, Action::Remove | Action::ConvertToStatement));

    if !has_changes {
        return false;
    }

    // Apply the actions
    // We need to rebuild the handler_ops vector since we can't easily mutate in place
    // and also need to replace some ops with statement ops
    let mut new_ops: OxcVec<'a, UpdateOp<'a>> =
        OxcVec::with_capacity_in(handler_ops.len(), allocator);

    for (idx, op) in handler_ops.iter().enumerate() {
        match &actions[idx] {
            Action::Keep => {
                // Clone the op - we need to move it to the new vec
                // Since we can't clone UpdateOp directly, we'll need a different approach
                // For now, we'll use unsafe to avoid the clone
            }
            Action::Remove => {
                // Skip this op
                continue;
            }
            Action::ConvertToStatement => {
                // Convert variable to expression statement
                if let UpdateOp::Variable(var) = op {
                    // Create an expression statement with the initializer wrapped
                    let wrapped = WrappedIrExpr {
                        node: oxc_allocator::Box::new_in(
                            var.initializer.clone_in(allocator),
                            allocator,
                        ),
                        source_span: None,
                    };
                    let expr_stmt = OutputStatement::Expression(oxc_allocator::Box::new_in(
                        ExpressionStatement {
                            expr: OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
                                wrapped, allocator,
                            )),
                            source_span: None,
                        },
                        allocator,
                    ));
                    new_ops.push(UpdateOp::Statement(StatementOp {
                        base: UpdateOpBase::default(),
                        statement: expr_stmt,
                    }));
                    continue;
                }
            }
        }
    }

    // Now we need to properly handle the "Keep" case
    // Since we already pushed ConvertToStatement cases, we need to handle Keep
    // Let's rewrite this more carefully

    // Build indices for removal and conversion to statements
    let mut convert_indices: HashSet<usize> = HashSet::new();
    let mut remove_indices: HashSet<usize> = HashSet::new();

    for (idx, action) in actions.iter().enumerate() {
        match action {
            Action::ConvertToStatement => {
                convert_indices.insert(idx);
            }
            Action::Remove => {
                remove_indices.insert(idx);
            }
            Action::Keep => {}
        }
    }

    // Create statement ops for converted variables
    let mut statement_replacements: HashMap<usize, UpdateOp<'a>> = HashMap::new();
    for idx in &convert_indices {
        if let UpdateOp::Variable(var) = &handler_ops[*idx] {
            let wrapped = WrappedIrExpr {
                node: oxc_allocator::Box::new_in(var.initializer.clone_in(allocator), allocator),
                source_span: None,
            };
            let expr_stmt = OutputStatement::Expression(oxc_allocator::Box::new_in(
                ExpressionStatement {
                    expr: OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
                        wrapped, allocator,
                    )),
                    source_span: None,
                },
                allocator,
            ));
            statement_replacements.insert(
                *idx,
                UpdateOp::Statement(StatementOp {
                    base: UpdateOpBase::default(),
                    statement: expr_stmt,
                }),
            );
        }
    }

    // Now rebuild the ops vector
    let mut result_ops: OxcVec<'a, UpdateOp<'a>> =
        OxcVec::with_capacity_in(handler_ops.len(), allocator);

    for (idx, op) in handler_ops.iter().enumerate() {
        if remove_indices.contains(&idx) {
            continue;
        }
        if let Some(replacement) = statement_replacements.remove(&idx) {
            result_ops.push(replacement);
        } else {
            // Clone the original op
            result_ops.push(clone_update_op(op, allocator));
        }
    }

    // Replace the original handler_ops
    *handler_ops = result_ops;

    true
}

/// Clone an UpdateOp, focusing on the types we care about in handler_ops.
fn clone_update_op<'a>(op: &UpdateOp<'a>, allocator: &'a oxc_allocator::Allocator) -> UpdateOp<'a> {
    match op {
        UpdateOp::Variable(var) => UpdateOp::Variable(UpdateVariableOp {
            base: UpdateOpBase::default(),
            xref: var.xref,
            kind: var.kind,
            name: var.name.clone(),
            initializer: oxc_allocator::Box::new_in(var.initializer.clone_in(allocator), allocator),
            flags: var.flags,
            view: var.view,
            local: var.local,
        }),
        UpdateOp::Statement(stmt) => UpdateOp::Statement(StatementOp {
            base: UpdateOpBase::default(),
            statement: clone_statement_with_ir_nodes(&stmt.statement, allocator),
        }),
        // For other op types that might appear in handler_ops, we'd need to clone them too
        // For now, these are the main ones we care about
        _ => {
            // This shouldn't happen in practice for handler_ops
            // but we need to handle it somehow
            panic!("Unexpected op type in handler_ops during clone")
        }
    }
}

/// Clone an OutputStatement, handling WrappedIrNode expressions specially.
/// Unlike `clone_output_statement`, this version can handle WrappedIrNode by cloning
/// the underlying IR expression.
fn clone_statement_with_ir_nodes<'a>(
    stmt: &OutputStatement<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            OutputStatement::Expression(oxc_allocator::Box::new_in(
                ExpressionStatement {
                    expr: clone_expr_with_ir_nodes(&expr_stmt.expr, allocator),
                    source_span: expr_stmt.source_span,
                },
                allocator,
            ))
        }
        OutputStatement::Return(ret_stmt) => OutputStatement::Return(oxc_allocator::Box::new_in(
            crate::output::ast::ReturnStatement {
                value: clone_expr_with_ir_nodes(&ret_stmt.value, allocator),
                source_span: ret_stmt.source_span,
            },
            allocator,
        )),
        // For other statement types, use the standard clone
        _ => crate::output::ast::clone_output_statement(stmt, allocator),
    }
}

/// Clone an OutputExpression, handling WrappedIrNode specially.
fn clone_expr_with_ir_nodes<'a>(
    expr: &OutputExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> OutputExpression<'a> {
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            // Clone the wrapped IR expression
            OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
                WrappedIrExpr {
                    node: oxc_allocator::Box::new_in(wrapped.node.clone_in(allocator), allocator),
                    source_span: wrapped.source_span,
                },
                allocator,
            ))
        }
        // For other expression types, use the standard clone
        _ => expr.clone_in(allocator),
    }
}

/// Count how many times each variable is referenced across all views.
fn count_variable_usages(job: &ComponentCompilationJob<'_>) -> HashMap<XrefId, usize> {
    let mut counts: HashMap<XrefId, usize> = HashMap::new();

    for view in job.all_views() {
        // Count usages in create ops
        for op in view.create.iter() {
            count_in_create_op(op, &mut counts);
        }

        // Count usages in update ops
        for op in view.update.iter() {
            count_in_update_op(op, &mut counts);
        }
    }

    counts
}

/// Count variable usages in a create operation.
fn count_in_create_op(op: &CreateOp<'_>, counts: &mut HashMap<XrefId, usize>) {
    match op {
        CreateOp::Variable(var) => {
            count_in_expression(&var.initializer, counts);
        }
        CreateOp::Listener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                count_in_update_op(handler_op, counts);
            }
            // Also count usages in handler_expression (the return value expression)
            if let Some(ref handler_expr) = listener.handler_expression {
                count_in_expression(handler_expr, counts);
            }
        }
        CreateOp::TwoWayListener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                count_in_update_op(handler_op, counts);
            }
        }
        CreateOp::AnimationListener(listener) => {
            for handler_op in listener.handler_ops.iter() {
                count_in_update_op(handler_op, counts);
            }
        }
        CreateOp::Animation(animation) => {
            for handler_op in animation.handler_ops.iter() {
                count_in_update_op(handler_op, counts);
            }
        }
        CreateOp::Conditional(_cond) => {
            // ConditionalOp (CREATE) no longer has test/branches/processed_expression.
            // Those are now on ConditionalUpdateOp (UPDATE) and handled in count_in_update_op.
        }
        CreateOp::RepeaterCreate(rep) => {
            count_in_expression(&rep.track, counts);
        }
        CreateOp::ExtractedAttribute(attr) => {
            if let Some(ref value) = attr.value {
                count_in_expression(value, counts);
            }
        }
        CreateOp::DeferOn(defer_on) => {
            if let Some(ref options) = defer_on.options {
                count_in_expression(options, counts);
            }
        }
        _ => {}
    }
}

/// Count variable usages in an update operation.
fn count_in_update_op(op: &UpdateOp<'_>, counts: &mut HashMap<XrefId, usize>) {
    match op {
        UpdateOp::Property(prop) => count_in_expression(&prop.expression, counts),
        UpdateOp::StyleProp(style) => count_in_expression(&style.expression, counts),
        UpdateOp::ClassProp(class) => count_in_expression(&class.expression, counts),
        UpdateOp::StyleMap(style) => count_in_expression(&style.expression, counts),
        UpdateOp::ClassMap(class) => count_in_expression(&class.expression, counts),
        UpdateOp::Attribute(attr) => count_in_expression(&attr.expression, counts),
        UpdateOp::DomProperty(dom) => count_in_expression(&dom.expression, counts),
        UpdateOp::TwoWayProperty(two_way) => count_in_expression(&two_way.expression, counts),
        UpdateOp::Binding(binding) => count_in_expression(&binding.expression, counts),
        UpdateOp::InterpolateText(text) => count_in_expression(&text.interpolation, counts),
        UpdateOp::Variable(var) => count_in_expression(&var.initializer, counts),
        UpdateOp::StoreLet(store) => count_in_expression(&store.value, counts),
        UpdateOp::Conditional(cond) => {
            // The conditionals phase creates a `processed` expression that is the final
            // collapsed form containing all branch logic. If `processed` is set, we should
            // only count usages in it (not in test/conditions), because `processed` incorporates
            // the test expression and branch conditions.
            //
            // For example, @switch(data.message.type) sets:
            // - test = data.message.type
            // - processed = ((tmp = data.message.type) === case1) ? slot1 : ...
            //
            // If we count both, we'd double-count the usage of `data`.
            if let Some(ref processed) = cond.processed {
                count_in_expression(processed, counts);
            } else {
                // Only count test/conditions if processed is not set
                if let Some(ref test) = cond.test {
                    count_in_expression(test, counts);
                }
                for condition in cond.conditions.iter() {
                    if let Some(ref expr) = condition.expr {
                        count_in_expression(expr, counts);
                    }
                }
            }
            if let Some(ref ctx_val) = cond.context_value {
                count_in_expression(ctx_val, counts);
            }
        }
        UpdateOp::Repeater(rep) => count_in_expression(&rep.collection, counts),
        UpdateOp::AnimationBinding(anim) => count_in_expression(&anim.expression, counts),
        UpdateOp::Control(ctrl) => count_in_expression(&ctrl.expression, counts),
        UpdateOp::I18nExpression(i18n) => count_in_expression(&i18n.expression, counts),
        UpdateOp::DeferWhen(defer_when) => count_in_expression(&defer_when.condition, counts),
        UpdateOp::Statement(stmt) => {
            // Statement ops may contain WrappedIrNode with IR expressions
            // These expressions can contain ReadVariable references that need to be counted
            count_in_output_statement(&stmt.statement, counts);
        }
        _ => {}
    }
}

/// Count variable usages in an expression.
fn count_in_expression(expr: &IrExpression<'_>, counts: &mut HashMap<XrefId, usize>) {
    // Collect xrefs first, then update counts
    let mut xrefs_found: Vec<XrefId> = Vec::new();
    collect_variable_xrefs(expr, &mut xrefs_found);
    for xref in xrefs_found {
        *counts.entry(xref).or_insert(0) += 1;
    }
}

/// Count variable usages in an output statement.
/// This handles Statement ops that contain WrappedIrNode expressions.
fn count_in_output_statement(stmt: &OutputStatement<'_>, counts: &mut HashMap<XrefId, usize>) {
    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            count_in_output_expression(&expr_stmt.expr, counts);
        }
        OutputStatement::If(if_stmt) => {
            count_in_output_expression(&if_stmt.condition, counts);
            for s in if_stmt.true_case.iter() {
                count_in_output_statement(s, counts);
            }
            for s in if_stmt.false_case.iter() {
                count_in_output_statement(s, counts);
            }
        }
        OutputStatement::Return(ret) => {
            count_in_output_expression(&ret.value, counts);
        }
        _ => {}
    }
}

/// Count variable usages in an output expression.
fn count_in_output_expression(expr: &OutputExpression<'_>, counts: &mut HashMap<XrefId, usize>) {
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            count_in_expression(&wrapped.node, counts);
        }
        OutputExpression::BinaryOperator(bin) => {
            count_in_output_expression(&bin.lhs, counts);
            count_in_output_expression(&bin.rhs, counts);
        }
        OutputExpression::Conditional(cond) => {
            count_in_output_expression(&cond.condition, counts);
            count_in_output_expression(&cond.true_case, counts);
            if let Some(ref false_case) = cond.false_case {
                count_in_output_expression(false_case, counts);
            }
        }
        OutputExpression::InvokeFunction(call) => {
            count_in_output_expression(&call.fn_expr, counts);
            for arg in call.args.iter() {
                count_in_output_expression(arg, counts);
            }
        }
        OutputExpression::Not(unary) => {
            count_in_output_expression(&unary.condition, counts);
        }
        OutputExpression::Instantiate(inst) => {
            count_in_output_expression(&inst.class_expr, counts);
            for arg in inst.args.iter() {
                count_in_output_expression(arg, counts);
            }
        }
        OutputExpression::ReadProp(prop) => {
            count_in_output_expression(&prop.receiver, counts);
        }
        OutputExpression::ReadKey(keyed) => {
            count_in_output_expression(&keyed.receiver, counts);
            count_in_output_expression(&keyed.index, counts);
        }
        OutputExpression::LiteralArray(arr) => {
            for elem in arr.entries.iter() {
                count_in_output_expression(elem, counts);
            }
        }
        OutputExpression::LiteralMap(map) => {
            for entry in map.entries.iter() {
                count_in_output_expression(&entry.value, counts);
            }
        }
        OutputExpression::Comma(comma) => {
            for part in comma.parts.iter() {
                count_in_output_expression(part, counts);
            }
        }
        OutputExpression::UnaryOperator(unary) => {
            count_in_output_expression(&unary.expr, counts);
        }
        OutputExpression::Parenthesized(paren) => {
            count_in_output_expression(&paren.expr, counts);
        }
        _ => {}
    }
}

/// Recursively collect variable xrefs from an expression.
fn collect_variable_xrefs(expr: &IrExpression<'_>, xrefs: &mut Vec<XrefId>) {
    match expr {
        IrExpression::ReadVariable(read_var) => {
            xrefs.push(read_var.xref);
        }
        IrExpression::PureFunction(pf) => {
            if let Some(ref body) = pf.body {
                collect_variable_xrefs(body, xrefs);
            }
            if let Some(ref fn_ref) = pf.fn_ref {
                collect_variable_xrefs(fn_ref, xrefs);
            }
            for arg in pf.args.iter() {
                collect_variable_xrefs(arg, xrefs);
            }
        }
        IrExpression::Interpolation(interp) => {
            for e in interp.expressions.iter() {
                collect_variable_xrefs(e, xrefs);
            }
        }
        IrExpression::RestoreView(rv) => {
            if let crate::ir::expression::RestoreViewTarget::Dynamic(ref inner) = rv.view {
                collect_variable_xrefs(inner, xrefs);
            }
        }
        IrExpression::ResetView(rv) => {
            collect_variable_xrefs(&rv.expr, xrefs);
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter() {
                collect_variable_xrefs(arg, xrefs);
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            collect_variable_xrefs(&pbv.args, xrefs);
        }
        IrExpression::SafePropertyRead(spr) => {
            collect_variable_xrefs(&spr.receiver, xrefs);
        }
        IrExpression::SafeKeyedRead(skr) => {
            collect_variable_xrefs(&skr.receiver, xrefs);
            collect_variable_xrefs(&skr.index, xrefs);
        }
        IrExpression::SafeInvokeFunction(sif) => {
            collect_variable_xrefs(&sif.receiver, xrefs);
            for arg in sif.args.iter() {
                collect_variable_xrefs(arg, xrefs);
            }
        }
        IrExpression::SafeTernary(st) => {
            collect_variable_xrefs(&st.guard, xrefs);
            collect_variable_xrefs(&st.expr, xrefs);
        }
        IrExpression::Ternary(t) => {
            collect_variable_xrefs(&t.condition, xrefs);
            collect_variable_xrefs(&t.true_expr, xrefs);
            collect_variable_xrefs(&t.false_expr, xrefs);
        }
        IrExpression::AssignTemporary(at) => {
            collect_variable_xrefs(&at.expr, xrefs);
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref e) = cc.expr {
                collect_variable_xrefs(e, xrefs);
            }
        }
        IrExpression::ConstCollected(cc) => {
            collect_variable_xrefs(&cc.expr, xrefs);
        }
        IrExpression::TwoWayBindingSet(twb) => {
            collect_variable_xrefs(&twb.target, xrefs);
            collect_variable_xrefs(&twb.value, xrefs);
        }
        IrExpression::StoreLet(sl) => {
            collect_variable_xrefs(&sl.value, xrefs);
        }
        IrExpression::Binary(binary) => {
            collect_variable_xrefs(&binary.lhs, xrefs);
            collect_variable_xrefs(&binary.rhs, xrefs);
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            collect_variable_xrefs(&rpr.receiver, xrefs);
        }
        IrExpression::ResolvedBinary(rb) => {
            collect_variable_xrefs(&rb.left, xrefs);
            collect_variable_xrefs(&rb.right, xrefs);
        }
        IrExpression::ResolvedCall(rc) => {
            collect_variable_xrefs(&rc.receiver, xrefs);
            for arg in rc.args.iter() {
                collect_variable_xrefs(arg, xrefs);
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            collect_variable_xrefs(&rkr.receiver, xrefs);
            collect_variable_xrefs(&rkr.key, xrefs);
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            collect_variable_xrefs(&rspr.receiver, xrefs);
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter() {
                collect_variable_xrefs(entry, xrefs);
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter() {
                collect_variable_xrefs(value, xrefs);
            }
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter() {
                collect_variable_xrefs(elem, xrefs);
            }
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter() {
                collect_variable_xrefs(value, xrefs);
            }
        }
        // Leaf expressions - no sub-expressions to recurse into
        IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_) => {}
        IrExpression::Not(n) => {
            collect_variable_xrefs(&n.expr, xrefs);
        }
        IrExpression::Unary(u) => {
            collect_variable_xrefs(&u.expr, xrefs);
        }
        IrExpression::Typeof(t) => {
            collect_variable_xrefs(&t.expr, xrefs);
        }
        IrExpression::Void(v) => {
            collect_variable_xrefs(&v.expr, xrefs);
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter() {
                collect_variable_xrefs(e, xrefs);
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            collect_variable_xrefs(&arrow_fn.body, xrefs);
        }
        IrExpression::Parenthesized(paren) => {
            collect_variable_xrefs(&paren.expr, xrefs);
        }
    }
}

/// Convert unused side-effectful variables to statement ops.
///
/// Per Angular's variable_optimization.ts lines 207-209:
/// When a variable is unused but has side effects (like StoreLet), we replace
/// the variable declaration with an expression statement that just calls the
/// initializer for its side effect.
fn convert_variables_to_statements(
    job: &mut ComponentCompilationJob<'_>,
    to_convert: &HashSet<XrefId>,
) {
    if to_convert.is_empty() {
        return;
    }

    let allocator = job.allocator;

    // Collect view xrefs to process
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        // We need to use an iterator that allows replacement
        let view = if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(view_xref) };

        if let Some(view) = view {
            // Process update ops - use cursor to allow replacement
            let mut cursor = view.update.cursor();
            while cursor.move_next() {
                if let Some(UpdateOp::Variable(var)) = cursor.current() {
                    if to_convert.contains(&var.xref) {
                        // Create a statement op from the variable's initializer
                        let wrapped = WrappedIrExpr {
                            node: oxc_allocator::Box::new_in(
                                var.initializer.clone_in(allocator),
                                allocator,
                            ),
                            source_span: None,
                        };
                        let expr_stmt = OutputStatement::Expression(oxc_allocator::Box::new_in(
                            ExpressionStatement {
                                expr: OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
                                    wrapped, allocator,
                                )),
                                source_span: None,
                            },
                            allocator,
                        ));
                        let statement_op = UpdateOp::Statement(StatementOp {
                            base: UpdateOpBase::default(),
                            statement: expr_stmt,
                        });
                        cursor.replace_current(statement_op);
                    }
                }
            }

            // Process create ops - need to check if there are variables to convert there
            // Note: In practice, side-effectful variables like StoreLet appear in update ops,
            // but we handle create ops for completeness (though they use CreateStatementOp)
        }
    }
}

/// Remove unused variables from all views.
fn remove_unused_variables(job: &mut ComponentCompilationJob<'_>, to_remove: &HashSet<XrefId>) {
    // Collect view xrefs to process
    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    for view_xref in view_xrefs {
        // Collect ops to remove from create list
        let mut create_ops_to_remove: Vec<NonNull<CreateOp<'_>>> = Vec::new();
        // Collect ops to remove from update list
        let mut update_ops_to_remove: Vec<NonNull<UpdateOp<'_>>> = Vec::new();

        {
            let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

            if let Some(view) = view {
                // Find variable create ops to remove
                for op in view.create.iter() {
                    if let CreateOp::Variable(var) = op {
                        if to_remove.contains(&var.xref) {
                            create_ops_to_remove.push(NonNull::from(op));
                        }
                    }
                }

                // Find variable update ops to remove
                for op in view.update.iter() {
                    if let UpdateOp::Variable(var) = op {
                        if to_remove.contains(&var.xref) {
                            update_ops_to_remove.push(NonNull::from(op));
                        }
                    }
                }
            }
        }

        // Now actually remove them
        {
            let view = if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(view_xref) };

            if let Some(view) = view {
                for op_ptr in create_ops_to_remove {
                    // SAFETY: The pointer came from the list and we're removing it
                    unsafe {
                        view.create.remove(op_ptr);
                    }
                }

                for op_ptr in update_ops_to_remove {
                    // SAFETY: The pointer came from the list and we're removing it
                    unsafe {
                        view.update.remove(op_ptr);
                    }
                }
            }
        }
    }
}

/// Optimizes variables for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn optimize_variables_for_host(_job: &mut HostBindingCompilationJob<'_>) {
    // Host bindings don't have template variables that can be optimized
    // This is a no-op for host bindings
}

// ============================================================================
// Expression Transformation Helpers
// ============================================================================

/// Transform expressions in an UpdateOp.
/// The transform function receives an expression and returns Some(new_expr) to replace it,
/// or None to keep the original.
fn transform_expressions_in_update_op<'a, F>(
    op: &mut UpdateOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    mut transform: F,
) where
    F: FnMut(&IrExpression<'a>) -> Option<IrExpression<'a>>,
{
    match op {
        UpdateOp::Variable(var) => {
            let new_expr = transform_expression(&var.initializer, allocator, &mut transform);
            *var.initializer = new_expr;
        }
        UpdateOp::Property(prop) => {
            let new_expr = transform_expression(&prop.expression, allocator, &mut transform);
            *prop.expression = new_expr;
        }
        UpdateOp::StyleProp(style) => {
            let new_expr = transform_expression(&style.expression, allocator, &mut transform);
            *style.expression = new_expr;
        }
        UpdateOp::ClassProp(class) => {
            let new_expr = transform_expression(&class.expression, allocator, &mut transform);
            *class.expression = new_expr;
        }
        UpdateOp::StyleMap(style) => {
            let new_expr = transform_expression(&style.expression, allocator, &mut transform);
            *style.expression = new_expr;
        }
        UpdateOp::ClassMap(class) => {
            let new_expr = transform_expression(&class.expression, allocator, &mut transform);
            *class.expression = new_expr;
        }
        UpdateOp::Attribute(attr) => {
            let new_expr = transform_expression(&attr.expression, allocator, &mut transform);
            *attr.expression = new_expr;
        }
        UpdateOp::DomProperty(dom) => {
            let new_expr = transform_expression(&dom.expression, allocator, &mut transform);
            *dom.expression = new_expr;
        }
        UpdateOp::TwoWayProperty(two_way) => {
            let new_expr = transform_expression(&two_way.expression, allocator, &mut transform);
            *two_way.expression = new_expr;
        }
        UpdateOp::Binding(binding) => {
            let new_expr = transform_expression(&binding.expression, allocator, &mut transform);
            *binding.expression = new_expr;
        }
        UpdateOp::InterpolateText(text) => {
            let new_expr = transform_expression(&text.interpolation, allocator, &mut transform);
            *text.interpolation = new_expr;
        }
        UpdateOp::StoreLet(store) => {
            let new_expr = transform_expression(&store.value, allocator, &mut transform);
            *store.value = new_expr;
        }
        UpdateOp::Conditional(cond) => {
            if let Some(ref test) = cond.test {
                let new_expr = transform_expression(test, allocator, &mut transform);
                cond.test = Some(OxcBox::new_in(new_expr, allocator));
            }
            for condition in cond.conditions.iter_mut() {
                if let Some(ref expr) = condition.expr {
                    let new_expr = transform_expression(expr, allocator, &mut transform);
                    condition.expr = Some(OxcBox::new_in(new_expr, allocator));
                }
            }
            if let Some(ref processed) = cond.processed {
                let new_expr = transform_expression(processed, allocator, &mut transform);
                cond.processed = Some(OxcBox::new_in(new_expr, allocator));
            }
            if let Some(ref ctx_val) = cond.context_value {
                let new_expr = transform_expression(ctx_val, allocator, &mut transform);
                cond.context_value = Some(OxcBox::new_in(new_expr, allocator));
            }
        }
        UpdateOp::Repeater(rep) => {
            let new_expr = transform_expression(&rep.collection, allocator, &mut transform);
            *rep.collection = new_expr;
        }
        UpdateOp::AnimationBinding(anim) => {
            let new_expr = transform_expression(&anim.expression, allocator, &mut transform);
            *anim.expression = new_expr;
        }
        UpdateOp::Control(ctrl) => {
            let new_expr = transform_expression(&ctrl.expression, allocator, &mut transform);
            *ctrl.expression = new_expr;
        }
        UpdateOp::I18nExpression(i18n) => {
            let new_expr = transform_expression(&i18n.expression, allocator, &mut transform);
            *i18n.expression = new_expr;
        }
        UpdateOp::DeferWhen(defer_when) => {
            let new_expr = transform_expression(&defer_when.condition, allocator, &mut transform);
            *defer_when.condition = new_expr;
        }
        UpdateOp::Statement(stmt) => {
            // Statement ops may contain WrappedIrNode with IR expressions
            // (e.g., TwoWayBindingSetExpr in two-way listener handlers)
            transform_expressions_in_output_statement(
                &mut stmt.statement,
                allocator,
                &mut transform,
            );
        }
        UpdateOp::ListEnd(_) | UpdateOp::Advance(_) | UpdateOp::I18nApply(_) => {
            // These ops don't have expressions to transform
        }
    }
}

/// Transform expressions inside an OutputStatement.
/// This handles WrappedIrNode expressions that contain IR expressions.
fn transform_expressions_in_output_statement<'a, F>(
    stmt: &mut crate::output::ast::OutputStatement<'a>,
    allocator: &'a oxc_allocator::Allocator,
    transform: &mut F,
) where
    F: FnMut(&IrExpression<'a>) -> Option<IrExpression<'a>>,
{
    use crate::output::ast::OutputStatement;

    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            transform_expressions_in_output_expression(&mut expr_stmt.expr, allocator, transform);
        }
        OutputStatement::Return(ret_stmt) => {
            transform_expressions_in_output_expression(&mut ret_stmt.value, allocator, transform);
        }
        OutputStatement::DeclareVar(decl) => {
            if let Some(ref mut value) = decl.value {
                transform_expressions_in_output_expression(value, allocator, transform);
            }
        }
        OutputStatement::If(if_stmt) => {
            transform_expressions_in_output_expression(
                &mut if_stmt.condition,
                allocator,
                transform,
            );
            for stmt in if_stmt.true_case.iter_mut() {
                transform_expressions_in_output_statement(stmt, allocator, transform);
            }
            for stmt in if_stmt.false_case.iter_mut() {
                transform_expressions_in_output_statement(stmt, allocator, transform);
            }
        }
        OutputStatement::DeclareFunction(_) => {
            // Function declarations don't contain IrExpressions to transform
        }
    }
}

/// Transform expressions inside an OutputExpression.
/// This handles WrappedIrNode which contains IR expressions.
fn transform_expressions_in_output_expression<'a, F>(
    expr: &mut crate::output::ast::OutputExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    transform: &mut F,
) where
    F: FnMut(&IrExpression<'a>) -> Option<IrExpression<'a>>,
{
    use crate::output::ast::OutputExpression;

    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            // This is the key case: transform the wrapped IR expression
            let new_expr = transform_expression(&wrapped.node, allocator, transform);
            *wrapped.node = new_expr;
        }
        OutputExpression::Conditional(cond) => {
            transform_expressions_in_output_expression(&mut cond.condition, allocator, transform);
            transform_expressions_in_output_expression(&mut cond.true_case, allocator, transform);
            if let Some(ref mut false_case) = cond.false_case {
                transform_expressions_in_output_expression(false_case, allocator, transform);
            }
        }
        OutputExpression::BinaryOperator(bin) => {
            transform_expressions_in_output_expression(&mut bin.lhs, allocator, transform);
            transform_expressions_in_output_expression(&mut bin.rhs, allocator, transform);
        }
        OutputExpression::UnaryOperator(un) => {
            transform_expressions_in_output_expression(&mut un.expr, allocator, transform);
        }
        OutputExpression::Not(not) => {
            transform_expressions_in_output_expression(&mut not.condition, allocator, transform);
        }
        OutputExpression::ReadProp(member) => {
            transform_expressions_in_output_expression(&mut member.receiver, allocator, transform);
        }
        OutputExpression::ReadKey(idx) => {
            transform_expressions_in_output_expression(&mut idx.receiver, allocator, transform);
            transform_expressions_in_output_expression(&mut idx.index, allocator, transform);
        }
        OutputExpression::InvokeFunction(call) => {
            transform_expressions_in_output_expression(&mut call.fn_expr, allocator, transform);
            for arg in call.args.iter_mut() {
                transform_expressions_in_output_expression(arg, allocator, transform);
            }
        }
        OutputExpression::Instantiate(inst) => {
            transform_expressions_in_output_expression(&mut inst.class_expr, allocator, transform);
            for arg in inst.args.iter_mut() {
                transform_expressions_in_output_expression(arg, allocator, transform);
            }
        }
        OutputExpression::ArrowFunction(arrow) => match &mut arrow.body {
            crate::output::ast::ArrowFunctionBody::Expression(body_expr) => {
                transform_expressions_in_output_expression(body_expr, allocator, transform);
            }
            crate::output::ast::ArrowFunctionBody::Statements(stmts) => {
                for stmt in stmts.iter_mut() {
                    transform_expressions_in_output_statement(stmt, allocator, transform);
                }
            }
        },
        OutputExpression::Function(func) => {
            for stmt in func.statements.iter_mut() {
                transform_expressions_in_output_statement(stmt, allocator, transform);
            }
        }
        OutputExpression::LiteralArray(arr) => {
            for elem in arr.entries.iter_mut() {
                transform_expressions_in_output_expression(elem, allocator, transform);
            }
        }
        OutputExpression::LiteralMap(map) => {
            for entry in map.entries.iter_mut() {
                transform_expressions_in_output_expression(&mut entry.value, allocator, transform);
            }
        }
        OutputExpression::Parenthesized(paren) => {
            transform_expressions_in_output_expression(&mut paren.expr, allocator, transform);
        }
        OutputExpression::Comma(comma) => {
            for expr in comma.parts.iter_mut() {
                transform_expressions_in_output_expression(expr, allocator, transform);
            }
        }
        OutputExpression::TaggedTemplateLiteral(tagged) => {
            transform_expressions_in_output_expression(&mut tagged.tag, allocator, transform);
            for expr in tagged.template.expressions.iter_mut() {
                transform_expressions_in_output_expression(expr, allocator, transform);
            }
        }
        OutputExpression::TemplateLiteral(template) => {
            for expr in template.expressions.iter_mut() {
                transform_expressions_in_output_expression(expr, allocator, transform);
            }
        }
        OutputExpression::Typeof(type_of) => {
            transform_expressions_in_output_expression(&mut type_of.expr, allocator, transform);
        }
        OutputExpression::Void(void_expr) => {
            transform_expressions_in_output_expression(&mut void_expr.expr, allocator, transform);
        }
        OutputExpression::SpreadElement(spread) => {
            transform_expressions_in_output_expression(&mut spread.expr, allocator, transform);
        }
        // Leaf expressions with no sub-expressions to transform
        OutputExpression::Literal(_)
        | OutputExpression::ReadVar(_)
        | OutputExpression::External(_)
        | OutputExpression::LocalizedString(_)
        | OutputExpression::WrappedNode(_)
        | OutputExpression::DynamicImport(_)
        | OutputExpression::RegularExpressionLiteral(_) => {}
    }
}

/// Transform expressions in a CreateOp.
fn transform_expressions_in_create_op<'a, F>(
    op: &mut CreateOp<'a>,
    allocator: &'a oxc_allocator::Allocator,
    mut transform: F,
) where
    F: FnMut(&IrExpression<'a>) -> Option<IrExpression<'a>>,
{
    match op {
        CreateOp::Variable(var) => {
            let new_expr = transform_expression(&var.initializer, allocator, &mut transform);
            *var.initializer = new_expr;
        }
        CreateOp::Listener(listener) => {
            for handler_op in listener.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, allocator, &mut transform);
            }
            if let Some(ref handler_expr) = listener.handler_expression {
                let new_expr = transform_expression(handler_expr, allocator, &mut transform);
                listener.handler_expression = Some(OxcBox::new_in(new_expr, allocator));
            }
        }
        CreateOp::TwoWayListener(listener) => {
            for handler_op in listener.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, allocator, &mut transform);
            }
        }
        CreateOp::AnimationListener(listener) => {
            for handler_op in listener.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, allocator, &mut transform);
            }
        }
        CreateOp::Animation(animation) => {
            for handler_op in animation.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, allocator, &mut transform);
            }
        }
        CreateOp::RepeaterCreate(rep) => {
            let new_expr = transform_expression(&rep.track, allocator, &mut transform);
            *rep.track = new_expr;
            // Also transform track_by_ops if present
            if let Some(ref mut track_by_ops) = rep.track_by_ops {
                for track_op in track_by_ops.iter_mut() {
                    transform_expressions_in_update_op(track_op, allocator, &mut transform);
                }
            }
        }
        CreateOp::ExtractedAttribute(attr) => {
            if let Some(ref value) = attr.value {
                let new_expr = transform_expression(value, allocator, &mut transform);
                attr.value = Some(OxcBox::new_in(new_expr, allocator));
            }
        }
        CreateOp::DeferOn(defer_on) => {
            if let Some(ref options) = defer_on.options {
                let new_expr = transform_expression(options, allocator, &mut transform);
                defer_on.options = Some(OxcBox::new_in(new_expr, allocator));
            }
        }
        _ => {}
    }
}

/// Recursively transform an expression.
/// Returns the transformed expression (either a replacement or the original cloned if no change).
fn transform_expression<'a, F>(
    expr: &IrExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
    transform: &mut F,
) -> IrExpression<'a>
where
    F: FnMut(&IrExpression<'a>) -> Option<IrExpression<'a>>,
{
    // First check if this expression should be replaced
    if let Some(replacement) = transform(expr) {
        return replacement;
    }

    // Otherwise, recursively transform sub-expressions
    match expr {
        IrExpression::ReadVariable(_)
        | IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_) => {
            // Leaf nodes - just clone
            expr.clone_in(allocator)
        }
        IrExpression::PureFunction(pf) => {
            use crate::ir::expression::PureFunctionExpr;
            let body = pf
                .body
                .as_ref()
                .map(|b| OxcBox::new_in(transform_expression(b, allocator, transform), allocator));
            let fn_ref = pf
                .fn_ref
                .as_ref()
                .map(|f| OxcBox::new_in(transform_expression(f, allocator, transform), allocator));
            let mut args = OxcVec::with_capacity_in(pf.args.len(), allocator);
            for arg in pf.args.iter() {
                args.push(transform_expression(arg, allocator, transform));
            }
            IrExpression::PureFunction(OxcBox::new_in(
                PureFunctionExpr {
                    body,
                    args,
                    fn_ref,
                    var_offset: pf.var_offset,
                    source_span: pf.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Interpolation(interp) => {
            use crate::ir::expression::Interpolation;
            let mut strings = OxcVec::with_capacity_in(interp.strings.len(), allocator);
            for s in interp.strings.iter() {
                strings.push(s.clone());
            }
            let mut expressions = OxcVec::with_capacity_in(interp.expressions.len(), allocator);
            for e in interp.expressions.iter() {
                expressions.push(transform_expression(e, allocator, transform));
            }
            let mut i18n_placeholders =
                OxcVec::with_capacity_in(interp.i18n_placeholders.len(), allocator);
            for ph in interp.i18n_placeholders.iter() {
                i18n_placeholders.push(ph.clone());
            }
            IrExpression::Interpolation(OxcBox::new_in(
                Interpolation {
                    strings,
                    expressions,
                    i18n_placeholders,
                    source_span: interp.source_span,
                },
                allocator,
            ))
        }
        IrExpression::RestoreView(rv) => {
            use crate::ir::expression::{RestoreViewExpr, RestoreViewTarget};
            let view = match &rv.view {
                RestoreViewTarget::Static(xref) => RestoreViewTarget::Static(*xref),
                RestoreViewTarget::Dynamic(inner) => RestoreViewTarget::Dynamic(OxcBox::new_in(
                    transform_expression(inner, allocator, transform),
                    allocator,
                )),
            };
            IrExpression::RestoreView(OxcBox::new_in(
                RestoreViewExpr { view, source_span: rv.source_span },
                allocator,
            ))
        }
        IrExpression::ResetView(rv) => {
            use crate::ir::expression::ResetViewExpr;
            IrExpression::ResetView(OxcBox::new_in(
                ResetViewExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&rv.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: rv.source_span,
                },
                allocator,
            ))
        }
        IrExpression::PipeBinding(pb) => {
            use crate::ir::expression::PipeBindingExpr;
            let mut args = OxcVec::with_capacity_in(pb.args.len(), allocator);
            for arg in pb.args.iter() {
                args.push(transform_expression(arg, allocator, transform));
            }
            IrExpression::PipeBinding(OxcBox::new_in(
                PipeBindingExpr {
                    target: pb.target,
                    target_slot: pb.target_slot,
                    name: pb.name.clone(),
                    args,
                    var_offset: pb.var_offset,
                    source_span: pb.source_span,
                },
                allocator,
            ))
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            use crate::ir::expression::PipeBindingVariadicExpr;
            IrExpression::PipeBindingVariadic(OxcBox::new_in(
                PipeBindingVariadicExpr {
                    target: pbv.target,
                    target_slot: pbv.target_slot,
                    name: pbv.name.clone(),
                    args: OxcBox::new_in(
                        transform_expression(&pbv.args, allocator, transform),
                        allocator,
                    ),
                    num_args: pbv.num_args,
                    var_offset: pbv.var_offset,
                    source_span: pbv.source_span,
                },
                allocator,
            ))
        }
        IrExpression::SafePropertyRead(spr) => {
            use crate::ir::expression::SafePropertyReadExpr;
            IrExpression::SafePropertyRead(OxcBox::new_in(
                SafePropertyReadExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&spr.receiver, allocator, transform),
                        allocator,
                    ),
                    name: spr.name.clone(),
                    source_span: spr.source_span,
                },
                allocator,
            ))
        }
        IrExpression::SafeKeyedRead(skr) => {
            use crate::ir::expression::SafeKeyedReadExpr;
            IrExpression::SafeKeyedRead(OxcBox::new_in(
                SafeKeyedReadExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&skr.receiver, allocator, transform),
                        allocator,
                    ),
                    index: OxcBox::new_in(
                        transform_expression(&skr.index, allocator, transform),
                        allocator,
                    ),
                    source_span: skr.source_span,
                },
                allocator,
            ))
        }
        IrExpression::SafeInvokeFunction(sif) => {
            use crate::ir::expression::SafeInvokeFunctionExpr;
            let mut args = OxcVec::with_capacity_in(sif.args.len(), allocator);
            for arg in sif.args.iter() {
                args.push(transform_expression(arg, allocator, transform));
            }
            IrExpression::SafeInvokeFunction(OxcBox::new_in(
                SafeInvokeFunctionExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&sif.receiver, allocator, transform),
                        allocator,
                    ),
                    args,
                    source_span: sif.source_span,
                },
                allocator,
            ))
        }
        IrExpression::SafeTernary(st) => {
            use crate::ir::expression::SafeTernaryExpr;
            IrExpression::SafeTernary(OxcBox::new_in(
                SafeTernaryExpr {
                    guard: OxcBox::new_in(
                        transform_expression(&st.guard, allocator, transform),
                        allocator,
                    ),
                    expr: OxcBox::new_in(
                        transform_expression(&st.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: st.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Ternary(t) => {
            use crate::ir::expression::TernaryExpr;
            IrExpression::Ternary(OxcBox::new_in(
                TernaryExpr {
                    condition: OxcBox::new_in(
                        transform_expression(&t.condition, allocator, transform),
                        allocator,
                    ),
                    true_expr: OxcBox::new_in(
                        transform_expression(&t.true_expr, allocator, transform),
                        allocator,
                    ),
                    false_expr: OxcBox::new_in(
                        transform_expression(&t.false_expr, allocator, transform),
                        allocator,
                    ),
                    source_span: t.source_span,
                },
                allocator,
            ))
        }
        IrExpression::AssignTemporary(at) => {
            use crate::ir::expression::AssignTemporaryExpr;
            IrExpression::AssignTemporary(OxcBox::new_in(
                AssignTemporaryExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&at.expr, allocator, transform),
                        allocator,
                    ),
                    xref: at.xref,
                    name: at.name.clone(),
                    source_span: at.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ConditionalCase(cc) => {
            use crate::ir::expression::ConditionalCaseExpr;
            IrExpression::ConditionalCase(OxcBox::new_in(
                ConditionalCaseExpr {
                    expr: cc.expr.as_ref().map(|e| {
                        OxcBox::new_in(transform_expression(e, allocator, transform), allocator)
                    }),
                    target: cc.target,
                    target_slot: cc.target_slot,
                    alias: cc.alias.clone(),
                    source_span: cc.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ConstCollected(cc) => {
            use crate::ir::expression::ConstCollectedExpr;
            IrExpression::ConstCollected(OxcBox::new_in(
                ConstCollectedExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&cc.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: cc.source_span,
                },
                allocator,
            ))
        }
        IrExpression::TwoWayBindingSet(twb) => {
            use crate::ir::expression::TwoWayBindingSetExpr;
            IrExpression::TwoWayBindingSet(OxcBox::new_in(
                TwoWayBindingSetExpr {
                    target: OxcBox::new_in(
                        transform_expression(&twb.target, allocator, transform),
                        allocator,
                    ),
                    value: OxcBox::new_in(
                        transform_expression(&twb.value, allocator, transform),
                        allocator,
                    ),
                    source_span: twb.source_span,
                },
                allocator,
            ))
        }
        IrExpression::StoreLet(sl) => {
            use crate::ir::expression::StoreLetExpr;
            IrExpression::StoreLet(OxcBox::new_in(
                StoreLetExpr {
                    target: sl.target,
                    value: OxcBox::new_in(
                        transform_expression(&sl.value, allocator, transform),
                        allocator,
                    ),
                    var_offset: sl.var_offset,
                    source_span: sl.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Binary(binary) => {
            use crate::ir::expression::BinaryExpr;
            IrExpression::Binary(OxcBox::new_in(
                BinaryExpr {
                    operator: binary.operator,
                    lhs: OxcBox::new_in(
                        transform_expression(&binary.lhs, allocator, transform),
                        allocator,
                    ),
                    rhs: OxcBox::new_in(
                        transform_expression(&binary.rhs, allocator, transform),
                        allocator,
                    ),
                    source_span: binary.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            use crate::ir::expression::ResolvedPropertyReadExpr;
            IrExpression::ResolvedPropertyRead(OxcBox::new_in(
                ResolvedPropertyReadExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&rpr.receiver, allocator, transform),
                        allocator,
                    ),
                    name: rpr.name.clone(),
                    source_span: rpr.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ResolvedBinary(rb) => {
            use crate::ir::expression::ResolvedBinaryExpr;
            IrExpression::ResolvedBinary(OxcBox::new_in(
                ResolvedBinaryExpr {
                    operator: rb.operator,
                    left: OxcBox::new_in(
                        transform_expression(&rb.left, allocator, transform),
                        allocator,
                    ),
                    right: OxcBox::new_in(
                        transform_expression(&rb.right, allocator, transform),
                        allocator,
                    ),
                    source_span: rb.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ResolvedCall(rc) => {
            use crate::ir::expression::ResolvedCallExpr;
            let mut args = OxcVec::with_capacity_in(rc.args.len(), allocator);
            for arg in rc.args.iter() {
                args.push(transform_expression(arg, allocator, transform));
            }
            IrExpression::ResolvedCall(OxcBox::new_in(
                ResolvedCallExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&rc.receiver, allocator, transform),
                        allocator,
                    ),
                    args,
                    source_span: rc.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            use crate::ir::expression::ResolvedKeyedReadExpr;
            IrExpression::ResolvedKeyedRead(OxcBox::new_in(
                ResolvedKeyedReadExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&rkr.receiver, allocator, transform),
                        allocator,
                    ),
                    key: OxcBox::new_in(
                        transform_expression(&rkr.key, allocator, transform),
                        allocator,
                    ),
                    source_span: rkr.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            use crate::ir::expression::ResolvedSafePropertyReadExpr;
            IrExpression::ResolvedSafePropertyRead(OxcBox::new_in(
                ResolvedSafePropertyReadExpr {
                    receiver: OxcBox::new_in(
                        transform_expression(&rspr.receiver, allocator, transform),
                        allocator,
                    ),
                    name: rspr.name.clone(),
                    source_span: rspr.source_span,
                },
                allocator,
            ))
        }
        IrExpression::DerivedLiteralArray(arr) => {
            use crate::ir::expression::DerivedLiteralArrayExpr;
            let mut entries = OxcVec::with_capacity_in(arr.entries.len(), allocator);
            for entry in arr.entries.iter() {
                entries.push(transform_expression(entry, allocator, transform));
            }
            IrExpression::DerivedLiteralArray(OxcBox::new_in(
                DerivedLiteralArrayExpr { entries, source_span: arr.source_span },
                allocator,
            ))
        }
        IrExpression::DerivedLiteralMap(map) => {
            use crate::ir::expression::DerivedLiteralMapExpr;
            let mut keys = OxcVec::with_capacity_in(map.keys.len(), allocator);
            for key in map.keys.iter() {
                keys.push(key.clone());
            }
            let mut values = OxcVec::with_capacity_in(map.values.len(), allocator);
            for value in map.values.iter() {
                values.push(transform_expression(value, allocator, transform));
            }
            let mut quoted = OxcVec::with_capacity_in(map.quoted.len(), allocator);
            for q in map.quoted.iter() {
                quoted.push(*q);
            }
            IrExpression::DerivedLiteralMap(OxcBox::new_in(
                DerivedLiteralMapExpr { keys, values, quoted, source_span: map.source_span },
                allocator,
            ))
        }
        IrExpression::LiteralArray(arr) => {
            use crate::ir::expression::IrLiteralArrayExpr;
            let mut elements = OxcVec::with_capacity_in(arr.elements.len(), allocator);
            for elem in arr.elements.iter() {
                elements.push(transform_expression(elem, allocator, transform));
            }
            IrExpression::LiteralArray(OxcBox::new_in(
                IrLiteralArrayExpr { elements, source_span: arr.source_span },
                allocator,
            ))
        }
        IrExpression::LiteralMap(map) => {
            use crate::ir::expression::IrLiteralMapExpr;
            let mut keys = OxcVec::with_capacity_in(map.keys.len(), allocator);
            for key in map.keys.iter() {
                keys.push(key.clone());
            }
            let mut values = OxcVec::with_capacity_in(map.values.len(), allocator);
            for value in map.values.iter() {
                values.push(transform_expression(value, allocator, transform));
            }
            let mut quoted = OxcVec::with_capacity_in(map.quoted.len(), allocator);
            for q in map.quoted.iter() {
                quoted.push(*q);
            }
            IrExpression::LiteralMap(OxcBox::new_in(
                IrLiteralMapExpr { keys, values, quoted, source_span: map.source_span },
                allocator,
            ))
        }
        IrExpression::Not(n) => {
            use crate::ir::expression::NotExpr;
            IrExpression::Not(OxcBox::new_in(
                NotExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&n.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: n.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Unary(u) => {
            use crate::ir::expression::UnaryExpr;
            IrExpression::Unary(OxcBox::new_in(
                UnaryExpr {
                    operator: u.operator,
                    expr: OxcBox::new_in(
                        transform_expression(&u.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: u.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Typeof(t) => {
            use crate::ir::expression::TypeofExpr;
            IrExpression::Typeof(OxcBox::new_in(
                TypeofExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&t.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: t.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Void(v) => {
            use crate::ir::expression::VoidExpr;
            IrExpression::Void(OxcBox::new_in(
                VoidExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&v.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: v.source_span,
                },
                allocator,
            ))
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            use crate::ir::expression::{IrTemplateLiteralElement, ResolvedTemplateLiteralExpr};
            let mut elements = OxcVec::with_capacity_in(rtl.elements.len(), allocator);
            for elem in rtl.elements.iter() {
                elements.push(IrTemplateLiteralElement {
                    text: elem.text.clone(),
                    source_span: elem.source_span,
                });
            }
            let mut expressions = OxcVec::with_capacity_in(rtl.expressions.len(), allocator);
            for e in rtl.expressions.iter() {
                expressions.push(transform_expression(e, allocator, transform));
            }
            IrExpression::ResolvedTemplateLiteral(OxcBox::new_in(
                ResolvedTemplateLiteralExpr { elements, expressions, source_span: rtl.source_span },
                allocator,
            ))
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            use crate::ir::expression::ArrowFunctionExpr;
            use crate::output::ast::FnParam;

            let mut params = OxcVec::with_capacity_in(arrow_fn.params.len(), allocator);
            for param in arrow_fn.params.iter() {
                params.push(FnParam { name: param.name.clone() });
            }

            let body = transform_expression(&arrow_fn.body, allocator, transform);

            IrExpression::ArrowFunction(OxcBox::new_in(
                ArrowFunctionExpr {
                    params,
                    body: OxcBox::new_in(body, allocator),
                    // ops are not transformed as they are transient data
                    ops: OxcVec::new_in(allocator),
                    var_offset: arrow_fn.var_offset,
                    source_span: arrow_fn.source_span,
                },
                allocator,
            ))
        }
        IrExpression::Parenthesized(paren) => {
            use crate::ir::expression::IrParenthesizedExpr;
            IrExpression::Parenthesized(OxcBox::new_in(
                IrParenthesizedExpr {
                    expr: OxcBox::new_in(
                        transform_expression(&paren.expr, allocator, transform),
                        allocator,
                    ),
                    source_span: paren.source_span,
                },
                allocator,
            ))
        }
    }
}

/// Inline Context variables (with NextContext initializers) into other Variable ops within handler_ops.
///
/// Per TypeScript's allowConservativeInlining (variable_optimization.ts lines 536-538):
/// ```typescript
/// case ir.SemanticVariableKind.Context:
///   // Context can only be inlined into other variables.
///   return target.kind === ir.OpKind.Variable;
/// ```
///
/// This transforms:
/// ```javascript
/// const ctx_r2 = i0.ɵɵnextContext();
/// const breadcrumb_r4 = ctx_r2.$implicit;
/// ```
/// Into:
/// ```javascript
/// const breadcrumb_r1 = i0.ɵɵnextContext().$implicit;
/// ```
///
/// Also handles RestoreView for conditional aliases like `@if (expr; as alias)`:
/// ```javascript
/// const ctx_r1 = i0.ɵɵrestoreView(_r1);
/// const parent_r3 = ctx_r1;
/// // becomes:
/// const parent_r1 = i0.ɵɵrestoreView(_r1);
/// ```
fn inline_context_vars_in_handler_ops<'a>(
    handler_ops: &mut OxcVec<'a, UpdateOp<'a>>,
    handler_expression: Option<&IrExpression<'a>>,
    allocator: &'a oxc_allocator::Allocator,
) {
    use crate::ir::enums::SemanticVariableKind;

    // First pass: collect Context variables with NextContext or RestoreView initializers
    // Per TypeScript's allowConservativeInlining (variable_optimization.ts lines 536-538):
    // "Context can only be inlined into other variables."
    let mut context_vars: HashMap<XrefId, IrExpression<'a>> = HashMap::new();
    for op in handler_ops.iter() {
        if let UpdateOp::Variable(var) = op {
            if var.kind == SemanticVariableKind::Context {
                // Handle both NextContext (for parent context access) and RestoreView (for save/restore)
                if matches!(
                    var.initializer.as_ref(),
                    IrExpression::NextContext(_) | IrExpression::RestoreView(_)
                ) {
                    context_vars.insert(var.xref, var.initializer.clone_in(allocator));
                }
            }
        }
    }

    if context_vars.is_empty() {
        return;
    }

    // Count usages of context variables, tracking which are used in Variable ops only
    let mut usage_counts: HashMap<XrefId, usize> = HashMap::new();
    let mut var_op_usages: HashMap<XrefId, HashSet<XrefId>> = HashMap::new(); // context_xref -> set of using var xrefs

    for op in handler_ops.iter() {
        if let UpdateOp::Variable(var) = op {
            // Skip the context variable itself
            if context_vars.contains_key(&var.xref) {
                continue;
            }
            // Check if this variable uses any context variable
            visit_all_expressions(&var.initializer, &mut |expr| {
                if let IrExpression::ReadVariable(read) = expr {
                    if context_vars.contains_key(&read.xref) {
                        *usage_counts.entry(read.xref).or_insert(0) += 1;
                        var_op_usages.entry(read.xref).or_default().insert(var.xref);
                    }
                }
            });
        } else {
            // Check usages in non-Variable ops (these prevent inlining)
            visit_expressions_in_update_op(op, |expr| {
                if let IrExpression::ReadVariable(read) = expr {
                    if context_vars.contains_key(&read.xref) {
                        *usage_counts.entry(read.xref).or_insert(0) += 1;
                        // Don't add to var_op_usages - non-Variable usage prevents inlining
                    }
                }
            });
        }
    }

    // Count usages in handler_expression (these also prevent inlining, per
    // allowConservativeInlining: "Context can only be inlined into other variables")
    if let Some(expr) = handler_expression {
        visit_all_expressions(expr, &mut |inner_expr| {
            if let IrExpression::ReadVariable(read) = inner_expr {
                if context_vars.contains_key(&read.xref) {
                    *usage_counts.entry(read.xref).or_insert(0) += 1;
                    // Don't add to var_op_usages - handler_expression usage prevents inlining
                }
            }
        });
    }

    // Determine which context variables can be inlined:
    // - Used exactly once
    // - Used only in a Variable op (not in property bindings, etc.)
    let mut to_inline: HashSet<XrefId> = HashSet::new();
    for (ctx_xref, count) in &usage_counts {
        if *count == 1 {
            // Check if the single usage is in a Variable op
            if let Some(using_vars) = var_op_usages.get(ctx_xref) {
                if using_vars.len() == 1 {
                    to_inline.insert(*ctx_xref);
                }
            }
        }
    }

    if to_inline.is_empty() {
        return;
    }

    // Second pass: inline the context variables into the Variable ops that use them
    for op in handler_ops.iter_mut() {
        if let UpdateOp::Variable(var) = op {
            // Skip the context variable itself
            if context_vars.contains_key(&var.xref) {
                continue;
            }
            // Transform the initializer
            transform_expression_in_place(&mut var.initializer, allocator, |expr| {
                if let IrExpression::ReadVariable(read) = expr {
                    if to_inline.contains(&read.xref) {
                        if let Some(initializer) = context_vars.get(&read.xref) {
                            return Some(initializer.clone_in(allocator));
                        }
                    }
                }
                None
            });
        }
    }

    // Third pass: remove the inlined context variable declarations
    // Build a new vector without the inlined variables
    let mut new_ops: OxcVec<'a, UpdateOp<'a>> =
        OxcVec::with_capacity_in(handler_ops.len(), allocator);
    for op in handler_ops.iter() {
        if let UpdateOp::Variable(var) = op {
            if to_inline.contains(&var.xref) {
                // Skip this variable - it was inlined
                continue;
            }
        }
        new_ops.push(clone_update_op(op, allocator));
    }

    *handler_ops = new_ops;
}

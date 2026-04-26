//! Strip non-required parentheses phase.
//!
//! In most cases we can drop user-added parentheses from expressions. However, in some cases
//! parentheses are needed for the expression to be considered valid JavaScript or for TypeScript
//! to generate the correct output.
//!
//! This phase strips all parentheses except in the following situations where they are required:
//!
//! 1. **Unary in exponentiation base**: `-2 ** 3` is not valid JavaScript, but `(-2) ** 3` is.
//!
//! 2. **Nullish coalescing with logical operators**: `a ?? b && c` is not valid JavaScript,
//!    but `a ?? (b && c)` is. Also handles `(a ?? b) && c` for TypeScript compatibility.
//!
//! 3. **Ternary in nullish coalescing**: TypeScript generates incorrect code if parentheses
//!    are missing. `(a ? b : c) ?? d` must keep the parentheses.
//!
//! Ported from Angular's `template/pipeline/src/phases/strip_nonrequired_parentheses.ts`.

use std::collections::HashSet;

use oxc_allocator::{Allocator, Box as ArenaBox};

use crate::ast::expression::{AngularExpression, BinaryOperator};
use crate::ir::expression::{
    IrExpression, VisitorContextFlag, transform_expressions_in_create_op,
    transform_expressions_in_update_op,
};
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Strips unnecessary parentheses from expressions.
///
/// This phase removes parentheses that don't affect the semantics of the expression,
/// while keeping those required for:
/// - Unary operators in exponentiation base
/// - Nullish coalescing with logical operators
/// - Ternary expressions in nullish coalescing
pub fn strip_nonrequired_parentheses(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Process all views
    for view in job.all_views_mut() {
        // Pass 1: Find required parentheses
        let mut required_parens: HashSet<*const ()> = HashSet::new();

        for op in view.create.iter() {
            visit_expressions_for_required_parens(op, &mut required_parens);
        }
        for op in view.update.iter() {
            visit_update_expressions_for_required_parens(op, &mut required_parens);
        }

        // Pass 2: Strip non-required parentheses
        for op in view.create.iter_mut() {
            transform_expressions_in_create_op(
                op,
                &|expr, _flags| {
                    strip_parens_in_expression(expr, &required_parens, allocator);
                },
                VisitorContextFlag::NONE,
            );
        }
        for op in view.update.iter_mut() {
            transform_expressions_in_update_op(
                op,
                &|expr, _flags| {
                    strip_parens_in_expression(expr, &required_parens, allocator);
                },
                VisitorContextFlag::NONE,
            );
        }
    }
}

/// Visit expressions in a create op to find required parentheses.
fn visit_expressions_for_required_parens(
    op: &crate::ir::ops::CreateOp<'_>,
    required: &mut HashSet<*const ()>,
) {
    use crate::ir::ops::CreateOp;

    match op {
        CreateOp::Variable(v) => {
            check_ir_expression_for_required_parens(&v.initializer, required);
        }
        CreateOp::Listener(l) => {
            if let Some(handler) = &l.handler_expression {
                check_ir_expression_for_required_parens(handler, required);
            }
            for handler_op in l.handler_ops.iter() {
                visit_update_expressions_for_required_parens(handler_op, required);
            }
        }
        CreateOp::Conditional(_c) => {
            // ConditionalOp (CREATE) no longer has test/branches.
            // Those are now on ConditionalUpdateOp (UPDATE) and handled there.
        }
        CreateOp::RepeaterCreate(r) => {
            check_ir_expression_for_required_parens(&r.track, required);
        }
        CreateOp::I18nMessage(_) => {
            // I18nMessage params are now formatted strings (Ident), not expressions
            // No expressions to check here
        }
        CreateOp::I18nContext(_) => {
            // I18nContext params are I18nParamValue, not IrExpressions
            // No expressions to check here
        }
        CreateOp::ExtractedAttribute(e) => {
            if let Some(value) = &e.value {
                check_ir_expression_for_required_parens(value, required);
            }
        }
        CreateOp::TwoWayListener(t) => {
            for handler_op in t.handler_ops.iter() {
                visit_update_expressions_for_required_parens(handler_op, required);
            }
        }
        CreateOp::AnimationListener(a) => {
            for handler_op in a.handler_ops.iter() {
                visit_update_expressions_for_required_parens(handler_op, required);
            }
        }
        CreateOp::Animation(a) => {
            for handler_op in a.handler_ops.iter() {
                visit_update_expressions_for_required_parens(handler_op, required);
            }
        }
        CreateOp::AnimationString(a) => {
            check_ir_expression_for_required_parens(&a.expression, required);
        }
        CreateOp::DeferOn(d) => {
            if let Some(options) = &d.options {
                check_ir_expression_for_required_parens(options, required);
            }
        }
        _ => {
            // Other create ops don't have expressions to check
        }
    }
}

/// Visit expressions in an update op to find required parentheses.
fn visit_update_expressions_for_required_parens(
    op: &crate::ir::ops::UpdateOp<'_>,
    required: &mut HashSet<*const ()>,
) {
    use crate::ir::ops::UpdateOp;

    match op {
        UpdateOp::Property(p) => {
            check_ir_expression_for_required_parens(&p.expression, required);
        }
        UpdateOp::InterpolateText(i) => {
            check_ir_expression_for_required_parens(&i.interpolation, required);
        }
        UpdateOp::Attribute(a) => {
            check_ir_expression_for_required_parens(&a.expression, required);
        }
        UpdateOp::StyleProp(s) => {
            check_ir_expression_for_required_parens(&s.expression, required);
        }
        UpdateOp::ClassProp(c) => {
            check_ir_expression_for_required_parens(&c.expression, required);
        }
        UpdateOp::StyleMap(s) => {
            check_ir_expression_for_required_parens(&s.expression, required);
        }
        UpdateOp::ClassMap(c) => {
            check_ir_expression_for_required_parens(&c.expression, required);
        }
        UpdateOp::DomProperty(d) => {
            check_ir_expression_for_required_parens(&d.expression, required);
        }
        UpdateOp::Conditional(c) => {
            if let Some(test) = &c.test {
                check_ir_expression_for_required_parens(test, required);
            }
            // Also check conditions (branch expressions)
            for condition in c.conditions.iter() {
                if let Some(expr) = &condition.expr {
                    check_ir_expression_for_required_parens(expr, required);
                }
            }
            if let Some(processed) = &c.processed {
                check_ir_expression_for_required_parens(processed, required);
            }
            if let Some(ctx_val) = &c.context_value {
                check_ir_expression_for_required_parens(ctx_val, required);
            }
        }
        UpdateOp::Repeater(r) => {
            check_ir_expression_for_required_parens(&r.collection, required);
        }
        UpdateOp::Binding(b) => {
            check_ir_expression_for_required_parens(&b.expression, required);
        }
        UpdateOp::I18nExpression(i) => {
            check_ir_expression_for_required_parens(&i.expression, required);
        }
        UpdateOp::AnimationBinding(a) => {
            check_ir_expression_for_required_parens(&a.expression, required);
        }
        UpdateOp::Variable(v) => {
            check_ir_expression_for_required_parens(&v.initializer, required);
        }
        UpdateOp::Control(c) => {
            check_ir_expression_for_required_parens(&c.expression, required);
        }
        UpdateOp::TwoWayProperty(t) => {
            check_ir_expression_for_required_parens(&t.expression, required);
        }
        UpdateOp::StoreLet(s) => {
            check_ir_expression_for_required_parens(&s.value, required);
        }
        UpdateOp::DeferWhen(d) => {
            check_ir_expression_for_required_parens(&d.condition, required);
        }
        UpdateOp::ListEnd(_)
        | UpdateOp::Advance(_)
        | UpdateOp::I18nApply(_)
        | UpdateOp::Statement(_) => {
            // These ops don't have expressions to check
        }
    }
}

/// Check an IR expression for binary operators that require parentheses.
fn check_ir_expression_for_required_parens(
    expr: &IrExpression<'_>,
    required: &mut HashSet<*const ()>,
) {
    match expr {
        IrExpression::Ast(ast_expr) => {
            check_ast_expression_for_required_parens(ast_expr, required);
        }
        IrExpression::SafeTernary(st) => {
            check_ir_expression_for_required_parens(&st.guard, required);
            check_ir_expression_for_required_parens(&st.expr, required);
        }
        IrExpression::Ternary(t) => {
            check_ir_expression_for_required_parens(&t.condition, required);
            check_ir_expression_for_required_parens(&t.true_expr, required);
            check_ir_expression_for_required_parens(&t.false_expr, required);
        }
        IrExpression::AssignTemporary(at) => {
            check_ir_expression_for_required_parens(&at.expr, required);
        }
        IrExpression::SafePropertyRead(sp) => {
            check_ir_expression_for_required_parens(&sp.receiver, required);
        }
        IrExpression::SafeKeyedRead(sk) => {
            check_ir_expression_for_required_parens(&sk.receiver, required);
            check_ir_expression_for_required_parens(&sk.index, required);
        }
        IrExpression::SafeInvokeFunction(sf) => {
            check_ir_expression_for_required_parens(&sf.receiver, required);
            for arg in sf.args.iter() {
                check_ir_expression_for_required_parens(arg, required);
            }
        }
        IrExpression::PipeBinding(pb) => {
            for arg in pb.args.iter() {
                check_ir_expression_for_required_parens(arg, required);
            }
        }
        IrExpression::PipeBindingVariadic(pbv) => {
            check_ir_expression_for_required_parens(&pbv.args, required);
        }
        IrExpression::PureFunction(pf) => {
            for arg in pf.args.iter() {
                check_ir_expression_for_required_parens(arg, required);
            }
        }
        IrExpression::Interpolation(i) => {
            for expr in i.expressions.iter() {
                check_ir_expression_for_required_parens(expr, required);
            }
        }
        IrExpression::RestoreView(rv) => {
            use crate::ir::expression::RestoreViewTarget;
            if let RestoreViewTarget::Dynamic(expr) = &rv.view {
                check_ir_expression_for_required_parens(expr, required);
            }
        }
        IrExpression::ResetView(rv) => {
            check_ir_expression_for_required_parens(&rv.expr, required);
        }
        IrExpression::ConditionalCase(cc) => {
            if let Some(expr) = &cc.expr {
                check_ir_expression_for_required_parens(expr, required);
            }
        }
        IrExpression::TwoWayBindingSet(tbs) => {
            check_ir_expression_for_required_parens(&tbs.target, required);
            check_ir_expression_for_required_parens(&tbs.value, required);
        }
        IrExpression::StoreLet(sl) => {
            check_ir_expression_for_required_parens(&sl.value, required);
        }
        IrExpression::ConstCollected(cc) => {
            check_ir_expression_for_required_parens(&cc.expr, required);
        }
        IrExpression::Binary(binary) => {
            check_ir_expression_for_required_parens(&binary.lhs, required);
            check_ir_expression_for_required_parens(&binary.rhs, required);
        }
        IrExpression::ResolvedPropertyRead(rpr) => {
            check_ir_expression_for_required_parens(&rpr.receiver, required);
        }
        IrExpression::ResolvedBinary(rb) => {
            check_ir_expression_for_required_parens(&rb.left, required);
            check_ir_expression_for_required_parens(&rb.right, required);
        }
        IrExpression::ResolvedCall(rc) => {
            check_ir_expression_for_required_parens(&rc.receiver, required);
            for arg in rc.args.iter() {
                check_ir_expression_for_required_parens(arg, required);
            }
        }
        IrExpression::ResolvedKeyedRead(rkr) => {
            check_ir_expression_for_required_parens(&rkr.receiver, required);
            check_ir_expression_for_required_parens(&rkr.key, required);
        }
        IrExpression::ResolvedSafePropertyRead(rspr) => {
            check_ir_expression_for_required_parens(&rspr.receiver, required);
        }
        IrExpression::DerivedLiteralArray(arr) => {
            for entry in arr.entries.iter() {
                check_ir_expression_for_required_parens(entry, required);
            }
        }
        IrExpression::DerivedLiteralMap(map) => {
            for value in map.values.iter() {
                check_ir_expression_for_required_parens(value, required);
            }
        }
        IrExpression::LiteralArray(arr) => {
            for elem in arr.elements.iter() {
                check_ir_expression_for_required_parens(elem, required);
            }
        }
        IrExpression::LiteralMap(map) => {
            for value in map.values.iter() {
                check_ir_expression_for_required_parens(value, required);
            }
        }
        // Leaf IR expressions - no nested expressions to check
        IrExpression::LexicalRead(_)
        | IrExpression::Reference(_)
        | IrExpression::Context(_)
        | IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::ReadVariable(_)
        | IrExpression::PureFunctionParameter(_)
        | IrExpression::Empty(_)
        | IrExpression::ReadTemporary(_)
        | IrExpression::SlotLiteral(_)
        | IrExpression::ConstReference(_)
        | IrExpression::ContextLetReference(_)
        | IrExpression::TrackContext(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_) => {
            // These don't contain nested expressions
        }
        IrExpression::Not(n) => {
            check_ir_expression_for_required_parens(&n.expr, required);
        }
        IrExpression::Unary(u) => {
            check_ir_expression_for_required_parens(&u.expr, required);
        }
        IrExpression::Typeof(t) => {
            check_ir_expression_for_required_parens(&t.expr, required);
        }
        IrExpression::Void(v) => {
            check_ir_expression_for_required_parens(&v.expr, required);
        }
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            for e in rtl.expressions.iter() {
                check_ir_expression_for_required_parens(e, required);
            }
        }

        IrExpression::ArrowFunction(arrow_fn) => {
            check_ir_expression_for_required_parens(&arrow_fn.body, required);
        }
        IrExpression::Parenthesized(paren) => {
            check_ir_expression_for_required_parens(&paren.expr, required);
        }
    }
}

/// Check an AST expression for binary operators that require parentheses.
fn check_ast_expression_for_required_parens(
    expr: &AngularExpression<'_>,
    required: &mut HashSet<*const ()>,
) {
    match expr {
        AngularExpression::Binary(binary) => {
            match binary.operation {
                BinaryOperator::Power => {
                    check_exponentiation_parens(expr, required);
                }
                BinaryOperator::NullishCoalescing => {
                    check_nullish_coalescing_parens(expr, required);
                }
                BinaryOperator::And | BinaryOperator::Or => {
                    check_and_or_parens(expr, required);
                }
                _ => {}
            }
            // Recursively check operands
            check_ast_expression_for_required_parens(&binary.left, required);
            check_ast_expression_for_required_parens(&binary.right, required);
        }
        AngularExpression::ParenthesizedExpression(paren) => {
            check_ast_expression_for_required_parens(&paren.expression, required);
        }
        AngularExpression::Conditional(cond) => {
            check_ast_expression_for_required_parens(&cond.condition, required);
            check_ast_expression_for_required_parens(&cond.true_exp, required);
            check_ast_expression_for_required_parens(&cond.false_exp, required);
        }
        AngularExpression::PropertyRead(prop) => {
            check_ast_expression_for_required_parens(&prop.receiver, required);
        }
        AngularExpression::SafePropertyRead(prop) => {
            check_ast_expression_for_required_parens(&prop.receiver, required);
        }
        AngularExpression::KeyedRead(keyed) => {
            check_ast_expression_for_required_parens(&keyed.receiver, required);
            check_ast_expression_for_required_parens(&keyed.key, required);
        }
        AngularExpression::SafeKeyedRead(keyed) => {
            check_ast_expression_for_required_parens(&keyed.receiver, required);
            check_ast_expression_for_required_parens(&keyed.key, required);
        }
        AngularExpression::Call(call) => {
            check_ast_expression_for_required_parens(&call.receiver, required);
            for arg in call.args.iter() {
                check_ast_expression_for_required_parens(arg, required);
            }
        }
        AngularExpression::SafeCall(call) => {
            check_ast_expression_for_required_parens(&call.receiver, required);
            for arg in call.args.iter() {
                check_ast_expression_for_required_parens(arg, required);
            }
        }
        AngularExpression::BindingPipe(pipe) => {
            check_ast_expression_for_required_parens(&pipe.exp, required);
            for arg in pipe.args.iter() {
                check_ast_expression_for_required_parens(arg, required);
            }
        }
        AngularExpression::Unary(unary) => {
            check_ast_expression_for_required_parens(&unary.expr, required);
        }
        AngularExpression::PrefixNot(prefix) => {
            check_ast_expression_for_required_parens(&prefix.expression, required);
        }
        AngularExpression::NonNullAssert(nna) => {
            check_ast_expression_for_required_parens(&nna.expression, required);
        }
        AngularExpression::LiteralArray(arr) => {
            for elem in arr.expressions.iter() {
                check_ast_expression_for_required_parens(elem, required);
            }
        }
        AngularExpression::LiteralMap(map) => {
            for val in map.values.iter() {
                check_ast_expression_for_required_parens(val, required);
            }
        }
        AngularExpression::Chain(chain) => {
            for expr in chain.expressions.iter() {
                check_ast_expression_for_required_parens(expr, required);
            }
        }
        AngularExpression::Interpolation(interp) => {
            for expr in interp.expressions.iter() {
                check_ast_expression_for_required_parens(expr, required);
            }
        }
        AngularExpression::TemplateLiteral(tl) => {
            for expr in tl.expressions.iter() {
                check_ast_expression_for_required_parens(expr, required);
            }
        }
        AngularExpression::TaggedTemplateLiteral(ttl) => {
            check_ast_expression_for_required_parens(&ttl.tag, required);
            for expr in ttl.template.expressions.iter() {
                check_ast_expression_for_required_parens(expr, required);
            }
        }
        AngularExpression::TypeofExpression(te) => {
            check_ast_expression_for_required_parens(&te.expression, required);
        }
        AngularExpression::VoidExpression(ve) => {
            check_ast_expression_for_required_parens(&ve.expression, required);
        }
        AngularExpression::SpreadElement(spread) => {
            check_ast_expression_for_required_parens(&spread.expression, required);
        }
        AngularExpression::ArrowFunction(arrow) => {
            check_ast_expression_for_required_parens(&arrow.body, required);
        }
        // Leaf expressions - no sub-expressions to check
        AngularExpression::Empty(_)
        | AngularExpression::ImplicitReceiver(_)
        | AngularExpression::ThisReceiver(_)
        | AngularExpression::LiteralPrimitive(_)
        | AngularExpression::RegularExpressionLiteral(_) => {}
    }
}

/// Checks if a unary operator in exponentiation base requires parentheses.
///
/// `-2 ** 3` is not valid JavaScript, but `(-2) ** 3` is valid.
fn check_exponentiation_parens(expr: &AngularExpression<'_>, required: &mut HashSet<*const ()>) {
    if let AngularExpression::Binary(binary) = expr {
        if binary.operation == BinaryOperator::Power {
            // Check if left side is a parenthesized unary expression
            if let AngularExpression::ParenthesizedExpression(paren) = &binary.left {
                if matches!(paren.expression, AngularExpression::Unary(_)) {
                    // Mark this parenthesized expression as required
                    let ptr = paren as *const _ as *const ();
                    required.insert(ptr);
                }
            }
        }
    }
}

/// Checks if parentheses are required around nullish coalescing operands.
///
/// Required cases:
/// - `(a && b) ?? c` - logical and/or on left
/// - `a ?? (b && c)` - logical and/or on right
/// - `(a ? b : c) ?? d` - ternary on left
/// - `a ?? (b ? c : d)` - ternary on right
fn check_nullish_coalescing_parens(
    expr: &AngularExpression<'_>,
    required: &mut HashSet<*const ()>,
) {
    if let AngularExpression::Binary(binary) = expr {
        if binary.operation == BinaryOperator::NullishCoalescing {
            // Check left operand
            if let AngularExpression::ParenthesizedExpression(paren) = &binary.left {
                if is_logical_and_or(&paren.expression)
                    || matches!(paren.expression, AngularExpression::Conditional(_))
                {
                    let ptr = paren as *const _ as *const ();
                    required.insert(ptr);
                }
            }

            // Check right operand
            if let AngularExpression::ParenthesizedExpression(paren) = &binary.right {
                if is_logical_and_or(&paren.expression)
                    || matches!(paren.expression, AngularExpression::Conditional(_))
                {
                    let ptr = paren as *const _ as *const ();
                    required.insert(ptr);
                }
            }
        }
    }
}

/// Checks if parentheses are required for and/or operators with nullish coalescing.
///
/// Due to TypeScript issue #62307, we need to keep parentheses around `??` when
/// used with and/or operators: `(a ?? b) && c`.
fn check_and_or_parens(expr: &AngularExpression<'_>, required: &mut HashSet<*const ()>) {
    if let AngularExpression::Binary(binary) = expr {
        if binary.operation == BinaryOperator::And || binary.operation == BinaryOperator::Or {
            // Check if left operand is a parenthesized nullish coalescing
            if let AngularExpression::ParenthesizedExpression(paren) = &binary.left {
                if let AngularExpression::Binary(inner) = &paren.expression {
                    if inner.operation == BinaryOperator::NullishCoalescing {
                        let ptr = paren as *const _ as *const ();
                        required.insert(ptr);
                    }
                }
            }
        }
    }
}

/// Checks if an expression is a logical AND or OR operation.
fn is_logical_and_or(expr: &AngularExpression<'_>) -> bool {
    if let AngularExpression::Binary(binary) = expr {
        matches!(binary.operation, BinaryOperator::And | BinaryOperator::Or)
    } else {
        false
    }
}

/// Strip non-required parentheses from an IR expression.
fn strip_parens_in_expression<'a>(
    expr: &mut IrExpression<'a>,
    required: &HashSet<*const ()>,
    allocator: &'a Allocator,
) {
    // Handle AST-level parenthesized expressions
    if let IrExpression::Ast(ast_expr) = expr {
        if let AngularExpression::ParenthesizedExpression(paren) = ast_expr.as_ref() {
            let ptr = paren as *const _ as *const ();
            if !required.contains(&ptr) {
                // We need to unwrap the AST parentheses
                // This is tricky because we need to clone the inner expression
                let inner_cloned =
                    crate::ir::expression::clone_angular_expression(&paren.expression, allocator);
                *expr = IrExpression::Ast(ArenaBox::new_in(inner_cloned, allocator));
            }
        }
    }
}

/// Strips non-required parentheses for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn strip_nonrequired_parentheses_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Pass 1: Find required parentheses
    let mut required_parens: HashSet<*const ()> = HashSet::new();

    for op in job.root.create.iter() {
        visit_expressions_for_required_parens(op, &mut required_parens);
    }
    for op in job.root.update.iter() {
        visit_update_expressions_for_required_parens(op, &mut required_parens);
    }

    // Pass 2: Strip non-required parentheses
    for op in job.root.create.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| {
                strip_parens_in_expression(expr, &required_parens, allocator);
            },
            VisitorContextFlag::NONE,
        );
    }
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                strip_parens_in_expression(expr, &required_parens, allocator);
            },
            VisitorContextFlag::NONE,
        );
    }
}

//! Expand safe reads phase.
//!
//! Safe read expressions such as `a?.b` have different semantics in Angular templates compared
//! to JavaScript. In particular, they default to `null` instead of `undefined`. This phase
//! finds all unresolved safe read expressions and converts them into the appropriate output AST
//! reads, guarded by null checks.
//!
//! ## Algorithm
//!
//! This phase performs two transformations:
//!
//! 1. **Safe Transform**: Converts safe access expressions to `SafeTernaryExpr`
//!    - `a?.b` → `SafeTernaryExpr { guard: a, expr: a.b }`
//!    - `a?.b?.c` → nested SafeTernaryExpr
//!    - `a?.foo()` → SafeTernaryExpr with function call
//!
//! 2. **Ternary Transform**: Converts `SafeTernaryExpr` to `ConditionalExpr`
//!    - `SafeTernaryExpr { guard, expr }` → `(guard == null ? null : expr)`
//!
//! ## Temporary Variables
//!
//! When the guard expression has side effects (e.g., function calls), a temporary
//! variable is generated to avoid re-evaluation:
//!
//! ```text
//! a?.foo()?.b
//! ↓
//! (tmp = a?.foo(), tmp == null ? null : tmp.b)
//! ```
//!
//! Expressions that require temporaries:
//! - Function invocations
//! - Array literals
//! - Object literals
//! - Pipe bindings
//! - Parenthesized expressions containing the above
//!
//! Ported from Angular's `template/pipeline/src/phases/expand_safe_reads.ts`.
//!
//! ## Current Implementation Status
//!
//! The current implementation handles safe expressions that come in as IR types
//! (`SafePropertyReadExpr`, `SafeKeyedReadExpr`, `SafeInvokeFunctionExpr`).
//!
//! Safe expressions that come in as AST expressions (`AngularExpression::SafePropertyRead`, etc.)
//! are handled directly in the reify phase when converting to output AST.

use std::cell::RefCell;

use oxc_allocator::{Allocator, Box as ArenaBox, Vec as ArenaVec};
use oxc_span::Span;

use crate::ir::expression::{
    AssignTemporaryExpr, IrExpression, ReadTemporaryExpr, ResolvedCallExpr, ResolvedKeyedReadExpr,
    ResolvedPropertyReadExpr, SafeTernaryExpr, VisitorContextFlag,
    transform_expressions_in_create_op, transform_expressions_in_update_op,
};
use crate::ir::ops::XrefId;
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Context for safe read expansion, providing access to the allocator and xref allocation.
struct SafeTransformContext<'a> {
    allocator: &'a Allocator,
    /// Counter for generating unique xref IDs (inner value of XrefId)
    next_xref: RefCell<u32>,
}

impl<'a> SafeTransformContext<'a> {
    fn allocate_xref_id(&self) -> XrefId {
        let mut next = self.next_xref.borrow_mut();
        let xref = XrefId::new(*next);
        *next += 1;
        xref
    }
}

/// Expands safe navigation expressions to null checks.
///
/// This phase transforms Angular's safe navigation operator (`?.`) into explicit
/// null checks with conditional expressions. It generates temporary variables
/// as needed to avoid re-evaluating expressions with side effects.
///
/// ## Transformations
///
/// - `SafePropertyRead` (`a?.b`) → `SafeTernary { guard: a, expr: PropertyRead }`
/// - `SafeKeyedRead` (`a?.[key]`) → `SafeTernary { guard: a, expr: KeyedRead }`
/// - `SafeInvokeFunction` (`a?.()`) → `SafeTernary { guard: a, expr: Invoke }`
///
/// The actual conversion from `SafeTernary` to conditional null check happens
/// in the reify phase when generating output JavaScript.
///
/// When the receiver might have side effects:
/// - `foo()?.bar` → `SafeTernary { guard: AssignTemp(foo()), expr: ReadTemp.bar }`
pub fn expand_safe_reads(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Get the current xref counter value - we'll track allocations ourselves
    // and sync back at the end
    let starting_xref = job.allocate_xref_id().0;
    let xref_counter = RefCell::new(starting_xref);

    // Process all views
    for view in job.all_views_mut() {
        let ctx = SafeTransformContext { allocator, next_xref: xref_counter.clone() };

        // Transform safe access expressions to SafeTernary
        for op in view.create.iter_mut() {
            transform_expressions_in_create_op(
                op,
                &|expr, _flags| {
                    safe_transform(expr, &ctx);
                },
                VisitorContextFlag::NONE,
            );
        }
        for op in view.update.iter_mut() {
            transform_expressions_in_update_op(
                op,
                &|expr, _flags| {
                    safe_transform(expr, &ctx);
                },
                VisitorContextFlag::NONE,
            );
        }
    }
}

/// Checks if an IR expression requires a temporary variable to avoid re-evaluation.
///
/// Returns true for expressions with side effects (function calls, pipe bindings),
/// or for compound expressions that may contain such sub-expressions.
///
/// Ported from Angular's `needsTemporaryInSafeAccess` in `expand_safe_reads.ts` lines 43-73.
fn needs_temporary_in_safe_access(expr: &IrExpression<'_>) -> bool {
    match expr {
        // Function calls always need temporaries (they may have side effects)
        IrExpression::ResolvedCall(_) => true,
        // Safe function invocations always need temporaries
        IrExpression::SafeInvokeFunction(_) => true,
        // Pipe bindings need temporaries (they may have side effects)
        IrExpression::PipeBinding(_) => true,
        IrExpression::PipeBindingVariadic(_) => true,
        // Array and object literals need temporaries
        IrExpression::LiteralArray(_) | IrExpression::DerivedLiteralArray(_) => true,
        IrExpression::LiteralMap(_) | IrExpression::DerivedLiteralMap(_) => true,
        // Unary operators need to check their operand
        IrExpression::Unary(u) => needs_temporary_in_safe_access(&u.expr),
        IrExpression::Not(n) => needs_temporary_in_safe_access(&n.expr),
        // Binary operators need to check both operands
        IrExpression::Binary(binary) => {
            needs_temporary_in_safe_access(&binary.lhs)
                || needs_temporary_in_safe_access(&binary.rhs)
        }
        IrExpression::ResolvedBinary(rb) => {
            needs_temporary_in_safe_access(&rb.left) || needs_temporary_in_safe_access(&rb.right)
        }
        // Conditional expressions need to check all branches
        IrExpression::Ternary(t) => {
            needs_temporary_in_safe_access(&t.condition)
                || needs_temporary_in_safe_access(&t.true_expr)
                || needs_temporary_in_safe_access(&t.false_expr)
        }
        // AssignTemporary expressions need to check their inner expression
        IrExpression::AssignTemporary(assign) => needs_temporary_in_safe_access(&assign.expr),
        // Safe property reads need to check their receiver
        IrExpression::SafePropertyRead(prop) => needs_temporary_in_safe_access(&prop.receiver),
        IrExpression::SafeKeyedRead(keyed) => {
            needs_temporary_in_safe_access(&keyed.receiver)
                || needs_temporary_in_safe_access(&keyed.index)
        }
        // Resolved property/keyed reads need to check their receiver
        IrExpression::ResolvedPropertyRead(prop) => needs_temporary_in_safe_access(&prop.receiver),
        IrExpression::ResolvedKeyedRead(keyed) => {
            needs_temporary_in_safe_access(&keyed.receiver)
                || needs_temporary_in_safe_access(&keyed.key)
        }
        // Binary operators need to check both operands
        IrExpression::Binary(bin) => {
            needs_temporary_in_safe_access(&bin.lhs) || needs_temporary_in_safe_access(&bin.rhs)
        }
        // Ternary needs to check all branches
        IrExpression::Ternary(ternary) => {
            needs_temporary_in_safe_access(&ternary.condition)
                || needs_temporary_in_safe_access(&ternary.true_expr)
                || needs_temporary_in_safe_access(&ternary.false_expr)
        }
        // Not expression needs to check operand
        IrExpression::Not(not) => needs_temporary_in_safe_access(&not.expr),
        // Unary operator needs to check operand
        IrExpression::Unary(unary) => needs_temporary_in_safe_access(&unary.expr),
        // Check AST expressions for function calls
        IrExpression::Ast(ast_expr) => needs_temporary_in_ast_expression(ast_expr),
        // Parenthesized expressions need to check their inner expression
        IrExpression::Parenthesized(paren) => needs_temporary_in_safe_access(&paren.expr),
        // Simple expressions don't need temporaries
        _ => false,
    }
}

/// Checks if an AST expression requires a temporary variable.
fn needs_temporary_in_ast_expression(expr: &crate::ast::expression::AngularExpression<'_>) -> bool {
    use crate::ast::expression::AngularExpression;

    match expr {
        // Function calls always need temporaries
        AngularExpression::Call(_) => true,
        AngularExpression::SafeCall(_) => true,
        // Pipe bindings need temporaries
        AngularExpression::BindingPipe(_) => true,
        // Array and object literals need temporaries
        AngularExpression::LiteralArray(_) => true,
        AngularExpression::LiteralMap(_) => true,
        // Parenthesized expressions need to check their inner expression
        AngularExpression::ParenthesizedExpression(paren) => {
            needs_temporary_in_ast_expression(&paren.expression)
        }
        // Unary and binary operators need to check operands
        AngularExpression::Unary(unary) => needs_temporary_in_ast_expression(&unary.expr),
        AngularExpression::Binary(binary) => {
            needs_temporary_in_ast_expression(&binary.left)
                || needs_temporary_in_ast_expression(&binary.right)
        }
        AngularExpression::Conditional(cond) => {
            needs_temporary_in_ast_expression(&cond.condition)
                || needs_temporary_in_ast_expression(&cond.true_exp)
                || needs_temporary_in_ast_expression(&cond.false_exp)
        }
        // Property reads need to check their receiver
        AngularExpression::PropertyRead(prop) => needs_temporary_in_ast_expression(&prop.receiver),
        AngularExpression::SafePropertyRead(prop) => {
            needs_temporary_in_ast_expression(&prop.receiver)
        }
        AngularExpression::KeyedRead(keyed) => {
            needs_temporary_in_ast_expression(&keyed.receiver)
                || needs_temporary_in_ast_expression(&keyed.key)
        }
        AngularExpression::SafeKeyedRead(keyed) => {
            needs_temporary_in_ast_expression(&keyed.receiver)
                || needs_temporary_in_ast_expression(&keyed.key)
        }
        // Simple expressions don't need temporaries
        _ => false,
    }
}

/// Creates a SafeTernary with a temporary variable if needed.
///
/// If the guard expression has side effects, wraps it in an AssignTemporary
/// and uses ReadTemporary in the body.
fn safe_ternary_with_temporary<'a, F>(
    guard: IrExpression<'a>,
    body: F,
    ctx: &SafeTransformContext<'a>,
) -> SafeTernaryExpr<'a>
where
    F: FnOnce(IrExpression<'a>) -> IrExpression<'a>,
{
    let allocator = ctx.allocator;

    if needs_temporary_in_safe_access(&guard) {
        // Allocate a temporary variable
        let xref = ctx.allocate_xref_id();

        // Create: (tmp = guard, body(tmp))
        let assign_temp = IrExpression::AssignTemporary(ArenaBox::new_in(
            AssignTemporaryExpr {
                expr: ArenaBox::new_in(guard, allocator),
                xref,
                name: None, // Name is resolved in a later phase
                source_span: None,
            },
            allocator,
        ));

        let read_temp = IrExpression::ReadTemporary(ArenaBox::new_in(
            ReadTemporaryExpr {
                xref,
                name: None, // Name is resolved in a later phase
                source_span: None,
            },
            allocator,
        ));

        SafeTernaryExpr {
            guard: ArenaBox::new_in(assign_temp, allocator),
            expr: ArenaBox::new_in(body(read_temp), allocator),
            source_span: None,
        }
    } else {
        // Clone the guard for use in body
        let guard_clone = guard.clone_in(allocator);

        SafeTernaryExpr {
            guard: ArenaBox::new_in(guard, allocator),
            expr: ArenaBox::new_in(body(guard_clone), allocator),
            source_span: None,
        }
    }
}

/// Data extracted from an access expression for processing.
enum AccessInfo<'a> {
    /// Property read: `.name`
    PropertyRead { name: oxc_span::Atom<'a>, source_span: Option<Span> },
    /// Keyed read: `[key]`
    KeyedRead { key: IrExpression<'a>, source_span: Option<Span> },
    /// Function call: `(args)`
    Call { args: ArenaVec<'a, IrExpression<'a>>, source_span: Option<Span> },
}

/// Checks if an expression is a safe access expression.
fn is_safe_access_expression(e: &IrExpression<'_>) -> bool {
    matches!(
        e,
        IrExpression::SafePropertyRead(_)
            | IrExpression::SafeKeyedRead(_)
            | IrExpression::SafeInvokeFunction(_)
    )
}

/// Checks if the receiver of an expression is a SafeTernary.
fn has_safe_ternary_receiver(e: &IrExpression<'_>) -> bool {
    match e {
        IrExpression::SafePropertyRead(p) => {
            matches!(p.receiver.as_ref(), IrExpression::SafeTernary(_))
        }
        IrExpression::SafeKeyedRead(k) => {
            matches!(k.receiver.as_ref(), IrExpression::SafeTernary(_))
        }
        IrExpression::SafeInvokeFunction(c) => {
            matches!(c.receiver.as_ref(), IrExpression::SafeTernary(_))
        }
        IrExpression::ResolvedPropertyRead(p) => {
            matches!(p.receiver.as_ref(), IrExpression::SafeTernary(_))
        }
        IrExpression::ResolvedKeyedRead(k) => {
            matches!(k.receiver.as_ref(), IrExpression::SafeTernary(_))
        }
        IrExpression::ResolvedCall(c) => {
            matches!(c.receiver.as_ref(), IrExpression::SafeTernary(_))
        }
        _ => false,
    }
}

/// Creates an empty placeholder expression.
fn make_placeholder<'a>(allocator: &'a Allocator) -> IrExpression<'a> {
    IrExpression::Empty(ArenaBox::new_in(
        crate::ir::expression::EmptyExpr { source_span: None },
        allocator,
    ))
}

/// Extract access info and take the receiver from an expression.
fn extract_access_info<'a>(
    expr: &mut IrExpression<'a>,
    allocator: &'a Allocator,
) -> Option<(AccessInfo<'a>, IrExpression<'a>, bool)> {
    match expr {
        IrExpression::SafePropertyRead(p) => {
            let receiver = std::mem::replace(p.receiver.as_mut(), make_placeholder(allocator));
            Some((
                AccessInfo::PropertyRead { name: p.name.clone(), source_span: p.source_span },
                receiver,
                true, // is_safe
            ))
        }
        IrExpression::SafeKeyedRead(k) => {
            let receiver = std::mem::replace(k.receiver.as_mut(), make_placeholder(allocator));
            let key = std::mem::replace(k.index.as_mut(), make_placeholder(allocator));
            Some((AccessInfo::KeyedRead { key, source_span: k.source_span }, receiver, true))
        }
        IrExpression::SafeInvokeFunction(c) => {
            let receiver = std::mem::replace(c.receiver.as_mut(), make_placeholder(allocator));
            let mut args = ArenaVec::with_capacity_in(c.args.len(), allocator);
            for arg in c.args.iter() {
                args.push(arg.clone_in(allocator));
            }
            Some((AccessInfo::Call { args, source_span: c.source_span }, receiver, true))
        }
        IrExpression::ResolvedPropertyRead(p) => {
            let receiver = std::mem::replace(p.receiver.as_mut(), make_placeholder(allocator));
            Some((
                AccessInfo::PropertyRead { name: p.name.clone(), source_span: p.source_span },
                receiver,
                false, // is_safe
            ))
        }
        IrExpression::ResolvedKeyedRead(k) => {
            let receiver = std::mem::replace(k.receiver.as_mut(), make_placeholder(allocator));
            let key = k.key.clone_in(allocator);
            Some((AccessInfo::KeyedRead { key, source_span: k.source_span }, receiver, false))
        }
        IrExpression::ResolvedCall(c) => {
            let receiver = std::mem::replace(c.receiver.as_mut(), make_placeholder(allocator));
            let mut args = ArenaVec::with_capacity_in(c.args.len(), allocator);
            for arg in c.args.iter() {
                args.push(arg.clone_in(allocator));
            }
            Some((AccessInfo::Call { args, source_span: c.source_span }, receiver, false))
        }
        _ => None,
    }
}

/// Creates a resolved access expression from access info.
fn create_access_expr<'a>(
    receiver: IrExpression<'a>,
    info: AccessInfo<'a>,
    allocator: &'a Allocator,
) -> IrExpression<'a> {
    match info {
        AccessInfo::PropertyRead { name, source_span } => {
            IrExpression::ResolvedPropertyRead(ArenaBox::new_in(
                ResolvedPropertyReadExpr {
                    receiver: ArenaBox::new_in(receiver, allocator),
                    name,
                    source_span,
                },
                allocator,
            ))
        }
        AccessInfo::KeyedRead { key, source_span } => {
            IrExpression::ResolvedKeyedRead(ArenaBox::new_in(
                ResolvedKeyedReadExpr {
                    receiver: ArenaBox::new_in(receiver, allocator),
                    key: ArenaBox::new_in(key, allocator),
                    source_span,
                },
                allocator,
            ))
        }
        AccessInfo::Call { args, source_span } => IrExpression::ResolvedCall(ArenaBox::new_in(
            ResolvedCallExpr { receiver: ArenaBox::new_in(receiver, allocator), args, source_span },
            allocator,
        )),
    }
}

/// Counts the depth of nested SafeTernary expressions.
fn count_safe_ternary_depth(expr: &IrExpression<'_>) -> usize {
    let mut depth = 0;
    let mut current = expr;
    while let IrExpression::SafeTernary(st) = current {
        depth += 1;
        current = st.expr.as_ref();
    }
    depth
}

/// Navigates to the SafeTernary at the given depth and returns a mutable reference.
fn get_safe_ternary_at_depth<'a, 'b>(
    expr: &'b mut IrExpression<'a>,
    depth: usize,
) -> &'b mut SafeTernaryExpr<'a> {
    let mut current = expr;
    for _ in 0..depth - 1 {
        if let IrExpression::SafeTernary(st) = current {
            current = st.expr.as_mut();
        }
    }
    if let IrExpression::SafeTernary(st) = current {
        st.as_mut()
    } else {
        unreachable!("Expected SafeTernary at depth")
    }
}

/// Finds and modifies the deepest SafeTernary in a SafeTernary chain.
///
/// Takes the current `expr` of the deepest SafeTernary and replaces it with `new_expr`.
fn modify_deepest_safe_ternary<'a>(
    receiver: &mut IrExpression<'a>,
    new_expr: IrExpression<'a>,
    allocator: &'a Allocator,
) -> IrExpression<'a> {
    // First count the depth (immutable borrow)
    let depth = count_safe_ternary_depth(receiver);
    if depth == 0 {
        // This shouldn't happen if has_safe_ternary_receiver returned true
        return new_expr;
    }

    // Then navigate to the deepest SafeTernary (mutable borrow)
    let deepest = get_safe_ternary_at_depth(receiver, depth);

    // Take the current expr from the deepest SafeTernary
    let old_expr = std::mem::replace(deepest.expr.as_mut(), make_placeholder(allocator));

    // Replace with the new expression
    deepest.expr = ArenaBox::new_in(new_expr, allocator);

    old_expr
}

/// Transform safe access expressions to SafeTernary.
///
/// This transforms:
/// - `SafePropertyRead { receiver, name }` → `SafeTernary { guard: receiver, expr: ResolvedPropertyRead { receiver, name } }`
/// - `SafeKeyedRead { receiver, index }` → `SafeTernary { guard: receiver, expr: ResolvedKeyedRead { receiver, key } }`
/// - `SafeInvokeFunction { receiver, args }` → `SafeTernary { guard: receiver, expr: ResolvedCall { receiver, args } }`
///
/// When the receiver is already a SafeTernary (from a previous safe access), this function
/// modifies the deepest SafeTernary's expr in place instead of creating a new wrapper.
/// This produces the correct nested structure:
///
/// For `a?.b?.c`:
/// - After first transform: `SafeTernary { guard: a, expr: a.b }`
/// - After second transform: `SafeTernary { guard: a, expr: SafeTernary { guard: a.b, expr: a.b.c } }`
///
/// This is different from incorrectly wrapping the entire expression:
/// - Wrong: `SafeTernary { guard: SafeTernary{...}, expr: SafeTernary{...}.c }`
fn safe_transform<'a>(expr: &mut IrExpression<'a>, ctx: &SafeTransformContext<'a>) {
    let allocator = ctx.allocator;

    // Check if receiver is a SafeTernary first (before mutating)
    let receiver_is_safe_ternary = has_safe_ternary_receiver(expr);

    // Only process:
    // 1. Safe access expressions (SafePropertyRead, SafeKeyedRead, SafeInvokeFunction)
    // 2. Unsafe access expressions whose receiver is a SafeTernary
    if !is_safe_access_expression(expr) && !receiver_is_safe_ternary {
        return;
    }

    // Extract access info from the expression
    let Some((info, mut receiver, is_safe)) = extract_access_info(expr, allocator) else {
        return;
    };

    if receiver_is_safe_ternary {
        // The receiver is a SafeTernary - modify the deepest one in place
        // First, get what's currently in the deepest SafeTernary's expr (this will be the base for our access)
        // We use a placeholder to get the old expr out
        let placeholder = IrExpression::Empty(ArenaBox::new_in(
            crate::ir::expression::EmptyExpr { source_span: None },
            allocator,
        ));
        let dst_expr = modify_deepest_safe_ternary(&mut receiver, placeholder, allocator);

        if is_safe {
            // Safe access: wrap in a new SafeTernary
            let st = safe_ternary_with_temporary(
                dst_expr,
                |r| create_access_expr(r, info, allocator),
                ctx,
            );
            let new_inner = IrExpression::SafeTernary(ArenaBox::new_in(st, allocator));
            // Put the new SafeTernary back into the deepest slot
            modify_deepest_safe_ternary(&mut receiver, new_inner, allocator);
        } else {
            // Unsafe access: just add the access to dst.expr
            let new_access = create_access_expr(dst_expr, info, allocator);
            modify_deepest_safe_ternary(&mut receiver, new_access, allocator);
        }

        // Return the receiver (the outer SafeTernary)
        *expr = receiver;
    } else {
        // No SafeTernary in receiver, and this is a safe access - create new SafeTernary
        // (We know is_safe is true here because of the early return above)
        let st =
            safe_ternary_with_temporary(receiver, |r| create_access_expr(r, info, allocator), ctx);
        *expr = IrExpression::SafeTernary(ArenaBox::new_in(st, allocator));
    }
}

/// Expands safe reads for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
pub fn expand_safe_reads_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Get the current xref counter value
    let starting_xref = job.allocate_xref_id().0;
    let xref_counter = RefCell::new(starting_xref);
    let ctx = SafeTransformContext { allocator, next_xref: xref_counter };

    // Transform safe access expressions to SafeTernary
    for op in job.root.create.iter_mut() {
        transform_expressions_in_create_op(
            op,
            &|expr, _flags| {
                safe_transform(expr, &ctx);
            },
            VisitorContextFlag::NONE,
        );
    }
    for op in job.root.update.iter_mut() {
        transform_expressions_in_update_op(
            op,
            &|expr, _flags| {
                safe_transform(expr, &ctx);
            },
            VisitorContextFlag::NONE,
        );
    }
}

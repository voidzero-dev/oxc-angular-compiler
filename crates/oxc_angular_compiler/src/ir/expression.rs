//! IR expression types.
//!
//! These expressions represent logical operations in the intermediate representation
//! that are transformed during the compilation phases.
//!
//! Ported from Angular's `template/pipeline/ir/src/expression.ts`.

use oxc_allocator::{Box, Vec};
use oxc_span::{Atom, Span};

use super::enums::ExpressionKind;
use super::ops::{SlotId, XrefId};
use crate::ast::expression::AngularExpression;
use crate::pipeline::expression_store::ExpressionId;

/// Slot handle for runtime slot references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotHandle {
    /// The slot ID, if allocated.
    pub slot: Option<SlotId>,
}

impl SlotHandle {
    /// Creates a new unallocated slot handle.
    pub fn new() -> Self {
        Self { slot: None }
    }

    /// Creates a slot handle with an allocated slot.
    pub fn with_slot(slot: SlotId) -> Self {
        Self { slot: Some(slot) }
    }
}

impl Default for SlotHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Visitor context flags for expression transformation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisitorContextFlag(u8);

impl VisitorContextFlag {
    /// No flags.
    pub const NONE: Self = Self(0b0000);
    /// Inside a child operation.
    pub const IN_CHILD_OPERATION: Self = Self(0b0001);
    /// Inside an arrow function operation.
    pub const IN_ARROW_FUNCTION_OPERATION: Self = Self(0b0010);

    /// Check if a flag is set.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Combine flags.
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// IR expression union type.
///
/// These expressions are used in the intermediate representation and are
/// transformed during compilation phases before being converted to output AST.
#[derive(Debug)]
pub enum IrExpression<'a> {
    /// Lexical read of a variable name.
    LexicalRead(Box<'a, LexicalReadExpr<'a>>),
    /// Read a local reference.
    Reference(Box<'a, ReferenceExpr>),
    /// Reference to current view context.
    Context(Box<'a, ContextExpr>),
    /// Navigate to next view context.
    NextContext(Box<'a, NextContextExpr>),
    /// Snapshot current view.
    GetCurrentView(Box<'a, GetCurrentViewExpr>),
    /// Restore a snapshotted view.
    RestoreView(Box<'a, RestoreViewExpr<'a>>),
    /// Reset view after restore.
    ResetView(Box<'a, ResetViewExpr<'a>>),
    /// Read a declared variable.
    ReadVariable(Box<'a, ReadVariableExpr<'a>>),
    /// Pure function expression (memoized).
    PureFunction(Box<'a, PureFunctionExpr<'a>>),
    /// Parameter reference in pure function.
    PureFunctionParameter(Box<'a, PureFunctionParameterExpr>),
    /// Pipe binding expression.
    PipeBinding(Box<'a, PipeBindingExpr<'a>>),
    /// Variadic pipe binding.
    PipeBindingVariadic(Box<'a, PipeBindingVariadicExpr<'a>>),
    /// Safe property read (obj?.prop).
    SafePropertyRead(Box<'a, SafePropertyReadExpr<'a>>),
    /// Safe keyed read (obj?.[key]).
    SafeKeyedRead(Box<'a, SafeKeyedReadExpr<'a>>),
    /// Safe function invocation (fn?.()).
    SafeInvokeFunction(Box<'a, SafeInvokeFunctionExpr<'a>>),
    /// Safe ternary expression.
    SafeTernary(Box<'a, SafeTernaryExpr<'a>>),
    /// Empty expression placeholder.
    Empty(Box<'a, EmptyExpr>),
    /// Assign to a temporary variable.
    AssignTemporary(Box<'a, AssignTemporaryExpr<'a>>),
    /// Read a temporary variable.
    ReadTemporary(Box<'a, ReadTemporaryExpr<'a>>),
    /// Slot literal for runtime.
    SlotLiteral(Box<'a, SlotLiteralExpr>),
    /// Conditional case expression.
    ConditionalCase(Box<'a, ConditionalCaseExpr<'a>>),
    /// Const-collected expression.
    ConstCollected(Box<'a, ConstCollectedExpr<'a>>),
    /// Two-way binding set expression.
    TwoWayBindingSet(Box<'a, TwoWayBindingSetExpr<'a>>),
    /// Reference to a const array index.
    ConstReference(Box<'a, ConstReferenceExpr>),
    /// Context @let reference.
    ContextLetReference(Box<'a, ContextLetReferenceExpr>),
    /// Store @let value.
    StoreLet(Box<'a, StoreLetExpr<'a>>),
    /// Track context for @for loop.
    TrackContext(Box<'a, TrackContextExpr>),
    /// Binary operator expression (for computed @for variables like $first, $last, etc.).
    Binary(Box<'a, BinaryExpr<'a>>),
    /// Ternary conditional expression (condition ? true_expr : false_expr).
    /// Used to hold conditional logic with IR children so slots can be updated after allocation.
    Ternary(Box<'a, TernaryExpr<'a>>),
    /// Interpolation expression.
    Interpolation(Box<'a, Interpolation<'a>>),
    /// Wrapped AST expression (from parsing, to be transformed by phases).
    Ast(Box<'a, AngularExpression<'a>>),
    /// Reference to an expression in the ExpressionStore.
    /// This is the preferred way to reference expressions in IR operations,
    /// avoiding the need for cloning.
    ExpressionRef(ExpressionId),
    /// An output AST expression (already in output format).
    /// Used for host attributes and other pre-computed expressions.
    OutputExpr(Box<'a, crate::output::ast::OutputExpression<'a>>),
    /// Property read with resolved receiver (created during name resolution).
    /// Used when an inner expression like `item` in `item.name` is resolved to a variable.
    ResolvedPropertyRead(Box<'a, ResolvedPropertyReadExpr<'a>>),
    /// Binary expression with resolved sub-expressions (created during name resolution).
    /// Used when a binary expression (e.g., assignment in event handler) has resolved variables.
    ResolvedBinary(Box<'a, ResolvedBinaryExpr<'a>>),
    /// Function call with resolved receiver and/or arguments (created during name resolution).
    /// Used when a call like `removeTodo(todo)` has arguments resolved to variables in scope.
    ResolvedCall(Box<'a, ResolvedCallExpr<'a>>),
    /// Keyed read with resolved receiver (created during name resolution).
    /// Used when an expression like `item[0]` has the receiver resolved to a variable.
    ResolvedKeyedRead(Box<'a, ResolvedKeyedReadExpr<'a>>),
    /// Safe property read with resolved receiver (created during name resolution).
    /// Used when an expression like `item?.name` has the receiver resolved to a variable.
    ResolvedSafePropertyRead(Box<'a, ResolvedSafePropertyReadExpr<'a>>),
    /// Derived literal array for pure function bodies.
    /// Contains IrExpression entries that can include PureFunctionParameter references.
    DerivedLiteralArray(Box<'a, DerivedLiteralArrayExpr<'a>>),
    /// Derived literal map for pure function bodies.
    /// Contains IrExpression values that can include PureFunctionParameter references.
    DerivedLiteralMap(Box<'a, DerivedLiteralMapExpr<'a>>),
    /// Literal array with IR expression elements.
    /// Used to preserve pipe bindings inside array literals during ingest.
    LiteralArray(Box<'a, IrLiteralArrayExpr<'a>>),
    /// Literal map (object) with IR expression values.
    /// Used to preserve pipe bindings inside object literals during ingest.
    LiteralMap(Box<'a, IrLiteralMapExpr<'a>>),
    /// Logical NOT expression (!expr).
    /// Used to preserve pipe bindings inside negation expressions during ingest.
    Not(Box<'a, NotExpr<'a>>),
    /// Unary operator expression (+expr or -expr).
    /// Used to preserve pipe bindings inside unary expressions during ingest.
    Unary(Box<'a, UnaryExpr<'a>>),
    /// Typeof expression (typeof expr).
    /// Used to preserve pipe bindings inside typeof expressions during ingest.
    Typeof(Box<'a, TypeofExpr<'a>>),
    /// Void expression (void expr).
    /// Used to preserve pipe bindings inside void expressions during ingest.
    Void(Box<'a, VoidExpr<'a>>),
    /// Template literal with resolved expressions (created during name resolution).
    /// Used when a template literal like `\`bwi ${menuItem.icon}\`` has expressions
    /// that need to be resolved to variables in scope.
    ResolvedTemplateLiteral(Box<'a, ResolvedTemplateLiteralExpr<'a>>),
    /// Arrow function expression.
    /// Created by the generateArrowFunctions phase to wrap user-defined arrow functions.
    ArrowFunction(Box<'a, ArrowFunctionExpr<'a>>),
    /// Parenthesized expression wrapper.
    /// Preserves grouping parentheses from source to affect safe navigation precedence.
    Parenthesized(Box<'a, IrParenthesizedExpr<'a>>),
}

impl<'a> IrExpression<'a> {
    /// Returns the expression kind.
    pub fn kind(&self) -> ExpressionKind {
        match self {
            IrExpression::LexicalRead(_) => ExpressionKind::LexicalRead,
            IrExpression::Reference(_) => ExpressionKind::Reference,
            IrExpression::Context(_) => ExpressionKind::Context,
            IrExpression::NextContext(_) => ExpressionKind::NextContext,
            IrExpression::GetCurrentView(_) => ExpressionKind::GetCurrentView,
            IrExpression::RestoreView(_) => ExpressionKind::RestoreView,
            IrExpression::ResetView(_) => ExpressionKind::ResetView,
            IrExpression::ReadVariable(_) => ExpressionKind::ReadVariable,
            IrExpression::PureFunction(_) => ExpressionKind::PureFunctionExpr,
            IrExpression::PureFunctionParameter(_) => ExpressionKind::PureFunctionParameterExpr,
            IrExpression::PipeBinding(_) => ExpressionKind::PipeBinding,
            IrExpression::PipeBindingVariadic(_) => ExpressionKind::PipeBindingVariadic,
            IrExpression::SafePropertyRead(_) => ExpressionKind::SafePropertyRead,
            IrExpression::SafeKeyedRead(_) => ExpressionKind::SafeKeyedRead,
            IrExpression::SafeInvokeFunction(_) => ExpressionKind::SafeInvokeFunction,
            IrExpression::SafeTernary(_) => ExpressionKind::SafeTernaryExpr,
            IrExpression::Empty(_) => ExpressionKind::EmptyExpr,
            IrExpression::AssignTemporary(_) => ExpressionKind::AssignTemporaryExpr,
            IrExpression::ReadTemporary(_) => ExpressionKind::ReadTemporaryExpr,
            IrExpression::SlotLiteral(_) => ExpressionKind::SlotLiteralExpr,
            IrExpression::ConditionalCase(_) => ExpressionKind::ConditionalCase,
            IrExpression::ConstCollected(_) => ExpressionKind::ConstCollected,
            IrExpression::ConstReference(_) => ExpressionKind::ConstReference,
            IrExpression::TwoWayBindingSet(_) => ExpressionKind::TwoWayBindingSet,
            IrExpression::ContextLetReference(_) => ExpressionKind::ContextLetReference,
            IrExpression::StoreLet(_) => ExpressionKind::StoreLet,
            IrExpression::TrackContext(_) => ExpressionKind::TrackContext,
            IrExpression::Binary(_) => ExpressionKind::Binary,
            IrExpression::Ternary(_) => ExpressionKind::Ternary,
            IrExpression::Interpolation(_) => ExpressionKind::Interpolation,
            // Ast expressions don't have a specific kind - they're transformed away
            IrExpression::Ast(_) => ExpressionKind::LexicalRead, // Placeholder, will be transformed
            // ExpressionRef references a stored expression - the kind comes from the stored expression
            IrExpression::ExpressionRef(_) => ExpressionKind::LexicalRead, // Placeholder, resolved at reify
            // OutputExpr is already an output expression - no IR kind needed
            IrExpression::OutputExpr(_) => ExpressionKind::LexicalRead, // Placeholder, already output
            // ResolvedPropertyRead is a property read with resolved receiver
            IrExpression::ResolvedPropertyRead(_) => ExpressionKind::ResolvedPropertyRead,
            // ResolvedBinary is a binary expression with resolved sub-expressions
            IrExpression::ResolvedBinary(_) => ExpressionKind::ResolvedBinary,
            // ResolvedCall is a function call with resolved receiver/arguments
            IrExpression::ResolvedCall(_) => ExpressionKind::ResolvedCall,
            // ResolvedKeyedRead is a keyed read with resolved receiver
            IrExpression::ResolvedKeyedRead(_) => ExpressionKind::ResolvedKeyedRead,
            // ResolvedSafePropertyRead is a safe property read with resolved receiver
            IrExpression::ResolvedSafePropertyRead(_) => ExpressionKind::ResolvedSafePropertyRead,
            // DerivedLiteralArray is a literal array for pure function bodies
            IrExpression::DerivedLiteralArray(_) => ExpressionKind::DerivedLiteralArray,
            // DerivedLiteralMap is a literal map for pure function bodies
            IrExpression::DerivedLiteralMap(_) => ExpressionKind::DerivedLiteralMap,
            // LiteralArray is an array literal with IR expression elements
            IrExpression::LiteralArray(_) => ExpressionKind::LiteralArray,
            // LiteralMap is an object literal with IR expression values
            IrExpression::LiteralMap(_) => ExpressionKind::LiteralMap,
            // Not is a logical NOT expression
            IrExpression::Not(_) => ExpressionKind::Not,
            // Unary is a unary operator expression (+/-)
            IrExpression::Unary(_) => ExpressionKind::Unary,
            // Typeof is a typeof expression
            IrExpression::Typeof(_) => ExpressionKind::Typeof,
            // Void is a void expression
            IrExpression::Void(_) => ExpressionKind::Void,
            // ResolvedTemplateLiteral is a template literal with resolved expressions
            IrExpression::ResolvedTemplateLiteral(_) => ExpressionKind::ResolvedTemplateLiteral,
            // ArrowFunction is an arrow function expression
            IrExpression::ArrowFunction(_) => ExpressionKind::ArrowFunction,
            IrExpression::Parenthesized(_) => ExpressionKind::Parenthesized,
        }
    }
}

impl<'a> IrExpression<'a> {
    /// Creates a new Ast expression wrapping an AngularExpression.
    pub fn from_ast(allocator: &'a oxc_allocator::Allocator, expr: AngularExpression<'a>) -> Self {
        IrExpression::Ast(Box::new_in(expr, allocator))
    }

    /// Creates a new Empty expression.
    pub fn empty(allocator: &'a oxc_allocator::Allocator, source_span: Option<Span>) -> Self {
        IrExpression::Empty(Box::new_in(EmptyExpr { source_span }, allocator))
    }

    /// Returns true if this is an empty expression.
    pub fn is_empty(&self) -> bool {
        matches!(self, IrExpression::Empty(_))
    }

    /// Returns true if this is an Ast expression.
    pub fn is_ast(&self) -> bool {
        matches!(self, IrExpression::Ast(_))
    }

    /// Returns true if this is an ExpressionRef.
    pub fn is_expression_ref(&self) -> bool {
        matches!(self, IrExpression::ExpressionRef(_))
    }

    /// Creates an ExpressionRef from an ExpressionId.
    pub fn from_ref(id: ExpressionId) -> Self {
        IrExpression::ExpressionRef(id)
    }

    /// Returns the ExpressionId if this is an ExpressionRef.
    pub fn as_expression_ref(&self) -> Option<ExpressionId> {
        match self {
            IrExpression::ExpressionRef(id) => Some(*id),
            _ => None,
        }
    }

    /// Clones this expression into a new allocator.
    ///
    /// This is the arena-safe cloning method required for expression transformations
    /// that need to duplicate expressions (e.g., safe navigation expansion).
    pub fn clone_in(&self, allocator: &'a oxc_allocator::Allocator) -> Self {
        match self {
            IrExpression::LexicalRead(e) => IrExpression::LexicalRead(Box::new_in(
                LexicalReadExpr { name: e.name.clone(), source_span: e.source_span },
                allocator,
            )),
            IrExpression::Reference(e) => IrExpression::Reference(Box::new_in(
                ReferenceExpr {
                    target: e.target,
                    target_slot: e.target_slot,
                    offset: e.offset,
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Context(e) => IrExpression::Context(Box::new_in(
                ContextExpr { view: e.view, source_span: e.source_span },
                allocator,
            )),
            IrExpression::NextContext(e) => IrExpression::NextContext(Box::new_in(
                NextContextExpr { steps: e.steps, source_span: e.source_span },
                allocator,
            )),
            IrExpression::GetCurrentView(e) => IrExpression::GetCurrentView(Box::new_in(
                GetCurrentViewExpr { source_span: e.source_span },
                allocator,
            )),
            IrExpression::RestoreView(e) => IrExpression::RestoreView(Box::new_in(
                RestoreViewExpr {
                    view: match &e.view {
                        RestoreViewTarget::Static(xref) => RestoreViewTarget::Static(*xref),
                        RestoreViewTarget::Dynamic(expr) => RestoreViewTarget::Dynamic(
                            Box::new_in(expr.clone_in(allocator), allocator),
                        ),
                    },
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ResetView(e) => IrExpression::ResetView(Box::new_in(
                ResetViewExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ReadVariable(e) => IrExpression::ReadVariable(Box::new_in(
                ReadVariableExpr { xref: e.xref, name: e.name.clone(), source_span: e.source_span },
                allocator,
            )),
            IrExpression::PureFunction(e) => {
                let body = e.body.as_ref().map(|b| Box::new_in(b.clone_in(allocator), allocator));
                let fn_ref =
                    e.fn_ref.as_ref().map(|f| Box::new_in(f.clone_in(allocator), allocator));
                let mut args = Vec::with_capacity_in(e.args.len(), allocator);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(allocator));
                }
                IrExpression::PureFunction(Box::new_in(
                    PureFunctionExpr {
                        body,
                        args,
                        fn_ref,
                        var_offset: e.var_offset,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::PureFunctionParameter(e) => {
                IrExpression::PureFunctionParameter(Box::new_in(
                    PureFunctionParameterExpr { index: e.index, source_span: e.source_span },
                    allocator,
                ))
            }
            IrExpression::PipeBinding(e) => {
                let mut args = Vec::with_capacity_in(e.args.len(), allocator);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(allocator));
                }
                IrExpression::PipeBinding(Box::new_in(
                    PipeBindingExpr {
                        target: e.target,
                        target_slot: e.target_slot,
                        name: e.name.clone(),
                        args,
                        var_offset: e.var_offset,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::PipeBindingVariadic(e) => IrExpression::PipeBindingVariadic(Box::new_in(
                PipeBindingVariadicExpr {
                    target: e.target,
                    target_slot: e.target_slot,
                    name: e.name.clone(),
                    args: Box::new_in(e.args.clone_in(allocator), allocator),
                    num_args: e.num_args,
                    var_offset: e.var_offset,
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::SafePropertyRead(e) => IrExpression::SafePropertyRead(Box::new_in(
                SafePropertyReadExpr {
                    receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                    name: e.name.clone(),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::SafeKeyedRead(e) => IrExpression::SafeKeyedRead(Box::new_in(
                SafeKeyedReadExpr {
                    receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                    index: Box::new_in(e.index.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::SafeInvokeFunction(e) => {
                let mut args = Vec::with_capacity_in(e.args.len(), allocator);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(allocator));
                }
                IrExpression::SafeInvokeFunction(Box::new_in(
                    SafeInvokeFunctionExpr {
                        receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                        args,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::SafeTernary(e) => IrExpression::SafeTernary(Box::new_in(
                SafeTernaryExpr {
                    guard: Box::new_in(e.guard.clone_in(allocator), allocator),
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Empty(e) => IrExpression::Empty(Box::new_in(
                EmptyExpr { source_span: e.source_span },
                allocator,
            )),
            IrExpression::AssignTemporary(e) => IrExpression::AssignTemporary(Box::new_in(
                AssignTemporaryExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    xref: e.xref,
                    name: e.name.clone(),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ReadTemporary(e) => IrExpression::ReadTemporary(Box::new_in(
                ReadTemporaryExpr {
                    xref: e.xref,
                    name: e.name.clone(),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::SlotLiteral(e) => IrExpression::SlotLiteral(Box::new_in(
                SlotLiteralExpr {
                    slot: e.slot,
                    target_xref: e.target_xref,
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ConditionalCase(e) => IrExpression::ConditionalCase(Box::new_in(
                ConditionalCaseExpr {
                    expr: e.expr.as_ref().map(|ex| Box::new_in(ex.clone_in(allocator), allocator)),
                    target: e.target,
                    target_slot: e.target_slot,
                    alias: e.alias.clone(),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ConstCollected(e) => IrExpression::ConstCollected(Box::new_in(
                ConstCollectedExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ConstReference(e) => IrExpression::ConstReference(Box::new_in(
                ConstReferenceExpr { index: e.index, source_span: e.source_span },
                allocator,
            )),
            IrExpression::TwoWayBindingSet(e) => IrExpression::TwoWayBindingSet(Box::new_in(
                TwoWayBindingSetExpr {
                    target: Box::new_in(e.target.clone_in(allocator), allocator),
                    value: Box::new_in(e.value.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ContextLetReference(e) => IrExpression::ContextLetReference(Box::new_in(
                ContextLetReferenceExpr {
                    target: e.target,
                    target_slot: e.target_slot,
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::StoreLet(e) => IrExpression::StoreLet(Box::new_in(
                StoreLetExpr {
                    target: e.target,
                    value: Box::new_in(e.value.clone_in(allocator), allocator),
                    var_offset: e.var_offset,
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::TrackContext(e) => IrExpression::TrackContext(Box::new_in(
                TrackContextExpr { view: e.view, source_span: e.source_span },
                allocator,
            )),
            IrExpression::Binary(e) => IrExpression::Binary(Box::new_in(
                BinaryExpr {
                    operator: e.operator,
                    lhs: Box::new_in(e.lhs.clone_in(allocator), allocator),
                    rhs: Box::new_in(e.rhs.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Ternary(e) => IrExpression::Ternary(Box::new_in(
                TernaryExpr {
                    condition: Box::new_in(e.condition.clone_in(allocator), allocator),
                    true_expr: Box::new_in(e.true_expr.clone_in(allocator), allocator),
                    false_expr: Box::new_in(e.false_expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Interpolation(e) => {
                let mut strings = Vec::with_capacity_in(e.strings.len(), allocator);
                for s in e.strings.iter() {
                    strings.push(s.clone());
                }
                let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(allocator));
                }
                let mut i18n_placeholders =
                    Vec::with_capacity_in(e.i18n_placeholders.len(), allocator);
                for ph in e.i18n_placeholders.iter() {
                    i18n_placeholders.push(ph.clone());
                }
                IrExpression::Interpolation(Box::new_in(
                    Interpolation {
                        strings,
                        expressions,
                        i18n_placeholders,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::Ast(ast_expr) => {
                // For AST expressions, we need to clone the AngularExpression
                // This is complex as AngularExpression has many variants
                // For now, we'll mark this as needing implementation
                // A full implementation would clone the entire AngularExpression tree
                IrExpression::Ast(Box::new_in(
                    clone_angular_expression(ast_expr, allocator),
                    allocator,
                ))
            }
            IrExpression::ExpressionRef(id) => IrExpression::ExpressionRef(*id),
            IrExpression::OutputExpr(e) => {
                // OutputExpr contains an already-converted output expression.
                // We need to clone the OutputExpression, which requires its own clone implementation.
                // For now, we'll use the output expression's CloneIn trait.
                IrExpression::OutputExpr(Box::new_in(e.clone_in(allocator), allocator))
            }
            IrExpression::ResolvedPropertyRead(e) => {
                IrExpression::ResolvedPropertyRead(Box::new_in(
                    ResolvedPropertyReadExpr {
                        receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                        name: e.name.clone(),
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::ResolvedBinary(e) => IrExpression::ResolvedBinary(Box::new_in(
                ResolvedBinaryExpr {
                    operator: e.operator,
                    left: Box::new_in(e.left.clone_in(allocator), allocator),
                    right: Box::new_in(e.right.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ResolvedCall(e) => {
                let mut args = Vec::with_capacity_in(e.args.len(), allocator);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(allocator));
                }
                IrExpression::ResolvedCall(Box::new_in(
                    ResolvedCallExpr {
                        receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                        args,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::ResolvedKeyedRead(e) => IrExpression::ResolvedKeyedRead(Box::new_in(
                ResolvedKeyedReadExpr {
                    receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                    key: Box::new_in(e.key.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ResolvedSafePropertyRead(e) => {
                IrExpression::ResolvedSafePropertyRead(Box::new_in(
                    ResolvedSafePropertyReadExpr {
                        receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                        name: e.name.clone(),
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::DerivedLiteralArray(e) => {
                let mut entries = Vec::with_capacity_in(e.entries.len(), allocator);
                for entry in e.entries.iter() {
                    entries.push(entry.clone_in(allocator));
                }
                IrExpression::DerivedLiteralArray(Box::new_in(
                    DerivedLiteralArrayExpr { entries, source_span: e.source_span },
                    allocator,
                ))
            }
            IrExpression::DerivedLiteralMap(e) => {
                let mut keys = Vec::with_capacity_in(e.keys.len(), allocator);
                for key in e.keys.iter() {
                    keys.push(key.clone());
                }
                let mut values = Vec::with_capacity_in(e.values.len(), allocator);
                for value in e.values.iter() {
                    values.push(value.clone_in(allocator));
                }
                let mut quoted = Vec::with_capacity_in(e.quoted.len(), allocator);
                for q in e.quoted.iter() {
                    quoted.push(*q);
                }
                IrExpression::DerivedLiteralMap(Box::new_in(
                    DerivedLiteralMapExpr { keys, values, quoted, source_span: e.source_span },
                    allocator,
                ))
            }
            IrExpression::LiteralArray(e) => {
                let mut elements = Vec::with_capacity_in(e.elements.len(), allocator);
                for elem in e.elements.iter() {
                    elements.push(elem.clone_in(allocator));
                }
                IrExpression::LiteralArray(Box::new_in(
                    IrLiteralArrayExpr { elements, source_span: e.source_span },
                    allocator,
                ))
            }
            IrExpression::LiteralMap(e) => {
                let mut keys = Vec::with_capacity_in(e.keys.len(), allocator);
                for key in e.keys.iter() {
                    keys.push(key.clone());
                }
                let mut values = Vec::with_capacity_in(e.values.len(), allocator);
                for value in e.values.iter() {
                    values.push(value.clone_in(allocator));
                }
                let mut quoted = Vec::with_capacity_in(e.quoted.len(), allocator);
                for q in e.quoted.iter() {
                    quoted.push(*q);
                }
                IrExpression::LiteralMap(Box::new_in(
                    IrLiteralMapExpr { keys, values, quoted, source_span: e.source_span },
                    allocator,
                ))
            }
            IrExpression::Not(e) => IrExpression::Not(Box::new_in(
                NotExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Unary(e) => IrExpression::Unary(Box::new_in(
                UnaryExpr {
                    operator: e.operator,
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Typeof(e) => IrExpression::Typeof(Box::new_in(
                TypeofExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::Void(e) => IrExpression::Void(Box::new_in(
                VoidExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            IrExpression::ResolvedTemplateLiteral(e) => {
                let mut elements = Vec::with_capacity_in(e.elements.len(), allocator);
                for elem in e.elements.iter() {
                    elements.push(IrTemplateLiteralElement {
                        text: elem.text.clone(),
                        source_span: elem.source_span,
                    });
                }
                let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(allocator));
                }
                IrExpression::ResolvedTemplateLiteral(Box::new_in(
                    ResolvedTemplateLiteralExpr {
                        elements,
                        expressions,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::ArrowFunction(e) => {
                let mut params = Vec::with_capacity_in(e.params.len(), allocator);
                for param in e.params.iter() {
                    params.push(crate::output::ast::FnParam { name: param.name.clone() });
                }
                IrExpression::ArrowFunction(Box::new_in(
                    ArrowFunctionExpr {
                        params,
                        body: Box::new_in(e.body.clone_in(allocator), allocator),
                        // ops are not cloned as they are transient data added during compilation
                        ops: Vec::new_in(allocator),
                        var_offset: e.var_offset,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            IrExpression::Parenthesized(e) => IrExpression::Parenthesized(Box::new_in(
                IrParenthesizedExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
        }
    }
}

// ============================================================================
// Expression Types
// ============================================================================

/// Parenthesized expression wrapper.
///
/// Preserves grouping parentheses from the source template to maintain correct
/// operator precedence during safe navigation expansion.
///
/// Ported from Angular's `o.ParenthesizedExpr` in `expression.ts`.
#[derive(Debug)]
pub struct IrParenthesizedExpr<'a> {
    /// The inner expression.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Lexical read of a variable name.
#[derive(Debug)]
pub struct LexicalReadExpr<'a> {
    /// Variable name to read.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Read a local reference (#ref).
#[derive(Debug)]
pub struct ReferenceExpr {
    /// Target element XrefId.
    pub target: XrefId,
    /// Target slot handle.
    pub target_slot: SlotHandle,
    /// Offset for the reference.
    pub offset: i32,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Reference to current view context (ctx).
#[derive(Debug)]
pub struct ContextExpr {
    /// View XrefId.
    pub view: XrefId,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Reference to current view context in track function.
#[derive(Debug)]
pub struct TrackContextExpr {
    /// View XrefId.
    pub view: XrefId,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Navigate to next view context.
#[derive(Debug)]
pub struct NextContextExpr {
    /// Number of steps to navigate.
    pub steps: u32,
    /// Source span.
    pub source_span: Option<Span>,
}

impl Default for NextContextExpr {
    fn default() -> Self {
        Self { steps: 1, source_span: None }
    }
}

/// Snapshot current view for later restoration.
#[derive(Debug)]
pub struct GetCurrentViewExpr {
    /// Source span.
    pub source_span: Option<Span>,
}

/// Restore a snapshotted view.
#[derive(Debug)]
pub struct RestoreViewExpr<'a> {
    /// View to restore (XrefId or expression).
    pub view: RestoreViewTarget<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Target for RestoreView - either a static XrefId or a dynamic expression.
#[derive(Debug)]
pub enum RestoreViewTarget<'a> {
    /// Static XrefId.
    Static(XrefId),
    /// Dynamic expression.
    Dynamic(Box<'a, IrExpression<'a>>),
}

/// Reset view after RestoreView.
#[derive(Debug)]
pub struct ResetViewExpr<'a> {
    /// Expression to evaluate.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Read a declared variable (ir.VariableOp).
#[derive(Debug)]
pub struct ReadVariableExpr<'a> {
    /// Variable XrefId.
    pub xref: XrefId,
    /// Resolved variable name.
    pub name: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Property read with resolved receiver.
///
/// Created during name resolution when an expression like `item.name` is resolved
/// and the inner `item` is found to be a variable in scope. The receiver becomes
/// a `ReadVariable` expression and the property name is preserved.
#[derive(Debug)]
pub struct ResolvedPropertyReadExpr<'a> {
    /// The resolved receiver expression (e.g., ReadVariable for a loop variable).
    pub receiver: Box<'a, IrExpression<'a>>,
    /// Property name to read.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Binary expression with resolved sub-expressions.
///
/// Created during name resolution when a binary expression (especially assignments
/// in event handlers like `todo.done = $event`) contains variable references that
/// are resolved to ReadVariable expressions.
#[derive(Debug)]
pub struct ResolvedBinaryExpr<'a> {
    /// The binary operator (e.g., Assign for =, AddAssign for +=).
    pub operator: crate::ast::expression::BinaryOperator,
    /// Left-hand side expression (resolved, e.g., ResolvedPropertyRead).
    pub left: Box<'a, IrExpression<'a>>,
    /// Right-hand side expression (resolved or original).
    pub right: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Function call with resolved receiver and/or arguments.
///
/// Created during name resolution when a function call like `removeTodo(todo)` in an
/// event handler has arguments that need to be resolved to variables in scope.
/// This preserves the call structure while allowing resolved arguments.
#[derive(Debug)]
pub struct ResolvedCallExpr<'a> {
    /// The receiver/function expression (resolved or original).
    pub receiver: Box<'a, IrExpression<'a>>,
    /// The call arguments (resolved or original).
    pub args: Vec<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Keyed read with resolved receiver.
///
/// Created during name resolution when an expression like `item[0]` is resolved
/// and the inner `item` is found to be a variable in scope. The receiver becomes
/// a `ReadVariable` expression and the key is preserved.
#[derive(Debug)]
pub struct ResolvedKeyedReadExpr<'a> {
    /// The resolved receiver expression (e.g., ReadVariable for a loop variable).
    pub receiver: Box<'a, IrExpression<'a>>,
    /// The key expression (original, e.g., a number or expression).
    pub key: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Safe property read with resolved receiver.
///
/// Created during name resolution when an expression like `item?.name` is resolved
/// and the inner `item` is found to be a variable in scope. The receiver becomes
/// a `ReadVariable` expression and the property name is preserved.
#[derive(Debug)]
pub struct ResolvedSafePropertyReadExpr<'a> {
    /// The resolved receiver expression (e.g., ReadVariable for a loop variable).
    pub receiver: Box<'a, IrExpression<'a>>,
    /// Property name to read.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Template literal with resolved expressions.
///
/// Created during name resolution when a template literal like `\`bwi ${menuItem.icon}\``
/// has expressions that reference variables in scope. The expressions are resolved to
/// `ReadVariable` or `ResolvedPropertyRead` expressions.
#[derive(Debug)]
pub struct ResolvedTemplateLiteralExpr<'a> {
    /// Template literal text elements (the static parts between expressions).
    pub elements: Vec<'a, IrTemplateLiteralElement<'a>>,
    /// Resolved expressions (the dynamic parts inside ${...}).
    pub expressions: Vec<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Template literal element (text part) for IR expressions.
#[derive(Debug, Clone)]
pub struct IrTemplateLiteralElement<'a> {
    /// The text content.
    pub text: Atom<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Derived literal array for pure function bodies.
/// This is used when a literal array contains non-constant entries that need
/// to be replaced with PureFunctionParameter references.
#[derive(Debug)]
pub struct DerivedLiteralArrayExpr<'a> {
    /// Array entries - can be Ast (constants) or PureFunctionParameter (refs).
    pub entries: Vec<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Derived literal map for pure function bodies.
/// This is used when a literal map contains non-constant values that need
/// to be replaced with PureFunctionParameter references.
#[derive(Debug)]
pub struct DerivedLiteralMapExpr<'a> {
    /// Map keys (string keys from the original literal map).
    pub keys: Vec<'a, Atom<'a>>,
    /// Map values - can be Ast (constants) or PureFunctionParameter (refs).
    pub values: Vec<'a, IrExpression<'a>>,
    /// Whether each key is quoted.
    pub quoted: Vec<'a, bool>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Literal array with IR expression elements.
/// Used during ingest to preserve pipe bindings inside array literals.
#[derive(Debug)]
pub struct IrLiteralArrayExpr<'a> {
    /// Array elements as IR expressions.
    pub elements: Vec<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Literal map (object) with IR expression values.
/// Used during ingest to preserve pipe bindings inside object literals.
#[derive(Debug)]
pub struct IrLiteralMapExpr<'a> {
    /// Map keys (string keys from the original literal map).
    pub keys: Vec<'a, Atom<'a>>,
    /// Map values as IR expressions.
    pub values: Vec<'a, IrExpression<'a>>,
    /// Whether each key is quoted.
    pub quoted: Vec<'a, bool>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Pure function expression (memoized computation).
#[derive(Debug)]
pub struct PureFunctionExpr<'a> {
    /// Body expression to memoize.
    pub body: Option<Box<'a, IrExpression<'a>>>,
    /// Arguments that act as memoization keys.
    pub args: Vec<'a, IrExpression<'a>>,
    /// Reference to extracted function (after extraction phase).
    pub fn_ref: Option<Box<'a, IrExpression<'a>>>,
    /// Variable offset.
    pub var_offset: Option<u32>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Parameter reference in a pure function.
#[derive(Debug)]
pub struct PureFunctionParameterExpr {
    /// Parameter index.
    pub index: u32,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Pipe binding expression.
#[derive(Debug)]
pub struct PipeBindingExpr<'a> {
    /// Target pipe XrefId.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotHandle,
    /// Pipe name.
    pub name: Atom<'a>,
    /// Pipe arguments.
    pub args: Vec<'a, IrExpression<'a>>,
    /// Variable offset.
    pub var_offset: Option<u32>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Variadic pipe binding (for pipes with many arguments).
#[derive(Debug)]
pub struct PipeBindingVariadicExpr<'a> {
    /// Target pipe XrefId.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotHandle,
    /// Pipe name.
    pub name: Atom<'a>,
    /// Arguments as an array expression.
    pub args: Box<'a, IrExpression<'a>>,
    /// Number of arguments.
    pub num_args: u32,
    /// Variable offset.
    pub var_offset: Option<u32>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Safe property read (obj?.prop).
#[derive(Debug)]
pub struct SafePropertyReadExpr<'a> {
    /// Receiver expression.
    pub receiver: Box<'a, IrExpression<'a>>,
    /// Property name.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Safe keyed read (obj?.[key]).
#[derive(Debug)]
pub struct SafeKeyedReadExpr<'a> {
    /// Receiver expression.
    pub receiver: Box<'a, IrExpression<'a>>,
    /// Key expression.
    pub index: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Safe function invocation (fn?.()).
#[derive(Debug)]
pub struct SafeInvokeFunctionExpr<'a> {
    /// Receiver expression.
    pub receiver: Box<'a, IrExpression<'a>>,
    /// Function arguments.
    pub args: Vec<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Safe ternary expression (guard ? expr : null).
#[derive(Debug)]
pub struct SafeTernaryExpr<'a> {
    /// Guard expression.
    pub guard: Box<'a, IrExpression<'a>>,
    /// Expression to evaluate if guard is truthy.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Empty expression placeholder.
#[derive(Debug)]
pub struct EmptyExpr {
    /// Source span.
    pub source_span: Option<Span>,
}

/// Assign to a temporary variable.
#[derive(Debug)]
pub struct AssignTemporaryExpr<'a> {
    /// Expression to assign.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Temporary variable XrefId.
    pub xref: XrefId,
    /// Resolved variable name.
    pub name: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Read a temporary variable.
#[derive(Debug)]
pub struct ReadTemporaryExpr<'a> {
    /// Temporary variable XrefId.
    pub xref: XrefId,
    /// Resolved variable name.
    pub name: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Slot literal for runtime.
#[derive(Debug)]
pub struct SlotLiteralExpr {
    /// Slot handle.
    pub slot: SlotHandle,
    /// Optional target xref for slot lookup during allocation.
    /// Used to update the slot after slot allocation phase.
    pub target_xref: Option<XrefId>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Conditional case expression (for @if/@switch).
#[derive(Debug)]
pub struct ConditionalCaseExpr<'a> {
    /// Condition expression (None for else case).
    pub expr: Option<Box<'a, IrExpression<'a>>>,
    /// Target view XrefId.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotHandle,
    /// Alias variable name.
    pub alias: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Const-collected expression (for constant pool).
#[derive(Debug)]
pub struct ConstCollectedExpr<'a> {
    /// Wrapped expression.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Reference to a const array index.
/// This replaces ConstCollectedExpr after the expression is stored in the const array.
#[derive(Debug)]
pub struct ConstReferenceExpr {
    /// Index into the const array.
    pub index: u32,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Two-way binding set expression.
#[derive(Debug)]
pub struct TwoWayBindingSetExpr<'a> {
    /// Target expression.
    pub target: Box<'a, IrExpression<'a>>,
    /// Value expression.
    pub value: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Context @let reference.
#[derive(Debug)]
pub struct ContextLetReferenceExpr {
    /// Target @let XrefId.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotHandle,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Store @let value expression.
#[derive(Debug)]
pub struct StoreLetExpr<'a> {
    /// Target @let XrefId.
    pub target: XrefId,
    /// Value expression.
    pub value: Box<'a, IrExpression<'a>>,
    /// Variable offset for change detection slot indexing.
    /// Assigned by the var_counting phase.
    pub var_offset: Option<u32>,
    /// Source span.
    pub source_span: Span,
}

/// Interpolation expression (strings interleaved with expressions).
#[derive(Debug)]
pub struct Interpolation<'a> {
    /// Static string parts.
    pub strings: Vec<'a, Atom<'a>>,
    /// Dynamic expression parts.
    pub expressions: Vec<'a, IrExpression<'a>>,
    /// I18n placeholder names for each expression.
    /// Used by convert_i18n_bindings phase to create I18nExpression ops.
    /// Empty if not in an i18n context.
    pub i18n_placeholders: Vec<'a, Atom<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

impl<'a> Interpolation<'a> {
    /// Returns true if this is a pure interpolation with no expressions.
    pub fn is_const_string(&self) -> bool {
        self.expressions.is_empty()
    }

    /// Returns the constant string value if this is a pure interpolation.
    pub fn const_value(&self) -> Option<&Atom<'a>> {
        if self.is_const_string() && self.strings.len() == 1 {
            Some(&self.strings[0])
        } else {
            None
        }
    }
}

/// Binary operator expression.
///
/// Used for computed expressions in @for loops, such as:
/// - `$first = $index === 0`
/// - `$last = $index === $count - 1`
/// - `$even = $index % 2 === 0`
/// - `$odd = $index % 2 !== 0`
#[derive(Debug)]
pub struct BinaryExpr<'a> {
    /// The binary operator.
    pub operator: IrBinaryOperator,
    /// Left-hand side expression.
    pub lhs: Box<'a, IrExpression<'a>>,
    /// Right-hand side expression.
    pub rhs: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Ternary conditional expression (condition ? true_expr : false_expr).
///
/// This holds conditional logic with IR children so slots can be updated
/// after slot allocation phase.
#[derive(Debug)]
pub struct TernaryExpr<'a> {
    /// Condition expression.
    pub condition: Box<'a, IrExpression<'a>>,
    /// Expression when condition is true.
    pub true_expr: Box<'a, IrExpression<'a>>,
    /// Expression when condition is false.
    pub false_expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Binary operators for IR expressions.
///
/// These are used for computed expressions in @for loops and for preserving
/// pipes nested in binary expressions like `a ?? (b | pipe)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrBinaryOperator {
    /// Addition (+)
    Plus,
    /// Subtraction (-)
    Minus,
    /// Multiplication (*)
    Multiply,
    /// Division (/)
    Divide,
    /// Modulo (%)
    Modulo,
    /// Exponentiation (**)
    Exponentiation,
    /// Loose equality (==)
    Equals,
    /// Loose inequality (!=)
    NotEquals,
    /// Strict equality (===)
    Identical,
    /// Strict inequality (!==)
    NotIdentical,
    /// Less than (<)
    Lower,
    /// Less than or equal (<=)
    LowerEquals,
    /// Greater than (>)
    Bigger,
    /// Greater than or equal (>=)
    BiggerEquals,
    /// Logical and (&&)
    And,
    /// Logical or (||)
    Or,
    /// Nullish coalescing (??)
    NullishCoalesce,
    /// In operator (in)
    In,
    /// Instanceof operator (instanceof)
    Instanceof,
    /// Assignment (=)
    Assign,
    /// Addition assignment (+=)
    AdditionAssignment,
    /// Subtraction assignment (-=)
    SubtractionAssignment,
    /// Multiplication assignment (*=)
    MultiplicationAssignment,
    /// Division assignment (/=)
    DivisionAssignment,
    /// Remainder assignment (%=)
    RemainderAssignment,
    /// Exponentiation assignment (**=)
    ExponentiationAssignment,
    /// And assignment (&&=)
    AndAssignment,
    /// Or assignment (||=)
    OrAssignment,
    /// Nullish coalescing assignment (??=)
    NullishCoalesceAssignment,
}

/// Unary operators for IR expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrUnaryOperator {
    /// Unary plus (+)
    Plus,
    /// Unary minus (-)
    Minus,
}

/// Logical NOT expression (!expr).
///
/// Used to preserve pipe bindings inside negation expressions like `!(x | async)`.
#[derive(Debug)]
pub struct NotExpr<'a> {
    /// The operand expression.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Unary operator expression (+expr or -expr).
///
/// Used to preserve pipe bindings inside unary expressions like `+(x | pipe)`.
#[derive(Debug)]
pub struct UnaryExpr<'a> {
    /// The unary operator.
    pub operator: IrUnaryOperator,
    /// The operand expression.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Typeof expression (typeof expr).
///
/// Used to preserve pipe bindings inside typeof expressions.
#[derive(Debug)]
pub struct TypeofExpr<'a> {
    /// The operand expression.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Void expression (void expr).
///
/// Used to preserve pipe bindings inside void expressions.
#[derive(Debug)]
pub struct VoidExpr<'a> {
    /// The operand expression.
    pub expr: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Arrow function expression.
///
/// Created by the generateArrowFunctions phase to wrap user-defined arrow functions
/// found in template expressions. Arrow functions in event listeners are preserved
/// in place (not wrapped) because they need to access $event.
///
/// The `ops` list is used to store Variable ops that are prepended by the
/// generate_variables phase. These ops are processed during the naming phase
/// to ensure variables are named in the correct order.
///
/// Ported from Angular's `ir.ArrowFunctionExpr` in `expression.ts`.
#[derive(Debug)]
pub struct ArrowFunctionExpr<'a> {
    /// Function parameters.
    pub params: Vec<'a, crate::output::ast::FnParam<'a>>,
    /// Function body expression.
    pub body: Box<'a, IrExpression<'a>>,
    /// Operations list for this arrow function.
    /// Initially empty, populated by generate_variables phase with Variable ops.
    /// These ops are processed by the naming phase before create/update ops.
    pub ops: Vec<'a, crate::ir::ops::UpdateOp<'a>>,
    /// Variable offset for change detection slot indexing.
    /// Assigned by the var_counting phase.
    pub var_offset: Option<u32>,
    /// Source span.
    pub source_span: Option<Span>,
}

// ============================================================================
// Expression Transformation
// ============================================================================

/// Type alias for expression transformer function.
pub type ExpressionTransform<'a> = dyn Fn(&mut IrExpression<'a>, VisitorContextFlag) -> () + 'a;

/// Transform all expressions in an IR expression tree.
pub fn transform_expressions_in_expression<'a, F>(
    expr: &mut IrExpression<'a>,
    transform: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&mut IrExpression<'a>, VisitorContextFlag),
{
    // First transform internal expressions
    match expr {
        IrExpression::RestoreView(e) => {
            if let RestoreViewTarget::Dynamic(ref mut inner) = e.view {
                transform_expressions_in_expression(inner, transform, flags);
            }
        }
        IrExpression::ResetView(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::PureFunction(e) => {
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            if let Some(ref mut body) = e.body {
                transform_expressions_in_expression(body, transform, child_flags);
            }
            if let Some(ref mut fn_ref) = e.fn_ref {
                transform_expressions_in_expression(fn_ref, transform, flags);
            }
            for arg in e.args.iter_mut() {
                transform_expressions_in_expression(arg, transform, flags);
            }
        }
        IrExpression::PipeBinding(e) => {
            for arg in e.args.iter_mut() {
                transform_expressions_in_expression(arg, transform, flags);
            }
        }
        IrExpression::PipeBindingVariadic(e) => {
            transform_expressions_in_expression(&mut e.args, transform, flags);
        }
        IrExpression::SafePropertyRead(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
        }
        IrExpression::SafeKeyedRead(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
            transform_expressions_in_expression(&mut e.index, transform, flags);
        }
        IrExpression::SafeInvokeFunction(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
            for arg in e.args.iter_mut() {
                transform_expressions_in_expression(arg, transform, flags);
            }
        }
        IrExpression::SafeTernary(e) => {
            transform_expressions_in_expression(&mut e.guard, transform, flags);
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::AssignTemporary(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::ConditionalCase(e) => {
            if let Some(ref mut condition) = e.expr {
                transform_expressions_in_expression(condition, transform, flags);
            }
        }
        IrExpression::ConstCollected(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::TwoWayBindingSet(e) => {
            transform_expressions_in_expression(&mut e.target, transform, flags);
            transform_expressions_in_expression(&mut e.value, transform, flags);
        }
        IrExpression::StoreLet(e) => {
            transform_expressions_in_expression(&mut e.value, transform, flags);
        }
        IrExpression::Interpolation(e) => {
            for expr in e.expressions.iter_mut() {
                transform_expressions_in_expression(expr, transform, flags);
            }
        }
        IrExpression::Binary(e) => {
            transform_expressions_in_expression(&mut e.lhs, transform, flags);
            transform_expressions_in_expression(&mut e.rhs, transform, flags);
        }
        IrExpression::ResolvedPropertyRead(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
        }
        IrExpression::ResolvedBinary(e) => {
            transform_expressions_in_expression(&mut e.left, transform, flags);
            transform_expressions_in_expression(&mut e.right, transform, flags);
        }
        IrExpression::ResolvedCall(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
            for arg in e.args.iter_mut() {
                transform_expressions_in_expression(arg, transform, flags);
            }
        }
        IrExpression::ResolvedKeyedRead(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
            transform_expressions_in_expression(&mut e.key, transform, flags);
        }
        IrExpression::ResolvedSafePropertyRead(e) => {
            transform_expressions_in_expression(&mut e.receiver, transform, flags);
        }
        IrExpression::DerivedLiteralArray(e) => {
            for entry in e.entries.iter_mut() {
                transform_expressions_in_expression(entry, transform, flags);
            }
        }
        IrExpression::DerivedLiteralMap(e) => {
            for value in e.values.iter_mut() {
                transform_expressions_in_expression(value, transform, flags);
            }
        }
        IrExpression::LiteralArray(e) => {
            for elem in e.elements.iter_mut() {
                transform_expressions_in_expression(elem, transform, flags);
            }
        }
        IrExpression::LiteralMap(e) => {
            for value in e.values.iter_mut() {
                transform_expressions_in_expression(value, transform, flags);
            }
        }
        IrExpression::Ternary(e) => {
            transform_expressions_in_expression(&mut e.condition, transform, flags);
            transform_expressions_in_expression(&mut e.true_expr, transform, flags);
            transform_expressions_in_expression(&mut e.false_expr, transform, flags);
        }
        IrExpression::Not(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::Unary(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::Typeof(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::Void(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        IrExpression::ResolvedTemplateLiteral(e) => {
            for expr in e.expressions.iter_mut() {
                transform_expressions_in_expression(expr, transform, flags);
            }
        }
        IrExpression::ArrowFunction(e) => {
            // Transform body with InChildOperation and InArrowFunctionOperation flags set
            let child_flags = flags
                .union(VisitorContextFlag::IN_CHILD_OPERATION)
                .union(VisitorContextFlag::IN_ARROW_FUNCTION_OPERATION);
            transform_expressions_in_expression(&mut e.body, transform, child_flags);
        }
        IrExpression::Parenthesized(e) => {
            transform_expressions_in_expression(&mut e.expr, transform, flags);
        }
        // These expressions have no internal expressions
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
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_) // ExpressionRef is a leaf - actual expression is in store
        | IrExpression::OutputExpr(_) => {} // OutputExpr is already an output expression
    }

    // Then apply the transform to this expression
    transform(expr, flags);
}

/// Visit all expressions in an IR expression tree (read-only).
pub fn visit_expressions_in_expression<'a, F>(
    expr: &IrExpression<'a>,
    visitor: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&IrExpression<'a>, VisitorContextFlag),
{
    // Visit this expression
    visitor(expr, flags);

    // Visit internal expressions
    match expr {
        IrExpression::RestoreView(e) => {
            if let RestoreViewTarget::Dynamic(ref inner) = e.view {
                visit_expressions_in_expression(inner, visitor, flags);
            }
        }
        IrExpression::ResetView(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::PureFunction(e) => {
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            if let Some(ref body) = e.body {
                visit_expressions_in_expression(body, visitor, child_flags);
            }
            if let Some(ref fn_ref) = e.fn_ref {
                visit_expressions_in_expression(fn_ref, visitor, flags);
            }
            for arg in e.args.iter() {
                visit_expressions_in_expression(arg, visitor, flags);
            }
        }
        IrExpression::PipeBinding(e) => {
            for arg in e.args.iter() {
                visit_expressions_in_expression(arg, visitor, flags);
            }
        }
        IrExpression::PipeBindingVariadic(e) => {
            visit_expressions_in_expression(&e.args, visitor, flags);
        }
        IrExpression::SafePropertyRead(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
        }
        IrExpression::SafeKeyedRead(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
            visit_expressions_in_expression(&e.index, visitor, flags);
        }
        IrExpression::SafeInvokeFunction(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
            for arg in e.args.iter() {
                visit_expressions_in_expression(arg, visitor, flags);
            }
        }
        IrExpression::SafeTernary(e) => {
            visit_expressions_in_expression(&e.guard, visitor, flags);
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::AssignTemporary(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::ConditionalCase(e) => {
            if let Some(ref condition) = e.expr {
                visit_expressions_in_expression(condition, visitor, flags);
            }
        }
        IrExpression::ConstCollected(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::TwoWayBindingSet(e) => {
            visit_expressions_in_expression(&e.target, visitor, flags);
            visit_expressions_in_expression(&e.value, visitor, flags);
        }
        IrExpression::StoreLet(e) => {
            visit_expressions_in_expression(&e.value, visitor, flags);
        }
        IrExpression::Interpolation(e) => {
            for expr in e.expressions.iter() {
                visit_expressions_in_expression(expr, visitor, flags);
            }
        }
        IrExpression::Binary(e) => {
            visit_expressions_in_expression(&e.lhs, visitor, flags);
            visit_expressions_in_expression(&e.rhs, visitor, flags);
        }
        IrExpression::ResolvedPropertyRead(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
        }
        IrExpression::ResolvedBinary(e) => {
            visit_expressions_in_expression(&e.left, visitor, flags);
            visit_expressions_in_expression(&e.right, visitor, flags);
        }
        IrExpression::ResolvedCall(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
            for arg in e.args.iter() {
                visit_expressions_in_expression(arg, visitor, flags);
            }
        }
        IrExpression::ResolvedKeyedRead(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
            visit_expressions_in_expression(&e.key, visitor, flags);
        }
        IrExpression::ResolvedSafePropertyRead(e) => {
            visit_expressions_in_expression(&e.receiver, visitor, flags);
        }
        IrExpression::DerivedLiteralArray(e) => {
            for entry in e.entries.iter() {
                visit_expressions_in_expression(entry, visitor, flags);
            }
        }
        IrExpression::DerivedLiteralMap(e) => {
            for value in e.values.iter() {
                visit_expressions_in_expression(value, visitor, flags);
            }
        }
        IrExpression::LiteralArray(e) => {
            for elem in e.elements.iter() {
                visit_expressions_in_expression(elem, visitor, flags);
            }
        }
        IrExpression::LiteralMap(e) => {
            for value in e.values.iter() {
                visit_expressions_in_expression(value, visitor, flags);
            }
        }
        IrExpression::Ternary(e) => {
            visit_expressions_in_expression(&e.condition, visitor, flags);
            visit_expressions_in_expression(&e.true_expr, visitor, flags);
            visit_expressions_in_expression(&e.false_expr, visitor, flags);
        }
        IrExpression::Not(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::Unary(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::Typeof(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::Void(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        IrExpression::ResolvedTemplateLiteral(e) => {
            for expr in e.expressions.iter() {
                visit_expressions_in_expression(expr, visitor, flags);
            }
        }
        IrExpression::ArrowFunction(e) => {
            // Visit body with InChildOperation and InArrowFunctionOperation flags set
            let child_flags = flags
                .union(VisitorContextFlag::IN_CHILD_OPERATION)
                .union(VisitorContextFlag::IN_ARROW_FUNCTION_OPERATION);
            visit_expressions_in_expression(&e.body, visitor, child_flags);
        }
        IrExpression::Parenthesized(e) => {
            visit_expressions_in_expression(&e.expr, visitor, flags);
        }
        // These expressions have no internal expressions
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
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_) // ExpressionRef is a leaf - actual expression is in store
        | IrExpression::OutputExpr(_) => {} // OutputExpr is already an output expression
    }
}

// ============================================================================
// Operation Expression Transformation
// ============================================================================

use crate::ir::ops::{CreateOp, UpdateOp};

/// Transform all expressions in an update operation.
pub fn transform_expressions_in_update_op<'a, F>(
    op: &mut UpdateOp<'a>,
    transform: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&mut IrExpression<'a>, VisitorContextFlag),
{
    match op {
        UpdateOp::Property(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::StyleProp(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::ClassProp(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::StyleMap(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::ClassMap(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::Attribute(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::DomProperty(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::TwoWayProperty(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::Binding(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::InterpolateText(op) => {
            transform_expressions_in_expression(&mut op.interpolation, transform, flags);
        }
        UpdateOp::Variable(op) => {
            transform_expressions_in_expression(&mut op.initializer, transform, flags);
        }
        UpdateOp::StoreLet(op) => {
            transform_expressions_in_expression(&mut op.value, transform, flags);
        }
        UpdateOp::Conditional(op) => {
            if let Some(ref mut test) = op.test {
                transform_expressions_in_expression(test, transform, flags);
            }
            // Transform expressions in conditions
            for cond in op.conditions.iter_mut() {
                if let Some(ref mut expr) = cond.expr {
                    transform_expressions_in_expression(expr, transform, flags);
                }
            }
            if let Some(ref mut processed) = op.processed {
                transform_expressions_in_expression(processed, transform, flags);
            }
            if let Some(ref mut ctx) = op.context_value {
                transform_expressions_in_expression(ctx, transform, flags);
            }
        }
        UpdateOp::Repeater(op) => {
            transform_expressions_in_expression(&mut op.collection, transform, flags);
        }
        UpdateOp::AnimationBinding(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::Control(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::I18nExpression(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        UpdateOp::DeferWhen(op) => {
            transform_expressions_in_expression(&mut op.condition, transform, flags);
        }
        UpdateOp::Statement(op) => {
            // Statement ops may contain WrappedIrNode which wraps IR expressions.
            // We need to transform these expressions so that phases like naming
            // can propagate variable names to ReadVariableExpr inside.
            transform_expressions_in_output_statement(&mut op.statement, transform, flags);
        }
        // Operations without expressions
        UpdateOp::ListEnd(_) | UpdateOp::Advance(_) | UpdateOp::I18nApply(_) => {}
    }
}

/// Transform expressions inside an OutputStatement.
/// This handles WrappedIrNode expressions that contain IR expressions.
fn transform_expressions_in_output_statement<'a, F>(
    stmt: &mut crate::output::ast::OutputStatement<'a>,
    transform: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&mut IrExpression<'a>, VisitorContextFlag),
{
    use crate::output::ast::OutputStatement;

    match stmt {
        OutputStatement::Expression(expr_stmt) => {
            transform_expressions_in_output_expression(&mut expr_stmt.expr, transform, flags);
        }
        OutputStatement::Return(ret_stmt) => {
            transform_expressions_in_output_expression(&mut ret_stmt.value, transform, flags);
        }
        OutputStatement::DeclareVar(decl) => {
            if let Some(ref mut value) = decl.value {
                transform_expressions_in_output_expression(value, transform, flags);
            }
        }
        OutputStatement::If(if_stmt) => {
            transform_expressions_in_output_expression(&mut if_stmt.condition, transform, flags);
            for stmt in if_stmt.true_case.iter_mut() {
                transform_expressions_in_output_statement(stmt, transform, flags);
            }
            for stmt in if_stmt.false_case.iter_mut() {
                transform_expressions_in_output_statement(stmt, transform, flags);
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
    transform: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&mut IrExpression<'a>, VisitorContextFlag),
{
    use crate::output::ast::OutputExpression;

    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            transform_expressions_in_expression(&mut wrapped.node, transform, flags);
        }
        OutputExpression::Conditional(cond) => {
            transform_expressions_in_output_expression(&mut cond.condition, transform, flags);
            transform_expressions_in_output_expression(&mut cond.true_case, transform, flags);
            if let Some(false_case) = &mut cond.false_case {
                transform_expressions_in_output_expression(false_case, transform, flags);
            }
        }
        OutputExpression::BinaryOperator(bin) => {
            transform_expressions_in_output_expression(&mut bin.lhs, transform, flags);
            transform_expressions_in_output_expression(&mut bin.rhs, transform, flags);
        }
        OutputExpression::UnaryOperator(un) => {
            transform_expressions_in_output_expression(&mut un.expr, transform, flags);
        }
        OutputExpression::Not(not) => {
            transform_expressions_in_output_expression(&mut not.condition, transform, flags);
        }
        OutputExpression::ReadProp(member) => {
            transform_expressions_in_output_expression(&mut member.receiver, transform, flags);
        }
        OutputExpression::ReadKey(idx) => {
            transform_expressions_in_output_expression(&mut idx.receiver, transform, flags);
            transform_expressions_in_output_expression(&mut idx.index, transform, flags);
        }
        OutputExpression::TaggedTemplateLiteral(tagged) => {
            transform_expressions_in_output_expression(&mut tagged.tag, transform, flags);
            for expr in tagged.template.expressions.iter_mut() {
                transform_expressions_in_output_expression(expr, transform, flags);
            }
        }
        OutputExpression::ArrowFunction(arrow) => match &mut arrow.body {
            crate::output::ast::ArrowFunctionBody::Expression(expr) => {
                transform_expressions_in_output_expression(expr, transform, flags);
            }
            crate::output::ast::ArrowFunctionBody::Statements(stmts) => {
                for stmt in stmts.iter_mut() {
                    transform_expressions_in_output_statement(stmt, transform, flags);
                }
            }
        },
        OutputExpression::LiteralArray(arr) => {
            for elem in arr.entries.iter_mut() {
                transform_expressions_in_output_expression(elem, transform, flags);
            }
        }
        OutputExpression::LiteralMap(obj) => {
            for entry in obj.entries.iter_mut() {
                transform_expressions_in_output_expression(&mut entry.value, transform, flags);
                if entry.quoted {
                    // Key is always a string, no sub-expressions
                }
            }
        }
        OutputExpression::InvokeFunction(inv) => {
            transform_expressions_in_output_expression(&mut inv.fn_expr, transform, flags);
            for arg in inv.args.iter_mut() {
                transform_expressions_in_output_expression(arg, transform, flags);
            }
        }
        OutputExpression::Function(func) => {
            for stmt in func.statements.iter_mut() {
                transform_expressions_in_output_statement(stmt, transform, flags);
            }
        }
        OutputExpression::Instantiate(inst) => {
            transform_expressions_in_output_expression(&mut inst.class_expr, transform, flags);
            for arg in inst.args.iter_mut() {
                transform_expressions_in_output_expression(arg, transform, flags);
            }
        }
        OutputExpression::Parenthesized(paren) => {
            transform_expressions_in_output_expression(&mut paren.expr, transform, flags);
        }
        OutputExpression::Comma(comma) => {
            for part in comma.parts.iter_mut() {
                transform_expressions_in_output_expression(part, transform, flags);
            }
        }
        OutputExpression::Typeof(typeof_expr) => {
            transform_expressions_in_output_expression(&mut typeof_expr.expr, transform, flags);
        }
        OutputExpression::Void(void_expr) => {
            transform_expressions_in_output_expression(&mut void_expr.expr, transform, flags);
        }
        OutputExpression::SpreadElement(spread) => {
            transform_expressions_in_output_expression(&mut spread.expr, transform, flags);
        }
        // Leaf expressions without sub-expressions
        OutputExpression::Literal(_)
        | OutputExpression::TemplateLiteral(_)
        | OutputExpression::RegularExpressionLiteral(_)
        | OutputExpression::ReadVar(_)
        | OutputExpression::External(_)
        | OutputExpression::LocalizedString(_)
        | OutputExpression::WrappedNode(_)
        | OutputExpression::DynamicImport(_) => {}
    }
}

/// Transform all expressions in a create operation.
pub fn transform_expressions_in_create_op<'a, F>(
    op: &mut CreateOp<'a>,
    transform: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&mut IrExpression<'a>, VisitorContextFlag),
{
    match op {
        CreateOp::Variable(op) => {
            transform_expressions_in_expression(&mut op.initializer, transform, flags);
        }
        CreateOp::Listener(op) => {
            // Process handler ops and handler_expression as child operations.
            // handler_expression is the return expression of the listener function and
            // must be treated as part of the handler scope (not the parent scope).
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            if let Some(handler_expr) = &mut op.handler_expression {
                transform_expressions_in_expression(handler_expr, transform, child_flags);
            }
            for handler_op in op.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, transform, child_flags);
            }
        }
        CreateOp::TwoWayListener(op) => {
            // Process handler ops in the two-way listener
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            for handler_op in op.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, transform, child_flags);
            }
        }
        CreateOp::AnimationListener(op) => {
            // Process handler ops in the animation listener
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            for handler_op in op.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, transform, child_flags);
            }
        }
        CreateOp::Animation(op) => {
            // Process handler ops in the animation op
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            for handler_op in op.handler_ops.iter_mut() {
                transform_expressions_in_update_op(handler_op, transform, child_flags);
            }
        }
        CreateOp::AnimationString(op) => {
            transform_expressions_in_expression(&mut op.expression, transform, flags);
        }
        CreateOp::ExtractedAttribute(op) => {
            if let Some(ref mut value) = op.value {
                transform_expressions_in_expression(value, transform, flags);
            }
        }
        CreateOp::DeferOn(op) => {
            if let Some(ref mut options) = op.options {
                transform_expressions_in_expression(options, transform, flags);
            }
        }
        CreateOp::RepeaterCreate(op) => {
            transform_expressions_in_expression(&mut op.track, transform, flags);
            // Process trackByOps if present (matching Angular's ops() generator behavior)
            if let Some(ref mut track_by_ops) = op.track_by_ops {
                let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
                for track_op in track_by_ops.iter_mut() {
                    transform_expressions_in_update_op(track_op, transform, child_flags);
                }
            }
        }
        // Defer: transform loadingConfig and placeholderConfig expressions.
        // Matches Angular TS expression.ts lines 1241-1251:
        //   case OpKind.Defer:
        //     if (op.loadingConfig !== null) { op.loadingConfig = transform(op.loadingConfig); }
        //     if (op.placeholderConfig !== null) { op.placeholderConfig = transform(op.placeholderConfig); }
        //     if (op.resolverFn !== null) { op.resolverFn = transform(op.resolverFn); }
        CreateOp::Defer(op) => {
            if let Some(ref mut config) = op.loading_config {
                transform_expressions_in_expression(config.as_mut(), transform, flags);
            }
            if let Some(ref mut config) = op.placeholder_config {
                transform_expressions_in_expression(config.as_mut(), transform, flags);
            }
        }
        // Operations without expressions (expressions are in the UPDATE op now)
        CreateOp::Conditional(_) => {}
        // Operations without expressions
        CreateOp::ListEnd(_)
        | CreateOp::ElementStart(_)
        | CreateOp::Element(_)
        | CreateOp::ElementEnd(_)
        | CreateOp::Template(_)
        | CreateOp::ContainerStart(_)
        | CreateOp::Container(_)
        | CreateOp::ContainerEnd(_)
        | CreateOp::DisableBindings(_)
        | CreateOp::EnableBindings(_)
        | CreateOp::Text(_)
        | CreateOp::Pipe(_)
        | CreateOp::I18nMessage(_)
        | CreateOp::Namespace(_)
        | CreateOp::ProjectionDef(_)
        | CreateOp::Projection(_)
        | CreateOp::DeclareLet(_)
        | CreateOp::I18nStart(_)
        | CreateOp::I18n(_)
        | CreateOp::I18nEnd(_)
        | CreateOp::IcuStart(_)
        | CreateOp::IcuEnd(_)
        | CreateOp::IcuPlaceholder(_)
        | CreateOp::I18nContext(_)
        | CreateOp::I18nAttributes(_)
        | CreateOp::SourceLocation(_)
        | CreateOp::ConditionalBranch(_)
        | CreateOp::ControlCreate(_)
        | CreateOp::Statement(_) => {}
    }
}

/// Visit all expressions in an update operation (read-only).
pub fn visit_expressions_in_update_op<'a, F>(
    op: &UpdateOp<'a>,
    visitor: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&IrExpression<'a>, VisitorContextFlag),
{
    match op {
        UpdateOp::Property(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::StyleProp(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::ClassProp(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::StyleMap(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::ClassMap(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::Attribute(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::DomProperty(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::TwoWayProperty(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::Binding(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::InterpolateText(op) => {
            visit_expressions_in_expression(&op.interpolation, visitor, flags);
        }
        UpdateOp::Variable(op) => {
            visit_expressions_in_expression(&op.initializer, visitor, flags);
        }
        UpdateOp::StoreLet(op) => {
            visit_expressions_in_expression(&op.value, visitor, flags);
        }
        UpdateOp::Conditional(op) => {
            if let Some(ref test) = op.test {
                visit_expressions_in_expression(test, visitor, flags);
            }
            for cond in op.conditions.iter() {
                if let Some(ref expr) = cond.expr {
                    visit_expressions_in_expression(expr, visitor, flags);
                }
            }
            if let Some(ref processed) = op.processed {
                visit_expressions_in_expression(processed, visitor, flags);
            }
            if let Some(ref ctx) = op.context_value {
                visit_expressions_in_expression(ctx, visitor, flags);
            }
        }
        UpdateOp::Repeater(op) => {
            visit_expressions_in_expression(&op.collection, visitor, flags);
        }
        UpdateOp::AnimationBinding(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::Control(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::I18nExpression(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        UpdateOp::DeferWhen(op) => {
            visit_expressions_in_expression(&op.condition, visitor, flags);
        }
        // Operations without expressions
        UpdateOp::ListEnd(_)
        | UpdateOp::Advance(_)
        | UpdateOp::I18nApply(_)
        | UpdateOp::Statement(_) => {}
    }
}

/// Visit all expressions in a create operation (read-only).
pub fn visit_expressions_in_create_op<'a, F>(
    op: &CreateOp<'a>,
    visitor: &F,
    flags: VisitorContextFlag,
) where
    F: Fn(&IrExpression<'a>, VisitorContextFlag),
{
    match op {
        CreateOp::Variable(op) => {
            visit_expressions_in_expression(&op.initializer, visitor, flags);
        }
        CreateOp::Listener(op) => {
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            if let Some(handler_expr) = &op.handler_expression {
                visit_expressions_in_expression(handler_expr, visitor, child_flags);
            }
            for handler_op in op.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor, child_flags);
            }
        }
        CreateOp::TwoWayListener(op) => {
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            for handler_op in op.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor, child_flags);
            }
        }
        CreateOp::AnimationListener(op) => {
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            for handler_op in op.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor, child_flags);
            }
        }
        CreateOp::Animation(op) => {
            let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
            for handler_op in op.handler_ops.iter() {
                visit_expressions_in_update_op(handler_op, visitor, child_flags);
            }
        }
        CreateOp::AnimationString(op) => {
            visit_expressions_in_expression(&op.expression, visitor, flags);
        }
        CreateOp::ExtractedAttribute(op) => {
            if let Some(ref value) = op.value {
                visit_expressions_in_expression(value, visitor, flags);
            }
        }
        CreateOp::DeferOn(op) => {
            if let Some(ref options) = op.options {
                visit_expressions_in_expression(options, visitor, flags);
            }
        }
        CreateOp::RepeaterCreate(op) => {
            visit_expressions_in_expression(&op.track, visitor, flags);
            // Visit trackByOps if present (matching Angular's ops() generator behavior)
            if let Some(ref track_by_ops) = op.track_by_ops {
                let child_flags = flags.union(VisitorContextFlag::IN_CHILD_OPERATION);
                for track_op in track_by_ops.iter() {
                    visit_expressions_in_update_op(track_op, visitor, child_flags);
                }
            }
        }
        // Defer: visit loadingConfig and placeholderConfig expressions.
        CreateOp::Defer(op) => {
            if let Some(ref config) = op.loading_config {
                visit_expressions_in_expression(config.as_ref(), visitor, flags);
            }
            if let Some(ref config) = op.placeholder_config {
                visit_expressions_in_expression(config.as_ref(), visitor, flags);
            }
        }
        // Operations without expressions
        CreateOp::Conditional(_)
        | CreateOp::ListEnd(_)
        | CreateOp::ElementStart(_)
        | CreateOp::Element(_)
        | CreateOp::ElementEnd(_)
        | CreateOp::Template(_)
        | CreateOp::ContainerStart(_)
        | CreateOp::Container(_)
        | CreateOp::ContainerEnd(_)
        | CreateOp::DisableBindings(_)
        | CreateOp::EnableBindings(_)
        | CreateOp::Text(_)
        | CreateOp::Pipe(_)
        | CreateOp::I18nMessage(_)
        | CreateOp::Namespace(_)
        | CreateOp::ProjectionDef(_)
        | CreateOp::Projection(_)
        | CreateOp::DeclareLet(_)
        | CreateOp::I18nStart(_)
        | CreateOp::I18n(_)
        | CreateOp::I18nEnd(_)
        | CreateOp::IcuStart(_)
        | CreateOp::IcuEnd(_)
        | CreateOp::IcuPlaceholder(_)
        | CreateOp::I18nContext(_)
        | CreateOp::I18nAttributes(_)
        | CreateOp::SourceLocation(_)
        | CreateOp::ConditionalBranch(_)
        | CreateOp::ControlCreate(_)
        | CreateOp::Statement(_) => {}
    }
}

// ============================================================================
// Angular Expression Cloning
// ============================================================================

use crate::ast::expression::{
    ArrowFunction as AstArrowFunction, ArrowFunctionParameter as AstArrowFunctionParameter, Binary,
    BindingPipe, Call, Chain, Conditional, EmptyExpr as AstEmptyExpr, ImplicitReceiver,
    Interpolation as AstInterpolation, KeyedRead, LiteralArray, LiteralMap, LiteralMapKey,
    LiteralMapPropertyKey, LiteralMapSpreadKey, LiteralPrimitive, LiteralValue, NonNullAssert,
    ParenthesizedExpression, PrefixNot, PropertyRead,
    RegularExpressionLiteral as AstRegularExpressionLiteral, SafeCall, SafeKeyedRead,
    SafePropertyRead, SpreadElement, TaggedTemplateLiteral, TemplateLiteral,
    TemplateLiteralElement, ThisReceiver, TypeofExpression, Unary, VoidExpression,
};

/// Clones an AngularExpression into a new allocator.
///
/// This is a deep clone that creates a new copy of the entire expression tree
/// in the provided allocator.
pub fn clone_angular_expression<'a>(
    expr: &AngularExpression<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> AngularExpression<'a> {
    match expr {
        AngularExpression::Empty(e) => AngularExpression::Empty(Box::new_in(
            AstEmptyExpr { span: e.span, source_span: e.source_span },
            allocator,
        )),
        AngularExpression::ImplicitReceiver(e) => AngularExpression::ImplicitReceiver(Box::new_in(
            ImplicitReceiver { span: e.span, source_span: e.source_span },
            allocator,
        )),
        AngularExpression::ThisReceiver(e) => AngularExpression::ThisReceiver(Box::new_in(
            ThisReceiver { span: e.span, source_span: e.source_span },
            allocator,
        )),
        AngularExpression::Chain(e) => {
            let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
            for expr in e.expressions.iter() {
                expressions.push(clone_angular_expression(expr, allocator));
            }
            AngularExpression::Chain(Box::new_in(
                Chain { expressions, span: e.span, source_span: e.source_span },
                allocator,
            ))
        }
        AngularExpression::Conditional(e) => AngularExpression::Conditional(Box::new_in(
            Conditional {
                condition: clone_angular_expression(&e.condition, allocator),
                true_exp: clone_angular_expression(&e.true_exp, allocator),
                false_exp: clone_angular_expression(&e.false_exp, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::PropertyRead(e) => AngularExpression::PropertyRead(Box::new_in(
            PropertyRead {
                receiver: clone_angular_expression(&e.receiver, allocator),
                name: e.name.clone(),
                name_span: e.name_span,
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::SafePropertyRead(e) => AngularExpression::SafePropertyRead(Box::new_in(
            SafePropertyRead {
                receiver: clone_angular_expression(&e.receiver, allocator),
                name: e.name.clone(),
                name_span: e.name_span,
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::KeyedRead(e) => AngularExpression::KeyedRead(Box::new_in(
            KeyedRead {
                receiver: clone_angular_expression(&e.receiver, allocator),
                key: clone_angular_expression(&e.key, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::SafeKeyedRead(e) => AngularExpression::SafeKeyedRead(Box::new_in(
            SafeKeyedRead {
                receiver: clone_angular_expression(&e.receiver, allocator),
                key: clone_angular_expression(&e.key, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::BindingPipe(e) => {
            let mut args = Vec::with_capacity_in(e.args.len(), allocator);
            for arg in e.args.iter() {
                args.push(clone_angular_expression(arg, allocator));
            }
            AngularExpression::BindingPipe(Box::new_in(
                BindingPipe {
                    exp: clone_angular_expression(&e.exp, allocator),
                    name: e.name.clone(),
                    name_span: e.name_span,
                    args,
                    pipe_type: e.pipe_type,
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
        AngularExpression::LiteralPrimitive(e) => AngularExpression::LiteralPrimitive(Box::new_in(
            LiteralPrimitive {
                value: clone_literal_value(&e.value, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::LiteralArray(e) => {
            let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
            for expr in e.expressions.iter() {
                expressions.push(clone_angular_expression(expr, allocator));
            }
            AngularExpression::LiteralArray(Box::new_in(
                LiteralArray { expressions, span: e.span, source_span: e.source_span },
                allocator,
            ))
        }
        AngularExpression::LiteralMap(e) => {
            let mut keys = Vec::with_capacity_in(e.keys.len(), allocator);
            for key in e.keys.iter() {
                keys.push(match key {
                    LiteralMapKey::Property(prop) => {
                        LiteralMapKey::Property(LiteralMapPropertyKey {
                            key: prop.key.clone(),
                            quoted: prop.quoted,
                            is_shorthand_initialized: prop.is_shorthand_initialized,
                        })
                    }
                    LiteralMapKey::Spread(spread) => LiteralMapKey::Spread(LiteralMapSpreadKey {
                        span: spread.span,
                        source_span: spread.source_span,
                    }),
                });
            }
            let mut values = Vec::with_capacity_in(e.values.len(), allocator);
            for val in e.values.iter() {
                values.push(clone_angular_expression(val, allocator));
            }
            AngularExpression::LiteralMap(Box::new_in(
                LiteralMap { keys, values, span: e.span, source_span: e.source_span },
                allocator,
            ))
        }
        AngularExpression::SpreadElement(e) => AngularExpression::SpreadElement(Box::new_in(
            SpreadElement {
                span: e.span,
                source_span: e.source_span,
                expression: clone_angular_expression(&e.expression, allocator),
            },
            allocator,
        )),
        AngularExpression::Interpolation(e) => {
            let mut strings = Vec::with_capacity_in(e.strings.len(), allocator);
            for s in e.strings.iter() {
                strings.push(s.clone());
            }
            let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
            for expr in e.expressions.iter() {
                expressions.push(clone_angular_expression(expr, allocator));
            }
            AngularExpression::Interpolation(Box::new_in(
                AstInterpolation { strings, expressions, span: e.span, source_span: e.source_span },
                allocator,
            ))
        }
        AngularExpression::Binary(e) => AngularExpression::Binary(Box::new_in(
            Binary {
                operation: e.operation,
                left: clone_angular_expression(&e.left, allocator),
                right: clone_angular_expression(&e.right, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::Unary(e) => AngularExpression::Unary(Box::new_in(
            Unary {
                operator: e.operator,
                expr: clone_angular_expression(&e.expr, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::PrefixNot(e) => AngularExpression::PrefixNot(Box::new_in(
            PrefixNot {
                expression: clone_angular_expression(&e.expression, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::TypeofExpression(e) => AngularExpression::TypeofExpression(Box::new_in(
            TypeofExpression {
                expression: clone_angular_expression(&e.expression, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::VoidExpression(e) => AngularExpression::VoidExpression(Box::new_in(
            VoidExpression {
                expression: clone_angular_expression(&e.expression, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::NonNullAssert(e) => AngularExpression::NonNullAssert(Box::new_in(
            NonNullAssert {
                expression: clone_angular_expression(&e.expression, allocator),
                span: e.span,
                source_span: e.source_span,
            },
            allocator,
        )),
        AngularExpression::Call(e) => {
            let mut args = Vec::with_capacity_in(e.args.len(), allocator);
            for arg in e.args.iter() {
                args.push(clone_angular_expression(arg, allocator));
            }
            AngularExpression::Call(Box::new_in(
                Call {
                    receiver: clone_angular_expression(&e.receiver, allocator),
                    args,
                    argument_span: e.argument_span,
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
        AngularExpression::SafeCall(e) => {
            let mut args = Vec::with_capacity_in(e.args.len(), allocator);
            for arg in e.args.iter() {
                args.push(clone_angular_expression(arg, allocator));
            }
            AngularExpression::SafeCall(Box::new_in(
                SafeCall {
                    receiver: clone_angular_expression(&e.receiver, allocator),
                    args,
                    argument_span: e.argument_span,
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
        AngularExpression::TaggedTemplateLiteral(e) => {
            AngularExpression::TaggedTemplateLiteral(Box::new_in(
                TaggedTemplateLiteral {
                    tag: clone_angular_expression(&e.tag, allocator),
                    template: clone_template_literal(&e.template, allocator),
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
        AngularExpression::TemplateLiteral(e) => AngularExpression::TemplateLiteral(Box::new_in(
            clone_template_literal(e, allocator),
            allocator,
        )),
        AngularExpression::ParenthesizedExpression(e) => {
            AngularExpression::ParenthesizedExpression(Box::new_in(
                ParenthesizedExpression {
                    expression: clone_angular_expression(&e.expression, allocator),
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
        AngularExpression::RegularExpressionLiteral(e) => {
            AngularExpression::RegularExpressionLiteral(Box::new_in(
                AstRegularExpressionLiteral {
                    body: e.body.clone(),
                    flags: e.flags.clone(),
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
        AngularExpression::ArrowFunction(e) => {
            let mut parameters = Vec::with_capacity_in(e.parameters.len(), allocator);
            for p in e.parameters.iter() {
                parameters.push(AstArrowFunctionParameter {
                    name: p.name.clone(),
                    span: p.span,
                    source_span: p.source_span,
                });
            }
            AngularExpression::ArrowFunction(Box::new_in(
                AstArrowFunction {
                    parameters,
                    body: clone_angular_expression(&e.body, allocator),
                    span: e.span,
                    source_span: e.source_span,
                },
                allocator,
            ))
        }
    }
}

/// Clone a literal value.
fn clone_literal_value<'a>(
    value: &LiteralValue<'a>,
    _allocator: &'a oxc_allocator::Allocator,
) -> LiteralValue<'a> {
    match value {
        LiteralValue::Null => LiteralValue::Null,
        LiteralValue::Undefined => LiteralValue::Undefined,
        LiteralValue::Boolean(b) => LiteralValue::Boolean(*b),
        LiteralValue::Number(n) => LiteralValue::Number(*n),
        LiteralValue::String(s) => LiteralValue::String(s.clone()),
    }
}

/// Clone a template literal.
fn clone_template_literal<'a>(
    tl: &TemplateLiteral<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> TemplateLiteral<'a> {
    let mut elements = Vec::with_capacity_in(tl.elements.len(), allocator);
    for elem in tl.elements.iter() {
        elements.push(TemplateLiteralElement {
            text: elem.text.clone(),
            span: elem.span,
            source_span: elem.source_span,
        });
    }
    let mut expressions = Vec::with_capacity_in(tl.expressions.len(), allocator);
    for expr in tl.expressions.iter() {
        expressions.push(clone_angular_expression(expr, allocator));
    }
    TemplateLiteral { elements, expressions, span: tl.span, source_span: tl.source_span }
}

// ============================================================================
// Variable Slot Counting
// ============================================================================

/// Counts the number of variable slots consumed by an IR expression.
///
/// This is used by the `var_counting` phase to determine how many variable
/// slots are needed for a view's update operations.
///
/// Variable slots are consumed by:
/// - `PureFunctionExpr`: 1 + args.len() (for memoization)
/// - `PipeBinding`: 1 + args.len() (for pipe instance + args)
/// - `PipeBindingVariadic`: 1 + num_args (for pipe instance + variadic args)
/// - `StoreLet`: 1 (for storing the @let value)
///
/// This function recursively traverses nested expressions to find all
/// variable-consuming expressions.
///
/// Ported from Angular's `varsUsedByIrExpression` in `expression.ts:1165-1184`.
pub fn vars_used_by_ir_expression(expr: &IrExpression<'_>) -> u32 {
    match expr {
        // PureFunction uses 1 slot for the pure function context + 1 per argument
        IrExpression::PureFunction(pf) => 1 + pf.args.len() as u32,

        // PipeBinding uses 1 slot for the pipe instance + 1 per argument
        IrExpression::PipeBinding(pb) => 1 + pb.args.len() as u32,

        // PipeBindingVariadic uses 1 slot for the pipe instance + num_args
        IrExpression::PipeBindingVariadic(pb) => 1 + pb.num_args,

        // StoreLet uses 1 slot for storing the @let value
        IrExpression::StoreLet(_) => 1,

        // Binary expressions may have nested expressions that consume vars
        IrExpression::Binary(b) => {
            vars_used_by_ir_expression(&b.lhs) + vars_used_by_ir_expression(&b.rhs)
        }

        // ResolvedBinary may have nested expressions
        IrExpression::ResolvedBinary(b) => {
            vars_used_by_ir_expression(&b.left) + vars_used_by_ir_expression(&b.right)
        }

        // SafePropertyRead has a receiver expression
        IrExpression::SafePropertyRead(spr) => vars_used_by_ir_expression(&spr.receiver),

        // SafeKeyedRead has receiver and index expressions
        IrExpression::SafeKeyedRead(skr) => {
            vars_used_by_ir_expression(&skr.receiver) + vars_used_by_ir_expression(&skr.index)
        }

        // SafeInvokeFunction has receiver and argument expressions
        IrExpression::SafeInvokeFunction(sif) => {
            let receiver_vars = vars_used_by_ir_expression(&sif.receiver);
            let args_vars: u32 = sif.args.iter().map(vars_used_by_ir_expression).sum();
            receiver_vars + args_vars
        }

        // SafeTernary has condition and expression
        IrExpression::SafeTernary(st) => {
            vars_used_by_ir_expression(&st.guard) + vars_used_by_ir_expression(&st.expr)
        }

        // ConditionalCase has a condition expression (optional)
        IrExpression::ConditionalCase(cc) => {
            cc.expr.as_ref().map_or(0, |e| vars_used_by_ir_expression(e))
        }

        // ResolvedPropertyRead has a receiver
        IrExpression::ResolvedPropertyRead(rpr) => vars_used_by_ir_expression(&rpr.receiver),

        // ResolvedCall has receiver and arguments
        IrExpression::ResolvedCall(rc) => {
            let receiver_vars = vars_used_by_ir_expression(&rc.receiver);
            let args_vars: u32 = rc.args.iter().map(vars_used_by_ir_expression).sum();
            receiver_vars + args_vars
        }

        // ResolvedKeyedRead has receiver and key
        IrExpression::ResolvedKeyedRead(rkr) => {
            vars_used_by_ir_expression(&rkr.receiver) + vars_used_by_ir_expression(&rkr.key)
        }

        // ResolvedSafePropertyRead has a receiver
        IrExpression::ResolvedSafePropertyRead(rspr) => vars_used_by_ir_expression(&rspr.receiver),

        // AssignTemporary has inner expression
        IrExpression::AssignTemporary(at) => vars_used_by_ir_expression(&at.expr),

        // ResetView has inner expression
        IrExpression::ResetView(rv) => vars_used_by_ir_expression(&rv.expr),

        // RestoreView - check if it has a dynamic expression
        IrExpression::RestoreView(rv) => {
            if let RestoreViewTarget::Dynamic(expr) = &rv.view {
                vars_used_by_ir_expression(expr)
            } else {
                0
            }
        }

        // TwoWayBindingSet has target and value expressions
        IrExpression::TwoWayBindingSet(twbs) => {
            vars_used_by_ir_expression(&twbs.target) + vars_used_by_ir_expression(&twbs.value)
        }

        // ConstCollected has inner expression
        IrExpression::ConstCollected(cc) => vars_used_by_ir_expression(&cc.expr),

        // Interpolation: each expression in the interpolation may consume vars
        IrExpression::Interpolation(interp) => {
            interp.expressions.iter().map(vars_used_by_ir_expression).sum()
        }

        // DerivedLiteralArray: sum of vars used by all entries
        IrExpression::DerivedLiteralArray(arr) => {
            arr.entries.iter().map(vars_used_by_ir_expression).sum()
        }

        // DerivedLiteralMap: sum of vars used by all values
        IrExpression::DerivedLiteralMap(map) => {
            map.values.iter().map(vars_used_by_ir_expression).sum()
        }

        // LiteralArray: sum of vars used by all elements
        IrExpression::LiteralArray(arr) => {
            arr.elements.iter().map(vars_used_by_ir_expression).sum()
        }

        // LiteralMap: sum of vars used by all values
        IrExpression::LiteralMap(map) => map.values.iter().map(vars_used_by_ir_expression).sum(),

        // Ternary: sum of vars used by all branches
        IrExpression::Ternary(ternary) => {
            vars_used_by_ir_expression(&ternary.condition)
                + vars_used_by_ir_expression(&ternary.true_expr)
                + vars_used_by_ir_expression(&ternary.false_expr)
        }

        // Not expression: vars used by inner expression
        IrExpression::Not(not) => vars_used_by_ir_expression(&not.expr),

        // Unary expression: vars used by inner expression
        IrExpression::Unary(unary) => vars_used_by_ir_expression(&unary.expr),

        // Typeof expression: vars used by inner expression
        IrExpression::Typeof(typeof_expr) => vars_used_by_ir_expression(&typeof_expr.expr),

        // Void expression: vars used by inner expression
        IrExpression::Void(void_expr) => vars_used_by_ir_expression(&void_expr.expr),

        // Parenthesized expression: vars used by inner expression
        IrExpression::Parenthesized(paren) => vars_used_by_ir_expression(&paren.expr),

        // ResolvedTemplateLiteral: vars used by all expressions inside
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            rtl.expressions.iter().map(vars_used_by_ir_expression).sum()
        }

        // All other expressions don't directly consume variable slots
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
        | IrExpression::Ast(_)
        | IrExpression::ExpressionRef(_)
        | IrExpression::OutputExpr(_)
        | IrExpression::ArrowFunction(_) => 0,
    }
}

//! AST expression conversion.
//!
//! This module converts parsed Angular template expressions (AngularExpression)
//! into IR expressions or Output expressions for code generation.
//!
//! Ported from Angular's `template/pipeline/src/ingest.ts` `convertAst` function
//! and `template/pipeline/src/conversion.ts`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_span::{Ident, Span};

use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, BinaryOperator as AstBinaryOperator, LiteralMapKey,
    UnaryOperator as AstUnaryOperator,
};
use crate::ir::expression::{
    ContextExpr, EmptyExpr as IrEmptyExpr, IrExpression, LexicalReadExpr, PipeBindingExpr,
    SafeInvokeFunctionExpr, SafeKeyedReadExpr, SafePropertyReadExpr, SlotHandle,
};
use crate::ir::ops::XrefId;
use crate::output::ast::{
    BinaryOperator as OutputBinaryOperator, BinaryOperatorExpr, ConditionalExpr,
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, NotExpr, OutputExpression, ParenthesizedExpr, ReadKeyExpr, ReadPropExpr,
    RegularExpressionLiteralExpr, TaggedTemplateLiteralExpr, TemplateLiteralElement,
    TemplateLiteralExpr, TypeofExpr, UnaryOperator as OutputUnaryOperator, UnaryOperatorExpr,
    VoidExpr,
};

use super::compilation::ComponentCompilationJob;

// ============================================================================
// Converted Expression
// ============================================================================

/// The result of converting an Angular expression.
///
/// Expressions can be converted to either IR expressions (which need further
/// transformation by compilation phases) or Output expressions (which are
/// ready for code generation).
#[derive(Debug)]
pub enum ConvertedExpression<'a> {
    /// An IR expression that will be transformed by compilation phases.
    Ir(IrExpression<'a>),
    /// An Output expression ready for code generation.
    Output(OutputExpression<'a>),
}

impl<'a> ConvertedExpression<'a> {
    /// Creates a new IR expression variant.
    pub fn ir(expr: IrExpression<'a>) -> Self {
        Self::Ir(expr)
    }

    /// Creates a new Output expression variant.
    pub fn output(expr: OutputExpression<'a>) -> Self {
        Self::Output(expr)
    }

    /// Returns true if this is an IR expression.
    pub fn is_ir(&self) -> bool {
        matches!(self, Self::Ir(_))
    }

    /// Returns true if this is an Output expression.
    pub fn is_output(&self) -> bool {
        matches!(self, Self::Output(_))
    }

    /// Tries to unwrap to an IR expression, returning `None` if this is an Output expression.
    pub fn try_unwrap_ir(self) -> Option<IrExpression<'a>> {
        match self {
            Self::Ir(expr) => Some(expr),
            Self::Output(_) => None,
        }
    }

    /// Unwraps to an IR expression, returning `None` if this is an Output expression.
    ///
    /// This is an alias for [`try_unwrap_ir`](Self::try_unwrap_ir).
    pub fn unwrap_ir(self) -> Option<IrExpression<'a>> {
        self.try_unwrap_ir()
    }

    /// Tries to unwrap to an Output expression, returning `None` if this is an IR expression.
    pub fn try_unwrap_output(self) -> Option<OutputExpression<'a>> {
        match self {
            Self::Output(expr) => Some(expr),
            Self::Ir(_) => None,
        }
    }

    /// Unwraps to an Output expression, returning `None` if this is an IR expression.
    ///
    /// This is an alias for [`try_unwrap_output`](Self::try_unwrap_output).
    pub fn unwrap_output(self) -> Option<OutputExpression<'a>> {
        self.try_unwrap_output()
    }

    /// Converts to an Output expression.
    ///
    /// IR expressions are wrapped in `OutputExpression::WrappedNode` for later processing.
    /// This is a fallback - ideally IR expressions should be handled separately.
    pub fn to_output(self, allocator: &'a Allocator) -> OutputExpression<'a> {
        match self {
            Self::Output(expr) => expr,
            Self::Ir(_ir_expr) => {
                // Wrap IR expression in IrExpression::OutputExpr is not possible here
                // since OutputExpression doesn't have an IR variant.
                // This case should be handled by the caller appropriately.
                // For now, we'll create a placeholder.
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                    allocator,
                ))
            }
        }
    }

    /// Converts to an IR expression.
    ///
    /// Output expressions are wrapped in `IrExpression::OutputExpr`.
    pub fn to_ir(self, allocator: &'a Allocator) -> IrExpression<'a> {
        match self {
            Self::Ir(expr) => expr,
            Self::Output(output_expr) => {
                IrExpression::OutputExpr(Box::new_in(output_expr, allocator))
            }
        }
    }
}

// ============================================================================
// Binary Operator Conversion
// ============================================================================

/// Converts an AST binary operator to an Output binary operator.
pub fn convert_binary_operator(op: AstBinaryOperator) -> OutputBinaryOperator {
    match op {
        AstBinaryOperator::Add => OutputBinaryOperator::Plus,
        AstBinaryOperator::Subtract => OutputBinaryOperator::Minus,
        AstBinaryOperator::Multiply => OutputBinaryOperator::Multiply,
        AstBinaryOperator::Divide => OutputBinaryOperator::Divide,
        AstBinaryOperator::Modulo => OutputBinaryOperator::Modulo,
        AstBinaryOperator::Power => OutputBinaryOperator::Exponentiation,
        AstBinaryOperator::Equal => OutputBinaryOperator::Equals,
        AstBinaryOperator::NotEqual => OutputBinaryOperator::NotEquals,
        AstBinaryOperator::StrictEqual => OutputBinaryOperator::Identical,
        AstBinaryOperator::StrictNotEqual => OutputBinaryOperator::NotIdentical,
        AstBinaryOperator::LessThan => OutputBinaryOperator::Lower,
        AstBinaryOperator::LessThanOrEqual => OutputBinaryOperator::LowerEquals,
        AstBinaryOperator::GreaterThan => OutputBinaryOperator::Bigger,
        AstBinaryOperator::GreaterThanOrEqual => OutputBinaryOperator::BiggerEquals,
        AstBinaryOperator::And => OutputBinaryOperator::And,
        AstBinaryOperator::Or => OutputBinaryOperator::Or,
        AstBinaryOperator::NullishCoalescing => OutputBinaryOperator::NullishCoalesce,
        AstBinaryOperator::In => OutputBinaryOperator::In,
        AstBinaryOperator::Instanceof => OutputBinaryOperator::Instanceof,
        AstBinaryOperator::Assign => OutputBinaryOperator::Assign,
        AstBinaryOperator::AddAssign => OutputBinaryOperator::AdditionAssignment,
        AstBinaryOperator::SubtractAssign => OutputBinaryOperator::SubtractionAssignment,
        AstBinaryOperator::MultiplyAssign => OutputBinaryOperator::MultiplicationAssignment,
        AstBinaryOperator::DivideAssign => OutputBinaryOperator::DivisionAssignment,
        AstBinaryOperator::ModuloAssign => OutputBinaryOperator::RemainderAssignment,
        AstBinaryOperator::PowerAssign => OutputBinaryOperator::ExponentiationAssignment,
        AstBinaryOperator::AndAssign => OutputBinaryOperator::AndAssignment,
        AstBinaryOperator::OrAssign => OutputBinaryOperator::OrAssignment,
        AstBinaryOperator::NullishCoalescingAssign => {
            OutputBinaryOperator::NullishCoalesceAssignment
        }
    }
}

/// Converts an AST unary operator to an Output unary operator.
pub fn convert_unary_operator(op: AstUnaryOperator) -> OutputUnaryOperator {
    match op {
        AstUnaryOperator::Plus => OutputUnaryOperator::Plus,
        AstUnaryOperator::Minus => OutputUnaryOperator::Minus,
    }
}

// ============================================================================
// Template Literal Raw Text
// ============================================================================

/// Converts cooked template literal text to raw text.
///
/// Template literals have two representations:
/// - **Cooked text**: The interpreted value with escape sequences processed
///   (e.g., `\n` becomes a newline character)
/// - **Raw text**: The literal source text with escape sequences preserved
///   (e.g., `\n` stays as two characters: backslash and 'n')
///
/// Since the Angular expression parser gives us cooked text, we need to
/// re-escape it for contexts that need raw text (template literal emission).
///
/// This function escapes:
/// - Backticks (`) to prevent closing the template literal
/// - `${` to prevent interpolation syntax
/// - Backslashes to preserve escape sequences
/// - Carriage returns and line feeds to their escape sequences
fn cooked_to_raw_text<'a>(allocator: &'a Allocator, cooked: &str) -> Ident<'a> {
    // Fast path: if no escaping needed, return as-is
    if !cooked.contains(['`', '$', '\\', '\r', '\n']) {
        return Ident::from(allocator.alloc_str(cooked));
    }

    // Escape special characters
    let mut raw = String::with_capacity(cooked.len() + 8);
    let mut chars = cooked.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => raw.push_str("\\`"),
            '\\' => raw.push_str("\\\\"),
            '\r' => raw.push_str("\\r"),
            '\n' => raw.push_str("\\n"),
            '$' => {
                // Only escape if followed by { to form ${
                if chars.peek() == Some(&'{') {
                    raw.push_str("\\$");
                } else {
                    raw.push('$');
                }
            }
            _ => raw.push(c),
        }
    }
    Ident::from(allocator.alloc_str(&raw))
}

// ============================================================================
// Source Span Conversion
// ============================================================================

/// Converts an AbsoluteSourceSpan to an oxc_span::Span.
pub fn convert_source_span(span: AbsoluteSourceSpan) -> Option<Span> {
    Some(span.to_span())
}

// ============================================================================
// AST Conversion
// ============================================================================

/// Context for AST conversion, containing the compilation job and allocator.
pub struct ConversionContext<'a, 'job> {
    /// The allocator for creating new nodes.
    pub allocator: &'a Allocator,
    /// The compilation job (for allocating xref IDs, etc.).
    pub job: &'job mut ComponentCompilationJob<'a>,
}

impl<'a, 'job> ConversionContext<'a, 'job> {
    /// Creates a new conversion context.
    pub fn new(allocator: &'a Allocator, job: &'job mut ComponentCompilationJob<'a>) -> Self {
        Self { allocator, job }
    }

    /// Allocates a new xref ID.
    pub fn allocate_xref_id(&mut self) -> XrefId {
        self.job.allocate_xref_id()
    }

    /// Returns the root view's xref ID.
    pub fn root_xref(&self) -> XrefId {
        self.job.root.xref
    }
}

/// Converts an Angular expression AST to a ConvertedExpression.
///
/// This is the main conversion function that recursively transforms parsed
/// Angular template expressions into IR or Output expressions.
///
/// Ported from Angular's `convertAst` function in `ingest.ts`.
pub fn convert_ast<'a>(
    allocator: &'a Allocator,
    ast: &AngularExpression<'a>,
    root_xref: XrefId,
    allocate_xref_id: &mut impl FnMut() -> XrefId,
) -> ConvertedExpression<'a> {
    match ast {
        // Empty expression
        AngularExpression::Empty(e) => ConvertedExpression::ir(IrExpression::Empty(Box::new_in(
            IrEmptyExpr { source_span: convert_source_span(e.source_span) },
            allocator,
        ))),

        // Implicit receiver - converted to lexical read or context
        AngularExpression::ImplicitReceiver(_) => {
            // ImplicitReceiver by itself doesn't make sense - it's usually
            // the receiver of a PropertyRead. Return empty as placeholder.
            ConvertedExpression::ir(IrExpression::Empty(Box::new_in(
                IrEmptyExpr { source_span: None },
                allocator,
            )))
        }

        // This receiver - represents access to component context
        AngularExpression::ThisReceiver(e) => {
            ConvertedExpression::ir(IrExpression::Context(Box::new_in(
                ContextExpr { view: root_xref, source_span: convert_source_span(e.source_span) },
                allocator,
            )))
        }

        // Property read - either lexical read (implicit receiver) or property access
        AngularExpression::PropertyRead(pr) => {
            // Check if the receiver is an implicit receiver (not explicit `this`)
            let is_implicit_receiver =
                matches!(&pr.receiver, AngularExpression::ImplicitReceiver(_))
                    && !matches!(&pr.receiver, AngularExpression::ThisReceiver(_));

            // Check if the receiver is explicit `this`
            let is_this_receiver = matches!(&pr.receiver, AngularExpression::ThisReceiver(_));

            if is_implicit_receiver {
                // Implicit receiver property read becomes a lexical read
                ConvertedExpression::ir(IrExpression::LexicalRead(Box::new_in(
                    LexicalReadExpr {
                        name: pr.name.clone(),
                        source_span: convert_source_span(pr.source_span),
                    },
                    allocator,
                )))
            } else if is_this_receiver {
                // Explicit `this` property read (e.g., `this.formGroup`) becomes a
                // ResolvedPropertyRead with Context receiver. This is critical for embedded
                // views because the resolve phases need to see the ContextExpr(root_xref)
                // to properly generate nextContext() calls.
                //
                // Without this, `this.formGroup` would go directly to OutputExpression::ReadProp,
                // bypassing the IR phases, and embedded views would incorrectly use `ctx.formGroup`
                // instead of `ctx_r.formGroup` (after nextContext()).
                ConvertedExpression::ir(IrExpression::ResolvedPropertyRead(Box::new_in(
                    crate::ir::expression::ResolvedPropertyReadExpr {
                        receiver: Box::new_in(
                            IrExpression::Context(Box::new_in(
                                ContextExpr {
                                    view: root_xref,
                                    source_span: convert_source_span(pr.source_span),
                                },
                                allocator,
                            )),
                            allocator,
                        ),
                        name: pr.name.clone(),
                        source_span: convert_source_span(pr.source_span),
                    },
                    allocator,
                )))
            } else {
                // Explicit receiver property read becomes ReadPropExpr
                let receiver = convert_ast(allocator, &pr.receiver, root_xref, allocate_xref_id);
                ConvertedExpression::output(OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(receiver.to_output(allocator), allocator),
                        name: pr.name.clone(),
                        optional: false,
                        source_span: convert_source_span(pr.source_span),
                    },
                    allocator,
                )))
            }
        }

        // Safe property read - becomes IR SafePropertyReadExpr
        AngularExpression::SafePropertyRead(spr) => {
            let receiver = convert_ast(allocator, &spr.receiver, root_xref, allocate_xref_id);
            ConvertedExpression::ir(IrExpression::SafePropertyRead(Box::new_in(
                SafePropertyReadExpr {
                    receiver: Box::new_in(receiver.to_ir(allocator), allocator),
                    name: spr.name.clone(),
                    source_span: convert_source_span(spr.source_span),
                },
                allocator,
            )))
        }

        // Keyed read - becomes ReadKeyExpr
        AngularExpression::KeyedRead(kr) => {
            let receiver = convert_ast(allocator, &kr.receiver, root_xref, allocate_xref_id);
            let key = convert_ast(allocator, &kr.key, root_xref, allocate_xref_id);
            ConvertedExpression::output(OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver.to_output(allocator), allocator),
                    index: Box::new_in(key.to_output(allocator), allocator),
                    optional: false,
                    source_span: convert_source_span(kr.source_span),
                },
                allocator,
            )))
        }

        // Safe keyed read - becomes IR SafeKeyedReadExpr
        AngularExpression::SafeKeyedRead(skr) => {
            let receiver = convert_ast(allocator, &skr.receiver, root_xref, allocate_xref_id);
            let key = convert_ast(allocator, &skr.key, root_xref, allocate_xref_id);
            ConvertedExpression::ir(IrExpression::SafeKeyedRead(Box::new_in(
                SafeKeyedReadExpr {
                    receiver: Box::new_in(receiver.to_ir(allocator), allocator),
                    index: Box::new_in(key.to_ir(allocator), allocator),
                    source_span: convert_source_span(skr.source_span),
                },
                allocator,
            )))
        }

        // Function call
        AngularExpression::Call(call) => {
            // Note: ImplicitReceiver in Call expression should be caught by parser.
            // If it reaches here, the receiver conversion will handle it gracefully.
            let receiver = convert_ast(allocator, &call.receiver, root_xref, allocate_xref_id);
            let mut args = Vec::with_capacity_in(call.args.len(), allocator);
            for arg in call.args.iter() {
                let converted = convert_ast(allocator, arg, root_xref, allocate_xref_id);
                args.push(converted.to_output(allocator));
            }

            ConvertedExpression::output(OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(receiver.to_output(allocator), allocator),
                    args,
                    pure: false,
                    optional: false,
                    source_span: convert_source_span(call.source_span),
                },
                allocator,
            )))
        }

        // Safe function call - becomes IR SafeInvokeFunctionExpr
        AngularExpression::SafeCall(sc) => {
            let receiver = convert_ast(allocator, &sc.receiver, root_xref, allocate_xref_id);
            let mut args = Vec::with_capacity_in(sc.args.len(), allocator);
            for arg in sc.args.iter() {
                let converted = convert_ast(allocator, arg, root_xref, allocate_xref_id);
                args.push(converted.to_ir(allocator));
            }

            ConvertedExpression::ir(IrExpression::SafeInvokeFunction(Box::new_in(
                SafeInvokeFunctionExpr {
                    receiver: Box::new_in(receiver.to_ir(allocator), allocator),
                    args,
                    source_span: convert_source_span(sc.source_span),
                },
                allocator,
            )))
        }

        // Pipe binding - becomes IR PipeBindingExpr
        AngularExpression::BindingPipe(pipe) => {
            let xref = allocate_xref_id();
            let exp = convert_ast(allocator, &pipe.exp, root_xref, allocate_xref_id);

            let mut args = Vec::with_capacity_in(pipe.args.len() + 1, allocator);
            args.push(exp.to_ir(allocator));
            for arg in pipe.args.iter() {
                let converted = convert_ast(allocator, arg, root_xref, allocate_xref_id);
                args.push(converted.to_ir(allocator));
            }

            ConvertedExpression::ir(IrExpression::PipeBinding(Box::new_in(
                PipeBindingExpr {
                    target: xref,
                    target_slot: SlotHandle::new(),
                    name: pipe.name.clone(),
                    args,
                    var_offset: None,
                    source_span: convert_source_span(pipe.source_span),
                },
                allocator,
            )))
        }

        // Literal primitive
        AngularExpression::LiteralPrimitive(lit) => {
            let value = match &lit.value {
                crate::ast::expression::LiteralValue::Null => LiteralValue::Null,
                crate::ast::expression::LiteralValue::Undefined => LiteralValue::Undefined,
                crate::ast::expression::LiteralValue::Boolean(b) => LiteralValue::Boolean(*b),
                crate::ast::expression::LiteralValue::Number(n) => LiteralValue::Number(*n),
                crate::ast::expression::LiteralValue::String(s) => LiteralValue::String(s.clone()),
            };
            ConvertedExpression::output(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value, source_span: convert_source_span(lit.source_span) },
                allocator,
            )))
        }

        // Literal array
        AngularExpression::LiteralArray(arr) => {
            let mut entries = Vec::with_capacity_in(arr.expressions.len(), allocator);
            for expr in arr.expressions.iter() {
                let converted = convert_ast(allocator, expr, root_xref, allocate_xref_id);
                entries.push(converted.to_output(allocator));
            }
            ConvertedExpression::output(OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: convert_source_span(arr.source_span) },
                allocator,
            )))
        }

        // Literal map (object)
        AngularExpression::LiteralMap(map) => {
            let mut entries = Vec::with_capacity_in(map.keys.len(), allocator);
            for (key, value) in map.keys.iter().zip(map.values.iter()) {
                let converted_value = convert_ast(allocator, value, root_xref, allocate_xref_id);
                // Only handle property keys for now; spread keys need special handling
                if let LiteralMapKey::Property(prop) = key {
                    entries.push(LiteralMapEntry {
                        key: prop.key.clone(),
                        value: converted_value.to_output(allocator),
                        quoted: prop.quoted,
                    });
                }
                // TODO: Handle spread keys when needed
            }
            ConvertedExpression::output(OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: convert_source_span(map.source_span) },
                allocator,
            )))
        }

        // Conditional (ternary)
        AngularExpression::Conditional(cond) => {
            let condition = convert_ast(allocator, &cond.condition, root_xref, allocate_xref_id);
            let true_case = convert_ast(allocator, &cond.true_exp, root_xref, allocate_xref_id);
            let false_case = convert_ast(allocator, &cond.false_exp, root_xref, allocate_xref_id);

            ConvertedExpression::output(OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(condition.to_output(allocator), allocator),
                    true_case: Box::new_in(true_case.to_output(allocator), allocator),
                    false_case: Some(Box::new_in(false_case.to_output(allocator), allocator)),
                    source_span: convert_source_span(cond.source_span),
                },
                allocator,
            )))
        }

        // Binary expression
        AngularExpression::Binary(bin) => {
            let operator = convert_binary_operator(bin.operation);
            let left = convert_ast(allocator, &bin.left, root_xref, allocate_xref_id);
            let right = convert_ast(allocator, &bin.right, root_xref, allocate_xref_id);

            ConvertedExpression::output(OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(left.to_output(allocator), allocator),
                    rhs: Box::new_in(right.to_output(allocator), allocator),
                    source_span: convert_source_span(bin.source_span),
                },
                allocator,
            )))
        }

        // Unary expression
        AngularExpression::Unary(unary) => {
            let operator = convert_unary_operator(unary.operator);
            let expr = convert_ast(allocator, &unary.expr, root_xref, allocate_xref_id);

            ConvertedExpression::output(OutputExpression::UnaryOperator(Box::new_in(
                UnaryOperatorExpr {
                    operator,
                    expr: Box::new_in(expr.to_output(allocator), allocator),
                    parens: false,
                    source_span: convert_source_span(unary.source_span),
                },
                allocator,
            )))
        }

        // Prefix not
        AngularExpression::PrefixNot(not) => {
            let expr = convert_ast(allocator, &not.expression, root_xref, allocate_xref_id);
            ConvertedExpression::output(OutputExpression::Not(Box::new_in(
                NotExpr {
                    condition: Box::new_in(expr.to_output(allocator), allocator),
                    source_span: convert_source_span(not.source_span),
                },
                allocator,
            )))
        }

        // Typeof expression
        AngularExpression::TypeofExpression(type_of) => {
            let expr = convert_ast(allocator, &type_of.expression, root_xref, allocate_xref_id);
            ConvertedExpression::output(OutputExpression::Typeof(Box::new_in(
                TypeofExpr {
                    expr: Box::new_in(expr.to_output(allocator), allocator),
                    source_span: convert_source_span(type_of.source_span),
                },
                allocator,
            )))
        }

        // Void expression
        AngularExpression::VoidExpression(void_expr) => {
            let expr = convert_ast(allocator, &void_expr.expression, root_xref, allocate_xref_id);
            ConvertedExpression::output(OutputExpression::Void(Box::new_in(
                VoidExpr {
                    expr: Box::new_in(expr.to_output(allocator), allocator),
                    source_span: convert_source_span(void_expr.source_span),
                },
                allocator,
            )))
        }

        // Non-null assertion - just unwrap, it doesn't affect runtime
        AngularExpression::NonNullAssert(nna) => {
            convert_ast(allocator, &nna.expression, root_xref, allocate_xref_id)
        }

        // Parenthesized expression
        AngularExpression::ParenthesizedExpression(paren) => {
            let expr = convert_ast(allocator, &paren.expression, root_xref, allocate_xref_id);
            ConvertedExpression::output(OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr {
                    expr: Box::new_in(expr.to_output(allocator), allocator),
                    source_span: convert_source_span(paren.source_span),
                },
                allocator,
            )))
        }

        // Template literal
        AngularExpression::TemplateLiteral(tl) => {
            let mut elements = Vec::with_capacity_in(tl.elements.len(), allocator);
            for elem in tl.elements.iter() {
                elements.push(TemplateLiteralElement {
                    text: elem.text.clone(),
                    raw_text: cooked_to_raw_text(allocator, &elem.text),
                    source_span: convert_source_span(elem.source_span),
                });
            }

            let mut expressions = Vec::with_capacity_in(tl.expressions.len(), allocator);
            for expr in tl.expressions.iter() {
                let converted = convert_ast(allocator, expr, root_xref, allocate_xref_id);
                expressions.push(converted.to_output(allocator));
            }

            ConvertedExpression::output(OutputExpression::TemplateLiteral(Box::new_in(
                TemplateLiteralExpr {
                    elements,
                    expressions,
                    source_span: convert_source_span(tl.source_span),
                },
                allocator,
            )))
        }

        // Tagged template literal
        AngularExpression::TaggedTemplateLiteral(ttl) => {
            let tag = convert_ast(allocator, &ttl.tag, root_xref, allocate_xref_id);

            let mut elements = Vec::with_capacity_in(ttl.template.elements.len(), allocator);
            for elem in ttl.template.elements.iter() {
                elements.push(TemplateLiteralElement {
                    text: elem.text.clone(),
                    raw_text: cooked_to_raw_text(allocator, &elem.text),
                    source_span: convert_source_span(elem.source_span),
                });
            }

            let mut expressions = Vec::with_capacity_in(ttl.template.expressions.len(), allocator);
            for expr in ttl.template.expressions.iter() {
                let converted = convert_ast(allocator, expr, root_xref, allocate_xref_id);
                expressions.push(converted.to_output(allocator));
            }

            ConvertedExpression::output(OutputExpression::TaggedTemplateLiteral(Box::new_in(
                TaggedTemplateLiteralExpr {
                    tag: Box::new_in(tag.to_output(allocator), allocator),
                    template: Box::new_in(
                        TemplateLiteralExpr {
                            elements,
                            expressions,
                            source_span: convert_source_span(ttl.template.source_span),
                        },
                        allocator,
                    ),
                    source_span: convert_source_span(ttl.source_span),
                },
                allocator,
            )))
        }

        // Regular expression literal
        AngularExpression::RegularExpressionLiteral(re) => {
            ConvertedExpression::output(OutputExpression::RegularExpressionLiteral(Box::new_in(
                RegularExpressionLiteralExpr {
                    body: re.body.clone(),
                    flags: re.flags.clone(),
                    source_span: convert_source_span(re.source_span),
                },
                allocator,
            )))
        }

        // Chain - multiple expressions (not typically used in templates)
        // Chain expressions should be handled at a higher level (e.g., event handlers).
        // If we encounter one here, return an empty placeholder as graceful fallback.
        AngularExpression::Chain(chain) => {
            ConvertedExpression::ir(IrExpression::Empty(Box::new_in(
                IrEmptyExpr { source_span: convert_source_span(chain.source_span) },
                allocator,
            )))
        }

        // Interpolation - converted to IR Interpolation
        AngularExpression::Interpolation(interp) => {
            let mut strings = Vec::with_capacity_in(interp.strings.len(), allocator);
            for s in interp.strings.iter() {
                strings.push(s.clone());
            }

            let mut expressions = Vec::with_capacity_in(interp.expressions.len(), allocator);
            for expr in interp.expressions.iter() {
                let converted = convert_ast(allocator, expr, root_xref, allocate_xref_id);
                expressions.push(converted.to_ir(allocator));
            }

            ConvertedExpression::ir(IrExpression::Interpolation(Box::new_in(
                crate::ir::expression::Interpolation {
                    strings,
                    expressions,
                    i18n_placeholders: Vec::new_in(allocator),
                    source_span: convert_source_span(interp.source_span),
                },
                allocator,
            )))
        }

        // Spread element - convert the inner expression
        AngularExpression::SpreadElement(spread) => {
            // SpreadElement is used inside arrays/objects/calls
            // For now, just convert the inner expression
            convert_ast(allocator, &spread.expression, root_xref, allocate_xref_id)
        }

        // Arrow function - convert parameters and body
        AngularExpression::ArrowFunction(arrow) => {
            // Arrow functions in Angular templates are used as callbacks
            // Convert the body expression and wrap in ArrowFunctionExpr
            let body = convert_ast(allocator, &arrow.body, root_xref, allocate_xref_id);
            let mut params = Vec::with_capacity_in(arrow.parameters.len(), allocator);
            for p in arrow.parameters.iter() {
                params.push(crate::output::ast::FnParam { name: p.name.clone() });
            }
            ConvertedExpression::ir(IrExpression::ArrowFunction(Box::new_in(
                crate::ir::expression::ArrowFunctionExpr {
                    params,
                    body: Box::new_in(body.to_ir(allocator), allocator),
                    ops: Vec::new_in(allocator),
                    var_offset: None,
                    source_span: convert_source_span(arrow.source_span),
                },
                allocator,
            )))
        }
    }
}

/// Converts an Angular expression with interpolation support.
///
/// If the expression is an Interpolation, it returns an IR Interpolation.
/// Otherwise, it converts the expression normally.
pub fn convert_ast_with_interpolation<'a>(
    allocator: &'a Allocator,
    ast: &AngularExpression<'a>,
    root_xref: XrefId,
    allocate_xref_id: &mut impl FnMut() -> XrefId,
) -> ConvertedExpression<'a> {
    // For now, just delegate to convert_ast
    // The TypeScript version handles i18n placeholders here
    convert_ast(allocator, ast, root_xref, allocate_xref_id)
}

// ============================================================================
// Namespace Utilities
// ============================================================================

use crate::ir::enums::Namespace;

/// Converts a namespace prefix key to a Namespace enum.
pub fn namespace_for_key(namespace_prefix_key: Option<&str>) -> Namespace {
    match namespace_prefix_key {
        Some("svg") => Namespace::Svg,
        Some("math") => Namespace::Math,
        _ => Namespace::Html,
    }
}

/// Converts a Namespace enum to its prefix key.
pub fn key_for_namespace(namespace: Namespace) -> Option<&'static str> {
    match namespace {
        Namespace::Svg => Some("svg"),
        Namespace::Math => Some("math"),
        Namespace::Html => None,
    }
}

/// Prefixes a tag name with its namespace.
pub fn prefix_with_namespace(stripped_tag: &str, namespace: Namespace) -> String {
    match key_for_namespace(namespace) {
        Some(ns_key) => format!(":{ns_key}:{stripped_tag}"),
        None => stripped_tag.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_operator_conversion() {
        assert!(matches!(
            convert_binary_operator(AstBinaryOperator::Add),
            OutputBinaryOperator::Plus
        ));
        assert!(matches!(
            convert_binary_operator(AstBinaryOperator::And),
            OutputBinaryOperator::And
        ));
        assert!(matches!(
            convert_binary_operator(AstBinaryOperator::StrictEqual),
            OutputBinaryOperator::Identical
        ));
    }

    #[test]
    fn test_unary_operator_conversion() {
        assert!(matches!(
            convert_unary_operator(AstUnaryOperator::Plus),
            OutputUnaryOperator::Plus
        ));
        assert!(matches!(
            convert_unary_operator(AstUnaryOperator::Minus),
            OutputUnaryOperator::Minus
        ));
    }

    #[test]
    fn test_namespace_for_key() {
        assert!(matches!(namespace_for_key(None), Namespace::Html));
        assert!(matches!(namespace_for_key(Some("svg")), Namespace::Svg));
        assert!(matches!(namespace_for_key(Some("math")), Namespace::Math));
        assert!(matches!(namespace_for_key(Some("unknown")), Namespace::Html));
    }

    #[test]
    fn test_prefix_with_namespace() {
        assert_eq!(prefix_with_namespace("div", Namespace::Html), "div");
        assert_eq!(prefix_with_namespace("rect", Namespace::Svg), ":svg:rect");
        assert_eq!(prefix_with_namespace("mi", Namespace::Math), ":math:mi");
    }

    #[test]
    fn test_cooked_to_raw_text() {
        use oxc_allocator::Allocator;

        let allocator = Allocator::default();

        // No escaping needed
        assert_eq!(super::cooked_to_raw_text(&allocator, "hello world").as_str(), "hello world");

        // Escape backticks
        assert_eq!(
            super::cooked_to_raw_text(&allocator, "hello `world`").as_str(),
            "hello \\`world\\`"
        );

        // Escape backslashes
        assert_eq!(
            super::cooked_to_raw_text(&allocator, "path\\to\\file").as_str(),
            "path\\\\to\\\\file"
        );

        // Escape newlines and carriage returns
        assert_eq!(
            super::cooked_to_raw_text(&allocator, "line1\nline2\rline3").as_str(),
            "line1\\nline2\\rline3"
        );

        // Escape ${
        assert_eq!(
            super::cooked_to_raw_text(&allocator, "value is ${x}").as_str(),
            "value is \\${x}"
        );

        // Do not escape $ without {
        assert_eq!(super::cooked_to_raw_text(&allocator, "price $5").as_str(), "price $5");
    }
}

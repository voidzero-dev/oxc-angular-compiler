//! Angular expression AST nodes.
//!
//! This module contains all AST node types for Angular template expressions,
//! ported from Angular's `expression_parser/ast.ts`.
//!
//! Angular expressions are a subset of JavaScript expressions with:
//! - Pipes: `value | pipeName:arg1:arg2`
//! - Safe navigation: `object?.property`
//! - Interpolation: `{{ expression }}`
//! - No assignment operators (except in event handlers)
//! - No bitwise operators

use oxc_allocator::{Allocator, Box, Vec};
use oxc_span::Span;
use oxc_str::Ident;

/// A span within the expression source.
#[derive(Debug, Clone, Copy)]
pub struct ParseSpan {
    /// Start offset (relative to expression start).
    pub start: u32,
    /// End offset (relative to expression start).
    pub end: u32,
}

impl ParseSpan {
    /// Creates a new `ParseSpan`.
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Converts to an absolute span.
    pub fn to_absolute(&self, absolute_offset: u32) -> AbsoluteSourceSpan {
        AbsoluteSourceSpan::new(absolute_offset + self.start, absolute_offset + self.end)
    }
}

/// An absolute span in the source file.
#[derive(Debug, Clone, Copy)]
pub struct AbsoluteSourceSpan {
    /// Start byte offset in the source file.
    pub start: u32,
    /// End byte offset in the source file.
    pub end: u32,
}

impl AbsoluteSourceSpan {
    /// Creates a new `AbsoluteSourceSpan`.
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Converts to an `oxc_span::Span`.
    pub fn to_span(&self) -> Span {
        Span::new(self.start, self.end)
    }
}

/// The main Angular expression enum containing all expression types.
///
/// All variants are boxed to maintain a 16-byte enum size.
#[derive(Debug)]
pub enum AngularExpression<'a> {
    /// An empty expression.
    Empty(Box<'a, EmptyExpr>),
    /// An implicit receiver (the component context).
    ImplicitReceiver(Box<'a, ImplicitReceiver>),
    /// An explicit `this` receiver.
    ThisReceiver(Box<'a, ThisReceiver>),
    /// Multiple expressions separated by semicolons.
    Chain(Box<'a, Chain<'a>>),
    /// A ternary conditional: `condition ? trueExpr : falseExpr`.
    Conditional(Box<'a, Conditional<'a>>),
    /// A property read: `receiver.property`.
    PropertyRead(Box<'a, PropertyRead<'a>>),
    /// A safe property read: `receiver?.property`.
    SafePropertyRead(Box<'a, SafePropertyRead<'a>>),
    /// A keyed read: `receiver[key]`.
    KeyedRead(Box<'a, KeyedRead<'a>>),
    /// A safe keyed read: `receiver?.[key]`.
    SafeKeyedRead(Box<'a, SafeKeyedRead<'a>>),
    /// A pipe expression: `value | pipeName:arg1:arg2`.
    BindingPipe(Box<'a, BindingPipe<'a>>),
    /// A primitive literal (string, number, boolean, null, undefined).
    LiteralPrimitive(Box<'a, LiteralPrimitive<'a>>),
    /// An array literal: `[a, b, c]`.
    LiteralArray(Box<'a, LiteralArray<'a>>),
    /// An object literal: `{a: 1, b: 2}`.
    LiteralMap(Box<'a, LiteralMap<'a>>),
    /// A string interpolation: `{{ expr1 }} text {{ expr2 }}`.
    Interpolation(Box<'a, Interpolation<'a>>),
    /// A binary expression: `left op right`.
    Binary(Box<'a, Binary<'a>>),
    /// A unary expression: `-expr` or `+expr`.
    Unary(Box<'a, Unary<'a>>),
    /// A prefix not: `!expr`.
    PrefixNot(Box<'a, PrefixNot<'a>>),
    /// A typeof expression: `typeof expr`.
    TypeofExpression(Box<'a, TypeofExpression<'a>>),
    /// A void expression: `void expr`.
    VoidExpression(Box<'a, VoidExpression<'a>>),
    /// A non-null assertion: `expr!`.
    NonNullAssert(Box<'a, NonNullAssert<'a>>),
    /// A function call: `receiver(args)`.
    Call(Box<'a, Call<'a>>),
    /// A safe function call: `receiver?.(args)`.
    SafeCall(Box<'a, SafeCall<'a>>),
    /// A tagged template literal: `tag\`template\``.
    TaggedTemplateLiteral(Box<'a, TaggedTemplateLiteral<'a>>),
    /// A template literal: `` `text ${expr} text` ``.
    TemplateLiteral(Box<'a, TemplateLiteral<'a>>),
    /// A parenthesized expression: `(expr)`.
    ParenthesizedExpression(Box<'a, ParenthesizedExpression<'a>>),
    /// A regular expression literal: `/pattern/flags`.
    RegularExpressionLiteral(Box<'a, RegularExpressionLiteral<'a>>),
    /// A spread element: `...expr`.
    SpreadElement(Box<'a, SpreadElement<'a>>),
    /// An arrow function: `(a, b) => expr`.
    ArrowFunction(Box<'a, ArrowFunction<'a>>),
}

impl<'a> AngularExpression<'a> {
    /// Returns the span of this expression.
    pub fn span(&self) -> ParseSpan {
        match self {
            Self::Empty(e) => e.span,
            Self::ImplicitReceiver(e) => e.span,
            Self::ThisReceiver(e) => e.span,
            Self::Chain(e) => e.span,
            Self::Conditional(e) => e.span,
            Self::PropertyRead(e) => e.span,
            Self::SafePropertyRead(e) => e.span,
            Self::KeyedRead(e) => e.span,
            Self::SafeKeyedRead(e) => e.span,
            Self::BindingPipe(e) => e.span,
            Self::LiteralPrimitive(e) => e.span,
            Self::LiteralArray(e) => e.span,
            Self::LiteralMap(e) => e.span,
            Self::Interpolation(e) => e.span,
            Self::Binary(e) => e.span,
            Self::Unary(e) => e.span,
            Self::PrefixNot(e) => e.span,
            Self::TypeofExpression(e) => e.span,
            Self::VoidExpression(e) => e.span,
            Self::NonNullAssert(e) => e.span,
            Self::Call(e) => e.span,
            Self::SafeCall(e) => e.span,
            Self::TaggedTemplateLiteral(e) => e.span,
            Self::TemplateLiteral(e) => e.span,
            Self::ParenthesizedExpression(e) => e.span,
            Self::RegularExpressionLiteral(e) => e.span,
            Self::SpreadElement(e) => e.span,
            Self::ArrowFunction(e) => e.span,
        }
    }

    /// Returns the source span of this expression.
    pub fn source_span(&self) -> AbsoluteSourceSpan {
        match self {
            Self::Empty(e) => e.source_span,
            Self::ImplicitReceiver(e) => e.source_span,
            Self::ThisReceiver(e) => e.source_span,
            Self::Chain(e) => e.source_span,
            Self::Conditional(e) => e.source_span,
            Self::PropertyRead(e) => e.source_span,
            Self::SafePropertyRead(e) => e.source_span,
            Self::KeyedRead(e) => e.source_span,
            Self::SafeKeyedRead(e) => e.source_span,
            Self::BindingPipe(e) => e.source_span,
            Self::LiteralPrimitive(e) => e.source_span,
            Self::LiteralArray(e) => e.source_span,
            Self::LiteralMap(e) => e.source_span,
            Self::Interpolation(e) => e.source_span,
            Self::Binary(e) => e.source_span,
            Self::Unary(e) => e.source_span,
            Self::PrefixNot(e) => e.source_span,
            Self::TypeofExpression(e) => e.source_span,
            Self::VoidExpression(e) => e.source_span,
            Self::NonNullAssert(e) => e.source_span,
            Self::Call(e) => e.source_span,
            Self::SafeCall(e) => e.source_span,
            Self::TaggedTemplateLiteral(e) => e.source_span,
            Self::TemplateLiteral(e) => e.source_span,
            Self::ParenthesizedExpression(e) => e.source_span,
            Self::RegularExpressionLiteral(e) => e.source_span,
            Self::SpreadElement(e) => e.source_span,
            Self::ArrowFunction(e) => e.source_span,
        }
    }

    /// Deep clones this expression into the given allocator.
    pub fn clone_in(&self, alloc: &'a Allocator) -> Self {
        match self {
            Self::Empty(e) => Self::Empty(Box::new_in(
                EmptyExpr { span: e.span, source_span: e.source_span },
                alloc,
            )),
            Self::ImplicitReceiver(e) => Self::ImplicitReceiver(Box::new_in(
                ImplicitReceiver { span: e.span, source_span: e.source_span },
                alloc,
            )),
            Self::ThisReceiver(e) => Self::ThisReceiver(Box::new_in(
                ThisReceiver { span: e.span, source_span: e.source_span },
                alloc,
            )),
            Self::Chain(e) => {
                let mut expressions = Vec::new_in(alloc);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(alloc));
                }
                Self::Chain(Box::new_in(
                    Chain { span: e.span, source_span: e.source_span, expressions },
                    alloc,
                ))
            }
            Self::Conditional(e) => Self::Conditional(Box::new_in(
                Conditional {
                    span: e.span,
                    source_span: e.source_span,
                    condition: e.condition.clone_in(alloc),
                    true_exp: e.true_exp.clone_in(alloc),
                    false_exp: e.false_exp.clone_in(alloc),
                },
                alloc,
            )),
            Self::PropertyRead(e) => Self::PropertyRead(Box::new_in(
                PropertyRead {
                    span: e.span,
                    source_span: e.source_span,
                    name_span: e.name_span,
                    receiver: e.receiver.clone_in(alloc),
                    name: e.name.clone(),
                },
                alloc,
            )),
            Self::SafePropertyRead(e) => Self::SafePropertyRead(Box::new_in(
                SafePropertyRead {
                    span: e.span,
                    source_span: e.source_span,
                    name_span: e.name_span,
                    receiver: e.receiver.clone_in(alloc),
                    name: e.name.clone(),
                },
                alloc,
            )),
            Self::KeyedRead(e) => Self::KeyedRead(Box::new_in(
                KeyedRead {
                    span: e.span,
                    source_span: e.source_span,
                    receiver: e.receiver.clone_in(alloc),
                    key: e.key.clone_in(alloc),
                },
                alloc,
            )),
            Self::SafeKeyedRead(e) => Self::SafeKeyedRead(Box::new_in(
                SafeKeyedRead {
                    span: e.span,
                    source_span: e.source_span,
                    receiver: e.receiver.clone_in(alloc),
                    key: e.key.clone_in(alloc),
                },
                alloc,
            )),
            Self::BindingPipe(e) => {
                let mut args = Vec::new_in(alloc);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(alloc));
                }
                Self::BindingPipe(Box::new_in(
                    BindingPipe {
                        span: e.span,
                        source_span: e.source_span,
                        name_span: e.name_span,
                        exp: e.exp.clone_in(alloc),
                        name: e.name.clone(),
                        args,
                        pipe_type: e.pipe_type,
                    },
                    alloc,
                ))
            }
            Self::LiteralPrimitive(e) => Self::LiteralPrimitive(Box::new_in(
                LiteralPrimitive {
                    span: e.span,
                    source_span: e.source_span,
                    value: e.value.clone(),
                },
                alloc,
            )),
            Self::LiteralArray(e) => {
                let mut expressions = Vec::new_in(alloc);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(alloc));
                }
                Self::LiteralArray(Box::new_in(
                    LiteralArray { span: e.span, source_span: e.source_span, expressions },
                    alloc,
                ))
            }
            Self::LiteralMap(e) => {
                let mut keys = Vec::new_in(alloc);
                for key in e.keys.iter() {
                    keys.push(match key {
                        LiteralMapKey::Property(prop) => {
                            LiteralMapKey::Property(LiteralMapPropertyKey {
                                key: prop.key.clone(),
                                quoted: prop.quoted,
                                is_shorthand_initialized: prop.is_shorthand_initialized,
                            })
                        }
                        LiteralMapKey::Spread(spread) => {
                            LiteralMapKey::Spread(LiteralMapSpreadKey {
                                span: spread.span,
                                source_span: spread.source_span,
                            })
                        }
                    });
                }
                let mut values = Vec::new_in(alloc);
                for val in e.values.iter() {
                    values.push(val.clone_in(alloc));
                }
                Self::LiteralMap(Box::new_in(
                    LiteralMap { span: e.span, source_span: e.source_span, keys, values },
                    alloc,
                ))
            }
            Self::Interpolation(e) => {
                let mut strings = Vec::new_in(alloc);
                for s in e.strings.iter() {
                    strings.push(s.clone());
                }
                let mut expressions = Vec::new_in(alloc);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(alloc));
                }
                Self::Interpolation(Box::new_in(
                    Interpolation {
                        span: e.span,
                        source_span: e.source_span,
                        strings,
                        expressions,
                    },
                    alloc,
                ))
            }
            Self::Binary(e) => Self::Binary(Box::new_in(
                Binary {
                    span: e.span,
                    source_span: e.source_span,
                    operation: e.operation.clone(),
                    left: e.left.clone_in(alloc),
                    right: e.right.clone_in(alloc),
                },
                alloc,
            )),
            Self::Unary(e) => Self::Unary(Box::new_in(
                Unary {
                    span: e.span,
                    source_span: e.source_span,
                    operator: e.operator.clone(),
                    expr: e.expr.clone_in(alloc),
                },
                alloc,
            )),
            Self::PrefixNot(e) => Self::PrefixNot(Box::new_in(
                PrefixNot {
                    span: e.span,
                    source_span: e.source_span,
                    expression: e.expression.clone_in(alloc),
                },
                alloc,
            )),
            Self::TypeofExpression(e) => Self::TypeofExpression(Box::new_in(
                TypeofExpression {
                    span: e.span,
                    source_span: e.source_span,
                    expression: e.expression.clone_in(alloc),
                },
                alloc,
            )),
            Self::VoidExpression(e) => Self::VoidExpression(Box::new_in(
                VoidExpression {
                    span: e.span,
                    source_span: e.source_span,
                    expression: e.expression.clone_in(alloc),
                },
                alloc,
            )),
            Self::NonNullAssert(e) => Self::NonNullAssert(Box::new_in(
                NonNullAssert {
                    span: e.span,
                    source_span: e.source_span,
                    expression: e.expression.clone_in(alloc),
                },
                alloc,
            )),
            Self::Call(e) => {
                let mut args = Vec::new_in(alloc);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(alloc));
                }
                Self::Call(Box::new_in(
                    Call {
                        span: e.span,
                        source_span: e.source_span,
                        receiver: e.receiver.clone_in(alloc),
                        args,
                        argument_span: e.argument_span,
                    },
                    alloc,
                ))
            }
            Self::SafeCall(e) => {
                let mut args = Vec::new_in(alloc);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(alloc));
                }
                Self::SafeCall(Box::new_in(
                    SafeCall {
                        span: e.span,
                        source_span: e.source_span,
                        receiver: e.receiver.clone_in(alloc),
                        args,
                        argument_span: e.argument_span,
                    },
                    alloc,
                ))
            }
            Self::TaggedTemplateLiteral(e) => {
                let mut elements = Vec::new_in(alloc);
                for el in e.template.elements.iter() {
                    elements.push(TemplateLiteralElement {
                        span: el.span,
                        source_span: el.source_span,
                        text: el.text.clone(),
                    });
                }
                let mut expressions = Vec::new_in(alloc);
                for expr in e.template.expressions.iter() {
                    expressions.push(expr.clone_in(alloc));
                }
                Self::TaggedTemplateLiteral(Box::new_in(
                    TaggedTemplateLiteral {
                        span: e.span,
                        source_span: e.source_span,
                        tag: e.tag.clone_in(alloc),
                        template: TemplateLiteral {
                            span: e.template.span,
                            source_span: e.template.source_span,
                            elements,
                            expressions,
                        },
                    },
                    alloc,
                ))
            }
            Self::TemplateLiteral(e) => {
                let mut elements = Vec::new_in(alloc);
                for el in e.elements.iter() {
                    elements.push(TemplateLiteralElement {
                        span: el.span,
                        source_span: el.source_span,
                        text: el.text.clone(),
                    });
                }
                let mut expressions = Vec::new_in(alloc);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(alloc));
                }
                Self::TemplateLiteral(Box::new_in(
                    TemplateLiteral {
                        span: e.span,
                        source_span: e.source_span,
                        elements,
                        expressions,
                    },
                    alloc,
                ))
            }
            Self::ParenthesizedExpression(e) => Self::ParenthesizedExpression(Box::new_in(
                ParenthesizedExpression {
                    span: e.span,
                    source_span: e.source_span,
                    expression: e.expression.clone_in(alloc),
                },
                alloc,
            )),
            Self::RegularExpressionLiteral(e) => Self::RegularExpressionLiteral(Box::new_in(
                RegularExpressionLiteral {
                    span: e.span,
                    source_span: e.source_span,
                    body: e.body.clone(),
                    flags: e.flags.clone(),
                },
                alloc,
            )),
            Self::SpreadElement(e) => Self::SpreadElement(Box::new_in(
                SpreadElement {
                    span: e.span,
                    source_span: e.source_span,
                    expression: e.expression.clone_in(alloc),
                },
                alloc,
            )),
            Self::ArrowFunction(e) => {
                let mut parameters = Vec::new_in(alloc);
                for param in e.parameters.iter() {
                    parameters.push(ArrowFunctionParameter {
                        name: param.name.clone(),
                        span: param.span,
                        source_span: param.source_span,
                    });
                }
                Self::ArrowFunction(Box::new_in(
                    ArrowFunction {
                        span: e.span,
                        source_span: e.source_span,
                        parameters,
                        body: e.body.clone_in(alloc),
                    },
                    alloc,
                ))
            }
        }
    }
}

// ============================================================================
// Expression node types
// ============================================================================

/// An empty expression (placeholder).
#[derive(Debug)]
pub struct EmptyExpr {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
}

/// An implicit receiver - represents access to the component context.
#[derive(Debug)]
pub struct ImplicitReceiver {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
}

/// An explicit `this` receiver.
#[derive(Debug)]
pub struct ThisReceiver {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
}

/// Multiple expressions separated by semicolons.
#[derive(Debug)]
pub struct Chain<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The expressions in the chain.
    pub expressions: Vec<'a, AngularExpression<'a>>,
}

/// A ternary conditional expression.
#[derive(Debug)]
pub struct Conditional<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The condition expression.
    pub condition: AngularExpression<'a>,
    /// The expression if true.
    pub true_exp: AngularExpression<'a>,
    /// The expression if false.
    pub false_exp: AngularExpression<'a>,
}

/// A property read expression: `receiver.property`.
#[derive(Debug)]
pub struct PropertyRead<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The span of the property name.
    pub name_span: AbsoluteSourceSpan,
    /// The receiver expression.
    pub receiver: AngularExpression<'a>,
    /// The property name.
    pub name: Ident<'a>,
}

/// A safe property read expression: `receiver?.property`.
#[derive(Debug)]
pub struct SafePropertyRead<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The span of the property name.
    pub name_span: AbsoluteSourceSpan,
    /// The receiver expression.
    pub receiver: AngularExpression<'a>,
    /// The property name.
    pub name: Ident<'a>,
}

/// A keyed read expression: `receiver[key]`.
#[derive(Debug)]
pub struct KeyedRead<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The receiver expression.
    pub receiver: AngularExpression<'a>,
    /// The key expression.
    pub key: AngularExpression<'a>,
}

/// A safe keyed read expression: `receiver?.[key]`.
#[derive(Debug)]
pub struct SafeKeyedRead<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The receiver expression.
    pub receiver: AngularExpression<'a>,
    /// The key expression.
    pub key: AngularExpression<'a>,
}

/// The type of pipe reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingPipeType {
    /// Pipe is referenced by its declared name.
    ReferencedByName,
    /// Pipe is referenced directly by class name.
    ReferencedDirectly,
}

/// A pipe expression: `value | pipeName:arg1:arg2`.
#[derive(Debug)]
pub struct BindingPipe<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The span of the pipe name.
    pub name_span: AbsoluteSourceSpan,
    /// The expression being piped.
    pub exp: AngularExpression<'a>,
    /// The pipe name.
    pub name: Ident<'a>,
    /// The pipe arguments.
    pub args: Vec<'a, AngularExpression<'a>>,
    /// The type of pipe reference.
    pub pipe_type: BindingPipeType,
}

/// A primitive literal value.
#[derive(Debug, Clone)]
pub enum LiteralValue<'a> {
    /// A string literal.
    String(Ident<'a>),
    /// A number literal.
    Number(f64),
    /// A boolean literal.
    Boolean(bool),
    /// The null literal.
    Null,
    /// The undefined literal.
    Undefined,
}

/// A primitive literal expression.
#[derive(Debug)]
pub struct LiteralPrimitive<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The literal value.
    pub value: LiteralValue<'a>,
}

/// An array literal expression.
#[derive(Debug)]
pub struct LiteralArray<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The array elements.
    pub expressions: Vec<'a, AngularExpression<'a>>,
}

/// A spread element: `...expr`.
#[derive(Debug)]
pub struct SpreadElement<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The expression being spread.
    pub expression: AngularExpression<'a>,
}

/// An arrow function parameter.
#[derive(Debug)]
pub struct ArrowFunctionParameter<'a> {
    /// The parameter name.
    pub name: Ident<'a>,
    /// The span of this parameter.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
}

/// An arrow function expression: `(a, b) => expr`.
#[derive(Debug)]
pub struct ArrowFunction<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The function parameters.
    pub parameters: Vec<'a, ArrowFunctionParameter<'a>>,
    /// The function body expression.
    pub body: AngularExpression<'a>,
}

/// A key in a literal map (object).
#[derive(Debug)]
pub enum LiteralMapKey<'a> {
    /// A property key: `key: value` or `"key": value`.
    Property(LiteralMapPropertyKey<'a>),
    /// A spread key: `...expr`.
    Spread(LiteralMapSpreadKey),
}

/// A property key in a literal map.
#[derive(Debug)]
pub struct LiteralMapPropertyKey<'a> {
    /// The key string.
    pub key: Ident<'a>,
    /// Whether the key is quoted.
    pub quoted: bool,
    /// Whether this is a shorthand initialization.
    pub is_shorthand_initialized: bool,
}

/// A spread key in a literal map.
#[derive(Debug)]
pub struct LiteralMapSpreadKey {
    /// The span of the spread key.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
}

/// An object literal expression.
#[derive(Debug)]
pub struct LiteralMap<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The object keys.
    pub keys: Vec<'a, LiteralMapKey<'a>>,
    /// The object values.
    pub values: Vec<'a, AngularExpression<'a>>,
}

/// An interpolation expression: `{{ expr }}`.
#[derive(Debug)]
pub struct Interpolation<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The static string parts.
    pub strings: Vec<'a, Ident<'a>>,
    /// The dynamic expression parts.
    pub expressions: Vec<'a, AngularExpression<'a>>,
}

/// A binary expression.
#[derive(Debug)]
pub struct Binary<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The operator.
    pub operation: BinaryOperator,
    /// The left operand.
    pub left: AngularExpression<'a>,
    /// The right operand.
    pub right: AngularExpression<'a>,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    // Arithmetic
    /// Addition `+`
    Add,
    /// Subtraction `-`
    Subtract,
    /// Multiplication `*`
    Multiply,
    /// Division `/`
    Divide,
    /// Modulo `%`
    Modulo,
    /// Exponentiation `**`
    Power,

    // Comparison
    /// Equality `==`
    Equal,
    /// Inequality `!=`
    NotEqual,
    /// Strict equality `===`
    StrictEqual,
    /// Strict inequality `!==`
    StrictNotEqual,
    /// Less than `<`
    LessThan,
    /// Less than or equal `<=`
    LessThanOrEqual,
    /// Greater than `>`
    GreaterThan,
    /// Greater than or equal `>=`
    GreaterThanOrEqual,
    /// In operator `in`
    In,
    /// Instanceof operator `instanceof`
    Instanceof,

    // Logical
    /// Logical AND `&&`
    And,
    /// Logical OR `||`
    Or,
    /// Nullish coalescing `??`
    NullishCoalescing,

    // Assignment (only in event handlers)
    /// Assignment `=`
    Assign,
    /// Addition assignment `+=`
    AddAssign,
    /// Subtraction assignment `-=`
    SubtractAssign,
    /// Multiplication assignment `*=`
    MultiplyAssign,
    /// Division assignment `/=`
    DivideAssign,
    /// Modulo assignment `%=`
    ModuloAssign,
    /// Exponentiation assignment `**=`
    PowerAssign,
    /// Logical AND assignment `&&=`
    AndAssign,
    /// Logical OR assignment `||=`
    OrAssign,
    /// Nullish coalescing assignment `??=`
    NullishCoalescingAssign,
}

impl BinaryOperator {
    /// Returns true if this is an assignment operator.
    pub fn is_assignment(&self) -> bool {
        matches!(
            self,
            Self::Assign
                | Self::AddAssign
                | Self::SubtractAssign
                | Self::MultiplyAssign
                | Self::DivideAssign
                | Self::ModuloAssign
                | Self::PowerAssign
                | Self::AndAssign
                | Self::OrAssign
                | Self::NullishCoalescingAssign
        )
    }

    /// Returns the string representation of this operator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Modulo => "%",
            Self::Power => "**",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::StrictEqual => "===",
            Self::StrictNotEqual => "!==",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
            Self::In => "in",
            Self::Instanceof => "instanceof",
            Self::And => "&&",
            Self::Or => "||",
            Self::NullishCoalescing => "??",
            Self::Assign => "=",
            Self::AddAssign => "+=",
            Self::SubtractAssign => "-=",
            Self::MultiplyAssign => "*=",
            Self::DivideAssign => "/=",
            Self::ModuloAssign => "%=",
            Self::PowerAssign => "**=",
            Self::AndAssign => "&&=",
            Self::OrAssign => "||=",
            Self::NullishCoalescingAssign => "??=",
        }
    }
}

/// A unary expression: `-expr` or `+expr`.
#[derive(Debug)]
pub struct Unary<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The operator.
    pub operator: UnaryOperator,
    /// The operand.
    pub expr: AngularExpression<'a>,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    /// Unary plus `+`
    Plus,
    /// Unary minus `-`
    Minus,
}

/// A prefix not expression: `!expr`.
#[derive(Debug)]
pub struct PrefixNot<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The operand.
    pub expression: AngularExpression<'a>,
}

/// A typeof expression: `typeof expr`.
#[derive(Debug)]
pub struct TypeofExpression<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The operand.
    pub expression: AngularExpression<'a>,
}

/// A void expression: `void expr`.
#[derive(Debug)]
pub struct VoidExpression<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The operand.
    pub expression: AngularExpression<'a>,
}

/// A non-null assertion: `expr!`.
#[derive(Debug)]
pub struct NonNullAssert<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The operand.
    pub expression: AngularExpression<'a>,
}

/// A function call expression.
#[derive(Debug)]
pub struct Call<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The function being called.
    pub receiver: AngularExpression<'a>,
    /// The arguments.
    pub args: Vec<'a, AngularExpression<'a>>,
    /// The span of the arguments list.
    pub argument_span: AbsoluteSourceSpan,
}

/// A safe function call expression: `receiver?.(args)`.
#[derive(Debug)]
pub struct SafeCall<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The function being called.
    pub receiver: AngularExpression<'a>,
    /// The arguments.
    pub args: Vec<'a, AngularExpression<'a>>,
    /// The span of the arguments list.
    pub argument_span: AbsoluteSourceSpan,
}

/// A tagged template literal: `tag\`template\``.
#[derive(Debug)]
pub struct TaggedTemplateLiteral<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The tag function.
    pub tag: AngularExpression<'a>,
    /// The template.
    pub template: TemplateLiteral<'a>,
}

/// A template literal: `` `text ${expr} text` ``.
#[derive(Debug)]
pub struct TemplateLiteral<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The static string elements.
    pub elements: Vec<'a, TemplateLiteralElement<'a>>,
    /// The dynamic expression parts.
    pub expressions: Vec<'a, AngularExpression<'a>>,
}

/// A static element in a template literal.
#[derive(Debug)]
pub struct TemplateLiteralElement<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The text content.
    pub text: Ident<'a>,
}

/// A parenthesized expression.
#[derive(Debug)]
pub struct ParenthesizedExpression<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The inner expression.
    pub expression: AngularExpression<'a>,
}

/// A regular expression literal.
#[derive(Debug)]
pub struct RegularExpressionLiteral<'a> {
    /// The span of this expression.
    pub span: ParseSpan,
    /// The absolute source span.
    pub source_span: AbsoluteSourceSpan,
    /// The regex pattern.
    pub body: Ident<'a>,
    /// The regex flags.
    pub flags: Option<Ident<'a>>,
}

// ============================================================================
// Template bindings (for structural directives)
// ============================================================================

/// A template binding in a microsyntax expression.
#[derive(Debug)]
pub enum TemplateBinding<'a> {
    /// A variable binding: `let item`.
    Variable(VariableBinding<'a>),
    /// An expression binding: `of items`.
    Expression(ExpressionBinding<'a>),
}

/// A variable binding: `let item` or `let x = y`.
#[derive(Debug)]
pub struct VariableBinding<'a> {
    /// The span of this binding.
    pub source_span: AbsoluteSourceSpan,
    /// The key (variable name).
    pub key: TemplateBindingIdentifier<'a>,
    /// The value (optional, for `let x = y`).
    pub value: Option<TemplateBindingIdentifier<'a>>,
}

/// An expression binding: `of items` or `trackBy: func`.
#[derive(Debug)]
pub struct ExpressionBinding<'a> {
    /// The span of this binding.
    pub source_span: AbsoluteSourceSpan,
    /// The key (binding name like `ngForOf`).
    pub key: TemplateBindingIdentifier<'a>,
    /// The value expression.
    pub value: Option<ASTWithSource<'a>>,
}

/// An identifier in a template binding.
#[derive(Debug, Clone)]
pub struct TemplateBindingIdentifier<'a> {
    /// The source text.
    pub source: Ident<'a>,
    /// The span.
    pub span: AbsoluteSourceSpan,
}

/// An AST node with its source context.
#[derive(Debug)]
pub struct ASTWithSource<'a> {
    /// The AST.
    pub ast: AngularExpression<'a>,
    /// The original source.
    pub source: Option<Ident<'a>>,
    /// The source location.
    pub location: Ident<'a>,
    /// The absolute offset in the template.
    pub absolute_offset: u32,
    // Note: Errors are collected separately in the parser/transformer context
    // rather than inline in AST nodes, to keep AST types arena-allocatable.
}

// ============================================================================
// Binding types (for template attributes)
// ============================================================================

/// The type of a parsed property binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedPropertyType {
    /// A regular property binding.
    Default,
    /// A literal attribute (not a binding).
    LiteralAttr,
    /// A legacy animation binding.
    LegacyAnimation,
    /// A two-way binding.
    TwoWay,
    /// An animation binding.
    Animation,
}

/// The type of a parsed event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedEventType {
    /// A regular DOM or directive event.
    Regular,
    /// A legacy animation event.
    LegacyAnimation,
    /// The event side of a two-way binding.
    TwoWay,
    /// An animation event.
    Animation,
}

/// The type of a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingType {
    /// A property binding: `[property]="expression"`.
    Property,
    /// An attribute binding: `[attr.name]="expression"`.
    Attribute,
    /// A class binding: `[class.name]="condition"`.
    Class,
    /// A style binding: `[style.rule]="expression"`.
    Style,
    /// A legacy animation binding.
    LegacyAnimation,
    /// A two-way binding: `[(ngModel)]="property"`.
    TwoWay,
    /// An animation binding.
    Animation,
}

// ============================================================================
// Parsed properties and events (for host bindings)
// ============================================================================

/// A parsed property binding from a component or directive.
///
/// This is used for host bindings in `@HostBinding()` decorators.
/// Ported from Angular's `expression_parser/ast.ts` ParsedProperty.
#[derive(Debug)]
pub struct ParsedProperty<'a> {
    /// The property name.
    pub name: Ident<'a>,
    /// The binding expression.
    pub expression: ASTWithSource<'a>,
    /// The type of property binding.
    pub property_type: ParsedPropertyType,
    /// The source span of the binding.
    pub source_span: oxc_span::Span,
    /// The span of the property name.
    pub key_span: oxc_span::Span,
    /// The span of the value expression (if present).
    pub value_span: Option<oxc_span::Span>,
}

impl<'a> ParsedProperty<'a> {
    /// Returns true if this is a literal attribute binding.
    pub fn is_literal(&self) -> bool {
        self.property_type == ParsedPropertyType::LiteralAttr
    }

    /// Returns true if this is a legacy animation binding.
    pub fn is_legacy_animation(&self) -> bool {
        self.property_type == ParsedPropertyType::LegacyAnimation
    }

    /// Returns true if this is an animation binding.
    pub fn is_animation(&self) -> bool {
        self.property_type == ParsedPropertyType::Animation
    }
}

/// A parsed event binding from a component or directive.
///
/// This is used for host listeners in `@HostListener()` decorators.
/// Ported from Angular's `expression_parser/ast.ts` ParsedEvent.
#[derive(Debug)]
pub struct ParsedEvent<'a> {
    /// The event name.
    pub name: Ident<'a>,
    /// The event target or animation phase.
    /// For regular events: "window", "document", "body", or None.
    /// For legacy animation events: the animation phase.
    pub target_or_phase: Option<Ident<'a>>,
    /// The type of event binding.
    pub event_type: ParsedEventType,
    /// The handler expression.
    pub handler: ASTWithSource<'a>,
    /// The source span of the binding.
    pub source_span: oxc_span::Span,
    /// The span of the handler expression.
    pub handler_span: oxc_span::Span,
    /// The span of the event name.
    pub key_span: oxc_span::Span,
}

//! Output AST types for JavaScript code generation.
//!
//! These types represent the final JavaScript AST that is emitted after
//! all IR transformation phases are complete.
//!
//! Ported from Angular's `output/output_ast.ts`.

use oxc_allocator::{Box, Vec};
use oxc_span::Span;
use oxc_str::Ident;

use crate::ir::expression::IrExpression;

// ============================================================================
// Operators
// ============================================================================

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    /// Negation (-)
    Minus,
    /// Plus (+)
    Plus,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    /// Equals (==)
    Equals,
    /// Not equals (!=)
    NotEquals,
    /// Assignment (=)
    Assign,
    /// Strict equals (===)
    Identical,
    /// Strict not equals (!==)
    NotIdentical,
    /// Subtraction (-)
    Minus,
    /// Addition (+)
    Plus,
    /// Division (/)
    Divide,
    /// Multiplication (*)
    Multiply,
    /// Modulo (%)
    Modulo,
    /// Logical AND (&&)
    And,
    /// Logical OR (||)
    Or,
    /// Bitwise OR (|)
    BitwiseOr,
    /// Bitwise AND (&)
    BitwiseAnd,
    /// Bitwise XOR (^)
    BitwiseXor,
    /// Left shift (<<)
    LeftShift,
    /// Right shift (>>)
    RightShift,
    /// Unsigned right shift (>>>)
    UnsignedRightShift,
    /// Less than (<)
    Lower,
    /// Less than or equal (<=)
    LowerEquals,
    /// Greater than (>)
    Bigger,
    /// Greater than or equal (>=)
    BiggerEquals,
    /// Nullish coalescing (??)
    NullishCoalesce,
    /// Exponentiation (**)
    Exponentiation,
    /// In operator (in)
    In,
    /// Instanceof operator (instanceof)
    Instanceof,
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
    /// Logical AND assignment (&&=)
    AndAssignment,
    /// Logical OR assignment (||=)
    OrAssignment,
    /// Nullish coalescing assignment (??=)
    NullishCoalesceAssignment,
}

impl BinaryOperator {
    /// Returns true if this is an assignment operator.
    pub fn is_assignment(self) -> bool {
        matches!(
            self,
            BinaryOperator::Assign
                | BinaryOperator::AdditionAssignment
                | BinaryOperator::SubtractionAssignment
                | BinaryOperator::MultiplicationAssignment
                | BinaryOperator::DivisionAssignment
                | BinaryOperator::RemainderAssignment
                | BinaryOperator::ExponentiationAssignment
                | BinaryOperator::AndAssignment
                | BinaryOperator::OrAssignment
                | BinaryOperator::NullishCoalesceAssignment
        )
    }
}

// ============================================================================
// Statement Modifiers
// ============================================================================

/// Statement modifiers (flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StmtModifier(u8);

impl StmtModifier {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Final/const variable.
    pub const FINAL: Self = Self(1 << 0);
    /// Private visibility.
    pub const PRIVATE: Self = Self(1 << 1);
    /// Exported.
    pub const EXPORTED: Self = Self(1 << 2);
    /// Static member.
    pub const STATIC: Self = Self(1 << 3);

    /// Check if a modifier is set.
    pub fn has(self, modifier: Self) -> bool {
        (self.0 & modifier.0) != 0
    }

    /// Combine modifiers.
    pub fn with(self, modifier: Self) -> Self {
        Self(self.0 | modifier.0)
    }
}

// ============================================================================
// Type System
// ============================================================================

/// Type modifiers (flags).
///
/// These modifiers can be applied to types to add qualifiers like `const`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeModifier(u8);

impl TypeModifier {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Const modifier (immutable).
    pub const CONST: Self = Self(1 << 0);

    /// Check if a modifier is set.
    pub fn has(self, modifier: Self) -> bool {
        (self.0 & modifier.0) != 0
    }

    /// Combine modifiers.
    pub fn with(self, modifier: Self) -> Self {
        Self(self.0 | modifier.0)
    }
}

/// Built-in type names.
///
/// These represent primitive and special types in the type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinTypeName {
    /// Dynamic type (any).
    Dynamic,
    /// Boolean type.
    Bool,
    /// String type.
    String,
    /// Integer type.
    Int,
    /// Number type (floating point).
    Number,
    /// Function type.
    Function,
    /// Type inferred by the compiler.
    Inferred,
    /// No type (void-like).
    None,
}

/// Output type union.
///
/// Represents types in the output AST for type annotations
/// and type-related code generation.
#[derive(Debug)]
pub enum OutputType<'a> {
    /// Built-in/primitive type.
    Builtin(Box<'a, BuiltinType>),
    /// Type expressed as an expression (class/interface types).
    Expression(Box<'a, ExpressionType<'a>>),
    /// Array type.
    Array(Box<'a, ArrayType<'a>>),
    /// Map/object type.
    Map(Box<'a, MapType<'a>>),
    /// Transplanted external type.
    Transplanted(Box<'a, TransplantedType<'a>>),
}

impl<'a> OutputType<'a> {
    /// Get the modifiers for this type.
    pub fn modifiers(&self) -> TypeModifier {
        match self {
            OutputType::Builtin(t) => t.modifiers,
            OutputType::Expression(t) => t.modifiers,
            OutputType::Array(t) => t.modifiers,
            OutputType::Map(t) => t.modifiers,
            OutputType::Transplanted(t) => t.modifiers,
        }
    }

    /// Check if this type has a specific modifier.
    pub fn has_modifier(&self, modifier: TypeModifier) -> bool {
        self.modifiers().has(modifier)
    }
}

/// Built-in type.
#[derive(Debug)]
pub struct BuiltinType {
    /// The built-in type name.
    pub name: BuiltinTypeName,
    /// Type modifiers.
    pub modifiers: TypeModifier,
}

impl BuiltinType {
    /// Create a new built-in type.
    pub fn new(name: BuiltinTypeName) -> Self {
        Self { name, modifiers: TypeModifier::NONE }
    }

    /// Create a new built-in type with modifiers.
    pub fn with_modifiers(name: BuiltinTypeName, modifiers: TypeModifier) -> Self {
        Self { name, modifiers }
    }
}

/// Expression type (wraps an expression as a type).
///
/// Used for class/interface types that are themselves expressions.
#[derive(Debug)]
pub struct ExpressionType<'a> {
    /// The expression representing the type.
    pub value: Box<'a, OutputExpression<'a>>,
    /// Type modifiers.
    pub modifiers: TypeModifier,
    /// Generic type parameters.
    pub type_params: Option<Vec<'a, OutputType<'a>>>,
}

/// Array type.
#[derive(Debug)]
pub struct ArrayType<'a> {
    /// Element type.
    pub of: Box<'a, OutputType<'a>>,
    /// Type modifiers.
    pub modifiers: TypeModifier,
}

/// Map/object type.
#[derive(Debug)]
pub struct MapType<'a> {
    /// Value type (keys are always strings).
    pub value_type: Option<Box<'a, OutputType<'a>>>,
    /// Type modifiers.
    pub modifiers: TypeModifier,
}

/// Transplanted type (wraps an external type).
///
/// Used to embed native/external types into the output AST.
/// The `type_repr` field stores a string representation of the external type.
#[derive(Debug)]
pub struct TransplantedType<'a> {
    /// String representation of the external type.
    pub type_repr: Ident<'a>,
    /// Type modifiers.
    pub modifiers: TypeModifier,
}

// ============================================================================
// Type Visitor
// ============================================================================

/// Visitor trait for output types.
pub trait TypeVisitor<'a> {
    /// The result type of visiting a type.
    type Result;

    /// Visit a built-in type.
    fn visit_builtin_type(&mut self, ty: &BuiltinType) -> Self::Result;

    /// Visit an expression type.
    fn visit_expression_type(&mut self, ty: &ExpressionType<'a>) -> Self::Result;

    /// Visit an array type.
    fn visit_array_type(&mut self, ty: &ArrayType<'a>) -> Self::Result;

    /// Visit a map type.
    fn visit_map_type(&mut self, ty: &MapType<'a>) -> Self::Result;

    /// Visit a transplanted type.
    fn visit_transplanted_type(&mut self, ty: &TransplantedType<'a>) -> Self::Result;
}

impl<'a> OutputType<'a> {
    /// Visit this type with a visitor.
    pub fn visit<V: TypeVisitor<'a>>(&self, visitor: &mut V) -> V::Result {
        match self {
            OutputType::Builtin(t) => visitor.visit_builtin_type(t),
            OutputType::Expression(t) => visitor.visit_expression_type(t),
            OutputType::Array(t) => visitor.visit_array_type(t),
            OutputType::Map(t) => visitor.visit_map_type(t),
            OutputType::Transplanted(t) => visitor.visit_transplanted_type(t),
        }
    }
}

// ============================================================================
// Type Constants
// ============================================================================

/// Create a dynamic type (any).
pub fn dynamic_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::Dynamic), allocator))
}

/// Create an inferred type.
pub fn inferred_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::Inferred), allocator))
}

/// Create a bool type.
pub fn bool_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::Bool), allocator))
}

/// Create an int type.
pub fn int_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::Int), allocator))
}

/// Create a number type.
pub fn number_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::Number), allocator))
}

/// Create a string type.
pub fn string_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::String), allocator))
}

/// Create a function type.
pub fn function_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::Function), allocator))
}

/// Create a none type (void).
pub fn none_type(allocator: &oxc_allocator::Allocator) -> OutputType<'_> {
    OutputType::Builtin(Box::new_in(BuiltinType::new(BuiltinTypeName::None), allocator))
}

// ============================================================================
// Expressions
// ============================================================================

/// Output expression union type.
#[derive(Debug)]
pub enum OutputExpression<'a> {
    // Literals
    /// Literal value (string, number, boolean, null, undefined).
    Literal(Box<'a, LiteralExpr<'a>>),
    /// Array literal.
    LiteralArray(Box<'a, LiteralArrayExpr<'a>>),
    /// Object literal.
    LiteralMap(Box<'a, LiteralMapExpr<'a>>),
    /// Regular expression literal.
    RegularExpressionLiteral(Box<'a, RegularExpressionLiteralExpr<'a>>),
    /// Template literal.
    TemplateLiteral(Box<'a, TemplateLiteralExpr<'a>>),
    /// Tagged template literal.
    TaggedTemplateLiteral(Box<'a, TaggedTemplateLiteralExpr<'a>>),

    // Variables
    /// Read a variable.
    ReadVar(Box<'a, ReadVarExpr<'a>>),

    // Property access
    /// Read a property (obj.prop).
    ReadProp(Box<'a, ReadPropExpr<'a>>),
    /// Read a key (obj[key]).
    ReadKey(Box<'a, ReadKeyExpr<'a>>),

    // Operators
    /// Binary operator expression.
    BinaryOperator(Box<'a, BinaryOperatorExpr<'a>>),
    /// Unary operator expression.
    UnaryOperator(Box<'a, UnaryOperatorExpr<'a>>),
    /// Conditional expression (cond ? true : false).
    Conditional(Box<'a, ConditionalExpr<'a>>),
    /// Logical NOT expression (!expr).
    Not(Box<'a, NotExpr<'a>>),
    /// Typeof expression.
    Typeof(Box<'a, TypeofExpr<'a>>),
    /// Void expression.
    Void(Box<'a, VoidExpr<'a>>),
    /// Parenthesized expression.
    Parenthesized(Box<'a, ParenthesizedExpr<'a>>),
    /// Comma expression.
    Comma(Box<'a, CommaExpr<'a>>),

    // Functions
    /// Function expression.
    Function(Box<'a, FunctionExpr<'a>>),
    /// Arrow function expression.
    ArrowFunction(Box<'a, ArrowFunctionExpr<'a>>),
    /// Function invocation.
    InvokeFunction(Box<'a, InvokeFunctionExpr<'a>>),
    /// Constructor invocation (new).
    Instantiate(Box<'a, InstantiateExpr<'a>>),
    /// Dynamic import.
    DynamicImport(Box<'a, DynamicImportExpr<'a>>),

    // External references
    /// External module reference.
    External(Box<'a, ExternalExpr<'a>>),

    // Localization
    /// Localized string ($localize).
    LocalizedString(Box<'a, LocalizedStringExpr<'a>>),

    // Wrapped nodes (for passing through TypeScript nodes)
    /// Wrapped external node.
    WrappedNode(Box<'a, WrappedNodeExpr<'a>>),

    // Wrapped IR expressions (for deferred processing during reify)
    /// Wrapped IR expression.
    WrappedIrNode(Box<'a, WrappedIrExpr<'a>>),

    // Spread element (for array spread)
    /// Spread element (...expr).
    SpreadElement(Box<'a, SpreadElementExpr<'a>>),

    // Raw source (preserved verbatim from source code)
    /// Raw source expression, used when the expression contains constructs that
    /// the output AST cannot represent (e.g., block-body arrow functions with
    /// complex statements like variable declarations, if/else, for loops, etc.).
    RawSource(Box<'a, RawSourceExpr<'a>>),
}

/// Literal expression.
#[derive(Debug)]
pub struct LiteralExpr<'a> {
    /// Literal value.
    pub value: LiteralValue<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Literal value types.
#[derive(Debug)]
pub enum LiteralValue<'a> {
    /// Null literal.
    Null,
    /// Undefined literal.
    Undefined,
    /// Boolean literal.
    Boolean(bool),
    /// Number literal.
    Number(f64),
    /// String literal.
    String(Ident<'a>),
}

/// Array literal expression.
#[derive(Debug)]
pub struct LiteralArrayExpr<'a> {
    /// Array entries.
    pub entries: Vec<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Object literal entry.
#[derive(Debug)]
pub struct LiteralMapEntry<'a> {
    pub key: Ident<'a>,
    pub value: OutputExpression<'a>,
    pub quoted: bool,
    pub is_spread: bool,
}

impl<'a> LiteralMapEntry<'a> {
    pub fn new(key: Ident<'a>, value: OutputExpression<'a>, quoted: bool) -> Self {
        Self { key, value, quoted, is_spread: false }
    }

    pub fn spread(value: OutputExpression<'a>) -> Self {
        Self { key: Ident::from(""), value, quoted: false, is_spread: true }
    }
}

/// Object literal expression.
#[derive(Debug)]
pub struct LiteralMapExpr<'a> {
    /// Object entries.
    pub entries: Vec<'a, LiteralMapEntry<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Regular expression literal.
#[derive(Debug)]
pub struct RegularExpressionLiteralExpr<'a> {
    /// Regex body.
    pub body: Ident<'a>,
    /// Regex flags.
    pub flags: Option<Ident<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Template literal element.
#[derive(Debug)]
pub struct TemplateLiteralElement<'a> {
    /// Cooked text.
    pub text: Ident<'a>,
    /// Raw text.
    pub raw_text: Ident<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Template literal expression.
#[derive(Debug)]
pub struct TemplateLiteralExpr<'a> {
    /// Template elements.
    pub elements: Vec<'a, TemplateLiteralElement<'a>>,
    /// Interpolated expressions.
    pub expressions: Vec<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Tagged template literal expression.
#[derive(Debug)]
pub struct TaggedTemplateLiteralExpr<'a> {
    /// Tag function.
    pub tag: Box<'a, OutputExpression<'a>>,
    /// Template literal.
    pub template: Box<'a, TemplateLiteralExpr<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Read variable expression.
#[derive(Debug)]
pub struct ReadVarExpr<'a> {
    /// Variable name.
    pub name: Ident<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Read property expression.
#[derive(Debug)]
pub struct ReadPropExpr<'a> {
    /// Receiver expression.
    pub receiver: Box<'a, OutputExpression<'a>>,
    /// Property name.
    pub name: Ident<'a>,
    /// Whether this is an optional chain access (`?.`).
    pub optional: bool,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Read key expression.
#[derive(Debug)]
pub struct ReadKeyExpr<'a> {
    /// Receiver expression.
    pub receiver: Box<'a, OutputExpression<'a>>,
    /// Key expression.
    pub index: Box<'a, OutputExpression<'a>>,
    /// Whether this is an optional chain access (`?.[`).
    pub optional: bool,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Binary operator expression.
#[derive(Debug)]
pub struct BinaryOperatorExpr<'a> {
    /// Operator.
    pub operator: BinaryOperator,
    /// Left-hand side.
    pub lhs: Box<'a, OutputExpression<'a>>,
    /// Right-hand side.
    pub rhs: Box<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Unary operator expression.
#[derive(Debug)]
pub struct UnaryOperatorExpr<'a> {
    /// Operator.
    pub operator: UnaryOperator,
    /// Operand.
    pub expr: Box<'a, OutputExpression<'a>>,
    /// Whether to wrap in parentheses.
    pub parens: bool,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Conditional expression.
#[derive(Debug)]
pub struct ConditionalExpr<'a> {
    /// Condition.
    pub condition: Box<'a, OutputExpression<'a>>,
    /// True case.
    pub true_case: Box<'a, OutputExpression<'a>>,
    /// False case.
    pub false_case: Option<Box<'a, OutputExpression<'a>>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Not expression.
#[derive(Debug)]
pub struct NotExpr<'a> {
    /// Condition to negate.
    pub condition: Box<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Typeof expression.
#[derive(Debug)]
pub struct TypeofExpr<'a> {
    /// Expression to check type of.
    pub expr: Box<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Void expression.
#[derive(Debug)]
pub struct VoidExpr<'a> {
    /// Expression to void.
    pub expr: Box<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Parenthesized expression.
#[derive(Debug)]
pub struct ParenthesizedExpr<'a> {
    /// Inner expression.
    pub expr: Box<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Comma expression.
#[derive(Debug)]
pub struct CommaExpr<'a> {
    /// Expression parts.
    pub parts: Vec<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Function parameter.
#[derive(Debug)]
pub struct FnParam<'a> {
    /// Parameter name.
    pub name: Ident<'a>,
}

/// Function expression.
#[derive(Debug)]
pub struct FunctionExpr<'a> {
    /// Function name (optional).
    pub name: Option<Ident<'a>>,
    /// Parameters.
    pub params: Vec<'a, FnParam<'a>>,
    /// Function body.
    pub statements: Vec<'a, OutputStatement<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Arrow function body.
#[derive(Debug)]
pub enum ArrowFunctionBody<'a> {
    /// Expression body (=> expr).
    Expression(Box<'a, OutputExpression<'a>>),
    /// Block body (=> { ... }).
    Statements(Vec<'a, OutputStatement<'a>>),
}

/// Arrow function expression.
#[derive(Debug)]
pub struct ArrowFunctionExpr<'a> {
    /// Parameters.
    pub params: Vec<'a, FnParam<'a>>,
    /// Function body.
    pub body: ArrowFunctionBody<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Function invocation expression.
#[derive(Debug)]
pub struct InvokeFunctionExpr<'a> {
    /// Function to invoke.
    pub fn_expr: Box<'a, OutputExpression<'a>>,
    /// Arguments.
    pub args: Vec<'a, OutputExpression<'a>>,
    /// Whether this is a pure function call.
    pub pure: bool,
    /// Whether this is an optional chain call (`?.()`).
    pub optional: bool,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Constructor invocation expression.
#[derive(Debug)]
pub struct InstantiateExpr<'a> {
    /// Class expression.
    pub class_expr: Box<'a, OutputExpression<'a>>,
    /// Constructor arguments.
    pub args: Vec<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Dynamic import expression.
#[derive(Debug)]
pub struct DynamicImportExpr<'a> {
    /// URL to import.
    pub url: DynamicImportUrl<'a>,
    /// URL comment for bundlers.
    pub url_comment: Option<Ident<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Dynamic import URL.
#[derive(Debug)]
pub enum DynamicImportUrl<'a> {
    /// Static string URL.
    String(Ident<'a>),
    /// Dynamic expression URL.
    Expression(Box<'a, OutputExpression<'a>>),
}

/// External reference.
#[derive(Debug)]
pub struct ExternalReference<'a> {
    /// Module name.
    pub module_name: Option<Ident<'a>>,
    /// Export name.
    pub name: Option<Ident<'a>>,
}

/// External expression (reference to external module).
#[derive(Debug)]
pub struct ExternalExpr<'a> {
    /// External reference.
    pub value: ExternalReference<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Localized string expression ($localize).
#[derive(Debug)]
pub struct LocalizedStringExpr<'a> {
    /// Message description.
    pub description: Option<Ident<'a>>,
    /// Message meaning.
    pub meaning: Option<Ident<'a>>,
    /// Custom message ID.
    pub custom_id: Option<Ident<'a>>,
    /// Message parts.
    pub message_parts: Vec<'a, Ident<'a>>,
    /// Placeholder names.
    pub placeholder_names: Vec<'a, Ident<'a>>,
    /// Interpolated expressions.
    pub expressions: Vec<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Wrapped node expression (for passing through external nodes).
#[derive(Debug)]
pub struct WrappedNodeExpr<'a> {
    /// Node identifier/reference.
    pub node_id: Ident<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Wrapped IR expression (for passing through IR expressions to reify phase).
///
/// This is used during ingestion to wrap IR expressions that will be
/// converted to output expressions during the reify phase.
#[derive(Debug)]
pub struct WrappedIrExpr<'a> {
    /// The wrapped IR expression.
    pub node: Box<'a, IrExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Spread element expression (...expr).
///
/// Used in array literals to spread an iterable into the array.
#[derive(Debug)]
pub struct SpreadElementExpr<'a> {
    /// The expression being spread.
    pub expr: Box<'a, OutputExpression<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Raw source expression (preserved verbatim from source code).
///
/// Used when the expression contains constructs that the output AST cannot
/// represent, such as block-body arrow functions with variable declarations,
/// if/else statements, for loops, try/catch, etc. Rather than silently
/// dropping these constructs, the raw source text is preserved and emitted
/// verbatim.
#[derive(Debug)]
pub struct RawSourceExpr<'a> {
    /// The raw source text of the expression.
    pub source: Ident<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

// ============================================================================
// Statements
// ============================================================================

/// Output statement union type.
#[derive(Debug)]
pub enum OutputStatement<'a> {
    /// Variable declaration.
    DeclareVar(Box<'a, DeclareVarStmt<'a>>),
    /// Function declaration.
    DeclareFunction(Box<'a, DeclareFunctionStmt<'a>>),
    /// Expression statement.
    Expression(Box<'a, ExpressionStatement<'a>>),
    /// Return statement.
    Return(Box<'a, ReturnStatement<'a>>),
    /// If statement.
    If(Box<'a, IfStmt<'a>>),
}

/// Variable declaration statement.
#[derive(Debug)]
pub struct DeclareVarStmt<'a> {
    /// Variable name.
    pub name: Ident<'a>,
    /// Initial value.
    pub value: Option<OutputExpression<'a>>,
    /// Statement modifiers.
    pub modifiers: StmtModifier,
    /// Leading comment (e.g., JSDoc for Closure Compiler).
    pub leading_comment: Option<LeadingComment<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Leading comment type.
#[derive(Debug, Clone)]
pub enum LeadingComment<'a> {
    /// JSDoc comment.
    JSDoc(JsDocComment<'a>),
    /// Single-line comment.
    SingleLine(Ident<'a>),
    /// Multi-line comment.
    MultiLine(Ident<'a>),
}

/// JSDoc comment for Closure Compiler compatibility.
#[derive(Debug, Clone)]
pub struct JsDocComment<'a> {
    /// Description tag (@desc).
    pub description: Option<Ident<'a>>,
    /// Meaning tag (@meaning).
    pub meaning: Option<Ident<'a>>,
    /// Suppress warnings (@suppress {msgDescriptions}).
    pub suppress_msg_descriptions: bool,
}

/// Function declaration statement.
#[derive(Debug)]
pub struct DeclareFunctionStmt<'a> {
    /// Function name.
    pub name: Ident<'a>,
    /// Parameters.
    pub params: Vec<'a, FnParam<'a>>,
    /// Function body.
    pub statements: Vec<'a, OutputStatement<'a>>,
    /// Statement modifiers.
    pub modifiers: StmtModifier,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Expression statement.
#[derive(Debug)]
pub struct ExpressionStatement<'a> {
    /// Expression.
    pub expr: OutputExpression<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// Return statement.
#[derive(Debug)]
pub struct ReturnStatement<'a> {
    /// Return value.
    pub value: OutputExpression<'a>,
    /// Source span.
    pub source_span: Option<Span>,
}

/// If statement.
#[derive(Debug)]
pub struct IfStmt<'a> {
    /// Condition.
    pub condition: OutputExpression<'a>,
    /// True case.
    pub true_case: Vec<'a, OutputStatement<'a>>,
    /// False case.
    pub false_case: Vec<'a, OutputStatement<'a>>,
    /// Source span.
    pub source_span: Option<Span>,
}

// ============================================================================
// Constants
// ============================================================================

impl<'a> OutputExpression<'a> {
    /// Returns true if this expression is a constant.
    pub fn is_constant(&self) -> bool {
        match self {
            OutputExpression::Literal(_) => true,
            OutputExpression::LiteralArray(arr) => arr.entries.iter().all(|e| e.is_constant()),
            OutputExpression::LiteralMap(map) => map.entries.iter().all(|e| e.value.is_constant()),
            OutputExpression::RegularExpressionLiteral(_) => true,
            _ => false,
        }
    }

    /// Checks if two output expressions are structurally equivalent.
    ///
    /// This is used for constant deduplication. Two expressions are equivalent if they
    /// would produce the same output code, ignoring source spans.
    pub fn is_equivalent(&self, other: &Self) -> bool {
        match (self, other) {
            (OutputExpression::Literal(a), OutputExpression::Literal(b)) => {
                literal_value_equivalent(&a.value, &b.value)
            }
            (OutputExpression::LiteralArray(a), OutputExpression::LiteralArray(b)) => {
                a.entries.len() == b.entries.len()
                    && a.entries.iter().zip(b.entries.iter()).all(|(x, y)| x.is_equivalent(y))
            }
            (OutputExpression::LiteralMap(a), OutputExpression::LiteralMap(b)) => {
                a.entries.len() == b.entries.len()
                    && a.entries.iter().zip(b.entries.iter()).all(|(x, y)| {
                        x.key == y.key && x.quoted == y.quoted && x.value.is_equivalent(&y.value)
                    })
            }
            (
                OutputExpression::RegularExpressionLiteral(a),
                OutputExpression::RegularExpressionLiteral(b),
            ) => a.body == b.body && a.flags == b.flags,
            (OutputExpression::ReadVar(a), OutputExpression::ReadVar(b)) => a.name == b.name,
            (OutputExpression::ReadProp(a), OutputExpression::ReadProp(b)) => {
                a.name == b.name
                    && a.optional == b.optional
                    && a.receiver.is_equivalent(&b.receiver)
            }
            (OutputExpression::ReadKey(a), OutputExpression::ReadKey(b)) => {
                a.optional == b.optional
                    && a.receiver.is_equivalent(&b.receiver)
                    && a.index.is_equivalent(&b.index)
            }
            (OutputExpression::BinaryOperator(a), OutputExpression::BinaryOperator(b)) => {
                a.operator == b.operator
                    && a.lhs.is_equivalent(&b.lhs)
                    && a.rhs.is_equivalent(&b.rhs)
            }
            (OutputExpression::UnaryOperator(a), OutputExpression::UnaryOperator(b)) => {
                a.operator == b.operator && a.parens == b.parens && a.expr.is_equivalent(&b.expr)
            }
            (OutputExpression::Conditional(a), OutputExpression::Conditional(b)) => {
                a.condition.is_equivalent(&b.condition)
                    && a.true_case.is_equivalent(&b.true_case)
                    && match (&a.false_case, &b.false_case) {
                        (Some(af), Some(bf)) => af.is_equivalent(bf),
                        (None, None) => true,
                        _ => false,
                    }
            }
            (OutputExpression::Not(a), OutputExpression::Not(b)) => {
                a.condition.is_equivalent(&b.condition)
            }
            (OutputExpression::Typeof(a), OutputExpression::Typeof(b)) => {
                a.expr.is_equivalent(&b.expr)
            }
            (OutputExpression::Void(a), OutputExpression::Void(b)) => a.expr.is_equivalent(&b.expr),
            (OutputExpression::Parenthesized(a), OutputExpression::Parenthesized(b)) => {
                a.expr.is_equivalent(&b.expr)
            }
            (OutputExpression::Comma(a), OutputExpression::Comma(b)) => {
                a.parts.len() == b.parts.len()
                    && a.parts.iter().zip(b.parts.iter()).all(|(x, y)| x.is_equivalent(y))
            }
            (OutputExpression::External(a), OutputExpression::External(b)) => {
                a.value.module_name == b.value.module_name && a.value.name == b.value.name
            }
            // Arrow functions (needed for track function deduplication)
            (OutputExpression::ArrowFunction(a), OutputExpression::ArrowFunction(b)) => {
                // Check params match
                if a.params.len() != b.params.len() {
                    return false;
                }
                if !a.params.iter().zip(b.params.iter()).all(|(x, y)| x.name == y.name) {
                    return false;
                }
                // Check body matches
                match (&a.body, &b.body) {
                    (ArrowFunctionBody::Expression(ae), ArrowFunctionBody::Expression(be)) => {
                        ae.is_equivalent(be)
                    }
                    (ArrowFunctionBody::Statements(as_), ArrowFunctionBody::Statements(bs)) => {
                        as_.len() == bs.len()
                            && as_.iter().zip(bs.iter()).all(|(x, y)| statement_is_equivalent(x, y))
                    }
                    _ => false,
                }
            }
            // Function expressions (needed for defer resolver deduplication)
            (OutputExpression::Function(a), OutputExpression::Function(b)) => {
                // Check name matches (both should be None or same value)
                if a.name != b.name {
                    return false;
                }
                // Check params match
                if a.params.len() != b.params.len() {
                    return false;
                }
                if !a.params.iter().zip(b.params.iter()).all(|(x, y)| x.name == y.name) {
                    return false;
                }
                // Check statements match
                a.statements.len() == b.statements.len()
                    && a.statements
                        .iter()
                        .zip(b.statements.iter())
                        .all(|(x, y)| statement_is_equivalent(x, y))
            }
            // Function invocations
            (OutputExpression::InvokeFunction(a), OutputExpression::InvokeFunction(b)) => {
                a.pure == b.pure
                    && a.optional == b.optional
                    && a.fn_expr.is_equivalent(&b.fn_expr)
                    && a.args.len() == b.args.len()
                    && a.args.iter().zip(b.args.iter()).all(|(x, y)| x.is_equivalent(y))
            }
            // Spread elements
            (OutputExpression::SpreadElement(a), OutputExpression::SpreadElement(b)) => {
                a.expr.is_equivalent(&b.expr)
            }
            // Raw source
            (OutputExpression::RawSource(a), OutputExpression::RawSource(b)) => {
                a.source == b.source
            }
            _ => false,
        }
    }

    /// Clones this expression into a new allocator.
    pub fn clone_in(&self, allocator: &'a oxc_allocator::Allocator) -> Self {
        match self {
            OutputExpression::Literal(e) => OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: clone_literal_value(&e.value, allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::LiteralArray(e) => {
                let mut entries = Vec::with_capacity_in(e.entries.len(), allocator);
                for entry in e.entries.iter() {
                    entries.push(entry.clone_in(allocator));
                }
                OutputExpression::LiteralArray(Box::new_in(
                    LiteralArrayExpr { entries, source_span: e.source_span },
                    allocator,
                ))
            }
            OutputExpression::LiteralMap(e) => {
                let mut entries = Vec::with_capacity_in(e.entries.len(), allocator);
                for entry in e.entries.iter() {
                    entries.push(LiteralMapEntry {
                        key: entry.key.clone(),
                        value: entry.value.clone_in(allocator),
                        quoted: entry.quoted,
                        is_spread: entry.is_spread,
                    });
                }
                OutputExpression::LiteralMap(Box::new_in(
                    LiteralMapExpr { entries, source_span: e.source_span },
                    allocator,
                ))
            }
            OutputExpression::RegularExpressionLiteral(e) => {
                OutputExpression::RegularExpressionLiteral(Box::new_in(
                    RegularExpressionLiteralExpr {
                        body: e.body.clone(),
                        flags: e.flags.clone(),
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::TemplateLiteral(e) => {
                let mut elements = Vec::with_capacity_in(e.elements.len(), allocator);
                for elem in e.elements.iter() {
                    elements.push(TemplateLiteralElement {
                        text: elem.text.clone(),
                        raw_text: elem.raw_text.clone(),
                        source_span: elem.source_span,
                    });
                }
                let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(allocator));
                }
                OutputExpression::TemplateLiteral(Box::new_in(
                    TemplateLiteralExpr { elements, expressions, source_span: e.source_span },
                    allocator,
                ))
            }
            OutputExpression::TaggedTemplateLiteral(e) => {
                let mut elements = Vec::with_capacity_in(e.template.elements.len(), allocator);
                for elem in e.template.elements.iter() {
                    elements.push(TemplateLiteralElement {
                        text: elem.text.clone(),
                        raw_text: elem.raw_text.clone(),
                        source_span: elem.source_span,
                    });
                }
                let mut expressions =
                    Vec::with_capacity_in(e.template.expressions.len(), allocator);
                for expr in e.template.expressions.iter() {
                    expressions.push(expr.clone_in(allocator));
                }
                OutputExpression::TaggedTemplateLiteral(Box::new_in(
                    TaggedTemplateLiteralExpr {
                        tag: Box::new_in(e.tag.clone_in(allocator), allocator),
                        template: Box::new_in(
                            TemplateLiteralExpr {
                                elements,
                                expressions,
                                source_span: e.template.source_span,
                            },
                            allocator,
                        ),
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::ReadVar(e) => OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: e.name.clone(), source_span: e.source_span },
                allocator,
            )),
            OutputExpression::ReadProp(e) => OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                    name: e.name.clone(),
                    optional: e.optional,
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::ReadKey(e) => OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(e.receiver.clone_in(allocator), allocator),
                    index: Box::new_in(e.index.clone_in(allocator), allocator),
                    optional: e.optional,
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::BinaryOperator(e) => OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: e.operator,
                    lhs: Box::new_in(e.lhs.clone_in(allocator), allocator),
                    rhs: Box::new_in(e.rhs.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::UnaryOperator(e) => OutputExpression::UnaryOperator(Box::new_in(
                UnaryOperatorExpr {
                    operator: e.operator,
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    parens: e.parens,
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::Conditional(e) => OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(e.condition.clone_in(allocator), allocator),
                    true_case: Box::new_in(e.true_case.clone_in(allocator), allocator),
                    false_case: e
                        .false_case
                        .as_ref()
                        .map(|fc| Box::new_in(fc.clone_in(allocator), allocator)),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::Not(e) => OutputExpression::Not(Box::new_in(
                NotExpr {
                    condition: Box::new_in(e.condition.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::Typeof(e) => OutputExpression::Typeof(Box::new_in(
                TypeofExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::Void(e) => OutputExpression::Void(Box::new_in(
                VoidExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::Parenthesized(e) => OutputExpression::Parenthesized(Box::new_in(
                ParenthesizedExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::Comma(e) => {
                let mut parts = Vec::with_capacity_in(e.parts.len(), allocator);
                for part in e.parts.iter() {
                    parts.push(part.clone_in(allocator));
                }
                OutputExpression::Comma(Box::new_in(
                    CommaExpr { parts, source_span: e.source_span },
                    allocator,
                ))
            }
            OutputExpression::Function(e) => {
                let mut params = Vec::with_capacity_in(e.params.len(), allocator);
                for param in e.params.iter() {
                    params.push(FnParam { name: param.name.clone() });
                }
                let mut statements = Vec::with_capacity_in(e.statements.len(), allocator);
                for stmt in e.statements.iter() {
                    statements.push(clone_output_statement(stmt, allocator));
                }
                OutputExpression::Function(Box::new_in(
                    FunctionExpr {
                        name: e.name.clone(),
                        params,
                        statements,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::ArrowFunction(e) => {
                let mut params = Vec::with_capacity_in(e.params.len(), allocator);
                for param in e.params.iter() {
                    params.push(FnParam { name: param.name.clone() });
                }
                let body = match &e.body {
                    ArrowFunctionBody::Expression(expr) => ArrowFunctionBody::Expression(
                        Box::new_in(expr.clone_in(allocator), allocator),
                    ),
                    ArrowFunctionBody::Statements(stmts) => {
                        let mut statements = Vec::with_capacity_in(stmts.len(), allocator);
                        for stmt in stmts.iter() {
                            statements.push(clone_output_statement(stmt, allocator));
                        }
                        ArrowFunctionBody::Statements(statements)
                    }
                };
                OutputExpression::ArrowFunction(Box::new_in(
                    ArrowFunctionExpr { params, body, source_span: e.source_span },
                    allocator,
                ))
            }
            OutputExpression::InvokeFunction(e) => {
                let mut args = Vec::with_capacity_in(e.args.len(), allocator);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(allocator));
                }
                OutputExpression::InvokeFunction(Box::new_in(
                    InvokeFunctionExpr {
                        fn_expr: Box::new_in(e.fn_expr.clone_in(allocator), allocator),
                        args,
                        pure: e.pure,
                        optional: e.optional,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::Instantiate(e) => {
                let mut args = Vec::with_capacity_in(e.args.len(), allocator);
                for arg in e.args.iter() {
                    args.push(arg.clone_in(allocator));
                }
                OutputExpression::Instantiate(Box::new_in(
                    InstantiateExpr {
                        class_expr: Box::new_in(e.class_expr.clone_in(allocator), allocator),
                        args,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::DynamicImport(e) => {
                let url = match &e.url {
                    DynamicImportUrl::String(s) => DynamicImportUrl::String(s.clone()),
                    DynamicImportUrl::Expression(expr) => DynamicImportUrl::Expression(
                        Box::new_in(expr.clone_in(allocator), allocator),
                    ),
                };
                OutputExpression::DynamicImport(Box::new_in(
                    DynamicImportExpr {
                        url,
                        url_comment: e.url_comment.clone(),
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::External(e) => OutputExpression::External(Box::new_in(
                ExternalExpr {
                    value: ExternalReference {
                        module_name: e.value.module_name.clone(),
                        name: e.value.name.clone(),
                    },
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::LocalizedString(e) => {
                let mut message_parts = Vec::with_capacity_in(e.message_parts.len(), allocator);
                for part in e.message_parts.iter() {
                    message_parts.push(part.clone());
                }
                let mut placeholder_names =
                    Vec::with_capacity_in(e.placeholder_names.len(), allocator);
                for name in e.placeholder_names.iter() {
                    placeholder_names.push(name.clone());
                }
                let mut expressions = Vec::with_capacity_in(e.expressions.len(), allocator);
                for expr in e.expressions.iter() {
                    expressions.push(expr.clone_in(allocator));
                }
                OutputExpression::LocalizedString(Box::new_in(
                    LocalizedStringExpr {
                        description: e.description.clone(),
                        meaning: e.meaning.clone(),
                        custom_id: e.custom_id.clone(),
                        message_parts,
                        placeholder_names,
                        expressions,
                        source_span: e.source_span,
                    },
                    allocator,
                ))
            }
            OutputExpression::WrappedNode(e) => OutputExpression::WrappedNode(Box::new_in(
                WrappedNodeExpr { node_id: e.node_id.clone(), source_span: e.source_span },
                allocator,
            )),
            OutputExpression::WrappedIrNode(_) => {
                // WrappedIrNode expressions wrap IR expressions for deferred processing.
                // They should be resolved during the reify phase before any cloning occurs.
                // Return a placeholder undefined literal as a safe fallback.
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                    allocator,
                ))
            }
            OutputExpression::SpreadElement(e) => OutputExpression::SpreadElement(Box::new_in(
                SpreadElementExpr {
                    expr: Box::new_in(e.expr.clone_in(allocator), allocator),
                    source_span: e.source_span,
                },
                allocator,
            )),
            OutputExpression::RawSource(e) => OutputExpression::RawSource(Box::new_in(
                RawSourceExpr { source: e.source.clone(), source_span: e.source_span },
                allocator,
            )),
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

/// Check if two literal values are equivalent for deduplication.
fn literal_value_equivalent(a: &LiteralValue<'_>, b: &LiteralValue<'_>) -> bool {
    match (a, b) {
        (LiteralValue::Null, LiteralValue::Null) => true,
        (LiteralValue::Undefined, LiteralValue::Undefined) => true,
        (LiteralValue::Boolean(a), LiteralValue::Boolean(b)) => a == b,
        (LiteralValue::Number(a), LiteralValue::Number(b)) => {
            // Handle NaN specially (NaN != NaN, but for dedup purposes they're equivalent)
            (a.is_nan() && b.is_nan()) || a == b
        }
        (LiteralValue::String(a), LiteralValue::String(b)) => a == b,
        _ => false,
    }
}

/// Check if two output statements are equivalent for deduplication.
fn statement_is_equivalent(a: &OutputStatement<'_>, b: &OutputStatement<'_>) -> bool {
    match (a, b) {
        (OutputStatement::Return(ra), OutputStatement::Return(rb)) => {
            ra.value.is_equivalent(&rb.value)
        }
        (OutputStatement::Expression(ea), OutputStatement::Expression(eb)) => {
            ea.expr.is_equivalent(&eb.expr)
        }
        (OutputStatement::DeclareVar(da), OutputStatement::DeclareVar(db)) => {
            da.name == db.name
                && da.modifiers == db.modifiers
                && match (&da.value, &db.value) {
                    (Some(va), Some(vb)) => va.is_equivalent(vb),
                    (None, None) => true,
                    _ => false,
                }
        }
        (OutputStatement::DeclareFunction(fa), OutputStatement::DeclareFunction(fb)) => {
            fa.name == fb.name
                && fa.modifiers == fb.modifiers
                && fa.params.len() == fb.params.len()
                && fa.params.iter().zip(fb.params.iter()).all(|(x, y)| x.name == y.name)
                && fa.statements.len() == fb.statements.len()
                && fa
                    .statements
                    .iter()
                    .zip(fb.statements.iter())
                    .all(|(x, y)| statement_is_equivalent(x, y))
        }
        (OutputStatement::If(ia), OutputStatement::If(ib)) => {
            ia.condition.is_equivalent(&ib.condition)
                && ia.true_case.len() == ib.true_case.len()
                && ia
                    .true_case
                    .iter()
                    .zip(ib.true_case.iter())
                    .all(|(x, y)| statement_is_equivalent(x, y))
                && ia.false_case.len() == ib.false_case.len()
                && ia
                    .false_case
                    .iter()
                    .zip(ib.false_case.iter())
                    .all(|(x, y)| statement_is_equivalent(x, y))
        }
        _ => false,
    }
}

/// Clone an output statement.
pub fn clone_output_statement<'a>(
    stmt: &OutputStatement<'a>,
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    match stmt {
        OutputStatement::DeclareVar(s) => OutputStatement::DeclareVar(Box::new_in(
            DeclareVarStmt {
                name: s.name.clone(),
                value: s.value.as_ref().map(|v| v.clone_in(allocator)),
                modifiers: s.modifiers,
                leading_comment: s.leading_comment.clone(),
                source_span: s.source_span,
            },
            allocator,
        )),
        OutputStatement::DeclareFunction(s) => {
            let mut params = Vec::with_capacity_in(s.params.len(), allocator);
            for param in s.params.iter() {
                params.push(FnParam { name: param.name.clone() });
            }
            let mut statements = Vec::with_capacity_in(s.statements.len(), allocator);
            for inner_stmt in s.statements.iter() {
                statements.push(clone_output_statement(inner_stmt, allocator));
            }
            OutputStatement::DeclareFunction(Box::new_in(
                DeclareFunctionStmt {
                    name: s.name.clone(),
                    params,
                    statements,
                    modifiers: s.modifiers,
                    source_span: s.source_span,
                },
                allocator,
            ))
        }
        OutputStatement::Expression(s) => OutputStatement::Expression(Box::new_in(
            ExpressionStatement { expr: s.expr.clone_in(allocator), source_span: s.source_span },
            allocator,
        )),
        OutputStatement::Return(s) => OutputStatement::Return(Box::new_in(
            ReturnStatement { value: s.value.clone_in(allocator), source_span: s.source_span },
            allocator,
        )),
        OutputStatement::If(s) => {
            let mut true_case = Vec::with_capacity_in(s.true_case.len(), allocator);
            for inner_stmt in s.true_case.iter() {
                true_case.push(clone_output_statement(inner_stmt, allocator));
            }
            let mut false_case = Vec::with_capacity_in(s.false_case.len(), allocator);
            for inner_stmt in s.false_case.iter() {
                false_case.push(clone_output_statement(inner_stmt, allocator));
            }
            OutputStatement::If(Box::new_in(
                IfStmt {
                    condition: s.condition.clone_in(allocator),
                    true_case,
                    false_case,
                    source_span: s.source_span,
                },
                allocator,
            ))
        }
    }
}

// ============================================================================
// Builder Helpers
// ============================================================================

/// Helper functions for building output AST nodes.
pub struct OutputAstBuilder;

impl OutputAstBuilder {
    /// Create a null literal.
    pub fn null<'a>(allocator: &'a oxc_allocator::Allocator) -> OutputExpression<'a> {
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        ))
    }

    /// Create a boolean literal.
    pub fn boolean<'a>(
        allocator: &'a oxc_allocator::Allocator,
        value: bool,
    ) -> OutputExpression<'a> {
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(value), source_span: None },
            allocator,
        ))
    }

    /// Create a number literal.
    pub fn number<'a>(allocator: &'a oxc_allocator::Allocator, value: f64) -> OutputExpression<'a> {
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(value), source_span: None },
            allocator,
        ))
    }

    /// Create a string literal.
    pub fn string<'a>(
        allocator: &'a oxc_allocator::Allocator,
        value: Ident<'a>,
    ) -> OutputExpression<'a> {
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(value), source_span: None },
            allocator,
        ))
    }

    /// Create a variable read.
    pub fn variable<'a>(
        allocator: &'a oxc_allocator::Allocator,
        name: Ident<'a>,
    ) -> OutputExpression<'a> {
        OutputExpression::ReadVar(Box::new_in(ReadVarExpr { name, source_span: None }, allocator))
    }
}

// ============================================================================
// Output AST Visitor
// ============================================================================

/// Recursive visitor for output AST expressions.
///
/// This trait provides a pattern for traversing the output AST, similar to
/// Angular's `RecursiveAstVisitor` in `output_ast.ts`.
///
/// Implementations can override specific `visit_*` methods to collect information
/// or transform the AST. The default implementations recursively visit child nodes.
pub trait RecursiveOutputAstVisitor<'a> {
    /// Visit a literal expression.
    fn visit_literal(&mut self, _expr: &LiteralExpr<'a>) {}

    /// Visit an array literal expression.
    fn visit_literal_array(&mut self, expr: &LiteralArrayExpr<'a>) {
        for entry in expr.entries.iter() {
            self.visit_expression(entry);
        }
    }

    /// Visit an object literal expression.
    fn visit_literal_map(&mut self, expr: &LiteralMapExpr<'a>) {
        for entry in expr.entries.iter() {
            self.visit_expression(&entry.value);
        }
    }

    /// Visit a regular expression literal.
    fn visit_regex_literal(&mut self, _expr: &RegularExpressionLiteralExpr<'a>) {}

    /// Visit a template literal expression.
    fn visit_template_literal(&mut self, expr: &TemplateLiteralExpr<'a>) {
        for expression in expr.expressions.iter() {
            self.visit_expression(expression);
        }
    }

    /// Visit a tagged template literal expression.
    fn visit_tagged_template_literal(&mut self, expr: &TaggedTemplateLiteralExpr<'a>) {
        self.visit_expression(&expr.tag);
        self.visit_template_literal(&expr.template);
    }

    /// Visit a variable read expression.
    fn visit_read_var(&mut self, _expr: &ReadVarExpr<'a>) {}

    /// Visit a property read expression.
    fn visit_read_prop(&mut self, expr: &ReadPropExpr<'a>) {
        self.visit_expression(&expr.receiver);
    }

    /// Visit a key read expression.
    fn visit_read_key(&mut self, expr: &ReadKeyExpr<'a>) {
        self.visit_expression(&expr.receiver);
        self.visit_expression(&expr.index);
    }

    /// Visit a binary operator expression.
    fn visit_binary_operator(&mut self, expr: &BinaryOperatorExpr<'a>) {
        self.visit_expression(&expr.lhs);
        self.visit_expression(&expr.rhs);
    }

    /// Visit a unary operator expression.
    fn visit_unary_operator(&mut self, expr: &UnaryOperatorExpr<'a>) {
        self.visit_expression(&expr.expr);
    }

    /// Visit a conditional expression.
    fn visit_conditional(&mut self, expr: &ConditionalExpr<'a>) {
        self.visit_expression(&expr.condition);
        self.visit_expression(&expr.true_case);
        if let Some(false_case) = &expr.false_case {
            self.visit_expression(false_case);
        }
    }

    /// Visit a not expression.
    fn visit_not(&mut self, expr: &NotExpr<'a>) {
        self.visit_expression(&expr.condition);
    }

    /// Visit a typeof expression.
    fn visit_typeof(&mut self, expr: &TypeofExpr<'a>) {
        self.visit_expression(&expr.expr);
    }

    /// Visit a void expression.
    fn visit_void(&mut self, expr: &VoidExpr<'a>) {
        self.visit_expression(&expr.expr);
    }

    /// Visit a parenthesized expression.
    fn visit_parenthesized(&mut self, expr: &ParenthesizedExpr<'a>) {
        self.visit_expression(&expr.expr);
    }

    /// Visit a comma expression.
    fn visit_comma(&mut self, expr: &CommaExpr<'a>) {
        for part in expr.parts.iter() {
            self.visit_expression(part);
        }
    }

    /// Visit a function expression.
    fn visit_function(&mut self, expr: &FunctionExpr<'a>) {
        for stmt in expr.statements.iter() {
            self.visit_statement(stmt);
        }
    }

    /// Visit an arrow function expression.
    fn visit_arrow_function(&mut self, expr: &ArrowFunctionExpr<'a>) {
        match &expr.body {
            ArrowFunctionBody::Expression(e) => self.visit_expression(e),
            ArrowFunctionBody::Statements(stmts) => {
                for stmt in stmts.iter() {
                    self.visit_statement(stmt);
                }
            }
        }
    }

    /// Visit a function invocation expression.
    fn visit_invoke_function(&mut self, expr: &InvokeFunctionExpr<'a>) {
        self.visit_expression(&expr.fn_expr);
        for arg in expr.args.iter() {
            self.visit_expression(arg);
        }
    }

    /// Visit a constructor invocation expression.
    fn visit_instantiate(&mut self, expr: &InstantiateExpr<'a>) {
        self.visit_expression(&expr.class_expr);
        for arg in expr.args.iter() {
            self.visit_expression(arg);
        }
    }

    /// Visit a dynamic import expression.
    fn visit_dynamic_import(&mut self, expr: &DynamicImportExpr<'a>) {
        if let DynamicImportUrl::Expression(url_expr) = &expr.url {
            self.visit_expression(url_expr);
        }
    }

    /// Visit an external reference expression.
    fn visit_external(&mut self, _expr: &ExternalExpr<'a>) {}

    /// Visit a localized string expression.
    fn visit_localized_string(&mut self, expr: &LocalizedStringExpr<'a>) {
        for expression in expr.expressions.iter() {
            self.visit_expression(expression);
        }
    }

    /// Visit a wrapped node expression.
    fn visit_wrapped_node(&mut self, _expr: &WrappedNodeExpr<'a>) {}

    /// Visit a wrapped IR expression.
    fn visit_wrapped_ir_node(&mut self, _expr: &WrappedIrExpr<'a>) {}

    /// Visit a spread element expression.
    fn visit_spread_element(&mut self, expr: &SpreadElementExpr<'a>) {
        self.visit_expression(&expr.expr);
    }

    /// Visit any output expression (dispatches to specific visit methods).
    fn visit_expression(&mut self, expr: &OutputExpression<'a>) {
        match expr {
            OutputExpression::Literal(e) => self.visit_literal(e),
            OutputExpression::LiteralArray(e) => self.visit_literal_array(e),
            OutputExpression::LiteralMap(e) => self.visit_literal_map(e),
            OutputExpression::RegularExpressionLiteral(e) => self.visit_regex_literal(e),
            OutputExpression::TemplateLiteral(e) => self.visit_template_literal(e),
            OutputExpression::TaggedTemplateLiteral(e) => self.visit_tagged_template_literal(e),
            OutputExpression::ReadVar(e) => self.visit_read_var(e),
            OutputExpression::ReadProp(e) => self.visit_read_prop(e),
            OutputExpression::ReadKey(e) => self.visit_read_key(e),
            OutputExpression::BinaryOperator(e) => self.visit_binary_operator(e),
            OutputExpression::UnaryOperator(e) => self.visit_unary_operator(e),
            OutputExpression::Conditional(e) => self.visit_conditional(e),
            OutputExpression::Not(e) => self.visit_not(e),
            OutputExpression::Typeof(e) => self.visit_typeof(e),
            OutputExpression::Void(e) => self.visit_void(e),
            OutputExpression::Parenthesized(e) => self.visit_parenthesized(e),
            OutputExpression::Comma(e) => self.visit_comma(e),
            OutputExpression::Function(e) => self.visit_function(e),
            OutputExpression::ArrowFunction(e) => self.visit_arrow_function(e),
            OutputExpression::InvokeFunction(e) => self.visit_invoke_function(e),
            OutputExpression::Instantiate(e) => self.visit_instantiate(e),
            OutputExpression::DynamicImport(e) => self.visit_dynamic_import(e),
            OutputExpression::External(e) => self.visit_external(e),
            OutputExpression::LocalizedString(e) => self.visit_localized_string(e),
            OutputExpression::WrappedNode(e) => self.visit_wrapped_node(e),
            OutputExpression::WrappedIrNode(e) => self.visit_wrapped_ir_node(e),
            OutputExpression::SpreadElement(e) => self.visit_spread_element(e),
            OutputExpression::RawSource(_) => {
                // Raw source is opaque — no sub-expressions to visit
            }
        }
    }

    /// Visit a variable declaration statement.
    fn visit_declare_var(&mut self, stmt: &DeclareVarStmt<'a>) {
        if let Some(value) = &stmt.value {
            self.visit_expression(value);
        }
    }

    /// Visit a function declaration statement.
    fn visit_declare_function(&mut self, stmt: &DeclareFunctionStmt<'a>) {
        for inner_stmt in stmt.statements.iter() {
            self.visit_statement(inner_stmt);
        }
    }

    /// Visit an expression statement.
    fn visit_expression_statement(&mut self, stmt: &ExpressionStatement<'a>) {
        self.visit_expression(&stmt.expr);
    }

    /// Visit a return statement.
    fn visit_return_statement(&mut self, stmt: &ReturnStatement<'a>) {
        self.visit_expression(&stmt.value);
    }

    /// Visit an if statement.
    fn visit_if_statement(&mut self, stmt: &IfStmt<'a>) {
        self.visit_expression(&stmt.condition);
        for inner_stmt in stmt.true_case.iter() {
            self.visit_statement(inner_stmt);
        }
        for inner_stmt in stmt.false_case.iter() {
            self.visit_statement(inner_stmt);
        }
    }

    /// Visit any output statement (dispatches to specific visit methods).
    fn visit_statement(&mut self, stmt: &OutputStatement<'a>) {
        match stmt {
            OutputStatement::DeclareVar(s) => self.visit_declare_var(s),
            OutputStatement::DeclareFunction(s) => self.visit_declare_function(s),
            OutputStatement::Expression(s) => self.visit_expression_statement(s),
            OutputStatement::Return(s) => self.visit_return_statement(s),
            OutputStatement::If(s) => self.visit_if_statement(s),
        }
    }
}

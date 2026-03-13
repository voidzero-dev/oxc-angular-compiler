//! Template ingestion: R3 AST → IR operations.
//!
//! This module transforms the R3 AST representation into IR operations
//! that can be processed by the compilation phases.
//!
//! ## Expression Handling
//!
//! Expressions from R3 nodes are stored in the `ExpressionStore` and referenced
//! via `ExpressionId` using the Reference + Index pattern. This avoids cloning
//! expressions and maintains proper ownership.
//!
//! The R3 nodes are consumed (moved) during ingestion, which allows us to take
//! ownership of the expressions and store them properly without cloning.
//!
//! Ported from Angular's `template/pipeline/src/ingest.ts`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_diagnostics::OxcDiagnostic;
use oxc_span::Atom;

use super::compilation::{
    CTX_REF, ComponentCompilationJob, DeferBlockDepsEmitMode, DeferMetadata,
    HostBindingCompilationJob, I18nMessageMetadata, TemplateCompilationMode,
};
use super::conversion::prefix_with_namespace;
use crate::ast::expression::{AngularExpression, ParsedEventType};
use crate::ast::r3::{
    I18nIcuPlaceholder, I18nMeta, I18nNode, R3BoundAttribute, R3BoundEvent, R3BoundText, R3Content,
    R3DeferredBlock, R3Element, R3ForLoopBlock, R3Icu, R3IcuPlaceholder, R3IfBlock,
    R3LetDeclaration, R3Node, R3SwitchBlock, R3Template, R3TemplateAttr, R3Text, R3TextAttribute,
    SecurityContext,
};
use crate::ir::enums::{
    AnimationKind, BindingKind, DeferOpModifierKind, DeferTriggerKind, Namespace, TemplateKind,
};
use crate::ir::expression::{
    BinaryExpr, ConditionalCaseExpr, EmptyExpr, IrBinaryOperator, IrExpression, LexicalReadExpr,
    PipeBindingExpr, ResolvedCallExpr, ResolvedKeyedReadExpr, ResolvedPropertyReadExpr,
    SafeInvokeFunctionExpr, SafeKeyedReadExpr, SafePropertyReadExpr, SlotHandle,
    TwoWayBindingSetExpr,
};
use crate::ir::ops::{
    BindingOp, ConditionalBranchCreateOp, ConditionalOp, ConditionalUpdateOp, ControlCreateOp,
    CreateOp, CreateOpBase, DeclareLetOp, DeferOnOp, DeferOp, DeferWhenOp, ElementEndOp,
    ElementStartOp, ExtractedAttributeOp, I18nAttributesOp, I18nEndOp, I18nPlaceholder,
    I18nSlotHandle, I18nStartOp, IcuEndOp, IcuStartOp, InterpolateTextOp, ListenerOp, LocalRef,
    ProjectionOp, RepeaterCreateOp, RepeaterOp, RepeaterVarNames, SlotId, StatementOp, StoreLetOp,
    TemplateOp, TextOp, TwoWayListenerOp, UpdateOp, UpdateOpBase, XrefId,
};
use crate::output::ast::OutputExpression;
use crate::pipeline::compilation::{AliasVariable, ContextVariable};
use rustc_hash::FxHashMap;

/// Options for ingesting a component template.
///
/// These options mirror the parameters passed to Angular's `ingestComponent()` function
/// in `template/pipeline/src/ingest.ts`.
///
/// Ported from Angular's ingestComponent parameters (lines 57-68 in ingest.ts).
#[derive(Debug)]
pub struct IngestOptions<'a> {
    /// Template compilation mode (Full or DomOnly).
    ///
    /// Use `DomOnly` when the component is standalone and has no directive dependencies.
    pub mode: TemplateCompilationMode,

    /// Relative path to the context file for i18n suffix generation.
    ///
    /// Used to generate unique, file-based variable names for i18n translations.
    pub relative_context_file_path: Option<Atom<'a>>,

    /// Whether to use external message IDs in i18n variable names.
    ///
    /// When true, generates variable names like `MSG_EXTERNAL_abc123$$SUFFIX`.
    /// When false, uses file-based naming like `MSG_SUFFIX_0`.
    pub i18n_use_external_ids: bool,

    /// Defer block emit mode (PerBlock or PerComponent).
    ///
    /// PerBlock is used in full compilation mode when the compiler has information
    /// about which dependencies belong to which defer block.
    /// PerComponent is used in local/JIT compilation.
    pub defer_block_deps_emit_mode: DeferBlockDepsEmitMode,

    /// Relative path to the template file for source location debugging.
    pub relative_template_path: Option<Atom<'a>>,

    /// Whether to enable debug source locations.
    ///
    /// When enabled, the compiler generates `ɵɵsourceLocation` calls for debugging.
    pub enable_debug_locations: bool,

    /// Template source text for computing line/column from byte offsets.
    ///
    /// Required when `enable_debug_locations` is true.
    pub template_source: Option<&'a str>,

    /// Reference to the deferrable dependencies function for PerComponent mode.
    ///
    /// This corresponds to Angular's `allDeferrableDepsFn` parameter.
    /// When using `DeferBlockDepsEmitMode::PerComponent`, this function provides
    /// all deferrable dependencies for the entire component.
    ///
    /// Ported from Angular's ingestComponent parameter (line 65 in ingest.ts).
    pub all_deferrable_deps_fn: Option<OutputExpression<'a>>,

    /// Starting index for the constant pool's name counter.
    ///
    /// This is used when compiling multiple components in the same file to ensure
    /// constant names don't conflict. Each component continues from where the
    /// previous component's pool left off.
    ///
    /// For example, if component 1 uses _c0, _c1, _c2, then component 2 should
    /// be created with `pool_starting_index: 3` to start with _c3.
    ///
    /// Default is 0 (start from _c0).
    pub pool_starting_index: u32,

    /// Angular version for feature-gated instruction selection.
    ///
    /// When set to a version < 20, the compiler emits `ɵɵtemplate` instead of
    /// `ɵɵconditionalCreate`/`ɵɵconditionalBranchCreate` for `@if`/`@switch` blocks.
    /// When `None`, assumes latest Angular version (v20+ behavior).
    pub angular_version: Option<crate::AngularVersion>,
}

impl Default for IngestOptions<'_> {
    fn default() -> Self {
        Self {
            mode: TemplateCompilationMode::Full,
            relative_context_file_path: None,
            i18n_use_external_ids: true,
            defer_block_deps_emit_mode: DeferBlockDepsEmitMode::PerBlock,
            relative_template_path: None,
            enable_debug_locations: false,
            template_source: None,
            all_deferrable_deps_fn: None,
            pool_starting_index: 0,
            angular_version: None,
        }
    }
}

/// Stores an expression from an R3 node and returns an IrExpression that references it.
///
/// This is the preferred way to handle expressions during ingestion. The expression
/// is stored in the CompilationJob's ExpressionStore and referenced by ID.
fn store_and_ref_expr<'a>(
    job: &mut ComponentCompilationJob<'a>,
    expr: AngularExpression<'a>,
) -> Box<'a, IrExpression<'a>> {
    let id = job.store_expression(expr);
    Box::new_in(IrExpression::ExpressionRef(id), job.allocator)
}

/// Converts an Angular expression to an IR expression during ingestion.
///
/// This function directly converts pipe expressions to their IR equivalents,
/// making them visible to subsequent phases like `pipe_creation`.
/// Safe navigation expressions are stored in the ExpressionStore to be processed
/// by the `expand_safe_reads` phase later.
/// Other expressions are stored in the ExpressionStore and referenced by ID.
///
/// This matches Angular's TypeScript `convertAst` function in `ingest.ts`.
fn convert_ast_to_ir<'a>(
    job: &mut ComponentCompilationJob<'a>,
    expr: AngularExpression<'a>,
) -> Box<'a, IrExpression<'a>> {
    let allocator = job.allocator;

    match expr {
        // Convert BindingPipe to IrExpression::PipeBinding
        // This makes pipes visible to the pipe_creation phase
        AngularExpression::BindingPipe(pipe) => {
            let pipe = pipe.unbox();
            let target = job.allocate_xref_id();

            // Convert the pipe input and arguments to IR expressions
            let mut args = Vec::with_capacity_in(1 + pipe.args.len(), allocator);

            // First argument is the pipe input expression
            let input_expr = convert_ast_to_ir(job, pipe.exp);
            args.push(input_expr.unbox());

            // Remaining arguments are the pipe arguments
            for arg in pipe.args {
                let arg_expr = convert_ast_to_ir(job, arg);
                args.push(arg_expr.unbox());
            }

            Box::new_in(
                IrExpression::PipeBinding(Box::new_in(
                    PipeBindingExpr {
                        target,
                        target_slot: SlotHandle::new(),
                        name: pipe.name,
                        args,
                        var_offset: None,
                        source_span: Some(pipe.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Non-null assertion is transparent - just unwrap and convert inner expression
        AngularExpression::NonNullAssert(nna) => {
            let nna = nna.unbox();
            convert_ast_to_ir(job, nna.expression)
        }

        // Convert SafePropertyRead (a?.b) to IR SafePropertyReadExpr
        // This makes safe reads visible to the expand_safe_reads phase
        AngularExpression::SafePropertyRead(safe) => {
            let safe = safe.unbox();
            let receiver = convert_ast_to_ir(job, safe.receiver);
            Box::new_in(
                IrExpression::SafePropertyRead(Box::new_in(
                    SafePropertyReadExpr {
                        receiver,
                        name: safe.name,
                        source_span: Some(safe.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert SafeKeyedRead (a?.[b]) to IR SafeKeyedReadExpr
        AngularExpression::SafeKeyedRead(safe) => {
            let safe = safe.unbox();
            let receiver = convert_ast_to_ir(job, safe.receiver);
            let index = convert_ast_to_ir(job, safe.key);
            Box::new_in(
                IrExpression::SafeKeyedRead(Box::new_in(
                    SafeKeyedReadExpr {
                        receiver,
                        index,
                        source_span: Some(safe.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert SafeCall (a?.()) to IR SafeInvokeFunctionExpr
        AngularExpression::SafeCall(safe) => {
            let safe = safe.unbox();
            let receiver = convert_ast_to_ir(job, safe.receiver);
            let mut args = Vec::with_capacity_in(safe.args.len(), allocator);
            for arg in safe.args {
                let arg_expr = convert_ast_to_ir(job, arg);
                args.push(arg_expr.unbox());
            }
            Box::new_in(
                IrExpression::SafeInvokeFunction(Box::new_in(
                    SafeInvokeFunctionExpr { receiver, args, source_span: None },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert LiteralArray - recursively convert elements to preserve pipes
        AngularExpression::LiteralArray(arr) => {
            let arr = arr.unbox();
            let mut elements = Vec::with_capacity_in(arr.expressions.len(), allocator);
            for elem in arr.expressions {
                let elem_expr = convert_ast_to_ir(job, elem);
                elements.push(elem_expr.unbox());
            }
            Box::new_in(
                IrExpression::LiteralArray(Box::new_in(
                    crate::ir::expression::IrLiteralArrayExpr {
                        elements,
                        source_span: Some(arr.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert LiteralMap (object literal) - recursively convert values to preserve pipes
        AngularExpression::LiteralMap(map) => {
            use crate::ast::expression::LiteralMapKey;
            let map = map.unbox();
            let mut keys = Vec::with_capacity_in(map.keys.len(), allocator);
            let mut values = Vec::with_capacity_in(map.values.len(), allocator);
            let mut quoted = Vec::with_capacity_in(map.keys.len(), allocator);

            for (key, value) in map.keys.into_iter().zip(map.values.into_iter()) {
                // Only handle property keys; spread keys need special handling
                if let LiteralMapKey::Property(prop) = key {
                    keys.push(prop.key);
                    quoted.push(prop.quoted);
                    let value_expr = convert_ast_to_ir(job, value);
                    values.push(value_expr.unbox());
                }
            }

            Box::new_in(
                IrExpression::LiteralMap(Box::new_in(
                    crate::ir::expression::IrLiteralMapExpr {
                        keys,
                        values,
                        quoted,
                        source_span: Some(map.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Binary expression - recursively convert operands to preserve pipes
        // This is needed for expressions like `a ?? (b | pipe)` where the pipe is nested
        AngularExpression::Binary(bin) => {
            let bin = bin.unbox();
            let lhs = convert_ast_to_ir(job, bin.left);
            let rhs = convert_ast_to_ir(job, bin.right);

            Box::new_in(
                IrExpression::Binary(oxc_allocator::Box::new_in(
                    crate::ir::expression::BinaryExpr {
                        operator: convert_binary_op(bin.operation),
                        lhs,
                        rhs,
                        source_span: Some(bin.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert ParenthesizedExpression - recursively convert inner expression to preserve pipes
        // This is needed for expressions like `(a | pipe)` where the pipe is inside parens
        AngularExpression::ParenthesizedExpression(paren) => {
            let paren = paren.unbox();
            let inner = convert_ast_to_ir(job, paren.expression);
            Box::new_in(
                IrExpression::Parenthesized(Box::new_in(
                    crate::ir::expression::IrParenthesizedExpr { expr: inner, source_span: None },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Conditional expression (ternary) - recursively convert operands to preserve pipes
        AngularExpression::Conditional(cond) => {
            let cond = cond.unbox();
            let condition = convert_ast_to_ir(job, cond.condition);
            let true_exp = convert_ast_to_ir(job, cond.true_exp);
            let false_exp = convert_ast_to_ir(job, cond.false_exp);

            Box::new_in(
                IrExpression::Ternary(oxc_allocator::Box::new_in(
                    crate::ir::expression::TernaryExpr {
                        condition,
                        true_expr: true_exp,
                        false_expr: false_exp,
                        source_span: Some(cond.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert PropertyRead - recursively convert receiver to preserve pipes
        // This handles expressions like `(root$ | async).nav` where the pipe is in the receiver
        // Matches Angular's convertAst for PropertyRead in ingest.ts
        AngularExpression::PropertyRead(prop) => {
            let prop = prop.unbox();

            // Check if this is a simple property read from implicit receiver (just a name)
            if matches!(prop.receiver, AngularExpression::ImplicitReceiver(_))
                && !matches!(prop.receiver, AngularExpression::ThisReceiver(_))
            {
                // This is like `foo` which becomes LexicalRead
                Box::new_in(
                    IrExpression::LexicalRead(oxc_allocator::Box::new_in(
                        LexicalReadExpr {
                            name: prop.name,
                            source_span: Some(prop.source_span.to_span()),
                        },
                        allocator,
                    )),
                    allocator,
                )
            } else if matches!(prop.receiver, AngularExpression::ThisReceiver(_)) {
                // Explicit `this` property read (e.g., `this.formGroup`) becomes a
                // ResolvedPropertyRead with Context receiver. This is critical for embedded
                // views because the resolve phases need to see the ContextExpr(root_xref)
                // to properly generate nextContext() calls.
                Box::new_in(
                    IrExpression::ResolvedPropertyRead(oxc_allocator::Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver: Box::new_in(
                                IrExpression::Context(oxc_allocator::Box::new_in(
                                    crate::ir::expression::ContextExpr {
                                        view: job.root.xref,
                                        source_span: Some(prop.source_span.to_span()),
                                    },
                                    allocator,
                                )),
                                allocator,
                            ),
                            name: prop.name,
                            source_span: Some(prop.source_span.to_span()),
                        },
                        allocator,
                    )),
                    allocator,
                )
            } else {
                // This is a nested property read like `(expr).name`
                // Recursively convert the receiver to preserve any pipes
                let receiver = convert_ast_to_ir(job, prop.receiver);
                Box::new_in(
                    IrExpression::ResolvedPropertyRead(oxc_allocator::Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver,
                            name: prop.name,
                            source_span: Some(prop.source_span.to_span()),
                        },
                        allocator,
                    )),
                    allocator,
                )
            }
        }

        // Convert KeyedRead - recursively convert receiver and key to preserve pipes
        // This handles expressions like `(items$ | async)[0]` where the pipe is in the receiver
        AngularExpression::KeyedRead(keyed) => {
            let keyed = keyed.unbox();
            let receiver = convert_ast_to_ir(job, keyed.receiver);
            let key = convert_ast_to_ir(job, keyed.key);
            Box::new_in(
                IrExpression::ResolvedKeyedRead(oxc_allocator::Box::new_in(
                    ResolvedKeyedReadExpr {
                        receiver,
                        key,
                        source_span: Some(keyed.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Call - recursively convert receiver and args to preserve pipes
        // This handles expressions like `(fn$ | async)(arg | pipe)` where pipes are in receiver or args
        AngularExpression::Call(call) => {
            let call = call.unbox();
            let receiver = convert_ast_to_ir(job, call.receiver);
            let mut args = Vec::with_capacity_in(call.args.len(), allocator);
            for arg in call.args {
                let arg_expr = convert_ast_to_ir(job, arg);
                args.push(arg_expr.unbox());
            }
            Box::new_in(
                IrExpression::ResolvedCall(oxc_allocator::Box::new_in(
                    ResolvedCallExpr {
                        receiver,
                        args,
                        source_span: Some(call.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert PrefixNot (!) - recursively convert operand to preserve pipes
        // This handles expressions like `!(value$ | async)` where the pipe is in the operand
        AngularExpression::PrefixNot(not) => {
            let not = not.unbox();
            let expr = convert_ast_to_ir(job, not.expression);
            Box::new_in(
                IrExpression::Not(oxc_allocator::Box::new_in(
                    crate::ir::expression::NotExpr {
                        expr,
                        source_span: Some(not.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Unary (+/-) - recursively convert operand to preserve pipes
        // This handles expressions like `+(value$ | pipe)` where the pipe is in the operand
        AngularExpression::Unary(unary) => {
            let unary = unary.unbox();
            let expr = convert_ast_to_ir(job, unary.expr);
            let operator = match unary.operator {
                crate::ast::expression::UnaryOperator::Plus => {
                    crate::ir::expression::IrUnaryOperator::Plus
                }
                crate::ast::expression::UnaryOperator::Minus => {
                    crate::ir::expression::IrUnaryOperator::Minus
                }
            };
            Box::new_in(
                IrExpression::Unary(oxc_allocator::Box::new_in(
                    crate::ir::expression::UnaryExpr {
                        operator,
                        expr,
                        source_span: Some(unary.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert TypeofExpression - recursively convert operand to preserve pipes
        AngularExpression::TypeofExpression(typeof_expr) => {
            let typeof_expr = typeof_expr.unbox();
            let expr = convert_ast_to_ir(job, typeof_expr.expression);
            Box::new_in(
                IrExpression::Typeof(oxc_allocator::Box::new_in(
                    crate::ir::expression::TypeofExpr {
                        expr,
                        source_span: Some(typeof_expr.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert VoidExpression - recursively convert operand to preserve pipes
        AngularExpression::VoidExpression(void_expr) => {
            let void_expr = void_expr.unbox();
            let expr = convert_ast_to_ir(job, void_expr.expression);
            Box::new_in(
                IrExpression::Void(oxc_allocator::Box::new_in(
                    crate::ir::expression::VoidExpr {
                        expr,
                        source_span: Some(void_expr.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Empty expression - convert directly to IrExpression::Empty
        // This ensures is_empty() check works in remove_empty_bindings phase
        // TypeScript reference: ingest.ts lines 1184-1185
        AngularExpression::Empty(empty) => {
            let empty = empty.unbox();
            Box::new_in(
                IrExpression::Empty(Box::new_in(
                    EmptyExpr { source_span: Some(empty.source_span.to_span()) },
                    allocator,
                )),
                allocator,
            )
        }

        // For all other expressions, store in ExpressionStore and return reference.
        other => store_and_ref_expr(job, other),
    }
}

/// Converts an AST binary operator to an IR binary operator.
fn convert_binary_op(
    op: crate::ast::expression::BinaryOperator,
) -> crate::ir::expression::IrBinaryOperator {
    use crate::ast::expression::BinaryOperator as AstOp;
    use crate::ir::expression::IrBinaryOperator as IrOp;

    match op {
        AstOp::Add => IrOp::Plus,
        AstOp::Subtract => IrOp::Minus,
        AstOp::Multiply => IrOp::Multiply,
        AstOp::Divide => IrOp::Divide,
        AstOp::Modulo => IrOp::Modulo,
        AstOp::Power => IrOp::Exponentiation,
        AstOp::Equal => IrOp::Equals,
        AstOp::NotEqual => IrOp::NotEquals,
        AstOp::StrictEqual => IrOp::Identical,
        AstOp::StrictNotEqual => IrOp::NotIdentical,
        AstOp::LessThan => IrOp::Lower,
        AstOp::LessThanOrEqual => IrOp::LowerEquals,
        AstOp::GreaterThan => IrOp::Bigger,
        AstOp::GreaterThanOrEqual => IrOp::BiggerEquals,
        AstOp::And => IrOp::And,
        AstOp::Or => IrOp::Or,
        AstOp::NullishCoalescing => IrOp::NullishCoalesce,
        AstOp::In => IrOp::In,
        AstOp::Instanceof => IrOp::Instanceof,
        AstOp::Assign => IrOp::Assign,
        AstOp::AddAssign => IrOp::AdditionAssignment,
        AstOp::SubtractAssign => IrOp::SubtractionAssignment,
        AstOp::MultiplyAssign => IrOp::MultiplicationAssignment,
        AstOp::DivideAssign => IrOp::DivisionAssignment,
        AstOp::ModuloAssign => IrOp::RemainderAssignment,
        AstOp::PowerAssign => IrOp::ExponentiationAssignment,
        AstOp::AndAssign => IrOp::AndAssignment,
        AstOp::OrAssign => IrOp::OrAssignment,
        AstOp::NullishCoalescingAssign => IrOp::NullishCoalesceAssignment,
    }
}

/// Converts an interpolation expression to an IR interpolation, storing inner expressions.
///
/// This is needed because interpolations contain inner expressions that need to be
/// resolved during name resolution. By converting to IR Interpolation, the inner
/// expressions become visible to the expression transformer.
fn convert_interpolation_to_ir<'a>(
    job: &mut ComponentCompilationJob<'a>,
    expr: AngularExpression<'a>,
) -> Box<'a, IrExpression<'a>> {
    let allocator = job.allocator;
    convert_interpolation_to_ir_with_i18n_placeholders(job, expr, Vec::new_in(allocator))
}

/// Converts an Angular expression to IR, handling interpolations with i18n placeholders.
///
/// This is used for bound text inside i18n blocks where the i18n metadata contains
/// placeholder names that need to be preserved for the i18n message generation.
///
/// Ported from Angular's ingestBoundText (ingest.ts lines 506-512) which passes
/// i18nPlaceholders to the Interpolation constructor.
fn convert_interpolation_to_ir_with_i18n_placeholders<'a>(
    job: &mut ComponentCompilationJob<'a>,
    expr: AngularExpression<'a>,
    i18n_placeholders: Vec<'a, Atom<'a>>,
) -> Box<'a, IrExpression<'a>> {
    let allocator = job.allocator;

    // If it's an interpolation, convert to IR interpolation with inner expressions
    if let AngularExpression::Interpolation(interp_box) = expr {
        // Unbox the interpolation to take ownership of its fields
        let interp = interp_box.unbox();

        let mut ir_expressions = Vec::new_in(allocator);
        for inner_expr in interp.expressions {
            // Convert each inner expression to IR (handles pipes, safe nav, etc.)
            let converted = convert_ast_to_ir(job, inner_expr);
            ir_expressions.push(converted.unbox());
        }

        Box::new_in(
            IrExpression::Interpolation(Box::new_in(
                crate::ir::expression::Interpolation {
                    strings: interp.strings,
                    expressions: ir_expressions,
                    i18n_placeholders,
                    source_span: Some(interp.source_span.to_span()),
                },
                allocator,
            )),
            allocator,
        )
    } else {
        // For non-interpolation expressions, convert to IR
        convert_ast_to_ir(job, expr)
    }
}
/// Ingests a component template into a compilation job.
///
/// This function consumes the R3 AST nodes, moving expressions into the IR.
/// The R3 nodes cannot be used after ingestion.
///
/// This is a convenience wrapper around `ingest_component_with_options` that uses
/// default options. For full control over compilation settings, use
/// `ingest_component_with_options` directly.
pub fn ingest_component<'a>(
    allocator: &'a Allocator,
    component_name: Atom<'a>,
    template: Vec<'a, R3Node<'a>>,
) -> ComponentCompilationJob<'a> {
    ingest_component_with_options(allocator, component_name, template, IngestOptions::default())
}

/// Ingests a component template into a compilation job with the given options.
///
/// This is the full-featured version of `ingest_component` that accepts all
/// compilation options, matching Angular's `ingestComponent()` function signature.
///
/// Ported from Angular's `ingestComponent()` in `template/pipeline/src/ingest.ts`.
///
/// # Parameters
///
/// - `allocator`: The allocator for memory allocation
/// - `component_name`: Name of the component being compiled
/// - `template`: The R3 AST nodes representing the template
/// - `options`: Compilation options including mode, i18n settings, and debug options
///
/// # Returns
///
/// A `ComponentCompilationJob` ready for transformation phases.
pub fn ingest_component_with_options<'a>(
    allocator: &'a Allocator,
    component_name: Atom<'a>,
    template: Vec<'a, R3Node<'a>>,
    options: IngestOptions<'a>,
) -> ComponentCompilationJob<'a> {
    // Create the job with the specified pool starting index.
    // This ensures that when compiling multiple components in the same file,
    // each component's constants have unique names.
    let mut job = ComponentCompilationJob::with_pool_starting_index(
        allocator,
        component_name,
        options.pool_starting_index,
    );

    // Apply options to the job
    job.mode = options.mode;
    job.relative_context_file_path = options.relative_context_file_path;
    job.i18n_use_external_ids = options.i18n_use_external_ids;
    job.enable_debug_locations = options.enable_debug_locations;
    job.relative_template_path = options.relative_template_path;
    job.template_source = options.template_source;

    // Set defer metadata based on emit mode
    // The all_deferrable_deps_fn is stored on the job and referenced during emit
    job.defer_meta = match options.defer_block_deps_emit_mode {
        DeferBlockDepsEmitMode::PerBlock => {
            DeferMetadata::PerBlock { blocks: FxHashMap::default() }
        }
        DeferBlockDepsEmitMode::PerComponent => {
            // In PerComponent mode, dependencies_fn is set from all_deferrable_deps_fn during emit
            DeferMetadata::PerComponent { dependencies_fn: None }
        }
    };

    // Store the all_deferrable_deps_fn reference for emit phase
    // This is used when DeferBlockDepsEmitMode::PerComponent to reference the shared deps function
    job.all_deferrable_deps_fn = options.all_deferrable_deps_fn;

    // Set Angular version for feature-gated instruction selection
    job.angular_version = options.angular_version;

    let root_xref = job.root.xref;

    for node in template {
        ingest_node(&mut job, root_xref, node);
    }

    job
}

/// Ingests a single R3 node into the appropriate view.
///
/// Consumes the node, taking ownership of expressions.
fn ingest_node<'a>(job: &mut ComponentCompilationJob<'a>, view_xref: XrefId, node: R3Node<'a>) {
    match node {
        R3Node::Text(text) => ingest_text(job, view_xref, text.unbox(), None),
        R3Node::BoundText(bound_text) => {
            ingest_bound_text(job, view_xref, bound_text.unbox(), None)
        }
        R3Node::Element(element) => ingest_element(job, view_xref, element.unbox()),
        R3Node::Template(template) => ingest_template(job, view_xref, template.unbox()),
        R3Node::Content(content) => ingest_content(job, view_xref, content.unbox()),
        R3Node::IfBlock(if_block) => ingest_if_block(job, view_xref, if_block.unbox()),
        R3Node::ForLoopBlock(for_block) => ingest_for_block(job, view_xref, for_block.unbox()),
        R3Node::SwitchBlock(switch_block) => {
            ingest_switch_block(job, view_xref, switch_block.unbox())
        }
        R3Node::DeferredBlock(defer_block) => {
            ingest_defer_block(job, view_xref, defer_block.unbox())
        }
        R3Node::LetDeclaration(let_decl) => {
            ingest_let_declaration(job, view_xref, let_decl.unbox())
        }
        R3Node::Comment(_) => {
            // Comments are not ingested into IR
        }
        R3Node::Icu(icu) => ingest_icu(job, view_xref, icu.unbox()),
        R3Node::UnknownBlock(_) => {
            // Unknown blocks are skipped with a warning
        }
        // The following are not standalone nodes in the template
        R3Node::Variable(_) | R3Node::Reference(_) => {
            // Variables and references are handled by their parent nodes
        }
        R3Node::DeferredBlockPlaceholder(_)
        | R3Node::DeferredBlockLoading(_)
        | R3Node::DeferredBlockError(_) => {
            // Defer sub-blocks are handled by the parent defer block
        }
        R3Node::ForLoopBlockEmpty(_) => {
            // Empty block is handled by the parent for block
        }
        R3Node::SwitchBlockCaseGroup(_) => {
            // Switch case groups are handled by the parent switch block
        }
        R3Node::IfBlockBranch(_) => {
            // If branches are handled by the parent if block
        }
        R3Node::Component(_) | R3Node::Directive(_) | R3Node::HostElement(_) => {
            // Components, directives, and host elements are resolved during binding/type checking
        }
    }
}

/// Ingests a static text node.
///
/// `icu_placeholder` is provided when this text is part of an ICU expression,
/// indicating the placeholder name for this text within the ICU message.
fn ingest_text<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    text: R3Text<'a>,
    icu_placeholder: Option<Atom<'a>>,
) {
    let xref = job.allocate_xref_id();

    let op = CreateOp::Text(TextOp {
        base: CreateOpBase { source_span: Some(text.source_span), ..Default::default() },
        xref,
        slot: None,
        initial_value: text.value,
        i18n_placeholder: None,
        icu_placeholder,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(op);
    }
}

/// Ingests a bound text node (with interpolation).
///
/// `icu_placeholder` is provided when this text is part of an ICU expression,
/// indicating the placeholder name for this text within the ICU message.
fn ingest_bound_text<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    bound_text: R3BoundText<'a>,
    icu_placeholder: Option<Atom<'a>>,
) {
    let allocator = job.allocator;
    let xref = job.allocate_xref_id();

    // Create the text slot
    let text_op = CreateOp::Text(TextOp {
        base: CreateOpBase { source_span: Some(bound_text.source_span), ..Default::default() },
        xref,
        slot: None,
        initial_value: Atom::from(""),
        i18n_placeholder: None,
        icu_placeholder,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(text_op);
    }

    // Extract i18n placeholders from the bound text's i18n metadata
    // Ported from Angular's ingestBoundText (ingest.ts lines 485-495)
    let i18n_placeholders: Vec<'_, Atom<'_>> = match &bound_text.i18n {
        Some(I18nMeta::Node(I18nNode::Container(container))) => {
            let mut placeholders = Vec::new_in(allocator);
            for child in container.children.iter() {
                if let I18nNode::Placeholder(placeholder) = child {
                    placeholders.push(placeholder.name.clone());
                }
            }
            placeholders
        }
        _ => Vec::new_in(allocator),
    };

    // Convert the interpolation expression to an IR interpolation.
    // This allows inner expressions to be resolved during name resolution.
    let interpolation = convert_interpolation_to_ir_with_i18n_placeholders(
        job,
        bound_text.value,
        i18n_placeholders,
    );

    // Create InterpolateText update op
    let update_op = UpdateOp::InterpolateText(InterpolateTextOp {
        base: UpdateOpBase { source_span: Some(bound_text.source_span), ..Default::default() },
        target: xref,
        interpolation,
        i18n_placeholder: None,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.update.push(update_op);
    }
}

/// Checks if the i18n metadata is a Message containing a single IcuPlaceholder.
/// Returns the ICU placeholder if so, None otherwise.
fn get_single_icu_placeholder<'a, 'b>(
    meta: &'b Option<I18nMeta<'a>>,
) -> Option<&'b I18nIcuPlaceholder<'a>> {
    if let Some(I18nMeta::Message(message)) = meta {
        if message.nodes.len() == 1 {
            if let I18nNode::IcuPlaceholder(icu_placeholder) = &message.nodes[0] {
                return Some(icu_placeholder);
            }
        }
    }
    None
}

/// Ingests an ICU expression node (plural, select, selectordinal).
///
/// Creates IcuStartOp and IcuEndOp to bracket the ICU expression,
/// and ingests all vars and placeholders within.
///
/// Ported from Angular's `ingestIcu` in `template/pipeline/src/ingest.ts`.
fn ingest_icu<'a>(job: &mut ComponentCompilationJob<'a>, view_xref: XrefId, icu: R3Icu<'a>) {
    // Check if the i18n metadata is a Message with a single IcuPlaceholder
    // TypeScript: if (icu.i18n instanceof i18n.Message && isSingleI18nIcu(icu.i18n))
    let icu_placeholder_name = match get_single_icu_placeholder(&icu.i18n) {
        Some(icu_placeholder) => icu_placeholder.name.clone(),
        None => {
            // TypeScript throws: Error(`Unhandled i18n metadata type for ICU: ${icu.i18n?.constructor.name}`)
            // We report as a diagnostic and return early
            job.diagnostics.push(OxcDiagnostic::error(
                "Unhandled i18n metadata type for ICU: expected Message with single IcuPlaceholder",
            ).with_label(icu.source_span));
            return;
        }
    };

    let xref = job.allocate_xref_id();

    // Create IcuStartOp
    let start_op = CreateOp::IcuStart(IcuStartOp {
        base: CreateOpBase { source_span: Some(icu.source_span), ..Default::default() },
        xref,
        context: None, // Will be set by create_i18n_contexts phase
        message: None, // Will be set by phases
        icu_placeholder: Some(icu_placeholder_name),
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(start_op);
    }

    // Process vars (bound text expressions)
    // In Rust, vars is typed as HashMap<Atom, R3BoundText> so no runtime check needed
    for (placeholder_name, bound_text) in icu.vars {
        ingest_bound_text(job, view_xref, bound_text, Some(placeholder_name));
    }

    // Process placeholders (text or bound text)
    for (placeholder_name, placeholder) in icu.placeholders {
        match placeholder {
            R3IcuPlaceholder::Text(text) => {
                ingest_text(job, view_xref, text, Some(placeholder_name));
            }
            R3IcuPlaceholder::BoundText(bound_text) => {
                ingest_bound_text(job, view_xref, bound_text, Some(placeholder_name));
            }
        }
    }

    // Create IcuEndOp
    let end_op = CreateOp::IcuEnd(IcuEndOp {
        base: CreateOpBase { source_span: Some(icu.source_span), ..Default::default() },
        xref,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(end_op);
    }
}

/// Splits a namespaced name like `:svg:path` into (namespace_key, element_name).
///
/// Ported from Angular's `splitNsName` in `src/ml_parser/tags.ts`.
fn split_ns_name(name: &str) -> (Option<&str>, &str) {
    if name.starts_with(':') {
        // Format is `:namespace:element` (e.g., `:svg:path`)
        if let Some(colon_idx) = name[1..].find(':') {
            let namespace_key = &name[1..colon_idx + 1];
            let element_name = &name[colon_idx + 2..];
            return (Some(namespace_key), element_name);
        }
    }
    // No namespace prefix
    (None, name)
}

/// Converts a namespace key to a Namespace enum.
fn namespace_for_key(key: Option<&str>) -> Namespace {
    match key {
        Some("svg") => Namespace::Svg,
        Some("math") => Namespace::Math,
        _ => Namespace::Html,
    }
}

/// Ingests an element node.
///
/// Ported from Angular's `ingestElement` in `template/pipeline/src/ingest.ts`.
fn ingest_element<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    element: R3Element<'a>,
) {
    // Validate i18n metadata type
    // Ported from Angular's ingest.ts lines 276-281
    if let Some(ref i18n) = element.i18n {
        match i18n {
            I18nMeta::Message(_) | I18nMeta::Node(I18nNode::TagPlaceholder(_)) => {
                // Valid i18n metadata types
            }
            _ => {
                job.diagnostics.push(
                    OxcDiagnostic::error("Unhandled i18n metadata type for element")
                        .with_label(element.source_span),
                );
                return;
            }
        }
    }

    let xref = job.allocate_xref_id();
    let allocator = job.allocator;

    // Split namespace prefix from element name (e.g., `:svg:path` -> ("svg", "path"))
    let (namespace_key, element_name) = split_ns_name(element.name.as_str());
    let namespace = namespace_for_key(namespace_key);

    // Allocate the stripped element name in the arena
    let tag: Atom<'a> = if namespace_key.is_some() {
        Atom::from(allocator.alloc_str(element_name))
    } else {
        element.name.clone()
    };

    // Process local references
    let local_refs = ingest_references_owned(allocator, element.references);

    // Check for formField property binding to create ControlCreateOp.
    // This matches TypeScript's ingest.ts which checks:
    // const fieldInput = element.inputs.find(
    //   (input) => input.name === 'formField' && input.type === e.BindingType.Property
    // );
    use crate::ast::expression::BindingType;
    let field_input_span = element.inputs.iter().find_map(|input| {
        if input.name.as_str() == "formField" && input.binding_type == BindingType::Property {
            Some(input.source_span)
        } else {
            None
        }
    });

    // Always create ElementStart/ElementEnd pairs, even for void/self-closing elements.
    // The empty_elements phase will collapse them to Element when appropriate.
    // This matches TypeScript Angular's ingest.ts which always creates start/end pairs.
    //
    // This is important because when there are listeners (outputs) on a void element,
    // the listener ops are placed between start and end, preventing collapse.
    // For example: <input (change)="..."/> should produce:
    //   domElementStart(0, "input", 1);
    //   domElementEnd();
    // Not: domElement(0, "input", 1);

    // Clone tag for use in listener naming
    let tag_for_listeners = tag.clone();

    // Extract i18n placeholder if element is inside an i18n block (TagPlaceholder)
    // Ported from Angular's ingest.ts line 291
    let i18n_placeholder = if let Some(I18nMeta::Node(I18nNode::TagPlaceholder(tag_placeholder))) =
        &element.i18n
    {
        Some(I18nPlaceholder::new(
            tag_placeholder.start_name.clone(),
            if tag_placeholder.is_void { None } else { Some(tag_placeholder.close_name.clone()) },
        ))
    } else {
        None
    };

    // Element with children: ElementStart ... ElementEnd
    let start_op = CreateOp::ElementStart(ElementStartOp {
        base: CreateOpBase { source_span: Some(element.start_source_span), ..Default::default() },
        xref,
        tag,
        slot: None,
        namespace,
        attribute_namespace: None,
        local_refs,
        local_refs_index: None, // Set by local_refs phase
        non_bindable: false,
        i18n_placeholder,
        attributes: None, // Set by const_collection phase
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(start_op);
    }

    // Ingest static attributes (must happen BEFORE bound inputs for proper order)
    // Static attributes are ingested as BindingOp with BindingKind::Attribute
    // so that binding_specialization can detect ngNonBindable and other special attributes.
    // Note: We preserve i18n metadata for i18n-marked text attributes (e.g., tooltip="text" i18n-tooltip)
    // Element attributes are not structural template attributes
    ingest_static_attributes_with_i18n(job, view_xref, xref, &element.attributes, false);

    // Ingest bindings BEFORE children to ensure update ops are in slot order.
    // This matches Angular's TypeScript implementation in ingest.ts.
    ingest_bindings_owned(
        job,
        view_xref,
        xref,
        Some(tag_for_listeners),
        element.inputs,
        element.outputs,
    );

    // Match TypeScript's buggy condition that ALWAYS allocates an I18nAttributesOp for every element.
    // In TypeScript (ingest.ts line 1416): `if (bindings.some((b) => b?.i18nMessage) !== null)`
    // This is always true because: `Array.some()` returns boolean, and `boolean !== null` is true.
    // This causes TypeScript to allocate an extra xref for EVERY element, which affects the
    // variable counter offset. We need to match this behavior for compatibility.
    let i18n_attrs_xref = job.allocate_xref_id();
    let i18n_attrs_op = CreateOp::I18nAttributes(I18nAttributesOp {
        base: CreateOpBase::default(),
        xref: i18n_attrs_xref,
        handle: I18nSlotHandle::Single(SlotId(0)), // Will be computed during slot allocation
        target: xref,
        configs: Vec::new_in(allocator),
        i18n_attributes_config: None,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(i18n_attrs_op);
    }

    // Start i18n block if element has i18n Message metadata AND has children.
    // Skip i18n for elements with no content (e.g., self-closing elements with i18n attr).
    // Ported from Angular's ingest.ts lines 300-307
    let i18n_block_id = if let Some(I18nMeta::Message(ref message)) = element.i18n {
        // Only create i18n block if there are children to translate
        if element.children.is_empty() {
            None
        } else {
            let i18n_xref = job.allocate_xref_id();
            let instance_id = message.instance_id;

            // Store i18n message metadata keyed by instance_id
            let mut legacy_ids = Vec::new_in(allocator);
            for id in message.legacy_ids.iter() {
                legacy_ids.push(id.clone());
            }

            let metadata = I18nMessageMetadata {
                message_id: if message.id.is_empty() { None } else { Some(message.id.clone()) },
                custom_id: if message.custom_id.is_empty() {
                    None
                } else {
                    Some(message.custom_id.clone())
                },
                meaning: if message.meaning.is_empty() {
                    None
                } else {
                    Some(message.meaning.clone())
                },
                description: if message.description.is_empty() {
                    None
                } else {
                    Some(message.description.clone())
                },
                legacy_ids,
                message_string: if message.message_string.is_empty() {
                    None
                } else {
                    Some(message.message_string.clone())
                },
            };
            job.i18n_message_metadata.insert(instance_id, metadata);

            // Create I18nStartOp
            let i18n_start = CreateOp::I18nStart(I18nStartOp {
                base: CreateOpBase {
                    source_span: Some(element.start_source_span),
                    ..Default::default()
                },
                xref: i18n_xref,
                slot: None,
                context: None,              // Will be set by create_i18n_contexts phase
                message: Some(instance_id), // Instance ID for metadata lookup
                i18n_placeholder: None,     // Root i18n block has no placeholder
                sub_template_index: None,   // Will be set by propagate_i18n_blocks phase
                root: None,                 // Root i18n block has no root
                message_index: None,        // Will be set by i18n_const_collection phase
            });

            if let Some(view) = job.view_mut(view_xref) {
                view.create.push(i18n_start);
            }

            Some(i18n_xref)
        }
    } else {
        None
    };

    // Ingest children (consuming them)
    for child in element.children {
        ingest_node(job, view_xref, child);
    }

    // The source span for the end op is typically the element closing tag. However, if no closing tag
    // exists, such as in `<input>`, we use the start source span instead. Usually the start and end
    // instructions will be collapsed into one `element` instruction, negating the purpose of this
    // fallback, but in cases when it is not collapsed (such as an input with a binding), we still
    // want to map the end instruction to the main element.
    let end_source_span = element.end_source_span.or(Some(element.start_source_span));

    // End i18n block before ElementEnd
    // Ported from Angular's ingest.ts lines 329-335
    if let Some(i18n_xref) = i18n_block_id {
        let i18n_end = CreateOp::I18nEnd(I18nEndOp {
            base: CreateOpBase { source_span: end_source_span, ..Default::default() },
            xref: i18n_xref,
        });

        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(i18n_end);
        }
    }

    let end_op = CreateOp::ElementEnd(ElementEndOp {
        base: CreateOpBase { source_span: end_source_span, ..Default::default() },
        xref,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(end_op);
    }

    // We want to ensure that the controlCreateOp is after the ops that create the element.
    // Ported from Angular's ingest.ts lines 319-327.
    // If the element has a [field] property binding, add ControlCreateOp.
    // This is used for form control bindings that require synchronization.
    if let Some(span) = field_input_span {
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(CreateOp::ControlCreate(ControlCreateOp {
                base: CreateOpBase { source_span: Some(span), ..Default::default() },
            }));
        }
    }
}

/// Ingests static attributes from R3TextAttribute, preserving i18n metadata.
///
/// This version takes R3TextAttribute directly so it can access the i18n field.
/// For i18n-marked text attributes (e.g., `tooltip="text" i18n-tooltip="@@same-key"`),
/// we create an i18n_message xref to ensure proper context assignment in later phases.
///
/// Ported from Angular's ingestElementBindings which passes attr.i18n to createBindingOp
/// (ingest.ts lines 1315-1332).
fn ingest_static_attributes_with_i18n<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    element_xref: XrefId,
    attributes: &[R3TextAttribute<'a>],
    is_structural_template_attribute: bool,
) {
    use crate::output::ast::{LiteralExpr, LiteralValue, OutputExpression};

    let allocator = job.allocator;

    for attr in attributes {
        let name = attr.name.clone();
        let value = attr.value.clone();

        // ngNonBindable and animate.* require special handling
        if name.as_str() == "ngNonBindable" || name.as_str().starts_with("animate.") {
            let literal_expr = OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(value), source_span: None },
                allocator,
            ));
            let value_expr = IrExpression::OutputExpr(Box::new_in(literal_expr, allocator));

            let binding = BindingOp {
                base: UpdateOpBase::default(),
                target: element_xref,
                kind: BindingKind::Attribute,
                name,
                expression: Box::new_in(value_expr, allocator),
                unit: None,
                security_context: SecurityContext::None,
                i18n_message: None,
                is_text_attribute: true,
            };

            if let Some(view) = job.view_mut(view_xref) {
                view.update.push(UpdateOp::Binding(binding));
            }
            continue;
        }

        // Handle i18n message if present (for i18n-* attribute markers)
        // This matches Angular's asMessage(attr.i18n) in ingest.ts line 1329
        //
        // Angular TS stores the i18n.Message object reference directly. We store the
        // instance_id as a dedup key. When the SAME attribute is encountered twice
        // (once for the conditional via ingestControlFlowInsertionPoint, once for the
        // element via this function), they share the same instance_id since it's assigned
        // during parsing and survives moves/copies.
        //
        // Different attributes (even with the same content) get DIFFERENT instance_ids,
        // which is crucial for correct const deduplication.
        let i18n_message = if let Some(I18nMeta::Message(ref message)) = attr.i18n {
            let instance_id = message.instance_id;

            // Store i18n message metadata for later phases (only if not already stored)
            if !job.i18n_message_metadata.contains_key(&instance_id) {
                let mut legacy_ids = Vec::new_in(allocator);
                for id in message.legacy_ids.iter() {
                    legacy_ids.push(id.clone());
                }

                let metadata = I18nMessageMetadata {
                    message_id: if message.id.is_empty() { None } else { Some(message.id.clone()) },
                    custom_id: if message.custom_id.is_empty() {
                        None
                    } else {
                        Some(message.custom_id.clone())
                    },
                    meaning: if message.meaning.is_empty() {
                        None
                    } else {
                        Some(message.meaning.clone())
                    },
                    description: if message.description.is_empty() {
                        None
                    } else {
                        Some(message.description.clone())
                    },
                    legacy_ids,
                    message_string: if message.message_string.is_empty() {
                        None
                    } else {
                        Some(message.message_string.clone())
                    },
                };
                job.i18n_message_metadata.insert(instance_id, metadata);
            }

            Some(instance_id)
        } else {
            None
        };

        // All other static attributes go to the create list as ExtractedAttributeOp
        let literal_expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(value), source_span: None },
            allocator,
        ));
        let value_expr = IrExpression::OutputExpr(Box::new_in(literal_expr, allocator));

        // Use Template kind for structural template attributes, Attribute otherwise
        let binding_kind = if is_structural_template_attribute {
            BindingKind::Template
        } else {
            BindingKind::Attribute
        };

        // Split namespace from attribute name (e.g., `:xmlns:xlink` → namespace="xmlns", name="xlink")
        let (ns, local_name) = split_ns_name(name.as_str());
        let namespace = ns.map(|n| Atom::from(n));
        let local_name = Atom::from(local_name);

        let extracted = ExtractedAttributeOp {
            base: CreateOpBase::default(),
            target: element_xref,
            binding_kind,
            namespace,
            name: local_name,
            value: Some(Box::new_in(value_expr, allocator)),
            security_context: SecurityContext::None,
            truthy_expression: false,
            i18n_context: None,
            i18n_message,
            trusted_value_fn: None,
        };

        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(CreateOp::ExtractedAttribute(extracted));
        }
    }
}

/// Ingests a single static attribute.
///
/// This is used for processing template_attrs in order, where we need to handle
/// each attribute inline rather than batching them. This maintains the correct
/// attribute ordering (e.g., "ngFor" before "ngForOf" in `*ngFor="let item of items"`).
///
/// For structural template attributes (`is_structural_template_attribute=true`), this creates
/// a BindingOp with `is_text_attribute=true` that goes to the update list. This ensures that
/// both text and bound structural template attributes go through the same code path and maintain
/// their original order when extracted by the attribute_extraction phase.
///
/// For regular attributes, this creates an ExtractedAttributeOp directly in the create list.
///
/// Ported from Angular's `createTemplateBinding` in ingest.ts which always creates BindingOp
/// for structural template attributes (lines 1697-1709).
fn ingest_single_static_attribute<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    element_xref: XrefId,
    name: Atom<'a>,
    value: Atom<'a>,
    is_structural_template_attribute: bool,
) {
    use crate::output::ast::{LiteralExpr, LiteralValue, OutputExpression};

    let allocator = job.allocator;

    let literal_expr = OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(value), source_span: None },
        allocator,
    ));
    let value_expr = IrExpression::OutputExpr(Box::new_in(literal_expr, allocator));

    if is_structural_template_attribute {
        // For structural template attributes, create a BindingOp that goes to the update list.
        // This matches Angular's behavior where createTemplateBinding returns BindingOp for
        // structural template attributes, and they're all pushed to the update list together.
        // This ensures correct ordering when attribute_extraction processes them later.
        let op = UpdateOp::Binding(BindingOp {
            base: UpdateOpBase::default(),
            target: element_xref,
            kind: BindingKind::Template,
            name,
            expression: Box::new_in(value_expr, allocator),
            unit: None,
            security_context: SecurityContext::None,
            i18n_message: None,
            is_text_attribute: true,
        });

        if let Some(view) = job.view_mut(view_xref) {
            view.update.push(op);
        }
    } else {
        // For regular (non-structural) attributes, create ExtractedAttributeOp directly
        // Split namespace from attribute name (e.g., `:xmlns:xlink` → namespace="xmlns", name="xlink")
        let (ns, local_name) = split_ns_name(name.as_str());
        let namespace = ns.map(|n| Atom::from(n));
        let local_name = Atom::from(local_name);

        let extracted = ExtractedAttributeOp {
            base: CreateOpBase::default(),
            target: element_xref,
            binding_kind: BindingKind::Attribute,
            namespace,
            name: local_name,
            value: Some(Box::new_in(value_expr, allocator)),
            security_context: SecurityContext::None,
            truthy_expression: false,
            i18n_context: None,
            i18n_message: None,
            trusted_value_fn: None,
        };

        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(CreateOp::ExtractedAttribute(extracted));
        }
    }
}

/// Ingests bindings by taking ownership of the input/output vectors.
fn ingest_bindings_owned<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    element_xref: XrefId,
    tag: Option<Atom<'a>>,
    inputs: Vec<'a, R3BoundAttribute<'a>>,
    outputs: Vec<'a, R3BoundEvent<'a>>,
) {
    // Ingest input bindings, consuming them
    for input in inputs {
        ingest_binding_owned(job, view_xref, element_xref, input, false);
    }

    // Ingest output bindings (listeners), consuming them
    for output in outputs {
        ingest_listener_owned(job, view_xref, element_xref, tag.clone(), output);
    }
}

/// Ingests a binding by taking ownership, moving the expression.
///
/// `is_structural_template_attribute` indicates whether this binding comes from a structural
/// directive syntax (e.g., `*ngIf`, `*cdkPortal`). If true and the binding type is Property,
/// the binding kind will be set to Template instead of Property. This ensures that structural
/// directive bindings are correctly extracted with the Template marker in the consts array.
fn ingest_binding_owned<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    element_xref: XrefId,
    input: R3BoundAttribute<'a>,
    is_structural_template_attribute: bool,
) {
    use crate::ast::expression::BindingType;

    let allocator = job.allocator;

    // Map R3 BindingType to IR BindingKind
    // For structural template attributes, use Template instead of Property
    // Ported from Angular's isStructuralTemplateAttribute handling in ingest.ts
    let binding_kind = match input.binding_type {
        BindingType::Property => {
            if is_structural_template_attribute {
                BindingKind::Template
            } else {
                BindingKind::Property
            }
        }
        BindingType::Attribute => BindingKind::Attribute,
        BindingType::Class => BindingKind::ClassName,
        BindingType::Style => BindingKind::StyleProperty,
        BindingType::TwoWay => BindingKind::TwoWayProperty,
        BindingType::Animation => BindingKind::Animation,
        BindingType::LegacyAnimation => BindingKind::LegacyAnimation,
    };

    // Convert the binding expression to IR, handling pipes and safe navigation.
    // This makes pipes visible to the pipe_creation phase.
    // For interpolated attributes (e.g., title="{{ 'text' | i18n }}"), use
    // convert_interpolation_to_ir to properly extract pipes from the interpolation.
    let expression = if matches!(&input.value, AngularExpression::Interpolation(_)) {
        convert_interpolation_to_ir(job, input.value)
    } else {
        convert_ast_to_ir(job, input.value)
    };

    // Handle i18n message if present (for i18n-* attribute bindings)
    // Ported from Angular's ingestElementBindings in ingest.ts
    //
    // Angular TS stores the i18n.Message object reference directly on the BindingOp
    // without allocating an xref. We store the instance_id as a dedup key instead.
    // The xref for the i18n context is allocated later in create_i18n_contexts.
    let i18n_message = if let Some(I18nMeta::Message(ref message)) = input.i18n {
        let instance_id = message.instance_id;

        // Store i18n message metadata for later phases (keyed by instance_id)
        if !job.i18n_message_metadata.contains_key(&instance_id) {
            let mut legacy_ids = Vec::new_in(allocator);
            for id in message.legacy_ids.iter() {
                legacy_ids.push(id.clone());
            }

            let metadata = I18nMessageMetadata {
                message_id: if message.id.is_empty() { None } else { Some(message.id.clone()) },
                custom_id: if message.custom_id.is_empty() {
                    None
                } else {
                    Some(message.custom_id.clone())
                },
                meaning: if message.meaning.is_empty() {
                    None
                } else {
                    Some(message.meaning.clone())
                },
                description: if message.description.is_empty() {
                    None
                } else {
                    Some(message.description.clone())
                },
                legacy_ids,
                message_string: if message.message_string.is_empty() {
                    None
                } else {
                    Some(message.message_string.clone())
                },
            };
            job.i18n_message_metadata.insert(instance_id, metadata);
        }

        Some(instance_id)
    } else {
        None
    };

    // Create a generic Binding op that will be specialized during binding_specialization phase
    let op = UpdateOp::Binding(BindingOp {
        base: UpdateOpBase { source_span: Some(input.source_span), ..Default::default() },
        target: element_xref,
        kind: binding_kind,
        name: input.name,
        expression,
        unit: input.unit,
        security_context: input.security_context,
        i18n_message,
        is_text_attribute: false,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.update.push(op);
    }
}

/// Ingests a listener by taking ownership.
///
/// Handles Chain expressions in event handlers by extracting all statements.
/// For example, `(blur)="isInputFocused.set(false); onTouch()"` produces:
/// - handler_ops: [ExpressionStatement(isInputFocused.set(false))]
/// - handler_expression: onTouch()
///
/// Ported from Angular's `makeListenerHandlerOps` in ingest.ts.
fn ingest_listener_owned<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    element_xref: XrefId,
    tag: Option<Atom<'a>>,
    output: R3BoundEvent<'a>,
) {
    let allocator = job.allocator;

    // Extract expressions from Chain if present, otherwise wrap single expression in a vec
    // Ported from Angular's makeListenerHandlerOps:
    // let handlerExprs: e.AST[] = handler instanceof e.Chain ? handler.expressions : [handler];
    let handler_exprs: std::vec::Vec<AngularExpression<'a>> =
        if let AngularExpression::Chain(chain) = output.handler {
            // Unbox the Chain to take ownership of the expressions
            let chain = chain.unbox();
            chain.expressions.into_iter().collect()
        } else {
            vec![output.handler]
        };

    // Check if this is a two-way binding
    let op = if output.event_type == ParsedEventType::TwoWay {
        // For two-way bindings, create TwoWayListenerOp with handler_ops
        // Ported from Angular's makeTwoWayListenerHandlerOps in ingest.ts
        // Two-way bindings should only have a single expression
        let handler_expr = if handler_exprs.len() == 1 {
            let expr = handler_exprs.into_iter().next();
            if let Some(e) = expr {
                convert_ast_to_ir(job, e)
            } else {
                // Empty handler - create empty expression
                Box::new_in(
                    IrExpression::Empty(Box::new_in(
                        crate::ir::expression::EmptyExpr { source_span: None },
                        allocator,
                    )),
                    allocator,
                )
            }
        } else {
            // Multiple expressions in two-way binding - this is validated during parsing,
            // but we handle gracefully by using the last expression
            let expr = handler_exprs.into_iter().last();
            if let Some(e) = expr {
                convert_ast_to_ir(job, e)
            } else {
                Box::new_in(
                    IrExpression::Empty(Box::new_in(
                        crate::ir::expression::EmptyExpr { source_span: None },
                        allocator,
                    )),
                    allocator,
                )
            }
        };

        let mut handler_ops = Vec::new_in(allocator);

        // Create $event reference
        let event_ref = IrExpression::LexicalRead(Box::new_in(
            LexicalReadExpr { name: Atom::from("$event"), source_span: None },
            allocator,
        ));

        // Create TwoWayBindingSetExpr(handlerExpr, $event)
        let two_way_set_expr = IrExpression::TwoWayBindingSet(Box::new_in(
            TwoWayBindingSetExpr {
                target: handler_expr,
                value: Box::new_in(event_ref.clone_in(allocator), allocator),
                source_span: Some(output.source_span),
            },
            allocator,
        ));

        // Wrap in output expression statement: ExpressionStatement(TwoWayBindingSetExpr)
        let expr_stmt = crate::output::ast::OutputStatement::Expression(Box::new_in(
            crate::output::ast::ExpressionStatement {
                expr: OutputExpression::WrappedIrNode(Box::new_in(
                    crate::output::ast::WrappedIrExpr {
                        node: Box::new_in(two_way_set_expr, allocator),
                        source_span: Some(output.source_span),
                    },
                    allocator,
                )),
                source_span: Some(output.source_span),
            },
            allocator,
        ));
        handler_ops.push(UpdateOp::Statement(StatementOp {
            base: UpdateOpBase::default(),
            statement: expr_stmt,
        }));

        // Create return statement: return $event
        let return_stmt = crate::output::ast::OutputStatement::Return(Box::new_in(
            crate::output::ast::ReturnStatement {
                value: OutputExpression::WrappedIrNode(Box::new_in(
                    crate::output::ast::WrappedIrExpr {
                        node: Box::new_in(event_ref, allocator),
                        source_span: None,
                    },
                    allocator,
                )),
                source_span: None,
            },
            allocator,
        ));
        handler_ops.push(UpdateOp::Statement(StatementOp {
            base: UpdateOpBase::default(),
            statement: return_stmt,
        }));

        CreateOp::TwoWayListener(TwoWayListenerOp {
            base: CreateOpBase { source_span: Some(output.source_span), ..Default::default() },
            target: element_xref,
            target_slot: SlotId(0), // Will be set during slot allocation
            tag,
            name: output.name,
            handler_ops,
            handler_fn_name: None,
        })
    } else {
        // Regular listener - handles Chain expressions (sequence of statements)
        // Ported from Angular's makeListenerHandlerOps in ingest.ts:
        // - All expressions except the last become ExpressionStatement ops in handler_ops
        // - The last expression becomes the handler_expression (wrapped in return)
        let mut handler_ops = Vec::new_in(allocator);
        let mut handler_expr: Option<Box<'a, IrExpression<'a>>> = None;

        let exprs_count = handler_exprs.len();
        for (i, expr) in handler_exprs.into_iter().enumerate() {
            let ir_expr = convert_ast_to_ir(job, expr);

            if i == exprs_count - 1 {
                // Last expression becomes handler_expression
                handler_expr = Some(ir_expr);
            } else {
                // Non-last expressions become ExpressionStatement ops
                let expr_stmt = crate::output::ast::OutputStatement::Expression(Box::new_in(
                    crate::output::ast::ExpressionStatement {
                        expr: OutputExpression::WrappedIrNode(Box::new_in(
                            crate::output::ast::WrappedIrExpr {
                                node: ir_expr,
                                source_span: Some(output.source_span),
                            },
                            allocator,
                        )),
                        source_span: Some(output.source_span),
                    },
                    allocator,
                ));
                handler_ops.push(UpdateOp::Statement(StatementOp {
                    base: UpdateOpBase::default(),
                    statement: expr_stmt,
                }));
            }
        }

        // Determine if this is an animation listener and extract animation phase
        let (is_animation_listener, animation_phase) = match output.event_type {
            ParsedEventType::Animation => (true, None),
            ParsedEventType::LegacyAnimation => {
                // For legacy animations, parse the phase from the output
                // Phase can be "start" or "done"
                let phase = output.phase.as_ref().and_then(|p| match p.as_str() {
                    "start" => Some(AnimationKind::Enter),
                    "done" => Some(AnimationKind::Leave),
                    _ => None,
                });
                (true, phase)
            }
            _ => (false, None),
        };

        CreateOp::Listener(ListenerOp {
            base: CreateOpBase { source_span: Some(output.source_span), ..Default::default() },
            target: element_xref,
            target_slot: SlotId(0), // Will be set during slot allocation
            tag,
            host_listener: false, // Template listeners are not host listeners
            name: output.name,
            handler_expression: handler_expr,
            handler_ops,
            handler_fn_name: None,
            consume_fn_name: None,
            is_animation_listener,
            animation_phase,
            event_target: output.target,
            consumes_dollar_event: false, // Set during resolve_dollar_event phase
        })
    };

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(op);
    }
}

/// Checks if a template is an explicit `<ng-template>` (as opposed to a structural directive).
/// Ported from Angular's `isPlainTemplate` in `ingest.ts`.
fn is_plain_template(tag_name: &Option<Atom>) -> bool {
    tag_name.as_ref().map_or(false, |tag| tag.as_str() == NG_TEMPLATE_TAG_NAME)
}

/// Ingests a template (ng-template or structural directive).
///
/// Ported from Angular's `ingestTemplate` in `template/pipeline/src/ingest.ts`.
fn ingest_template<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    template: R3Template<'a>,
) {
    // Validate i18n metadata type
    // Ported from Angular's ingest.ts lines 342-347
    if let Some(ref i18n) = template.i18n {
        match i18n {
            I18nMeta::Message(_) | I18nMeta::Node(I18nNode::TagPlaceholder(_)) => {
                // Valid i18n metadata types
            }
            _ => {
                job.diagnostics.push(
                    OxcDiagnostic::error("Unhandled i18n metadata type for template")
                        .with_label(template.source_span),
                );
                return;
            }
        }
    }

    let allocator = job.allocator;

    // Determine template kind: NgTemplate for explicit <ng-template>, Structural for *ngIf etc.
    // Ported from Angular's `isPlainTemplate` check in `ingestTemplate`.
    let template_kind = if is_plain_template(&template.tag_name) {
        TemplateKind::NgTemplate
    } else {
        TemplateKind::Structural
    };

    // Extract i18n message data and spans before consuming the template
    // Ported from Angular's ingest.ts lines 384-397
    let i18n_message = if template_kind == TemplateKind::NgTemplate {
        if let Some(I18nMeta::Message(ref message)) = template.i18n {
            let instance_id = message.instance_id;
            // Clone legacy_ids using the allocator
            let mut legacy_ids = Vec::new_in(allocator);
            for id in message.legacy_ids.iter() {
                legacy_ids.push(id.clone());
            }

            Some((
                instance_id,
                I18nMessageMetadata {
                    message_id: if message.id.is_empty() { None } else { Some(message.id.clone()) },
                    custom_id: if message.custom_id.is_empty() {
                        None
                    } else {
                        Some(message.custom_id.clone())
                    },
                    meaning: if message.meaning.is_empty() {
                        None
                    } else {
                        Some(message.meaning.clone())
                    },
                    description: if message.description.is_empty() {
                        None
                    } else {
                        Some(message.description.clone())
                    },
                    legacy_ids,
                    message_string: if message.message_string.is_empty() {
                        None
                    } else {
                        Some(message.message_string.clone())
                    },
                },
            ))
        } else {
            None
        }
    } else {
        None
    };
    let template_start_span = template.start_source_span;
    let template_end_span = template.end_source_span;

    // Destructure template to control ownership - we need references early for TemplateOp,
    // but children and variables need to be used later (after bindings/i18n processing).
    // This matches Angular's order in ingest.ts lines 374-382.
    let R3Template {
        tag_name,
        attributes,
        inputs,
        outputs,
        template_attrs,
        children,
        references,
        variables,
        source_span,
        i18n,
        ..
    } = template;

    // Extract i18n placeholder if template is inside an i18n block (TagPlaceholder).
    // Ported from Angular's ingest.ts line 357:
    //   const i18nPlaceholder = tmpl.i18n instanceof i18n.TagPlaceholder ? tmpl.i18n : undefined;
    let i18n_placeholder = if let Some(I18nMeta::Node(I18nNode::TagPlaceholder(tag_placeholder))) =
        &i18n
    {
        Some(I18nPlaceholder::new(
            tag_placeholder.start_name.clone(),
            if tag_placeholder.is_void { None } else { Some(tag_placeholder.close_name.clone()) },
        ))
    } else {
        None
    };

    // Create embedded view for template content.
    // In TypeScript, allocateView() returns the embedded view, and its xref is used as the
    // TemplateOp's xref. There is NO separate xref allocation - TemplateOp.xref IS the embedded
    // view's xref.
    let xref = job.allocate_view(Some(view_xref));

    // Parse namespace from tag name (e.g., `:svg:path` → ("svg", "path")).
    // Matches TypeScript's ingestTemplate (lines 351-358 in ingest.ts):
    //   let tagNameWithoutNamespace = tmpl.tagName;
    //   if (tmpl.tagName) {
    //     [namespacePrefix, tagNameWithoutNamespace] = splitNsName(tmpl.tagName);
    //   }
    //   const namespace = namespaceForKey(namespacePrefix);
    let (namespace_key, tag_name_without_namespace) =
        tag_name.as_ref().map_or((None, None), |tag| {
            let (ns, stripped) = split_ns_name(tag.as_str());
            (ns, Some(stripped))
        });
    let namespace = namespace_for_key(namespace_key);

    // Compute fn_name_suffix from stripped tag name, matching TypeScript's behavior.
    // TypeScript (lines 359-360):
    //   const functionNameSuffix = tagNameWithoutNamespace === null
    //     ? '' : prefixWithNamespace(tagNameWithoutNamespace, namespace);
    // prefixWithNamespace returns `:svg:tagName` for SVG, `:math:tagName` for Math, or just
    // `tagName` for HTML. The sanitizeIdentifier function later replaces non-word chars with `_`.
    let fn_name_suffix = tag_name_without_namespace.map(|stripped_tag| {
        let suffix = prefix_with_namespace(stripped_tag, namespace);
        Atom::from(allocator.alloc_str(&suffix))
    });

    // Build the tag atom from the stripped tag name (without namespace prefix).
    // TypeScript passes `tagNameWithoutNamespace` to createTemplateOp (line 367).
    let tag = tag_name_without_namespace.map(|s| Atom::from(allocator.alloc_str(s)));

    // Convert references to local refs - needed for template op creation
    let local_refs = ingest_references_owned(allocator, references);

    // Create template op in parent view
    // Note: xref and embedded_view are the SAME value, matching TypeScript's design where
    // TemplateOp.xref IS the embedded view's xref.
    let op = CreateOp::Template(TemplateOp {
        base: CreateOpBase { source_span: Some(source_span), ..Default::default() },
        xref,
        embedded_view: xref,
        slot: None,
        tag,
        namespace,
        template_kind,
        fn_name_suffix,
        block: None,
        decl_count: None,
        vars: None,
        attributes: None, // Set by attribute extraction phase
        local_refs,
        local_refs_index: None, // Set by local_refs extraction phase
        i18n_placeholder,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(op);
    }

    // Process template_attrs (structural directive bindings like *ngIf, *ngFor, *cdkPortal).
    // These create property bindings that will appear in the update phase (rf & 2).
    // The is_structural_template_attribute=true flag ensures these bindings are extracted
    // with BindingKind::Template, which places them under the Template marker in the consts.
    //
    // IMPORTANT: Process each attribute in order as it appears in template_attrs.
    // Angular's ingestTemplateBindings iterates through templateAttrs and processes each one
    // inline, maintaining the original order (e.g., "ngFor" before "ngForOf").
    // Previously, we collected Text attrs separately which broke the ordering.
    // Ported from Angular's `ingestTemplateBindings` in ingest.ts (lines 1434-1469).
    for template_attr in template_attrs {
        match template_attr {
            R3TemplateAttr::Bound(input) => {
                // Structural template attributes (e.g., ngForOf from *ngFor="let item of items")
                ingest_binding_owned(job, view_xref, xref, input, true);
            }
            R3TemplateAttr::Text(attr) => {
                // Static structural template attributes (e.g., ngFor from *ngFor="let item of items")
                // Process inline to preserve ordering
                ingest_single_static_attribute(job, view_xref, xref, attr.name, attr.value, true);
            }
        }
    }

    // Process hoisted static attributes from the wrapped element.
    // Ported from Angular's `ingestTemplateBindings` - attributes processing (lines 1471-1501).
    // IMPORTANT: Use ingest_static_attributes_with_i18n to preserve i18n metadata (attr.i18n).
    // Angular's TS passes asMessage(attr.i18n) to createTemplateBinding (ingest.ts line 1497).
    if !attributes.is_empty() {
        // Hoisted attributes from wrapped element are regular attributes, not Template
        ingest_static_attributes_with_i18n(job, view_xref, xref, &attributes, false);
    }

    // Process hoisted inputs from the wrapped element (e.g., [class]="..." on <div *ngIf>).
    // These are inputs that were on the wrapped element and need to be bound on the template.
    //
    // For structural directives, these bindings should NOT create update ops because they
    // actually target the child element inside the embedded view, not the template itself.
    // We only create ExtractedAttributeOp for directive matching purposes.
    // Ported from Angular's `createTemplateBinding` (lines 1621-1666 in ingest.ts).
    for input in inputs {
        if template_kind == TemplateKind::Structural {
            // For structural templates, non-structural-template-attribute bindings should not
            // result in an update instruction. They only need ExtractedAttributeOp for directive
            // matching.
            use crate::ast::expression::BindingType;
            match input.binding_type {
                BindingType::Property | BindingType::Class | BindingType::Style => {
                    // Create ExtractedAttributeOp only - no update op.
                    // The actual binding will be created on the child element inside the embedded view.
                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase {
                            source_span: Some(input.source_span),
                            ..Default::default()
                        },
                        target: xref,
                        binding_kind: BindingKind::Property,
                        namespace: None,
                        name: input.name,
                        value: None,
                        security_context: input.security_context,
                        truthy_expression: false,
                        i18n_context: None,
                        i18n_message: None,
                        trusted_value_fn: None,
                    };
                    if let Some(view) = job.view_mut(view_xref) {
                        view.create.push(CreateOp::ExtractedAttribute(extracted));
                    }
                }
                BindingType::TwoWay => {
                    // Create ExtractedAttributeOp with TwoWayProperty kind - no update op.
                    let extracted = ExtractedAttributeOp {
                        base: CreateOpBase {
                            source_span: Some(input.source_span),
                            ..Default::default()
                        },
                        target: xref,
                        binding_kind: BindingKind::TwoWayProperty,
                        namespace: None,
                        name: input.name,
                        value: None,
                        security_context: input.security_context,
                        truthy_expression: false,
                        i18n_context: None,
                        i18n_message: None,
                        trusted_value_fn: None,
                    };
                    if let Some(view) = job.view_mut(view_xref) {
                        view.create.push(CreateOp::ExtractedAttribute(extracted));
                    }
                }
                BindingType::Attribute | BindingType::Animation | BindingType::LegacyAnimation => {
                    // For dynamic attribute or animation bindings on structural templates,
                    // skip entirely - they don't show up on the ng-template const array.
                }
            }
        } else {
            // For explicit <ng-template>, process bindings normally.
            // These are not structural template attributes, so use false.
            ingest_binding_owned(job, view_xref, xref, input, false);
        }
    }

    // Process hoisted outputs from the wrapped element (e.g., (click)="..." on <div *ngIf>).
    // For ng-template: create ListenerOps (the listener is on the template itself).
    // For structural directives: create ExtractedAttributeOps only (the listener is on the
    // child element inside the embedded view, which has its own copy of the outputs).
    // Ported from Angular's `ingestTemplateBindings` (lines 1515-1567 in ingest.ts).
    for output in outputs {
        if template_kind == TemplateKind::NgTemplate {
            // For explicit <ng-template>, create listeners at the template level.
            ingest_listener_owned(job, view_xref, xref, tag_name.clone(), output);
        } else {
            // For structural directives, create ExtractedAttributeOp for directive matching.
            // The actual listener will be created when the child element is ingested.
            // Skip animation listeners as they are excluded from the const array.
            if !matches!(
                output.event_type,
                crate::ast::expression::ParsedEventType::LegacyAnimation
            ) {
                let extracted = ExtractedAttributeOp {
                    base: CreateOpBase {
                        source_span: Some(output.source_span),
                        ..Default::default()
                    },
                    target: xref,
                    binding_kind: BindingKind::Property,
                    namespace: None,
                    name: output.name,
                    value: None,
                    security_context: SecurityContext::None,
                    truthy_expression: false,
                    i18n_context: None,
                    i18n_message: None,
                    trusted_value_fn: None,
                };
                if let Some(view) = job.view_mut(view_xref) {
                    view.create.push(CreateOp::ExtractedAttribute(extracted));
                }
            }
        }
    }

    // Match TypeScript's buggy condition that ALWAYS allocates an I18nAttributesOp for every template.
    // In TypeScript (ingest.ts line 1570): `if (bindings.some((b) => b?.i18nMessage) !== null)`
    // This is always true because: `Array.some()` returns boolean, and `boolean !== null` is true.
    // This causes TypeScript to allocate an extra xref for EVERY template, which affects the
    // variable counter offset. We need to match this behavior for compatibility.
    let i18n_attrs_xref = job.allocate_xref_id();
    let i18n_attrs_op = CreateOp::I18nAttributes(I18nAttributesOp {
        base: CreateOpBase::default(),
        xref: i18n_attrs_xref,
        handle: I18nSlotHandle::Single(SlotId(0)), // Will be computed during slot allocation
        target: xref,
        configs: Vec::new_in(allocator),
        i18n_attributes_config: None,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(i18n_attrs_op);
    }

    // Ingest children into embedded view - MUST happen AFTER bindings and references processing.
    // This matches Angular's order in ingest.ts lines 376-378:
    //   ingestTemplateBindings(unit, templateOp, tmpl, templateKind);
    //   ingestReferences(templateOp, tmpl);
    //   ingestNodes(childView, tmpl.children);
    for child in children {
        ingest_node(job, xref, child);
    }

    // Add template variables to context_variables.
    // Per TypeScript's ingest.ts lines 380-382:
    // for (const {name, value} of tmpl.variables) {
    //   childView.contextVariables.set(name, value !== '' ? value : '$implicit');
    // }
    // This is critical for child views to properly resolve these variables.
    // NOTE: This happens AFTER children are ingested, matching Angular's order.
    for variable in variables {
        let context_value = if variable.value.is_empty() {
            Atom::from("$implicit")
        } else {
            variable.value.clone()
        };
        if let Some(view) = job.view_mut(xref) {
            view.context_variables.push(ContextVariable {
                name: variable.name.clone(),
                value: context_value,
                xref,
            });
        }
    }

    // NOTE: We do NOT call ingest_variable here.
    // Angular's ingest.ts only sets contextVariables (which we did above).
    // The actual VariableOp with XrefId allocation happens later in generate_variables phase.
    // This matches Angular's XrefId allocation sequence exactly.

    // If this is a plain template and there is an i18n message associated with it, insert i18n start
    // and end ops. For structural directive templates, the i18n ops will be added when ingesting the
    // element/template the directive is placed on.
    // Ported from Angular's ingest.ts lines 384-397
    if let Some((instance_id, metadata)) = i18n_message {
        let i18n_xref = job.allocate_xref_id();

        // Store i18n message metadata keyed by instance_id
        job.i18n_message_metadata.insert(instance_id, metadata);

        // Create I18nStartOp and insert after the head of the child view's create list
        let i18n_start = I18nStartOp {
            base: CreateOpBase { source_span: Some(template_start_span), ..Default::default() },
            xref: i18n_xref,
            slot: None,
            context: None,              // Will be set by create_i18n_contexts phase
            message: Some(instance_id), // Instance ID for metadata lookup
            i18n_placeholder: None,     // Root i18n block has no placeholder
            sub_template_index: None,   // Will be set by propagate_i18n_blocks phase
            root: None,                 // Root i18n block has no root
            message_index: None,        // Will be set by i18n_const_collection phase
        };

        // Create I18nEndOp and insert before the tail of the child view's create list
        let i18n_end = I18nEndOp {
            base: CreateOpBase {
                source_span: template_end_span.or(Some(template_start_span)),
                ..Default::default()
            },
            xref: i18n_xref,
        };

        // Insert the ops into the child view's create list.
        // TypeScript uses OpList.insertAfter(head) and OpList.insertBefore(tail),
        // but Angular's OpList has sentinel nodes at head/tail, so insertAfter(head)
        // means "insert as first real element" and insertBefore(tail) means "insert
        // as last real element". Our OpList doesn't have sentinels, so we use
        // push_front for I18nStart (first element) and push for I18nEnd (last element).
        if let Some(view) = job.view_mut(xref) {
            view.create.push_front(CreateOp::I18nStart(i18n_start));
            view.create.push(CreateOp::I18nEnd(i18n_end));
        }
    }
}

/// Ingests ng-content.
///
/// For each attribute on the ng-content element, creates a BindingOp with
/// BindingKind::Attribute. These are later extracted and serialized into
/// the attributes array that is passed to the projection instruction.
///
/// Ported from Angular's `ingestContent` in `ingest.ts` (lines 403-451).
fn ingest_content<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    content: R3Content<'a>,
) {
    let allocator = job.allocator;
    let xref = job.allocate_xref_id();

    // Handle fallback content if ng-content has meaningful children.
    // Ported from Angular's ingestContent (lines 408-422 in ingest.ts):
    // Don't capture default content that's only made up of empty text nodes and comments.
    // Note that we process the default content before the projection in order to match the
    // insertion order at runtime.
    let has_non_trivial_children = content.children.iter().any(|child| {
        match child {
            // Comments don't count as non-trivial
            R3Node::Comment(_) => false,
            // Empty or whitespace-only text doesn't count
            R3Node::Text(text) => !text.value.trim().is_empty(),
            // Everything else counts as non-trivial
            _ => true,
        }
    });

    let fallback = if has_non_trivial_children {
        let fallback_xref = job.allocate_view(Some(view_xref));
        // Ingest all children into the fallback view
        for child in content.children {
            ingest_node(job, fallback_xref, child);
        }
        Some(fallback_xref)
    } else {
        None
    };

    let op = CreateOp::Projection(ProjectionOp {
        base: CreateOpBase { source_span: Some(content.source_span), ..Default::default() },
        xref,
        slot: None,
        projection_slot_index: 0, // Will be set during projection phase
        i18n_placeholder: None,
        selector: Some(content.selector.clone()),
        fallback,
        fallback_i18n_placeholder: None,
        attributes: None, // Set by const_collection phase
    });

    // Create BindingOps for each attribute on ng-content
    // Ported from Angular's ingestContent (lines 432-449 in ingest.ts):
    //   for (const attr of content.attributes) {
    //     unit.update.push(ir.createBindingOp(...));
    //   }
    let mut binding_ops: std::vec::Vec<UpdateOp<'a>> = std::vec::Vec::new();
    for attr in content.attributes {
        // Create a string literal expression for the attribute value
        let value_expr = create_string_literal_atom(allocator, attr.value);
        let binding_op = UpdateOp::Binding(BindingOp {
            base: UpdateOpBase { source_span: Some(attr.source_span), ..Default::default() },
            target: xref,
            kind: BindingKind::Attribute,
            name: attr.name,
            expression: Box::new_in(value_expr, allocator),
            unit: None,
            security_context: SecurityContext::None,
            i18n_message: None,
            is_text_attribute: true, // Static attributes are always text attributes
        });
        binding_ops.push(binding_op);
    }

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(op);
        // Add binding ops to the update list
        for binding_op in binding_ops {
            view.update.push(binding_op);
        }
    }
}

/// Ingests an @if block.
///
/// Creates one CREATE op per branch (ConditionalOp for first, ConditionalBranchCreateOp for rest)
/// and one UPDATE op (ConditionalUpdateOp) containing all conditions.
///
/// Ported from Angular's `ingestIfBlock` in `ingest.ts`.
fn ingest_if_block<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    if_block: R3IfBlock<'a>,
) {
    let allocator = job.allocator;

    let mut first_xref: Option<XrefId> = None;
    let mut conditions: Vec<'a, ConditionalCaseExpr<'a>> = Vec::new_in(allocator);
    let mut create_ops: std::vec::Vec<CreateOp<'a>> = std::vec::Vec::new();

    for (i, branch) in if_block.branches.into_iter().enumerate() {
        // Allocate a new view for this branch
        let branch_view_xref = job.allocate_view(Some(view_xref));

        // Handle expression alias for the view's context variables.
        // Per Angular's ingest.ts line 527-528: use CTX_REF instead of $implicit.
        // This makes the alias resolve to `ctx` directly, not `ctx.$implicit`.
        if let Some(ref alias) = branch.expression_alias {
            if let Some(view) = job.view_mut(branch_view_xref) {
                view.context_variables.push(ContextVariable {
                    name: alias.name.clone(),
                    value: Atom::from(CTX_REF),
                    xref: branch_view_xref,
                });
            }
        }

        // Extract i18n placeholder metadata from the branch.
        // Angular throws for unexpected types; we return early to avoid emitting broken IR.
        let i18n_placeholder = match convert_i18n_meta_to_placeholder(
            branch.i18n,
            &mut job.diagnostics,
            branch.source_span,
            "@if",
        ) {
            Ok(placeholder) => placeholder,
            Err(()) => return,
        };

        // Infer tag name from single root element for content projection
        let tag_name =
            ingest_control_flow_insertion_point(job, view_xref, branch_view_xref, &branch.children);

        // Tag name is passed directly for content projection (including namespace prefix).
        // This matches TypeScript's ingestIfBlock which passes tagName directly from
        // ingestControlFlowInsertionPoint without stripping the namespace.
        let tag = tag_name.clone();

        // fn_name_suffix is hardcoded to "Conditional" without namespace prefix
        // This matches Angular's ingestIfBlock which passes 'Conditional' directly
        let fn_name_suffix = Atom::from("Conditional");

        // Create the appropriate CREATE op
        // Namespace is always HTML for control flow blocks, matching Angular's hardcoded ir.Namespace.HTML
        let create_op = if i == 0 {
            // First branch uses ConditionalOp (ConditionalCreate)
            CreateOp::Conditional(ConditionalOp {
                base: CreateOpBase { source_span: Some(branch.source_span), ..Default::default() },
                xref: branch_view_xref,
                slot: None,
                namespace: Namespace::Html,
                template_kind: TemplateKind::Block,
                fn_name_suffix: fn_name_suffix.clone(),
                tag: tag.clone(),
                decls: None,
                vars: None,
                local_refs: Vec::new_in(allocator),
                local_refs_index: None, // Set by local_refs phase
                i18n_placeholder,
                attributes: None,
                non_bindable: false,
            })
        } else {
            // Subsequent branches use ConditionalBranchCreateOp
            CreateOp::ConditionalBranch(ConditionalBranchCreateOp {
                base: CreateOpBase { source_span: Some(branch.source_span), ..Default::default() },
                xref: branch_view_xref,
                slot: None,
                namespace: Namespace::Html,
                template_kind: TemplateKind::Block,
                fn_name_suffix: fn_name_suffix.clone(),
                tag: tag.clone(),
                decls: None,
                vars: None,
                local_refs: Vec::new_in(allocator),
                local_refs_index: None, // Set by local_refs phase
                i18n_placeholder,
                attributes: None,
                non_bindable: false,
            })
        };

        create_ops.push(create_op);

        // Track the first branch's xref for the update op
        if first_xref.is_none() {
            first_xref = Some(branch_view_xref);
        }

        // Convert the branch condition expression (None for @else)
        // convert_ast_to_ir returns Box<IrExpression>, so we don't need to wrap it again
        let case_expr = branch.expression.map(|expr| convert_ast_to_ir(job, expr));

        // Build the ConditionalCaseExpr for this branch
        let conditional_case = ConditionalCaseExpr {
            expr: case_expr,
            target: branch_view_xref,
            target_slot: SlotHandle::new(),
            alias: branch.expression_alias.map(|v| v.name),
            source_span: Some(branch.source_span),
        };
        conditions.push(conditional_case);

        // Ingest branch children into the branch view
        for child in branch.children {
            ingest_node(job, branch_view_xref, child);
        }
    }

    // Push all create ops to the view
    if let Some(view) = job.view_mut(view_xref) {
        for op in create_ops {
            view.create.push(op);
        }
    }

    // Create the update op with all conditions
    // For @if blocks, test is None (the conditions contain the test expressions)
    if let Some(target) = first_xref {
        let update_op = UpdateOp::Conditional(ConditionalUpdateOp {
            base: UpdateOpBase { source_span: Some(if_block.source_span), ..Default::default() },
            target,
            test: None, // @if has no separate test expression (unlike @switch)
            conditions,
            processed: None,
            context_value: None,
        });

        if let Some(view) = job.view_mut(view_xref) {
            view.update.push(update_op);
        }
    }
}

/// Ingests a @for block.
///
/// This function follows the Angular template pipeline's ingestion pattern for @for blocks.
/// It creates unique index/count variable names for nested loop disambiguation, sets up
/// context variables and aliases for the repeater view, and creates the IR operations.
///
/// Ported from Angular's `ingestForBlock` in `ingest.ts`.
fn ingest_for_block<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    for_block: R3ForLoopBlock<'a>,
) {
    let allocator = job.allocator;

    // Create embedded view for loop body.
    // Note: Unlike some other ops, we do NOT allocate a separate xref for the RepeaterCreate op.
    // The RepeaterCreate's xref IS the body view's xref, matching TypeScript's behavior where
    // createRepeaterCreateOp takes primaryView (the body view) and uses it as the op's xref.
    // This is important for correct variable naming (e.g., ɵ$index_N uses the body view's xref).
    let body_xref = job.allocate_view(Some(view_xref));

    // Create unique names for $index and $count that are suffixed with the view xref
    // to disambiguate nested @for loops. This matches Angular's TemplateDefinitionBuilder pattern.
    let index_name: Atom<'a> = {
        let s = allocator.alloc_str(&format!("ɵ$index_{}", body_xref.0));
        Atom::from(s)
    };
    let count_name: Atom<'a> = {
        let s = allocator.alloc_str(&format!("ɵ$count_{}", body_xref.0));
        Atom::from(s)
    };

    // Collect context variables and aliases for the body view
    let mut context_variables: Vec<'a, ContextVariable<'a>> = Vec::new_in(allocator);
    let mut aliases: Vec<'a, AliasVariable<'a>> = Vec::new_in(allocator);

    // Add the item variable (maps to $implicit in the context)
    context_variables.push(ContextVariable {
        name: for_block.item.name.clone(),
        value: Atom::from("$implicit"),
        xref: body_xref,
    });

    // Build var_names for the repeater, tracking user-defined aliases
    #[allow(clippy::needless_update)]
    let mut var_names = RepeaterVarNames {
        item: Some(for_block.item.name.clone()),
        count: None,
        index: oxc_allocator::Vec::new_in(allocator),
        first: None,
        last: None,
        even: None,
        odd: None,
    };

    // Process context variables from the for block.
    // Following TypeScript's ingest.ts lines 931-946, we:
    // 1. First pass: collect all names that reference $index (for indexVarNames)
    // 2. Main logic: if name === '$index', add context variables; else create alias
    //
    // The key distinction is based on var.name, NOT var.value:
    // - {name: '$index', value: '$index'} -> add to contextVariables
    // - {name: 'i', value: '$index'} -> create an alias with expression referencing ɵ$index_N

    // First pass: collect index var names (all names that reference $index by value)
    // This is used for track expression variable replacement
    for var in for_block.context_variables.iter() {
        if var.value.as_str() == "$index" {
            var_names.index.push(var.name.clone());
        }
    }

    // Main pass: process each context variable based on its NAME (not value)
    for var in for_block.context_variables.iter() {
        match var.name.as_str() {
            "$index" => {
                // This is the implicit $index variable (name and value are both $index)
                // Add both $index and the unique indexed name to context
                context_variables.push(ContextVariable {
                    name: Atom::from("$index"),
                    value: var.value.clone(),
                    xref: body_xref,
                });
                context_variables.push(ContextVariable {
                    name: index_name.clone(),
                    value: var.value.clone(),
                    xref: body_xref,
                });
            }
            "$count" => {
                // This is the implicit $count variable (name and value are both $count)
                // Add both $count and the unique counted name to context
                context_variables.push(ContextVariable {
                    name: Atom::from("$count"),
                    value: var.value.clone(),
                    xref: body_xref,
                });
                context_variables.push(ContextVariable {
                    name: count_name.clone(),
                    value: var.value.clone(),
                    xref: body_xref,
                });
            }
            _ => {
                // User-defined alias (e.g., 'let i = $index', 'let isFirst = $first')
                // Create an alias with the appropriate expression.
                // Angular throws for unknown variables; we return early to avoid
                // emitting broken IR.
                let expression = match get_computed_for_loop_variable_expression(
                    allocator,
                    var.value.as_str(),
                    &index_name,
                    &count_name,
                    &mut job.diagnostics,
                ) {
                    Ok(expr) => expr,
                    Err(()) => return,
                };
                aliases.push(AliasVariable { identifier: var.name.clone(), expression });

                // Track in var_names for track expression variable replacement
                match var.value.as_str() {
                    "$first" => var_names.first = Some(var.name.clone()),
                    "$last" => var_names.last = Some(var.name.clone()),
                    "$even" => var_names.even = Some(var.name.clone()),
                    "$odd" => var_names.odd = Some(var.name.clone()),
                    "$count" => var_names.count = Some(var.name.clone()),
                    // $index is already tracked in the first pass
                    _ => {}
                }
            }
        }
    }

    // Apply context variables and aliases to the body view
    if let Some(body_view) = job.view_mut(body_xref) {
        body_view.context_variables = context_variables;
        body_view.aliases = aliases;
    }

    // Infer tag name from single root element for content projection (body)
    let tag = ingest_control_flow_insertion_point(job, view_xref, body_xref, &for_block.children);

    // Ingest loop children
    for child in for_block.children {
        ingest_node(job, body_xref, child);
    }

    // Handle @empty block if present
    let (empty_view, empty_tag, empty_i18n_placeholder) = if let Some(empty) = for_block.empty {
        let empty_xref = job.allocate_view(Some(view_xref));
        // Infer tag name from single root element for content projection (@empty)
        let empty_tag =
            ingest_control_flow_insertion_point(job, view_xref, empty_xref, &empty.children);

        // Extract i18n placeholder from @empty block if present.
        // Per Angular's ingest.ts lines 970-974, only BlockPlaceholder is valid for @empty.
        // Angular throws for unexpected types; we return early to avoid emitting broken IR.
        let empty_i18n_placeholder = match convert_i18n_meta_to_placeholder(
            empty.i18n,
            &mut job.diagnostics,
            empty.source_span,
            "@empty",
        ) {
            Ok(placeholder) => placeholder,
            Err(()) => return,
        };

        for child in empty.children {
            ingest_node(job, empty_xref, child);
        }
        (Some(empty_xref), empty_tag, empty_i18n_placeholder)
    } else {
        (None, None, None)
    };

    // Extract i18n placeholder from @for block if present.
    // Per Angular's ingest.ts lines 967-969, only BlockPlaceholder is valid for @for.
    // Angular throws for unexpected types; we return early to avoid emitting broken IR.
    let i18n_placeholder = match convert_i18n_meta_to_placeholder(
        for_block.i18n,
        &mut job.diagnostics,
        for_block.source_span,
        "@for",
    ) {
        Ok(placeholder) => placeholder,
        Err(()) => return,
    };

    // Convert the track expression from the for block.
    // ASTWithSource wraps the expression, so we extract the inner ast.
    let track = convert_ast_to_ir(job, for_block.track_by.ast);

    let op = CreateOp::RepeaterCreate(RepeaterCreateOp {
        base: CreateOpBase { source_span: Some(for_block.source_span), ..Default::default() },
        xref: body_xref,
        body_view: body_xref,
        slot: None,
        track,
        track_fn_name: None,
        track_by_ops: None,
        uses_component_instance: false,
        empty_view,
        empty_slot: None,
        empty_decl_count: None,
        empty_var_count: None,
        decls: None,
        vars: None,
        var_names,
        tag,
        attributes: None,
        empty_tag,
        empty_attributes: None,
        i18n_placeholder,
        empty_i18n_placeholder,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(op);
    }

    // Create the update operation for the repeater
    // This emits ɵɵrepeater(collection) in the update phase
    let collection = convert_ast_to_ir(job, for_block.expression.ast);
    let update_op = UpdateOp::Repeater(RepeaterOp {
        base: UpdateOpBase { source_span: Some(for_block.source_span), ..Default::default() },
        target: body_xref,
        target_slot: SlotId(0), // Will be set during slot allocation
        collection,
    });
    if let Some(view) = job.view_mut(view_xref) {
        view.update.push(update_op);
    }
}

/// Creates a computed expression for @for loop variables.
///
/// Ported from Angular's `getComputedForLoopVariableExpression` in `ingest.ts`.
/// Returns `Ok(expression)` for known loop variables, or `Err(())` for unknown
/// variables (matching Angular's throw behavior). A diagnostic is pushed on error.
fn get_computed_for_loop_variable_expression<'a>(
    allocator: &'a Allocator,
    value: &str,
    index_name: &Atom<'a>,
    count_name: &Atom<'a>,
    diagnostics: &mut std::vec::Vec<OxcDiagnostic>,
) -> Result<IrExpression<'a>, ()> {
    match value {
        "$index" => {
            // Return LexicalRead of the index variable
            Ok(IrExpression::LexicalRead(Box::new_in(
                LexicalReadExpr { name: index_name.clone(), source_span: None },
                allocator,
            )))
        }
        "$count" => {
            // Return LexicalRead of the count variable
            Ok(IrExpression::LexicalRead(Box::new_in(
                LexicalReadExpr { name: count_name.clone(), source_span: None },
                allocator,
            )))
        }
        "$first" => {
            // $index === 0
            Ok(create_binary_identical(
                allocator,
                create_lexical_read(allocator, index_name),
                create_number_literal(allocator, 0.0),
            ))
        }
        "$last" => {
            // $index === $count - 1
            Ok(create_binary_identical(
                allocator,
                create_lexical_read(allocator, index_name),
                create_binary_minus(
                    allocator,
                    create_lexical_read(allocator, count_name),
                    create_number_literal(allocator, 1.0),
                ),
            ))
        }
        "$even" => {
            // $index % 2 === 0
            Ok(create_binary_identical(
                allocator,
                create_binary_modulo(
                    allocator,
                    create_lexical_read(allocator, index_name),
                    create_number_literal(allocator, 2.0),
                ),
                create_number_literal(allocator, 0.0),
            ))
        }
        "$odd" => {
            // $index % 2 !== 0
            Ok(create_binary_not_identical(
                allocator,
                create_binary_modulo(
                    allocator,
                    create_lexical_read(allocator, index_name),
                    create_number_literal(allocator, 2.0),
                ),
                create_number_literal(allocator, 0.0),
            ))
        }
        _ => {
            // Angular throws: "AssertionError: unknown @for loop variable ${variable.value}"
            // This should not happen if the parser correctly validates loop variables.
            // We report a diagnostic and return Err to stop ingestion of this block,
            // matching Angular's fail-fast behavior.
            diagnostics.push(OxcDiagnostic::error(format!(
                "AssertionError: unknown @for loop variable {value}"
            )));
            Err(())
        }
    }
}

/// Helper: create a LexicalRead expression
fn create_lexical_read<'a>(allocator: &'a Allocator, name: &Atom<'a>) -> IrExpression<'a> {
    IrExpression::LexicalRead(Box::new_in(
        LexicalReadExpr { name: name.clone(), source_span: None },
        allocator,
    ))
}

/// Helper: create a number literal as an AST expression wrapped in IR
fn create_number_literal<'a>(allocator: &'a Allocator, value: f64) -> IrExpression<'a> {
    use crate::ast::expression::{AbsoluteSourceSpan, LiteralPrimitive, LiteralValue, ParseSpan};

    IrExpression::Ast(Box::new_in(
        crate::ast::expression::AngularExpression::LiteralPrimitive(Box::new_in(
            LiteralPrimitive {
                value: LiteralValue::Number(value),
                span: ParseSpan::new(0, 0),
                source_span: AbsoluteSourceSpan::new(0, 0),
            },
            allocator,
        )),
        allocator,
    ))
}

/// Helper: create a string literal as an AST expression wrapped in IR
fn create_string_literal_atom<'a>(allocator: &'a Allocator, value: Atom<'a>) -> IrExpression<'a> {
    use crate::ast::expression::{AbsoluteSourceSpan, LiteralPrimitive, LiteralValue, ParseSpan};

    IrExpression::Ast(Box::new_in(
        crate::ast::expression::AngularExpression::LiteralPrimitive(Box::new_in(
            LiteralPrimitive {
                value: LiteralValue::String(value),
                span: ParseSpan::new(0, 0),
                source_span: AbsoluteSourceSpan::new(0, 0),
            },
            allocator,
        )),
        allocator,
    ))
}

/// Helper: create a binary identical (===) expression
fn create_binary_identical<'a>(
    allocator: &'a Allocator,
    lhs: IrExpression<'a>,
    rhs: IrExpression<'a>,
) -> IrExpression<'a> {
    IrExpression::Binary(Box::new_in(
        BinaryExpr {
            operator: IrBinaryOperator::Identical,
            lhs: Box::new_in(lhs, allocator),
            rhs: Box::new_in(rhs, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Helper: create a binary not identical (!==) expression
fn create_binary_not_identical<'a>(
    allocator: &'a Allocator,
    lhs: IrExpression<'a>,
    rhs: IrExpression<'a>,
) -> IrExpression<'a> {
    IrExpression::Binary(Box::new_in(
        BinaryExpr {
            operator: IrBinaryOperator::NotIdentical,
            lhs: Box::new_in(lhs, allocator),
            rhs: Box::new_in(rhs, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Helper: create a binary minus (-) expression
fn create_binary_minus<'a>(
    allocator: &'a Allocator,
    lhs: IrExpression<'a>,
    rhs: IrExpression<'a>,
) -> IrExpression<'a> {
    IrExpression::Binary(Box::new_in(
        BinaryExpr {
            operator: IrBinaryOperator::Minus,
            lhs: Box::new_in(lhs, allocator),
            rhs: Box::new_in(rhs, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Helper: create a binary modulo (%) expression
fn create_binary_modulo<'a>(
    allocator: &'a Allocator,
    lhs: IrExpression<'a>,
    rhs: IrExpression<'a>,
) -> IrExpression<'a> {
    IrExpression::Binary(Box::new_in(
        BinaryExpr {
            operator: IrBinaryOperator::Modulo,
            lhs: Box::new_in(lhs, allocator),
            rhs: Box::new_in(rhs, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Ingests a @switch block.
///
/// Creates one CREATE op per case (ConditionalOp for first, ConditionalBranchCreateOp for rest)
/// and one UPDATE op (ConditionalUpdateOp) containing all conditions.
///
/// Angular's `ingestSwitchBlock` in `ingest.ts` iterates groups in source order, but the
/// `generateConditionalExpressions` phase later splices `@default` out and uses it as the
/// ternary fallback. Because the Rust pipeline's conditional codegen expects `@default` last,
/// we reorder here so that slot allocation, function naming, and the conditional expression
/// all match Angular's compiled output.
///
/// Ported from Angular's `ingestSwitchBlock` in `ingest.ts`.
fn ingest_switch_block<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    switch_block: R3SwitchBlock<'a>,
) {
    // Don't ingest empty switches since they won't render anything
    if switch_block.groups.is_empty() {
        return;
    }

    let allocator = job.allocator;

    // Convert the main switch expression as the test
    let test = convert_ast_to_ir(job, switch_block.expression);

    // Iterate groups in source order, matching Angular TS's ingestSwitchBlock.
    // The downstream generate_conditional_expressions phase handles @default at
    // any position by splicing it out as the ternary fallback base.
    let mut first_xref: Option<XrefId> = None;
    let mut conditions: Vec<'a, ConditionalCaseExpr<'a>> = Vec::new_in(allocator);
    let mut create_ops: std::vec::Vec<CreateOp<'a>> = std::vec::Vec::new();

    for (i, group) in switch_block.groups.into_iter().enumerate() {
        // Allocate a new view for this group
        let group_view_xref = job.allocate_view(Some(view_xref));

        // Extract i18n placeholder metadata from the group.
        // Angular throws for unexpected types; we return early to avoid emitting broken IR.
        let i18n_placeholder = match convert_i18n_meta_to_placeholder(
            group.i18n,
            &mut job.diagnostics,
            group.source_span,
            "@switch",
        ) {
            Ok(placeholder) => placeholder,
            Err(()) => return,
        };

        // Infer tag name from single root element for content projection
        let tag_name =
            ingest_control_flow_insertion_point(job, view_xref, group_view_xref, &group.children);

        // Tag name is passed directly for content projection (including namespace prefix).
        // This matches TypeScript's ingestSwitchBlock which passes tagName directly from
        // ingestControlFlowInsertionPoint without stripping the namespace.
        let tag = tag_name.clone();

        // fn_name_suffix is hardcoded to "Case" without namespace prefix
        // This matches Angular's ingestSwitchBlock which passes 'Case' directly
        let fn_name_suffix = Atom::from("Case");

        // Create the appropriate CREATE op
        // Namespace is always HTML for control flow blocks, matching Angular's hardcoded ir.Namespace.HTML
        let create_op = if i == 0 {
            // First group uses ConditionalOp (ConditionalCreate)
            CreateOp::Conditional(ConditionalOp {
                base: CreateOpBase { source_span: Some(group.source_span), ..Default::default() },
                xref: group_view_xref,
                slot: None,
                namespace: Namespace::Html,
                template_kind: TemplateKind::Block,
                fn_name_suffix: fn_name_suffix.clone(),
                tag: tag.clone(),
                decls: None,
                vars: None,
                local_refs: Vec::new_in(allocator),
                local_refs_index: None, // Set by local_refs phase
                i18n_placeholder,
                attributes: None,
                non_bindable: false,
            })
        } else {
            // Subsequent groups use ConditionalBranchCreateOp
            CreateOp::ConditionalBranch(ConditionalBranchCreateOp {
                base: CreateOpBase { source_span: Some(group.source_span), ..Default::default() },
                xref: group_view_xref,
                slot: None,
                namespace: Namespace::Html,
                template_kind: TemplateKind::Block,
                fn_name_suffix: fn_name_suffix.clone(),
                tag: tag.clone(),
                decls: None,
                vars: None,
                local_refs: Vec::new_in(allocator),
                local_refs_index: None, // Set by local_refs phase
                i18n_placeholder,
                attributes: None,
                non_bindable: false,
            })
        };

        create_ops.push(create_op);

        // Track the first group's xref for the update op
        if first_xref.is_none() {
            first_xref = Some(group_view_xref);
        }

        // Process each case in the group - all cases in a group map to the same view
        for switch_case in group.cases {
            // Convert the case expression (None for @default)
            let case_expr = switch_case.expression.map(|expr| convert_ast_to_ir(job, expr));

            // Build the ConditionalCaseExpr for this case
            let conditional_case = ConditionalCaseExpr {
                expr: case_expr,
                target: group_view_xref,
                target_slot: SlotHandle::new(),
                alias: None, // @switch cases don't have aliases
                source_span: Some(switch_case.source_span),
            };
            conditions.push(conditional_case);
        }

        // Ingest group children into the group view
        for child in group.children {
            ingest_node(job, group_view_xref, child);
        }
    }

    // Push all create ops to the view
    if let Some(view) = job.view_mut(view_xref) {
        for op in create_ops {
            view.create.push(op);
        }
    }

    // Create the update op with all conditions
    // For @switch blocks, test is the switch expression
    if let Some(target) = first_xref {
        let update_op = UpdateOp::Conditional(ConditionalUpdateOp {
            base: UpdateOpBase {
                source_span: Some(switch_block.source_span),
                ..Default::default()
            },
            target,
            test: Some(test), // test is already Box<IrExpression> from convert_ast_to_ir
            conditions,
            processed: None,
            context_value: None,
        });

        if let Some(view) = job.view_mut(view_xref) {
            view.update.push(update_op);
        }
    }
}

/// Creates a defer child view with its TemplateOp.
///
/// This is the Rust port of Angular's `ingestDeferView()` function.
/// It creates an embedded view for the defer block content (main, loading, placeholder, error),
/// ingests the children into that view, creates a TemplateOp with the appropriate
/// `Defer{suffix}` function name suffix, and pushes the TemplateOp to the parent view.
///
/// Returns the TemplateOp's xref if children exist, None otherwise.
/// Note: In Angular, TemplateOp.xref IS the embedded view's xref. We follow this pattern
/// so that DeferOp.placeholder_view etc. can be used both for slot resolution (via TemplateOp.xref)
/// and for target resolution (by looking up the view by xref).
fn ingest_defer_view<'a>(
    job: &mut ComponentCompilationJob<'a>,
    parent_xref: XrefId,
    suffix: &str,
    i18n: Option<I18nMeta<'a>>,
    children: Option<Vec<'a, R3Node<'a>>>,
    source_span: Option<oxc_span::Span>,
) -> Option<XrefId> {
    let children = children?;

    // Create a secondary view for this defer block content
    let secondary_view = job.allocate_view(Some(parent_xref));

    // Ingest children into the new view
    for child in children {
        ingest_node(job, secondary_view, child);
    }

    // Create a TemplateOp for this view, like Angular does in ingestDeferView.
    // This TemplateOp will be reified to a `ɵɵdomTemplate()` call.
    // IMPORTANT: In Angular, TemplateOp.xref IS the embedded view's xref (secondaryView.xref).
    // We use the same pattern here so that defer_resolve_targets can find elements by view xref.
    let fn_name_suffix = Some(Atom::from(job.allocator.alloc_str(&format!("Defer{suffix}"))));

    // Convert i18n metadata to placeholder, matching Angular's ingestDeferView which passes
    // i18nMeta through to createTemplateOp. This enables propagate_i18n_blocks to wrap the
    // deferred template with i18nStart/i18nEnd when inside an i18n context.
    // Angular throws for unexpected types; we return early to avoid emitting broken IR.
    let i18n_placeholder = match convert_i18n_meta_to_placeholder(
        i18n,
        &mut job.diagnostics,
        source_span.unwrap_or(oxc_span::SPAN),
        "@defer",
    ) {
        Ok(placeholder) => placeholder,
        Err(()) => return None,
    };

    let template_op = CreateOp::Template(TemplateOp {
        base: CreateOpBase { source_span, ..Default::default() },
        xref: secondary_view, // Use view xref as TemplateOp xref, matching Angular
        embedded_view: secondary_view,
        slot: None,
        tag: None,
        namespace: Namespace::Html,
        template_kind: TemplateKind::Block,
        fn_name_suffix,
        block: None,
        decl_count: None,
        vars: None,
        attributes: None,
        local_refs: Vec::new_in(job.allocator),
        local_refs_index: None,
        i18n_placeholder,
    });

    // Push the TemplateOp to the parent view's create ops
    if let Some(view) = job.view_mut(parent_xref) {
        view.create.push(template_op);
    }

    Some(secondary_view)
}

/// Ingests a @defer block.
fn ingest_defer_block<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    defer_block: R3DeferredBlock<'a>,
) {
    let xref = job.allocate_xref_id();

    // Extract timing values, source spans, and i18n metadata before consuming the blocks
    let placeholder_minimum_time = defer_block.placeholder.as_ref().and_then(|p| p.minimum_time);
    let loading_minimum_time = defer_block.loading.as_ref().and_then(|l| l.minimum_time);
    let loading_after_time = defer_block.loading.as_ref().and_then(|l| l.after_time);
    let loading_source_span = defer_block.loading.as_ref().map(|l| l.source_span);
    let placeholder_source_span = defer_block.placeholder.as_ref().map(|p| p.source_span);
    let error_source_span = defer_block.error.as_ref().map(|e| e.source_span);

    // Generate the defer main view and all secondary views using ingest_defer_view.
    // This creates TemplateOps for each, which will be reified to ɵɵdomTemplate() calls.
    let main_template_xref = ingest_defer_view(
        job,
        view_xref,
        "", // Empty suffix for main content - becomes "Defer"
        defer_block.i18n,
        Some(defer_block.children),
        Some(defer_block.source_span),
    );

    // Destructure sub-blocks to extract both children and i18n before consuming
    let (loading_children, loading_i18n) = match defer_block.loading {
        Some(l) => (Some(l.children), l.i18n),
        None => (None, None),
    };
    let loading_template_xref = ingest_defer_view(
        job,
        view_xref,
        "Loading",
        loading_i18n,
        loading_children,
        loading_source_span,
    );

    let (placeholder_children, placeholder_i18n) = match defer_block.placeholder {
        Some(p) => (Some(p.children), p.i18n),
        None => (None, None),
    };
    let placeholder_template_xref = ingest_defer_view(
        job,
        view_xref,
        "Placeholder",
        placeholder_i18n,
        placeholder_children,
        placeholder_source_span,
    );

    let (error_children, error_i18n) = match defer_block.error {
        Some(e) => (Some(e.children), e.i18n),
        None => (None, None),
    };
    let error_template_xref =
        ingest_defer_view(job, view_xref, "Error", error_i18n, error_children, error_source_span);

    // Set own_resolver_fn based on emit mode
    // This matches Angular's ingestDeferBlock behavior (ingest.ts lines 663-672)
    let own_resolver_fn = match &mut job.defer_meta {
        DeferMetadata::PerBlock { blocks } => {
            // In PerBlock mode, look up the resolver from the blocks map using source_span
            // Use remove() to take ownership (move) since we can't clone OutputExpression
            // TypeScript throws if the block is not in the map at all
            match blocks.remove(&defer_block.source_span) {
                Some(value) => {
                    // Key exists - value may be None (no lazy deps) or Some (has deps)
                    value
                }
                None => {
                    // TypeScript: throw Error(`AssertionError: unable to find a dependency function for this deferred block`)
                    job.diagnostics.push(OxcDiagnostic::error(
                        "AssertionError: unable to find a dependency function for this deferred block",
                    ).with_label(defer_block.source_span));
                    None
                }
            }
        }
        DeferMetadata::PerComponent { .. } => {
            // In PerComponent mode, own_resolver_fn is null
            // The shared all_deferrable_deps_fn is used instead
            None
        }
    };

    // In PerComponent mode, all defer blocks share the same allDeferrableDepsFn reference.
    // We clone it so each defer block gets its own copy (matching TypeScript's behavior
    // where ReadVarExpr is shared by reference).
    let resolver_fn = job.all_deferrable_deps_fn.as_ref().map(|expr| expr.clone_in(job.allocator));

    // Calculate flags based on whether hydrate triggers exist.
    // Matches Angular's calcDeferBlockFlags function in ingest.ts
    let flags = if defer_block.hydrate_triggers.has_any() {
        // TDeferDetailsFlags.HasHydrateTriggers = 1
        Some(1u32)
    } else {
        None
    };

    // Create the DeferOp. The main_slot, loading_slot, placeholder_slot, error_slot
    // will be resolved during slot_allocation phase by looking up the TemplateOp xrefs.
    let op = CreateOp::Defer(DeferOp {
        base: CreateOpBase { source_span: Some(defer_block.source_span), ..Default::default() },
        xref,
        slot: None,
        main_view: main_template_xref, // Now stores the TemplateOp xref, not the view xref
        main_slot: None,               // Will be resolved during slot allocation
        placeholder_view: placeholder_template_xref,
        placeholder_slot: None,
        loading_view: loading_template_xref,
        loading_slot: None,
        error_view: error_template_xref,
        error_slot: None,
        placeholder_minimum_time,
        loading_minimum_time,
        loading_after_time,
        // Config indices are set by configure_defer_instructions phase
        placeholder_config: None,
        loading_config: None,
        // resolver_fn is set by the resolve_defer_deps_fns phase if own_resolver_fn exists
        resolver_fn,
        own_resolver_fn,
        ssr_unique_id: None,
        flags,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(op);
    }

    // Ingest triggers in the order: hydrate -> regular -> prefetch
    // Angular ingests hydrate triggers first since they set up all other triggers during SSR.
    ingest_defer_triggers(
        job,
        view_xref,
        xref,
        defer_block.hydrate_triggers,
        DeferOpModifierKind::Hydrate,
    );
    ingest_defer_triggers(job, view_xref, xref, defer_block.triggers, DeferOpModifierKind::None);
    ingest_defer_triggers(
        job,
        view_xref,
        xref,
        defer_block.prefetch_triggers,
        DeferOpModifierKind::Prefetch,
    );
}

/// Ingests deferred block triggers into DeferOnOp and DeferWhenOp operations.
fn ingest_defer_triggers<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    defer_xref: XrefId,
    triggers: crate::ast::r3::R3DeferredBlockTriggers<'a>,
    modifier: DeferOpModifierKind,
) {
    // Check if any triggers are specified
    let has_any_trigger = triggers.when.is_some()
        || triggers.idle.is_some()
        || triggers.immediate.is_some()
        || triggers.hover.is_some()
        || triggers.timer.is_some()
        || triggers.interaction.is_some()
        || triggers.viewport.is_some()
        || triggers.never.is_some();

    // If no triggers specified and this is the main trigger set (not prefetch/hydrate),
    // add a default idle trigger
    if !has_any_trigger && matches!(modifier, DeferOpModifierKind::None) {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase::default(),
            defer: defer_xref,
            trigger: DeferTriggerKind::Idle,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: None,
            delay: None,
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle 'when' condition trigger (creates DeferWhenOp)
    if let Some(when_trigger) = triggers.when {
        // Convert the when trigger expression to IR
        let condition = convert_ast_to_ir(job, when_trigger.value);
        let op = UpdateOp::DeferWhen(DeferWhenOp {
            base: UpdateOpBase {
                source_span: Some(when_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            condition,
            modifier,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.update.push(op);
        }
    }

    // Handle idle trigger
    if let Some(idle_trigger) = triggers.idle {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(idle_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Idle,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: None,
            delay: None,
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle immediate trigger
    if let Some(immediate_trigger) = triggers.immediate {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(immediate_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Immediate,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: None,
            delay: None,
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle timer trigger
    if let Some(timer_trigger) = triggers.timer {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(timer_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Timer,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: None,
            delay: Some(timer_trigger.delay),
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle hover trigger
    if let Some(hover_trigger) = triggers.hover {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(hover_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Hover,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: hover_trigger.reference,
            delay: None,
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle interaction trigger
    if let Some(interaction_trigger) = triggers.interaction {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(interaction_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Interaction,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: interaction_trigger.reference,
            delay: None,
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle viewport trigger
    if let Some(viewport_trigger) = triggers.viewport {
        let options = viewport_trigger.options.map(|opts| convert_ast_to_ir(job, opts));
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(viewport_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Viewport,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: viewport_trigger.reference,
            delay: None,
            options,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }

    // Handle never trigger
    if let Some(never_trigger) = triggers.never {
        let op = CreateOp::DeferOn(DeferOnOp {
            base: CreateOpBase {
                source_span: Some(never_trigger.source_span),
                ..Default::default()
            },
            defer: defer_xref,
            trigger: DeferTriggerKind::Never,
            modifier,
            target_xref: None,
            target_view: None,
            target_slot: None,
            target_slot_view_steps: None,
            target_name: None,
            delay: None,
            options: None,
        });
        if let Some(view) = job.view_mut(view_xref) {
            view.create.push(op);
        }
    }
}

/// Ingests a @let declaration.
fn ingest_let_declaration<'a>(
    job: &mut ComponentCompilationJob<'a>,
    view_xref: XrefId,
    let_decl: R3LetDeclaration<'a>,
) {
    let xref = job.allocate_xref_id();

    // Create op
    let declared_name = let_decl.name.clone();
    let create_op = CreateOp::DeclareLet(DeclareLetOp {
        base: CreateOpBase { source_span: Some(let_decl.source_span), ..Default::default() },
        xref,
        slot: None,
        name: let_decl.name,
    });

    // Convert the value expression from the let declaration to IR.
    let value = convert_ast_to_ir(job, let_decl.value);

    // Update op
    let update_op = UpdateOp::StoreLet(StoreLetOp {
        base: UpdateOpBase { source_span: Some(let_decl.source_span), ..Default::default() },
        target: xref,
        target_slot: SlotId(0),
        declared_name,
        value,
    });

    if let Some(view) = job.view_mut(view_xref) {
        view.create.push(create_op);
        view.update.push(update_op);
    }
}

/// Converts R3 references to IR LocalRefs by taking ownership.
fn ingest_references_owned<'a>(
    allocator: &'a Allocator,
    references: Vec<'a, crate::ast::r3::R3Reference<'a>>,
) -> Vec<'a, LocalRef<'a>> {
    let mut local_refs = Vec::new_in(allocator);

    for reference in references {
        local_refs.push(LocalRef { name: reference.name, target: reference.value });
    }

    local_refs
}

// ============================================================================
// Host Binding Ingestion
// ============================================================================

/// Input for ingesting host bindings.
///
/// This struct contains the parsed host binding metadata from a component or directive.
/// It mirrors Angular TypeScript's `HostBindingInput` interface in `ingest.ts`.
pub struct HostBindingInput<'a> {
    /// Name of the component/directive.
    pub component_name: Atom<'a>,
    /// CSS selector of the component/directive.
    pub component_selector: Atom<'a>,
    /// Host property bindings (`[prop]="expr"`).
    pub properties: Vec<'a, R3BoundAttribute<'a>>,
    /// Static host attributes (`attr="value"`).
    /// Uses OutputExpression to match TypeScript's `{[key: string]: o.Expression}`.
    pub attributes: FxHashMap<Atom<'a>, crate::output::ast::OutputExpression<'a>>,
    /// Host event bindings (`(event)="handler"`).
    pub events: Vec<'a, R3BoundEvent<'a>>,
}

/// Stores an expression from a host binding and returns an IrExpression that references it.
fn host_store_and_ref_expr<'a>(
    job: &mut HostBindingCompilationJob<'a>,
    expr: AngularExpression<'a>,
) -> Box<'a, IrExpression<'a>> {
    let id = job.store_expression(expr);
    Box::new_in(IrExpression::ExpressionRef(id), job.allocator)
}

/// Converts an Angular expression to an IR expression for host bindings.
///
/// This function directly converts pipe expressions to their IR equivalents,
/// making them visible to subsequent phases like `pipe_creation`.
/// Other expressions are stored in the ExpressionStore and referenced by ID.
fn host_convert_ast_to_ir<'a>(
    job: &mut HostBindingCompilationJob<'a>,
    expr: AngularExpression<'a>,
) -> Box<'a, IrExpression<'a>> {
    let allocator = job.allocator;

    match expr {
        // Convert BindingPipe to IrExpression::PipeBinding
        AngularExpression::BindingPipe(pipe) => {
            let pipe = pipe.unbox();
            let target = job.allocate_xref_id();

            let mut args = Vec::with_capacity_in(1 + pipe.args.len(), allocator);

            // First argument is the pipe input expression
            let input_expr = host_convert_ast_to_ir(job, pipe.exp);
            args.push(input_expr.unbox());

            // Remaining arguments are the pipe arguments
            for arg in pipe.args {
                let arg_expr = host_convert_ast_to_ir(job, arg);
                args.push(arg_expr.unbox());
            }

            Box::new_in(
                IrExpression::PipeBinding(Box::new_in(
                    PipeBindingExpr {
                        target,
                        target_slot: SlotHandle::new(),
                        name: pipe.name,
                        args,
                        var_offset: None,
                        source_span: Some(pipe.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Non-null assertion is transparent - just unwrap and convert inner expression
        AngularExpression::NonNullAssert(nna) => {
            let nna = nna.unbox();
            host_convert_ast_to_ir(job, nna.expression)
        }

        // Convert SafePropertyRead (a?.b) to IR SafePropertyReadExpr
        AngularExpression::SafePropertyRead(safe) => {
            let safe = safe.unbox();
            let receiver = host_convert_ast_to_ir(job, safe.receiver);
            Box::new_in(
                IrExpression::SafePropertyRead(Box::new_in(
                    SafePropertyReadExpr {
                        receiver,
                        name: safe.name,
                        source_span: Some(safe.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert SafeKeyedRead (a?.[b]) to IR SafeKeyedReadExpr
        AngularExpression::SafeKeyedRead(safe) => {
            let safe = safe.unbox();
            let receiver = host_convert_ast_to_ir(job, safe.receiver);
            let index = host_convert_ast_to_ir(job, safe.key);
            Box::new_in(
                IrExpression::SafeKeyedRead(Box::new_in(
                    SafeKeyedReadExpr {
                        receiver,
                        index,
                        source_span: Some(safe.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert SafeCall (a?.()) to IR SafeInvokeFunctionExpr
        AngularExpression::SafeCall(safe) => {
            let safe = safe.unbox();
            let receiver = host_convert_ast_to_ir(job, safe.receiver);
            let mut args = Vec::with_capacity_in(safe.args.len(), allocator);
            for arg in safe.args {
                let arg_expr = host_convert_ast_to_ir(job, arg);
                args.push(arg_expr.unbox());
            }
            Box::new_in(
                IrExpression::SafeInvokeFunction(Box::new_in(
                    SafeInvokeFunctionExpr { receiver, args, source_span: None },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Binary expression - recursively convert operands to preserve pipes
        AngularExpression::Binary(bin) => {
            let bin = bin.unbox();
            let lhs = host_convert_ast_to_ir(job, bin.left);
            let rhs = host_convert_ast_to_ir(job, bin.right);

            Box::new_in(
                IrExpression::Binary(oxc_allocator::Box::new_in(
                    crate::ir::expression::BinaryExpr {
                        operator: convert_binary_op(bin.operation),
                        lhs,
                        rhs,
                        source_span: Some(bin.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert ParenthesizedExpression - recursively convert inner expression to preserve pipes
        AngularExpression::ParenthesizedExpression(paren) => {
            let paren = paren.unbox();
            host_convert_ast_to_ir(job, paren.expression)
        }

        // Convert Conditional expression (ternary) - recursively convert operands to preserve pipes
        AngularExpression::Conditional(cond) => {
            let cond = cond.unbox();
            let condition = host_convert_ast_to_ir(job, cond.condition);
            let true_exp = host_convert_ast_to_ir(job, cond.true_exp);
            let false_exp = host_convert_ast_to_ir(job, cond.false_exp);

            Box::new_in(
                IrExpression::Ternary(oxc_allocator::Box::new_in(
                    crate::ir::expression::TernaryExpr {
                        condition,
                        true_expr: true_exp,
                        false_expr: false_exp,
                        source_span: Some(cond.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert PropertyRead - recursively convert receiver to preserve pipes
        AngularExpression::PropertyRead(prop) => {
            let prop = prop.unbox();

            if matches!(prop.receiver, AngularExpression::ImplicitReceiver(_)) {
                Box::new_in(
                    IrExpression::LexicalRead(oxc_allocator::Box::new_in(
                        LexicalReadExpr {
                            name: prop.name,
                            source_span: Some(prop.source_span.to_span()),
                        },
                        allocator,
                    )),
                    allocator,
                )
            } else {
                let receiver = host_convert_ast_to_ir(job, prop.receiver);
                Box::new_in(
                    IrExpression::ResolvedPropertyRead(oxc_allocator::Box::new_in(
                        ResolvedPropertyReadExpr {
                            receiver,
                            name: prop.name,
                            source_span: Some(prop.source_span.to_span()),
                        },
                        allocator,
                    )),
                    allocator,
                )
            }
        }

        // Convert KeyedRead - recursively convert receiver and key to preserve pipes
        AngularExpression::KeyedRead(keyed) => {
            let keyed = keyed.unbox();
            let receiver = host_convert_ast_to_ir(job, keyed.receiver);
            let key = host_convert_ast_to_ir(job, keyed.key);
            Box::new_in(
                IrExpression::ResolvedKeyedRead(oxc_allocator::Box::new_in(
                    ResolvedKeyedReadExpr {
                        receiver,
                        key,
                        source_span: Some(keyed.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Call - recursively convert receiver and args to preserve pipes
        AngularExpression::Call(call) => {
            let call = call.unbox();
            let receiver = host_convert_ast_to_ir(job, call.receiver);
            let mut args = Vec::with_capacity_in(call.args.len(), allocator);
            for arg in call.args {
                let arg_expr = host_convert_ast_to_ir(job, arg);
                args.push(arg_expr.unbox());
            }
            Box::new_in(
                IrExpression::ResolvedCall(oxc_allocator::Box::new_in(
                    ResolvedCallExpr {
                        receiver,
                        args,
                        source_span: Some(call.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert PrefixNot (!) - recursively convert operand to preserve pipes
        AngularExpression::PrefixNot(not) => {
            let not = not.unbox();
            let expr = host_convert_ast_to_ir(job, not.expression);
            Box::new_in(
                IrExpression::Not(oxc_allocator::Box::new_in(
                    crate::ir::expression::NotExpr {
                        expr,
                        source_span: Some(not.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert Unary (+/-) - recursively convert operand to preserve pipes
        AngularExpression::Unary(unary) => {
            let unary = unary.unbox();
            let expr = host_convert_ast_to_ir(job, unary.expr);
            let operator = match unary.operator {
                crate::ast::expression::UnaryOperator::Plus => {
                    crate::ir::expression::IrUnaryOperator::Plus
                }
                crate::ast::expression::UnaryOperator::Minus => {
                    crate::ir::expression::IrUnaryOperator::Minus
                }
            };
            Box::new_in(
                IrExpression::Unary(oxc_allocator::Box::new_in(
                    crate::ir::expression::UnaryExpr {
                        operator,
                        expr,
                        source_span: Some(unary.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert TypeofExpression - recursively convert operand to preserve pipes
        AngularExpression::TypeofExpression(typeof_expr) => {
            let typeof_expr = typeof_expr.unbox();
            let expr = host_convert_ast_to_ir(job, typeof_expr.expression);
            Box::new_in(
                IrExpression::Typeof(oxc_allocator::Box::new_in(
                    crate::ir::expression::TypeofExpr {
                        expr,
                        source_span: Some(typeof_expr.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // Convert VoidExpression - recursively convert operand to preserve pipes
        AngularExpression::VoidExpression(void_expr) => {
            let void_expr = void_expr.unbox();
            let expr = host_convert_ast_to_ir(job, void_expr.expression);
            Box::new_in(
                IrExpression::Void(oxc_allocator::Box::new_in(
                    crate::ir::expression::VoidExpr {
                        expr,
                        source_span: Some(void_expr.source_span.to_span()),
                    },
                    allocator,
                )),
                allocator,
            )
        }

        // For all other expressions, store in ExpressionStore and return reference
        other => host_store_and_ref_expr(job, other),
    }
}

/// Converts an interpolation expression to an IR interpolation for host bindings.
fn host_convert_interpolation_to_ir<'a>(
    job: &mut HostBindingCompilationJob<'a>,
    expr: AngularExpression<'a>,
) -> Box<'a, IrExpression<'a>> {
    let allocator = job.allocator;

    if let AngularExpression::Interpolation(interp_box) = expr {
        let interp = interp_box.unbox();

        let mut ir_expressions = Vec::new_in(allocator);
        for inner_expr in interp.expressions {
            let converted = host_convert_ast_to_ir(job, inner_expr);
            ir_expressions.push(converted.unbox());
        }

        Box::new_in(
            IrExpression::Interpolation(Box::new_in(
                crate::ir::expression::Interpolation {
                    strings: interp.strings,
                    expressions: ir_expressions,
                    i18n_placeholders: Vec::new_in(allocator),
                    source_span: Some(interp.source_span.to_span()),
                },
                allocator,
            )),
            allocator,
        )
    } else {
        host_convert_ast_to_ir(job, expr)
    }
}

/// Process a host binding AST and convert it into a `HostBindingCompilationJob`.
///
/// This function is the entry point for host binding ingestion, mirroring
/// Angular TypeScript's `ingestHostBinding` function in `ingest.ts`.
///
/// The `pool_starting_index` parameter is used to ensure constant names don't conflict
/// when the host binding compilation follows template compilation. In Angular TypeScript,
/// both template and host binding compilation share the same ConstantPool instance.
/// In our implementation, we achieve the same effect by passing the template pool's
/// next index to the host binding compilation.
pub fn ingest_host_binding<'a>(
    allocator: &'a Allocator,
    input: HostBindingInput<'a>,
    pool_starting_index: u32,
) -> HostBindingCompilationJob<'a> {
    let mut job = HostBindingCompilationJob::with_pool_starting_index(
        allocator,
        input.component_name,
        input.component_selector,
        pool_starting_index,
    );

    // Ingest host properties
    for property in input.properties {
        ingest_host_dom_property(&mut job, property);
    }

    // Ingest host attributes
    for (name, expr) in input.attributes {
        ingest_host_attribute(&mut job, name, expr);
    }

    // Ingest host events
    for event in input.events {
        ingest_host_event(&mut job, event);
    }

    job
}

/// Ingests a host DOM property binding.
///
/// This function processes `@HostBinding('property')` and `[hostProperty]` bindings.
/// It creates a `BindingOp` in the host binding unit's update list.
///
/// Ported from Angular's `ingestDomProperty` for host bindings in `ingest.ts`.
fn ingest_host_dom_property<'a>(
    job: &mut HostBindingCompilationJob<'a>,
    property: R3BoundAttribute<'a>,
) {
    use crate::ast::expression::BindingType;

    let allocator = job.allocator;

    // Determine binding kind, handling special prefixes
    let (binding_kind, name) = if property.name.starts_with("attr.") {
        // Handle `attr.` prefix
        let stripped = &property.name[5..];
        (BindingKind::Attribute, Atom::from(allocator.alloc_str(stripped)))
    } else {
        let kind = match property.binding_type {
            BindingType::Property => BindingKind::Property,
            BindingType::Attribute => BindingKind::Attribute,
            BindingType::Class => BindingKind::ClassName,
            BindingType::Style => BindingKind::StyleProperty,
            BindingType::TwoWay => BindingKind::TwoWayProperty,
            BindingType::Animation => BindingKind::Animation,
            BindingType::LegacyAnimation => BindingKind::LegacyAnimation,
        };
        (kind, property.name)
    };

    // Convert expression, handling interpolations
    let expression = if matches!(&property.value, AngularExpression::Interpolation(_)) {
        host_convert_interpolation_to_ir(job, property.value)
    } else {
        host_convert_ast_to_ir(job, property.value)
    };

    // Create a binding op in the update list
    let op = UpdateOp::Binding(BindingOp {
        base: UpdateOpBase { source_span: Some(property.source_span), ..Default::default() },
        target: job.root.xref,
        kind: binding_kind,
        name,
        expression,
        unit: property.unit,
        security_context: property.security_context,
        i18n_message: None,
        is_text_attribute: false,
    });

    job.root.update.push(op);
}

/// Computes the security context for an attribute binding.
///
/// This is a simplified implementation of Angular's `calcPossibleSecurityContexts`
/// that handles the most common cases based on element and property names.
///
/// Ported from Angular's `binding_parser.ts` and `dom_security_schema.ts`.
fn compute_security_context(selector: &str, attr_name: &str) -> SecurityContext {
    use crate::schema::{calc_security_context_for_unknown_element, get_security_context};

    // Extract element name from selector if present (e.g., "a[myDirective]" → "a")
    let element = extract_element_from_selector(selector);

    match element {
        Some(element_name) => {
            // Element is known - use the specific lookup
            get_security_context(&element_name, attr_name)
        }
        None => {
            // Element is unknown (e.g., attribute-only directive like [myDirective])
            // Use the ambiguous lookup that checks all possible elements
            calc_security_context_for_unknown_element(attr_name)
        }
    }
}

/// Extracts the element name from a CSS selector.
///
/// Examples:
/// - "a[myDirective]" → Some("a")
/// - "div.my-class" → Some("div")
/// - "[myDirective]" → None
/// - ".my-class" → None
fn extract_element_from_selector(selector: &str) -> Option<String> {
    // Skip leading whitespace
    let s = selector.trim();

    // If starts with [, ., or :, there's no element
    if s.starts_with('[') || s.starts_with('.') || s.starts_with(':') || s.starts_with('#') {
        return None;
    }

    // Find the element name (alphanumeric and hyphens until a special char)
    let mut element_end = 0;
    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            element_end = i + c.len_utf8();
        } else {
            break;
        }
    }

    if element_end > 0 { Some(s[..element_end].to_lowercase()) } else { None }
}

/// Ingests a static host attribute.
///
/// Host attributes are static attributes that should be extracted to `hostAttrs`
/// on the component/directive definition. They are always marked for extraction.
///
/// Ported from Angular's `ingestHostAttribute` in `ingest.ts`.
/// Uses OutputExpression directly to match TypeScript's `o.Expression` parameter.
fn ingest_host_attribute<'a>(
    job: &mut HostBindingCompilationJob<'a>,
    name: Atom<'a>,
    value: crate::output::ast::OutputExpression<'a>,
) {
    use crate::ir::expression::IrExpression;
    use crate::ir::ops::{BindingOp, UpdateOp, UpdateOpBase};

    let allocator = job.allocator;

    // Compute security context based on selector and attribute name
    let security_context = compute_security_context(job.component_selector.as_str(), name.as_str());

    // Wrap the OutputExpression in IrExpression::OutputExpr
    // This matches TypeScript which passes o.Expression directly to the IR
    let expression =
        Box::new_in(IrExpression::OutputExpr(Box::new_in(value, allocator)), allocator);

    // Create a BindingOp and add it to the UPDATE list, just like Angular's ingestHostAttribute.
    // The binding is marked as is_text_attribute: true, which means it will be extracted to
    // hostAttrs by the attribute_extraction phase.
    //
    // Angular's ingestHostAttribute (ingest.ts lines 178-200) does:
    //   const attrBinding = ir.createBindingOp(..., /* isTextAttribute */ true, ...);
    //   job.root.update.push(attrBinding);
    let binding_op = UpdateOp::Binding(BindingOp {
        base: UpdateOpBase { source_span: None, ..Default::default() },
        target: job.root.xref,
        kind: BindingKind::Attribute,
        name,
        expression,
        unit: None,
        security_context,
        i18n_message: None,
        // Host attributes should always be extracted to const hostAttrs, even if they are not
        // strictly text literals (see Angular's comment in ingest.ts line 191-192)
        is_text_attribute: true,
    });

    job.root.update.push(binding_op);
}

/// Ingests a host event binding.
///
/// This function processes `@HostListener('event')` and `(hostEvent)` bindings.
/// It creates a `ListenerOp` in the host binding unit's create list.
///
/// Ported from Angular's `ingestHostEvent` in `ingest.ts`.
/// The handler uses `makeListenerHandlerOps` which handles Chain expressions
/// (multiple statements separated by semicolons) by converting non-last
/// statements to ExpressionStatement ops and the last statement to the return.
fn ingest_host_event<'a>(job: &mut HostBindingCompilationJob<'a>, event: R3BoundEvent<'a>) {
    use crate::ast::expression::ParsedEventType;
    use crate::ir::enums::AnimationKind;

    let allocator = job.allocator;

    // Extract expressions from Chain if present, otherwise wrap single expression in a vec.
    // Ported from Angular's makeListenerHandlerOps:
    // let handlerExprs: e.AST[] = handler instanceof e.Chain ? handler.expressions : [handler];
    let handler_exprs: std::vec::Vec<AngularExpression<'a>> =
        if let AngularExpression::Chain(chain) = event.handler {
            // Unbox the Chain to take ownership of the expressions
            let chain = chain.unbox();
            chain.expressions.into_iter().collect()
        } else {
            vec![event.handler]
        };

    // Handle Chain expressions (sequence of statements).
    // Ported from Angular's makeListenerHandlerOps in ingest.ts:
    // - All expressions except the last become ExpressionStatement ops in handler_ops
    // - The last expression becomes the handler_expression (wrapped in return)
    let mut handler_ops = Vec::new_in(allocator);
    let mut handler_expr: Option<oxc_allocator::Box<'a, IrExpression<'a>>> = None;

    let exprs_count = handler_exprs.len();
    for (i, expr) in handler_exprs.into_iter().enumerate() {
        let ir_expr = host_convert_ast_to_ir(job, expr);

        if i == exprs_count - 1 {
            // Last expression becomes handler_expression
            handler_expr = Some(ir_expr);
        } else {
            // Non-last expressions become ExpressionStatement ops
            let expr_stmt =
                crate::output::ast::OutputStatement::Expression(oxc_allocator::Box::new_in(
                    crate::output::ast::ExpressionStatement {
                        expr: OutputExpression::WrappedIrNode(oxc_allocator::Box::new_in(
                            crate::output::ast::WrappedIrExpr {
                                node: ir_expr,
                                source_span: Some(event.source_span),
                            },
                            allocator,
                        )),
                        source_span: Some(event.source_span),
                    },
                    allocator,
                ));
            handler_ops.push(UpdateOp::Statement(StatementOp {
                base: UpdateOpBase::default(),
                statement: expr_stmt,
            }));
        }
    }

    // Determine event target and animation phase based on event type
    let (animation_phase, target) = match event.event_type {
        ParsedEventType::LegacyAnimation => {
            // Convert phase string to AnimationKind
            let phase = event.phase.as_ref().and_then(|p| match p.as_str() {
                "start" => Some(AnimationKind::Enter),
                "done" => Some(AnimationKind::Leave),
                _ => None,
            });
            (phase, None)
        }
        _ => (None, event.target.clone()),
    };

    // Check if this is an animation event
    let is_animation = matches!(event.event_type, ParsedEventType::Animation);

    let op = CreateOp::Listener(ListenerOp {
        base: CreateOpBase { source_span: Some(event.source_span), ..Default::default() },
        target: job.root.xref,
        target_slot: SlotId(0), // Will be set during slot allocation
        tag: None,              // Host bindings don't have an element tag
        host_listener: true,
        name: event.name,
        handler_expression: handler_expr,
        handler_ops,
        handler_fn_name: None,
        consume_fn_name: None,
        is_animation_listener: is_animation,
        animation_phase,
        event_target: target,
        consumes_dollar_event: false, // Set during resolve_dollar_event phase
    });

    job.root.create.push(op);
}

/// Converts an i18n metadata from R3 AST to IR I18nPlaceholder.
///
/// Angular's ingestIfBlock and ingestSwitchBlock check that branch/case i18n metadata
/// is specifically a BlockPlaceholder type and extract its start_name/close_name.
///
/// Ported from Angular's i18n handling in `ingest.ts` (lines 531-537, 1088-1094).
/// Returns `Ok(Some(placeholder))` for valid BlockPlaceholder metadata,
/// `Ok(None)` when no i18n metadata is present, or `Err(())` when an
/// unexpected metadata type is encountered (matching Angular's throw behavior).
fn convert_i18n_meta_to_placeholder<'a>(
    i18n: Option<I18nMeta<'a>>,
    diagnostics: &mut std::vec::Vec<OxcDiagnostic>,
    source_span: oxc_span::Span,
    block_name: &str,
) -> Result<Option<I18nPlaceholder<'a>>, ()> {
    match i18n {
        Some(I18nMeta::Node(I18nNode::BlockPlaceholder(bp))) => {
            Ok(Some(I18nPlaceholder::new(bp.start_name, Some(bp.close_name))))
        }
        Some(I18nMeta::BlockPlaceholder(bp)) => {
            Ok(Some(I18nPlaceholder::new(bp.start_name, Some(bp.close_name))))
        }
        // Reference: ingest.ts lines 533-537, 587-591
        // Angular throws an assertion error for unexpected i18n metadata types.
        // We report a diagnostic and return Err to stop ingestion of this block,
        // matching Angular's fail-fast behavior.
        Some(_) => {
            diagnostics.push(
                OxcDiagnostic::error(format!("Unhandled i18n metadata type for {block_name}"))
                    .with_label(source_span),
            );
            Err(())
        }
        None => Ok(None),
    }
}

/// Animation attribute prefix that should be skipped for content projection.
const ANIMATE_PREFIX: &str = "animate.";

/// The ng-template tag name - should not be passed along for directive matching.
const NG_TEMPLATE_TAG_NAME: &str = "ng-template";

/// Infers tag name and attributes for content projection from control flow blocks.
///
/// When converting from `*ngIf` to `@if`, content projection behavior changes because the
/// conditional is placed *around* elements rather than *on* them. This function aims to
/// preserve the old behavior by copying the tag name and attributes from a single root
/// element to the surrounding template.
///
/// Returns the tag name to use for the control flow template, or None if:
/// - There are multiple root nodes (excluding comments and @let)
/// - The single root is not an element or template
/// - The tag name is "ng-template"
///
/// Ported from Angular's `ingestControlFlowInsertionPoint` in `ingest.ts` (lines 1834-1918).
fn ingest_control_flow_insertion_point<'a, 'b>(
    job: &mut ComponentCompilationJob<'a>,
    parent_xref: XrefId,
    xref: XrefId,
    children: &'b [R3Node<'a>],
) -> Option<Atom<'a>> {
    // Find the single root element or template
    let mut root: Option<RootNodeRef<'a, 'b>> = None;

    for child in children {
        // Skip over comment nodes and @let declarations since
        // it doesn't matter where they end up in the DOM.
        // NOTE: TypeScript does NOT skip whitespace-only text nodes here,
        // so we must not skip them either to match the behavior.
        match child {
            R3Node::Comment(_) | R3Node::LetDeclaration(_) => continue,
            R3Node::Element(elem) => {
                // We can only infer the tag name/attributes if there's a single root node.
                if root.is_some() {
                    return None;
                }
                root = Some(RootNodeRef::Element(elem));
            }
            R3Node::Template(tmpl) if tmpl.tag_name.is_some() => {
                // Templates with a tag name (e.g., `<div *foo></div>`)
                if root.is_some() {
                    return None;
                }
                root = Some(RootNodeRef::Template(tmpl));
            }
            _ => {
                // Any other node type means we can't infer
                return None;
            }
        }
    }

    // If we've found a single root node, its tag name and attributes can be
    // copied to the surrounding template to be used for content projection.
    let root = root?;
    let allocator = job.allocator;

    // Collect static attributes for content projection purposes.
    let attributes = root.attributes();
    for attr in attributes {
        let attr_name = attr.name.as_str();
        // Skip animation attributes
        if attr_name.starts_with(ANIMATE_PREFIX) {
            continue;
        }

        let security_context = crate::schema::get_security_context(NG_TEMPLATE_TAG_NAME, attr_name);
        let value_expr = create_string_literal_atom(allocator, attr.value.clone());

        // Handle i18n message if present (for i18n-* attribute markers)
        // This matches Angular's asMessage(attr.i18n) in ingest.ts line 1879
        //
        // Angular TS stores the i18n.Message object reference directly. We store the
        // instance_id as a dedup key. When the SAME attribute is encountered twice
        // (once for the conditional via ingestControlFlowInsertionPoint, once for the
        // element via ingestStaticAttributes), they share the same instance_id since
        // it's assigned during parsing and survives moves/copies.
        //
        // Different attributes (even with the same content) get DIFFERENT instance_ids,
        // which is crucial for correct const deduplication.
        let i18n_message = if let Some(I18nMeta::Message(ref message)) = attr.i18n {
            let instance_id = message.instance_id;

            // Store i18n message metadata for later phases (only if not already stored)
            if !job.i18n_message_metadata.contains_key(&instance_id) {
                let mut legacy_ids = Vec::new_in(allocator);
                for id in message.legacy_ids.iter() {
                    legacy_ids.push(id.clone());
                }

                let metadata = I18nMessageMetadata {
                    message_id: if message.id.is_empty() { None } else { Some(message.id.clone()) },
                    custom_id: if message.custom_id.is_empty() {
                        None
                    } else {
                        Some(message.custom_id.clone())
                    },
                    meaning: if message.meaning.is_empty() {
                        None
                    } else {
                        Some(message.meaning.clone())
                    },
                    description: if message.description.is_empty() {
                        None
                    } else {
                        Some(message.description.clone())
                    },
                    legacy_ids,
                    message_string: if message.message_string.is_empty() {
                        None
                    } else {
                        Some(message.message_string.clone())
                    },
                };
                job.i18n_message_metadata.insert(instance_id, metadata);
            }

            Some(instance_id)
        } else {
            None
        };

        let binding_op = UpdateOp::Binding(BindingOp {
            base: UpdateOpBase { source_span: Some(attr.source_span), ..Default::default() },
            target: xref,
            kind: BindingKind::Attribute,
            name: attr.name.clone(),
            expression: Box::new_in(value_expr, allocator),
            unit: None,
            security_context,
            i18n_message,
            is_text_attribute: true, // Static attributes are text attributes
        });

        // Add to PARENT view's update list (not the embedded view).
        // The target is still the embedded view's xref, but the op belongs to the parent.
        if let Some(view) = job.view_mut(parent_xref) {
            view.update.push(binding_op);
        }
    }

    // Also collect the inputs since they participate in content projection as well.
    // Note: TDB used to collect outputs but didn't pass them to the template instruction.
    let inputs = root.inputs();
    for input in inputs {
        // Skip animation, legacy animation, and attribute bindings
        // (matching TypeScript's ingestControlFlowInsertionPoint behavior)
        let binding_type = input.binding_type;
        if binding_type == crate::ast::expression::BindingType::Animation
            || binding_type == crate::ast::expression::BindingType::LegacyAnimation
            || binding_type == crate::ast::expression::BindingType::Attribute
        {
            continue;
        }

        let security_context =
            crate::schema::get_security_context(NG_TEMPLATE_TAG_NAME, &input.name);

        let extracted_attr_op = CreateOp::ExtractedAttribute(ExtractedAttributeOp {
            base: CreateOpBase { source_span: Some(input.source_span), ..Default::default() },
            target: xref,
            binding_kind: BindingKind::Property,
            namespace: None,
            name: input.name.clone(),
            value: None,
            security_context,
            truthy_expression: false,
            i18n_context: None,
            i18n_message: None,
            trusted_value_fn: None,
        });

        // Add to PARENT view's create list (not the embedded view).
        if let Some(view) = job.view_mut(parent_xref) {
            view.create.push(extracted_attr_op);
        }
    }

    // Get the tag name
    let tag_name = root.tag_name();

    // Don't pass along `ng-template` tag name since it enables directive matching.
    if tag_name.as_str() == NG_TEMPLATE_TAG_NAME { None } else { Some(tag_name) }
}

/// Reference to either an R3Element or R3Template for the root node.
enum RootNodeRef<'a, 'b> {
    Element(&'b R3Element<'a>),
    Template(&'b R3Template<'a>),
}

impl<'a, 'b> RootNodeRef<'a, 'b> {
    fn attributes(&self) -> &[R3TextAttribute<'a>] {
        match self {
            RootNodeRef::Element(elem) => &elem.attributes,
            RootNodeRef::Template(tmpl) => &tmpl.attributes,
        }
    }

    fn inputs(&self) -> &[R3BoundAttribute<'a>] {
        match self {
            RootNodeRef::Element(elem) => &elem.inputs,
            RootNodeRef::Template(tmpl) => &tmpl.inputs,
        }
    }

    fn tag_name(&self) -> Atom<'a> {
        match self {
            RootNodeRef::Element(elem) => elem.name.clone(),
            RootNodeRef::Template(tmpl) => {
                // Template should have a tag_name since we checked for it
                tmpl.tag_name.clone().unwrap_or_else(|| Atom::from(""))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::r3::I18nMessage;
    use oxc_allocator::Allocator;

    /// Issue #1: convert_i18n_meta_to_placeholder should return Err for unexpected
    /// i18n metadata types, matching Angular's throw behavior.
    /// Angular reference: ingest.ts lines 533-537, 587-591, 970-974
    #[test]
    fn convert_i18n_meta_to_placeholder_returns_err_for_unexpected_type() {
        let allocator = Allocator::default();
        let mut diagnostics = std::vec::Vec::new();

        // Create an unexpected i18n metadata type (Message instead of BlockPlaceholder).
        // Control flow blocks should only have BlockPlaceholder metadata.
        let unexpected_i18n = I18nMeta::Message(I18nMessage {
            instance_id: 0,
            nodes: Vec::new_in(&allocator),
            meaning: Atom::from(""),
            description: Atom::from(""),
            custom_id: Atom::from(""),
            id: Atom::from(""),
            legacy_ids: Vec::new_in(&allocator),
            message_string: Atom::from(""),
        });

        let result = convert_i18n_meta_to_placeholder(
            Some(unexpected_i18n),
            &mut diagnostics,
            oxc_span::SPAN,
            "@for",
        );

        assert!(result.is_err(), "Should return Err for unexpected i18n metadata type");
        assert_eq!(diagnostics.len(), 1, "Should push exactly one diagnostic");
        assert!(
            diagnostics[0].message.contains("Unhandled i18n metadata type for @for"),
            "Diagnostic message should name the specific block type, got: {}",
            diagnostics[0].message,
        );
    }

    /// Issue #1: convert_i18n_meta_to_placeholder should return Ok(None) when
    /// no i18n metadata is present.
    #[test]
    fn convert_i18n_meta_to_placeholder_returns_none_for_absent_metadata() {
        let mut diagnostics = std::vec::Vec::new();

        let result =
            convert_i18n_meta_to_placeholder(None, &mut diagnostics, oxc_span::SPAN, "@if");

        assert!(result.is_ok(), "Should return Ok for absent metadata");
        assert!(result.unwrap().is_none(), "Should return None when no i18n metadata");
        assert!(diagnostics.is_empty(), "Should not push any diagnostics");
    }

    /// Issue #2: get_computed_for_loop_variable_expression should return Err for
    /// unknown loop variables, matching Angular's AssertionError throw.
    /// Angular reference: ingest.ts lines 1043-1044
    #[test]
    fn get_computed_for_loop_variable_expression_returns_err_for_unknown_var() {
        let allocator = Allocator::default();
        let mut diagnostics = std::vec::Vec::new();
        let index_name = Atom::from("ɵ$index_0");
        let count_name = Atom::from("ɵ$count_0");

        let result = get_computed_for_loop_variable_expression(
            &allocator,
            "$unknown",
            &index_name,
            &count_name,
            &mut diagnostics,
        );

        assert!(result.is_err(), "Should return Err for unknown loop variable");
        assert_eq!(diagnostics.len(), 1, "Should push exactly one diagnostic");
        assert!(
            diagnostics[0].message.contains("unknown @for loop variable $unknown"),
            "Diagnostic should name the unknown variable"
        );
    }

    /// Issue #2: get_computed_for_loop_variable_expression should return Ok for
    /// all known loop variables ($index, $count, $first, $last, $even, $odd).
    #[test]
    fn get_computed_for_loop_variable_expression_returns_ok_for_known_vars() {
        let allocator = Allocator::default();
        let index_name = Atom::from("ɵ$index_0");
        let count_name = Atom::from("ɵ$count_0");

        for var in &["$index", "$count", "$first", "$last", "$even", "$odd"] {
            let mut diagnostics = std::vec::Vec::new();
            let result = get_computed_for_loop_variable_expression(
                &allocator,
                var,
                &index_name,
                &count_name,
                &mut diagnostics,
            );

            assert!(result.is_ok(), "Should return Ok for known variable {var}");
            assert!(diagnostics.is_empty(), "Should not push diagnostics for {var}");
        }
    }
}

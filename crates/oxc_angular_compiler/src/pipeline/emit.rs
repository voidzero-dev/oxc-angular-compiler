//! Template emission and pipeline orchestration.
//!
//! This module provides the entry point for compiling Angular templates
//! and emitting JavaScript code.
//!
//! Ported from Angular's `template/pipeline/src/emit.ts`.

use oxc_allocator::{Allocator, Vec as OxcVec};

use crate::ir::expression::IrExpression;
use crate::output::ast::{
    BinaryOperator, BinaryOperatorExpr, DeclareFunctionStmt, FnParam, FunctionExpr, IfStmt,
    LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadVarExpr, StmtModifier,
    clone_output_statement,
};
use crate::r3::{Identifiers, get_interpolate_instruction, get_pipe_bind_instruction};
use oxc_allocator::Box;
use oxc_span::Atom;

use crate::ir::ops::XrefId;

use super::compilation::{ComponentCompilationJob, HostBindingCompilationJob, ViewCompilationUnit};
use super::phases;

// ============================================================================
// Transform
// ============================================================================

/// Run all transformation phases for template compilation.
///
/// After this function completes, the compilation job is ready for emission.
pub fn transform(job: &mut ComponentCompilationJob<'_>) {
    phases::transform_template(job);
}

/// Run all transformation phases for host binding compilation.
///
/// Takes a HostBindingCompilationJob and runs only the phases applicable to host bindings.
pub fn transform_host(job: &mut HostBindingCompilationJob<'_>) {
    phases::transform_host_job(job);
}

// ============================================================================
// Emit
// ============================================================================

/// Compile all views in the given component compilation job into the final
/// template function.
///
/// Returns the root template function expression. Child views are added to
/// the job's constant pool as declarations.
pub fn emit_template_fn<'a>(job: &mut ComponentCompilationJob<'a>) -> FunctionExpr<'a> {
    let root_fn = emit_view(job.allocator, &job.root);

    // Emit child views depth-first, adding them to job.pool.statements
    emit_child_views_to_job(job);

    root_fn
}

/// Emit child views to job.pool.statements.
///
/// This function collects view xrefs first to avoid borrowing issues,
/// then emits each child view depth-first.
fn emit_child_views_to_job<'a>(job: &mut ComponentCompilationJob<'a>) {
    // Collect all view xrefs to avoid borrow checker issues
    let view_xrefs: std::vec::Vec<_> = job.views.keys().copied().collect();

    // Process views in order, emitting children before parents
    emit_child_views_recursive(job, job.root.xref, &view_xrefs);
}

/// Recursively emit child views for a given parent.
fn emit_child_views_recursive<'a>(
    job: &mut ComponentCompilationJob<'a>,
    parent_xref: XrefId,
    all_xrefs: &[XrefId],
) {
    // Find children of this parent
    let children: std::vec::Vec<_> = all_xrefs
        .iter()
        .filter(|&&xref| {
            job.views.get(&xref).map(|unit| unit.parent == Some(parent_xref)).unwrap_or(false)
        })
        .copied()
        .collect();

    // Process each child depth-first
    for child_xref in children {
        // First, emit this child's children
        emit_child_views_recursive(job, child_xref, all_xrefs);

        // Then emit this child view
        if let Some(unit) = job.views.get(&child_xref) {
            let view_fn = emit_view(job.allocator, unit);

            if let Some(fn_name) = view_fn.name.clone() {
                let fn_stmt = OutputStatement::DeclareFunction(Box::new_in(
                    DeclareFunctionStmt {
                        name: fn_name,
                        params: clone_params(job.allocator, &view_fn.params),
                        statements: clone_statements(job.allocator, &view_fn.statements),
                        modifiers: StmtModifier::NONE,
                        source_span: None,
                    },
                    job.allocator,
                ));
                job.pool.statements.push(fn_stmt);
            }
        }
    }
}

/// Emit a template function for an individual view.
///
/// After the reify phase, view.create_statements and view.update_statements
/// contain the JavaScript statements for the template function.
fn emit_view<'a>(allocator: &'a Allocator, view: &ViewCompilationUnit<'a>) -> FunctionExpr<'a> {
    let fn_name = view.fn_name.clone();

    // Clone the statements from the view (reify phase has populated these)
    let create_statements = clone_statements(allocator, &view.create_statements);
    let update_statements = clone_statements(allocator, &view.update_statements);

    // Generate rf block conditions
    // Angular templates use a render flag (rf) to distinguish between create and update phases:
    // - rf & 1: Creation phase (runs once when component is created)
    // - rf & 2: Update phase (runs on each change detection cycle)
    let create_block = maybe_generate_rf_block(allocator, 1, create_statements);
    let update_block = maybe_generate_rf_block(allocator, 2, update_statements);

    // Combine into function body
    let mut body: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);
    body.extend(create_block);
    body.extend(update_block);

    // Create function parameters: (rf, ctx)
    // - rf: Render flags (1 = create, 2 = update)
    // - ctx: Component context (this)
    let mut params: OxcVec<'a, FnParam<'a>> = OxcVec::new_in(allocator);
    params.push(FnParam { name: Atom::from("rf") });
    params.push(FnParam { name: Atom::from("ctx") });

    FunctionExpr { name: fn_name, params, statements: body, source_span: None }
}

/// Clone a vec of statements for use in emission.
fn clone_statements<'a>(
    allocator: &'a Allocator,
    statements: &OxcVec<'a, OutputStatement<'a>>,
) -> OxcVec<'a, OutputStatement<'a>> {
    let mut result = OxcVec::new_in(allocator);
    for stmt in statements.iter() {
        result.push(clone_output_statement(stmt, allocator));
    }
    result
}

/// Clone function parameters.
fn clone_params<'a>(
    allocator: &'a Allocator,
    params: &OxcVec<'a, FnParam<'a>>,
) -> OxcVec<'a, FnParam<'a>> {
    let mut result = OxcVec::new_in(allocator);
    for param in params.iter() {
        result.push(FnParam { name: param.name.clone() });
    }
    result
}

/// Generate an if block for the given render flag.
///
/// Returns an empty vec if there are no statements.
fn maybe_generate_rf_block<'a>(
    allocator: &'a Allocator,
    flag: i32,
    statements: OxcVec<'a, OutputStatement<'a>>,
) -> OxcVec<'a, OutputStatement<'a>> {
    if statements.is_empty() {
        return OxcVec::new_in(allocator);
    }

    // Create condition: rf & flag
    let condition = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::BitwiseAnd,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Atom::from("rf"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(flag as f64), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            source_span: None,
        },
        allocator,
    ));

    // Create if statement
    let if_stmt = OutputStatement::If(Box::new_in(
        IfStmt {
            condition,
            true_case: statements,
            false_case: OxcVec::new_in(allocator),
            source_span: None,
        },
        allocator,
    ));

    let mut result: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);
    result.push(if_stmt);
    result
}

/// Emit a host binding function from HostBindingCompilationJob.
///
/// Returns None if there are no host bindings.
///
/// Ported from Angular's `emitHostBindingFunction` in `emit.ts`.
pub fn emit_host_binding_function<'a>(
    job: &HostBindingCompilationJob<'a>,
) -> Option<FunctionExpr<'a>> {
    let allocator = job.allocator;
    let unit = &job.root;

    // Clone the statements from the unit (reify phase has populated these)
    let create_statements = clone_statements(allocator, &unit.create_statements);
    let update_statements = clone_statements(allocator, &unit.update_statements);

    // Return None if there are no statements
    if create_statements.is_empty() && update_statements.is_empty() {
        return None;
    }

    // Generate rf block conditions
    let create_block = maybe_generate_rf_block(allocator, 1, create_statements);
    let update_block = maybe_generate_rf_block(allocator, 2, update_statements);

    // Combine into function body
    let mut body: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);
    body.extend(create_block);
    body.extend(update_block);

    // Create function parameters: (rf, ctx)
    let mut params: OxcVec<'a, FnParam<'a>> = OxcVec::new_in(allocator);
    params.push(FnParam { name: Atom::from("rf") });
    params.push(FnParam { name: Atom::from("ctx") });

    Some(FunctionExpr { name: unit.fn_name.clone(), params, statements: body, source_span: None })
}

// ============================================================================
// Full Compilation Pipeline
// ============================================================================

/// Result of compiling an Angular template.
pub struct TemplateCompilationResult<'a> {
    /// The main template function.
    pub template_fn: FunctionExpr<'a>,
    /// Additional declarations (child view functions, constants).
    pub declarations: OxcVec<'a, OutputStatement<'a>>,
}

/// Compile an Angular template from start to finish.
///
/// This is the main entry point for template compilation:
/// 1. Runs all transformation phases
/// 2. Emits the template function
/// 3. Collects all declarations
pub fn compile_template<'a>(
    job: &mut ComponentCompilationJob<'a>,
) -> TemplateCompilationResult<'a> {
    use crate::output::ast::DeclareVarStmt;

    let allocator = job.allocator;

    // Run all transformation phases (populates job.pool with constants)
    transform(job);

    // Emit the template function (adds child views to job.pool.statements)
    let template_fn = emit_template_fn(job);

    // Collect declarations from job.pool constants and statements
    let mut declarations = OxcVec::new_in(allocator);

    // Generate const declarations from pooled constants
    // Use mutable access to allow taking ownership of expressions that can't be cloned
    for constant in job.pool.constants_mut() {
        let value = emit_pooled_constant_value(allocator, &mut constant.kind);

        declarations.push(OutputStatement::DeclareVar(Box::new_in(
            DeclareVarStmt {
                name: constant.name.clone(),
                value: Some(value),
                modifiers: StmtModifier::FINAL, // TypeScript uses const for constant pool entries
                leading_comment: None,
                source_span: None,
            },
            allocator,
        )));
    }

    // Add child view function declarations from job.pool
    for stmt in job.pool.statements.iter() {
        declarations.push(clone_output_statement(stmt, allocator));
    }

    TemplateCompilationResult { template_fn, declarations }
}

/// Emit pool constants that were added after `compile_template`.
///
/// This is used to emit constants that are added during definition generation
/// (e.g., attrs array from selectors), which happens after `compile_template`
/// drains the pool to `declarations`.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `job` - The compilation job with the constant pool
/// * `start_index` - Index of the first constant to emit (constants before this are skipped)
///
/// # Returns
///
/// A vector of output statements for the new constants.
pub fn emit_additional_pool_constants<'a>(
    allocator: &'a Allocator,
    job: &mut ComponentCompilationJob<'a>,
    start_index: usize,
) -> OxcVec<'a, OutputStatement<'a>> {
    use crate::output::ast::DeclareVarStmt;

    let mut declarations = OxcVec::new_in(allocator);

    // Only emit constants starting from start_index
    let constants = job.pool.constants_mut();
    for constant in constants.iter_mut().skip(start_index) {
        let value = emit_pooled_constant_value(allocator, &mut constant.kind);

        declarations.push(OutputStatement::DeclareVar(Box::new_in(
            DeclareVarStmt {
                name: constant.name.clone(),
                value: Some(value),
                modifiers: StmtModifier::FINAL,
                leading_comment: None,
                source_span: None,
            },
            allocator,
        )));
    }

    declarations
}

/// Converts a pure function body expression to an output expression.
///
/// This function handles `PureFunctionParameterExpr` by converting them to
/// variable references (a0, a1, etc.) that match the function parameters.
/// Other expression types are recursively converted.
///
/// Ported from Angular's transformation in `pure_function_extraction.ts`.
fn convert_pure_function_body<'a>(
    allocator: &'a Allocator,
    expr: &IrExpression<'a>,
    params: &[Atom<'a>],
) -> OutputExpression<'a> {
    use crate::ir::expression::*;
    use crate::output::ast::*;

    match expr {
        // Core case: convert parameter references to variable references
        IrExpression::PureFunctionParameter(pfp) => {
            let param_name = if (pfp.index as usize) < params.len() {
                params[pfp.index as usize].clone()
            } else {
                // Fallback if index out of range
                Atom::from(allocator.alloc_str(&format!("a{}", pfp.index)))
            };
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: param_name, source_span: None },
                allocator,
            ))
        }

        // Lexical reads: special case for "ctx" (from resolve_contexts), otherwise ctx.property
        IrExpression::LexicalRead(lexical) => {
            if lexical.name.as_str() == "ctx" {
                // Direct reference to context parameter - emit as just `ctx`
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Atom::from("ctx"), source_span: None },
                    allocator,
                ))
            } else {
                // Property access on ctx - emit as `ctx.property`
                OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(
                            OutputExpression::ReadVar(Box::new_in(
                                ReadVarExpr { name: Atom::from("ctx"), source_span: None },
                                allocator,
                            )),
                            allocator,
                        ),
                        name: lexical.name.clone(),
                        optional: false,
                        source_span: None,
                    },
                    allocator,
                ))
            }
        }

        // Context reference becomes ctx
        IrExpression::Context(_) => OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("ctx"), source_span: None },
            allocator,
        )),

        // TrackContext reference becomes this (for track functions)
        IrExpression::TrackContext(_) => OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("this"), source_span: None },
            allocator,
        )),

        // Read variable with resolved name
        IrExpression::ReadVariable(var) => {
            let var_name = var
                .name
                .clone()
                .unwrap_or_else(|| Atom::from(allocator.alloc_str(&format!("_v{}", var.xref.0))));
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: var_name, source_span: None },
                allocator,
            ))
        }

        // Read temporary variable
        IrExpression::ReadTemporary(tmp) => {
            let var_name = tmp.name.clone().unwrap_or_else(|| Atom::from("_tmp"));
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: var_name, source_span: None },
                allocator,
            ))
        }

        // Assign temporary variable: _tmp = expr
        IrExpression::AssignTemporary(assign) => {
            let var_name = assign.name.clone().unwrap_or_else(|| Atom::from("_tmp"));
            let value = convert_pure_function_body(allocator, &assign.expr, params);
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: var_name, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    rhs: Box::new_in(value, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Empty becomes undefined
        IrExpression::Empty(_) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
            allocator,
        )),

        // Binary expression
        IrExpression::Binary(binary) => {
            let lhs = convert_pure_function_body(allocator, &binary.lhs, params);
            let rhs = convert_pure_function_body(allocator, &binary.rhs, params);
            let operator = convert_ir_binary_operator(binary.operator);
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(lhs, allocator),
                    rhs: Box::new_in(rhs, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Interpolation - emit as interpolate call
        IrExpression::Interpolation(interp) => {
            let expr_count = interp.expressions.len();
            let mut args = OxcVec::new_in(allocator);

            // Build args: [s0, v0, s1, v1, s2, ...] (strings and expressions interleaved)
            for (i, ir_expr) in interp.expressions.iter().enumerate() {
                if i < interp.strings.len() {
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::String(interp.strings[i].clone()),
                            source_span: None,
                        },
                        allocator,
                    )));
                }
                args.push(convert_pure_function_body(allocator, ir_expr, params));
            }
            // Add trailing string if present
            if interp.strings.len() > interp.expressions.len() {
                if let Some(trailing) = interp.strings.last() {
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::String(trailing.clone()),
                            source_span: None,
                        },
                        allocator,
                    )));
                }
            }

            // Choose instruction based on expression count
            let instruction = get_interpolate_instruction(expr_count);

            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::External(Box::new_in(
                            ExternalExpr {
                                value: ExternalReference {
                                    module_name: Some(Atom::from("@angular/core")),
                                    name: Some(Atom::from(allocator.alloc_str(instruction))),
                                },
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args,
                    pure: true,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Slot literal - emit as number
        IrExpression::SlotLiteral(slot_lit) => {
            if let Some(slot) = slot_lit.slot.slot {
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(slot.0 as f64), source_span: None },
                    allocator,
                ))
            } else {
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Null, source_span: None },
                    allocator,
                ))
            }
        }

        // Const reference - emit as number (index into const array)
        IrExpression::ConstReference(cr) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(cr.index as f64), source_span: None },
            allocator,
        )),

        // Pipe binding - convert recursively
        IrExpression::PipeBinding(pb) => {
            // ɵɵpipeBind{n}(slot, varOffset, pipeRef, ...args)
            let mut call_args = OxcVec::new_in(allocator);
            call_args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(pb.target_slot.slot.map_or(0.0, |s| s.0 as f64)),
                    source_span: None,
                },
                allocator,
            )));
            call_args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(pb.var_offset.map_or(0.0, |v| v as f64)),
                    source_span: None,
                },
                allocator,
            )));
            for arg in pb.args.iter() {
                call_args.push(convert_pure_function_body(allocator, arg, params));
            }
            let fn_name = get_pipe_bind_instruction(pb.args.len());
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::External(Box::new_in(
                            ExternalExpr {
                                value: ExternalReference {
                                    module_name: Some(Atom::from("@angular/core")),
                                    name: Some(Atom::from(fn_name)),
                                },
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args: call_args,
                    pure: true,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Variadic pipe binding
        IrExpression::PipeBindingVariadic(pb) => {
            let mut call_args = OxcVec::new_in(allocator);
            call_args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(pb.target_slot.slot.map_or(0.0, |s| s.0 as f64)),
                    source_span: None,
                },
                allocator,
            )));
            call_args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(pb.var_offset.map_or(0.0, |v| v as f64)),
                    source_span: None,
                },
                allocator,
            )));
            call_args.push(convert_pure_function_body(allocator, &pb.args, params));

            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::External(Box::new_in(
                            ExternalExpr {
                                value: ExternalReference {
                                    module_name: Some(Atom::from("@angular/core")),
                                    name: Some(Atom::from(Identifiers::PIPE_BIND_V)),
                                },
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args: call_args,
                    pure: true,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Resolved property read
        IrExpression::ResolvedPropertyRead(resolved) => {
            let receiver = convert_pure_function_body(allocator, &resolved.receiver, params);
            OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(receiver, allocator),
                    name: resolved.name.clone(),
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Resolved binary expression
        IrExpression::ResolvedBinary(resolved) => {
            let left = convert_pure_function_body(allocator, &resolved.left, params);
            let right = convert_pure_function_body(allocator, &resolved.right, params);
            use crate::ast::expression::BinaryOperator as AngularOp;
            let operator = match resolved.operator {
                AngularOp::Add => BinaryOperator::Plus,
                AngularOp::Subtract => BinaryOperator::Minus,
                AngularOp::Multiply => BinaryOperator::Multiply,
                AngularOp::Divide => BinaryOperator::Divide,
                AngularOp::Modulo => BinaryOperator::Modulo,
                AngularOp::Power => BinaryOperator::Exponentiation,
                AngularOp::Equal => BinaryOperator::Equals,
                AngularOp::NotEqual => BinaryOperator::NotEquals,
                AngularOp::StrictEqual => BinaryOperator::Identical,
                AngularOp::StrictNotEqual => BinaryOperator::NotIdentical,
                AngularOp::LessThan => BinaryOperator::Lower,
                AngularOp::LessThanOrEqual => BinaryOperator::LowerEquals,
                AngularOp::GreaterThan => BinaryOperator::Bigger,
                AngularOp::GreaterThanOrEqual => BinaryOperator::BiggerEquals,
                AngularOp::In => BinaryOperator::In,
                AngularOp::Instanceof => BinaryOperator::Instanceof,
                AngularOp::And => BinaryOperator::And,
                AngularOp::Or => BinaryOperator::Or,
                AngularOp::NullishCoalescing => BinaryOperator::NullishCoalesce,
                AngularOp::Assign => BinaryOperator::Assign,
                AngularOp::AddAssign => BinaryOperator::AdditionAssignment,
                AngularOp::SubtractAssign => BinaryOperator::SubtractionAssignment,
                AngularOp::MultiplyAssign => BinaryOperator::MultiplicationAssignment,
                AngularOp::DivideAssign => BinaryOperator::DivisionAssignment,
                AngularOp::ModuloAssign => BinaryOperator::RemainderAssignment,
                AngularOp::PowerAssign => BinaryOperator::ExponentiationAssignment,
                AngularOp::AndAssign => BinaryOperator::AndAssignment,
                AngularOp::OrAssign => BinaryOperator::OrAssignment,
                AngularOp::NullishCoalescingAssign => BinaryOperator::NullishCoalesceAssignment,
            };
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator,
                    lhs: Box::new_in(left, allocator),
                    rhs: Box::new_in(right, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Resolved call expression
        IrExpression::ResolvedCall(resolved) => {
            let receiver = convert_pure_function_body(allocator, &resolved.receiver, params);
            let mut args = OxcVec::new_in(allocator);
            for arg in resolved.args.iter() {
                args.push(convert_pure_function_body(allocator, arg, params));
            }
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(receiver, allocator),
                    args,
                    pure: false,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Resolved keyed read
        IrExpression::ResolvedKeyedRead(resolved) => {
            let receiver = convert_pure_function_body(allocator, &resolved.receiver, params);
            let key = convert_pure_function_body(allocator, &resolved.key, params);
            OutputExpression::ReadKey(Box::new_in(
                ReadKeyExpr {
                    receiver: Box::new_in(receiver, allocator),
                    index: Box::new_in(key, allocator),
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Resolved safe property read
        IrExpression::ResolvedSafePropertyRead(resolved) => {
            let receiver = convert_pure_function_body(allocator, &resolved.receiver, params);
            let receiver_clone = convert_pure_function_body(allocator, &resolved.receiver, params);
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(
                        OutputExpression::BinaryOperator(Box::new_in(
                            BinaryOperatorExpr {
                                operator: BinaryOperator::Equals,
                                lhs: Box::new_in(receiver, allocator),
                                rhs: Box::new_in(
                                    OutputExpression::Literal(Box::new_in(
                                        LiteralExpr {
                                            value: LiteralValue::Null,
                                            source_span: None,
                                        },
                                        allocator,
                                    )),
                                    allocator,
                                ),
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    true_case: Box::new_in(
                        OutputExpression::Literal(Box::new_in(
                            LiteralExpr { value: LiteralValue::Null, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    false_case: Some(Box::new_in(
                        OutputExpression::ReadProp(Box::new_in(
                            ReadPropExpr {
                                receiver: Box::new_in(receiver_clone, allocator),
                                name: resolved.name.clone(),
                                optional: false,
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Safe property read (obj?.prop) - emit as conditional
        IrExpression::SafePropertyRead(spr) => {
            let receiver = convert_pure_function_body(allocator, &spr.receiver, params);
            let receiver_clone = convert_pure_function_body(allocator, &spr.receiver, params);
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(
                        OutputExpression::BinaryOperator(Box::new_in(
                            BinaryOperatorExpr {
                                operator: BinaryOperator::Equals,
                                lhs: Box::new_in(receiver, allocator),
                                rhs: Box::new_in(
                                    OutputExpression::Literal(Box::new_in(
                                        LiteralExpr {
                                            value: LiteralValue::Null,
                                            source_span: None,
                                        },
                                        allocator,
                                    )),
                                    allocator,
                                ),
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    true_case: Box::new_in(
                        OutputExpression::Literal(Box::new_in(
                            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    false_case: Some(Box::new_in(
                        OutputExpression::ReadProp(Box::new_in(
                            ReadPropExpr {
                                receiver: Box::new_in(receiver_clone, allocator),
                                name: spr.name.clone(),
                                optional: false,
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Safe keyed read (obj?.[key]) - emit as conditional
        IrExpression::SafeKeyedRead(skr) => {
            let receiver = convert_pure_function_body(allocator, &skr.receiver, params);
            let receiver_clone = convert_pure_function_body(allocator, &skr.receiver, params);
            let index = convert_pure_function_body(allocator, &skr.index, params);
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(
                        OutputExpression::BinaryOperator(Box::new_in(
                            BinaryOperatorExpr {
                                operator: BinaryOperator::Equals,
                                lhs: Box::new_in(receiver, allocator),
                                rhs: Box::new_in(
                                    OutputExpression::Literal(Box::new_in(
                                        LiteralExpr {
                                            value: LiteralValue::Null,
                                            source_span: None,
                                        },
                                        allocator,
                                    )),
                                    allocator,
                                ),
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    true_case: Box::new_in(
                        OutputExpression::Literal(Box::new_in(
                            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    false_case: Some(Box::new_in(
                        OutputExpression::ReadKey(Box::new_in(
                            ReadKeyExpr {
                                receiver: Box::new_in(receiver_clone, allocator),
                                index: Box::new_in(index, allocator),
                                optional: false,
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Safe function invocation (fn?.()) - emit as conditional
        IrExpression::SafeInvokeFunction(sif) => {
            let receiver = convert_pure_function_body(allocator, &sif.receiver, params);
            let receiver_clone = convert_pure_function_body(allocator, &sif.receiver, params);
            let mut args = OxcVec::with_capacity_in(sif.args.len(), allocator);
            for arg in sif.args.iter() {
                args.push(convert_pure_function_body(allocator, arg, params));
            }
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(
                        OutputExpression::BinaryOperator(Box::new_in(
                            BinaryOperatorExpr {
                                operator: BinaryOperator::Equals,
                                lhs: Box::new_in(receiver, allocator),
                                rhs: Box::new_in(
                                    OutputExpression::Literal(Box::new_in(
                                        LiteralExpr {
                                            value: LiteralValue::Null,
                                            source_span: None,
                                        },
                                        allocator,
                                    )),
                                    allocator,
                                ),
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    true_case: Box::new_in(
                        OutputExpression::Literal(Box::new_in(
                            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    false_case: Some(Box::new_in(
                        OutputExpression::InvokeFunction(Box::new_in(
                            InvokeFunctionExpr {
                                fn_expr: Box::new_in(receiver_clone, allocator),
                                args,
                                pure: false,
                                optional: false,
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Safe ternary (guard && expr)
        IrExpression::SafeTernary(st) => {
            let guard = convert_pure_function_body(allocator, &st.guard, params);
            let expr = convert_pure_function_body(allocator, &st.expr, params);
            OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::And,
                    lhs: Box::new_in(guard, allocator),
                    rhs: Box::new_in(expr, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // OutputExpr - already in output format
        IrExpression::OutputExpr(output) => output.clone_in(allocator),

        // Const collected - unwrap and convert inner expression
        IrExpression::ConstCollected(cc) => convert_pure_function_body(allocator, &cc.expr, params),

        // Two-way binding set
        IrExpression::TwoWayBindingSet(tbs) => {
            let target = convert_pure_function_body(allocator, &tbs.target, params);
            let value = convert_pure_function_body(allocator, &tbs.value, params);
            // Create: i0.ɵɵtwoWayBindingSet(target, value)
            let mut args = OxcVec::new_in(allocator);
            args.push(target);
            args.push(value);
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::External(Box::new_in(
                            ExternalExpr {
                                value: ExternalReference {
                                    module_name: Some(Atom::from("@angular/core")),
                                    name: Some(Atom::from(Identifiers::TWO_WAY_BINDING_SET)),
                                },
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args,
                    pure: false,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Context @let reference
        IrExpression::ContextLetReference(ctx_let) => {
            let mut args = OxcVec::new_in(allocator);
            if let Some(slot) = ctx_let.target_slot.slot {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(slot.0 as f64), source_span: None },
                    allocator,
                )));
            }
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::External(Box::new_in(
                            ExternalExpr {
                                value: ExternalReference {
                                    module_name: Some(Atom::from("@angular/core")),
                                    name: Some(Atom::from(Identifiers::READ_CONTEXT_LET)),
                                },
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args,
                    pure: true,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Store @let value
        IrExpression::StoreLet(store) => {
            let mut args = OxcVec::new_in(allocator);
            args.push(convert_pure_function_body(allocator, &store.value, params));
            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::External(Box::new_in(
                            ExternalExpr {
                                value: ExternalReference {
                                    module_name: Some(Atom::from("@angular/core")),
                                    name: Some(Atom::from(Identifiers::STORE_LET)),
                                },
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args,
                    pure: false,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // ConditionalCase - emit condition or null if no condition (else case)
        IrExpression::ConditionalCase(cc) => {
            if let Some(ref condition) = cc.expr {
                convert_pure_function_body(allocator, condition, params)
            } else {
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Null, source_span: None },
                    allocator,
                ))
            }
        }

        // Ast expressions in pure function bodies (from pure_literal_structures phase)
        // Only constant expressions should appear here - convert them to output expressions
        IrExpression::Ast(ast) => convert_ast_for_pure_function_body(allocator, ast, params),
        IrExpression::ExpressionRef(_) => {
            // ExpressionRef should be resolved before pure function extraction.
            // If we reach here, it indicates a bug in the pipeline phases.
            // Return undefined as a fallback to avoid runtime panic.
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                allocator,
            ))
        }

        // View-related expressions that shouldn't appear in pure function bodies
        IrExpression::NextContext(_)
        | IrExpression::GetCurrentView(_)
        | IrExpression::RestoreView(_)
        | IrExpression::ResetView(_)
        | IrExpression::Reference(_)
        | IrExpression::PureFunction(_) => {
            // These are view-context expressions that shouldn't appear in extracted pure function bodies
            // Return undefined as a fallback
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                allocator,
            ))
        }

        // Ternary expression: convert to conditional output expression
        IrExpression::Ternary(ternary) => {
            let condition = convert_pure_function_body(allocator, &ternary.condition, params);
            let true_case = convert_pure_function_body(allocator, &ternary.true_expr, params);
            let false_case = convert_pure_function_body(allocator, &ternary.false_expr, params);
            OutputExpression::Conditional(Box::new_in(
                ConditionalExpr {
                    condition: Box::new_in(condition, allocator),
                    true_case: Box::new_in(true_case, allocator),
                    false_case: Some(Box::new_in(false_case, allocator)),
                    source_span: ternary.source_span,
                },
                allocator,
            ))
        }

        // DerivedLiteralArray: convert to a literal array with nested conversions
        IrExpression::DerivedLiteralArray(arr) => {
            let mut entries = OxcVec::with_capacity_in(arr.entries.len(), allocator);
            for entry in arr.entries.iter() {
                entries.push(convert_pure_function_body(allocator, entry, params));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ))
        }

        // DerivedLiteralMap: convert to a literal map with nested conversions
        IrExpression::DerivedLiteralMap(map) => {
            let mut entries = OxcVec::with_capacity_in(map.keys.len(), allocator);
            for i in 0..map.keys.len() {
                let key = map.keys[i].clone();
                let value = convert_pure_function_body(allocator, &map.values[i], params);
                let quoted = map.quoted.get(i).copied().unwrap_or(false);
                entries.push(LiteralMapEntry { key, value, quoted });
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: None },
                allocator,
            ))
        }

        // LiteralArray: convert to a literal array with nested conversions
        IrExpression::LiteralArray(arr) => {
            let mut entries = OxcVec::with_capacity_in(arr.elements.len(), allocator);
            for elem in arr.elements.iter() {
                entries.push(convert_pure_function_body(allocator, elem, params));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ))
        }

        // LiteralMap: convert to a literal map with nested conversions
        IrExpression::LiteralMap(map) => {
            let mut entries = OxcVec::with_capacity_in(map.keys.len(), allocator);
            for i in 0..map.keys.len() {
                let key = map.keys[i].clone();
                let value = convert_pure_function_body(allocator, &map.values[i], params);
                let quoted = map.quoted.get(i).copied().unwrap_or(false);
                entries.push(LiteralMapEntry { key, value, quoted });
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: None },
                allocator,
            ))
        }

        // Not expression: convert operand and wrap in NOT
        IrExpression::Not(not_expr) => {
            let inner = convert_pure_function_body(allocator, &not_expr.expr, params);
            OutputExpression::Not(Box::new_in(
                crate::output::ast::NotExpr {
                    condition: Box::new_in(inner, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Unary expression: convert operand and wrap in unary operator
        IrExpression::Unary(unary) => {
            let inner = convert_pure_function_body(allocator, &unary.expr, params);
            let operator = match unary.operator {
                crate::ir::expression::IrUnaryOperator::Plus => UnaryOperator::Plus,
                crate::ir::expression::IrUnaryOperator::Minus => UnaryOperator::Minus,
            };
            OutputExpression::UnaryOperator(Box::new_in(
                UnaryOperatorExpr {
                    operator,
                    expr: Box::new_in(inner, allocator),
                    parens: false,
                    source_span: None,
                },
                allocator,
            ))
        }

        // Typeof expression: convert operand and wrap in typeof
        IrExpression::Typeof(typeof_expr) => {
            let inner = convert_pure_function_body(allocator, &typeof_expr.expr, params);
            OutputExpression::Typeof(Box::new_in(
                crate::output::ast::TypeofExpr {
                    expr: Box::new_in(inner, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // Void expression: convert operand and wrap in void
        IrExpression::Void(void_expr) => {
            let inner = convert_pure_function_body(allocator, &void_expr.expr, params);
            OutputExpression::Void(Box::new_in(
                crate::output::ast::VoidExpr {
                    expr: Box::new_in(inner, allocator),
                    source_span: None,
                },
                allocator,
            ))
        }

        // ResolvedTemplateLiteral: convert to template literal with resolved expressions
        IrExpression::ResolvedTemplateLiteral(rtl) => {
            let mut elements = OxcVec::new_in(allocator);
            let mut expressions = OxcVec::new_in(allocator);

            for elem in rtl.elements.iter() {
                elements.push(crate::output::ast::TemplateLiteralElement {
                    text: elem.text.clone(),
                    raw_text: elem.text.clone(),
                    source_span: elem.source_span,
                });
            }

            for expr in rtl.expressions.iter() {
                expressions.push(convert_pure_function_body(allocator, expr, params));
            }

            OutputExpression::TemplateLiteral(Box::new_in(
                crate::output::ast::TemplateLiteralExpr {
                    elements,
                    expressions,
                    source_span: rtl.source_span,
                },
                allocator,
            ))
        }

        // Arrow function - convert body and preserve parameters
        IrExpression::ArrowFunction(arrow_fn) => {
            let mut params_vec = OxcVec::with_capacity_in(arrow_fn.params.len(), allocator);
            for param in arrow_fn.params.iter() {
                params_vec.push(crate::output::ast::FnParam { name: param.name.clone() });
            }

            // Convert the body expression
            let body_expr = convert_pure_function_body(allocator, &arrow_fn.body, params);

            OutputExpression::ArrowFunction(Box::new_in(
                crate::output::ast::ArrowFunctionExpr {
                    params: params_vec,
                    body: crate::output::ast::ArrowFunctionBody::Expression(Box::new_in(
                        body_expr, allocator,
                    )),
                    source_span: arrow_fn.source_span,
                },
                allocator,
            ))
        }
        // Parenthesized expression - convert inner and wrap
        IrExpression::Parenthesized(paren) => {
            let inner = convert_pure_function_body(allocator, &paren.expr, params);
            OutputExpression::Parenthesized(Box::new_in(
                crate::output::ast::ParenthesizedExpr {
                    expr: Box::new_in(inner, allocator),
                    source_span: paren.source_span,
                },
                allocator,
            ))
        }
    }
}

/// Convert an AST expression to an output expression for pure function bodies.
/// This handles constant expressions (literals, arrays, maps) that appear in pure function body.
fn convert_ast_for_pure_function_body<'a>(
    allocator: &'a Allocator,
    ast: &crate::ast::expression::AngularExpression<'a>,
    params: &[Atom<'a>],
) -> OutputExpression<'a> {
    use crate::ast::expression::{AngularExpression, LiteralMapKey};
    use crate::output::ast::{LiteralArrayExpr, LiteralMapEntry, LiteralMapExpr};

    match ast {
        AngularExpression::LiteralPrimitive(lit) => {
            let value = match &lit.value {
                crate::ast::expression::LiteralValue::String(s) => LiteralValue::String(s.clone()),
                crate::ast::expression::LiteralValue::Number(n) => LiteralValue::Number(*n),
                crate::ast::expression::LiteralValue::Boolean(b) => LiteralValue::Boolean(*b),
                crate::ast::expression::LiteralValue::Null => LiteralValue::Null,
                crate::ast::expression::LiteralValue::Undefined => LiteralValue::Undefined,
            };
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value, source_span: None },
                allocator,
            ))
        }
        AngularExpression::LiteralArray(arr) => {
            let mut entries = OxcVec::with_capacity_in(arr.expressions.len(), allocator);
            for entry in arr.expressions.iter() {
                entries.push(convert_ast_for_pure_function_body(allocator, entry, params));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ))
        }
        AngularExpression::LiteralMap(map) => {
            let mut entries = OxcVec::with_capacity_in(map.keys.len(), allocator);
            for (i, key) in map.keys.iter().enumerate() {
                // Only handle property keys; skip spread keys
                if let LiteralMapKey::Property(prop) = key {
                    let key_value = prop.key.clone();
                    let value = if i < map.values.len() {
                        convert_ast_for_pure_function_body(allocator, &map.values[i], params)
                    } else {
                        OutputExpression::Literal(Box::new_in(
                            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                            allocator,
                        ))
                    };
                    entries.push(LiteralMapEntry { key: key_value, value, quoted: prop.quoted });
                }
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: None },
                allocator,
            ))
        }
        AngularExpression::Empty(_) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
            allocator,
        )),
        // For property reads on implicit receiver, convert to ctx.property
        AngularExpression::PropertyRead(prop) => {
            if matches!(&prop.receiver, AngularExpression::ImplicitReceiver(_)) {
                OutputExpression::ReadProp(Box::new_in(
                    crate::output::ast::ReadPropExpr {
                        receiver: Box::new_in(
                            OutputExpression::ReadVar(Box::new_in(
                                crate::output::ast::ReadVarExpr {
                                    name: Atom::from("ctx"),
                                    source_span: None,
                                },
                                allocator,
                            )),
                            allocator,
                        ),
                        name: prop.name.clone(),
                        optional: false,
                        source_span: None,
                    },
                    allocator,
                ))
            } else {
                // Fallback for non-implicit receivers - shouldn't happen for constants
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Undefined, source_span: None },
                    allocator,
                ))
            }
        }
        // Other expressions shouldn't appear in constant pure function bodies
        _ => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Undefined, source_span: None },
            allocator,
        )),
    }
}

/// Emit a pooled constant as an OutputExpression.
/// Takes mutable reference to allow taking ownership of expressions that can't be cloned.
fn emit_pooled_constant_value<'a>(
    allocator: &'a Allocator,
    kind: &mut crate::pipeline::constant_pool::PooledConstantKind<'a>,
) -> OutputExpression<'a> {
    use crate::output::ast::{LiteralArrayExpr, LiteralMapEntry, LiteralMapExpr};
    use crate::pipeline::constant_pool::PooledConstantKind;

    match kind {
        PooledConstantKind::String(s) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(s.clone()), source_span: None },
            allocator,
        )),
        PooledConstantKind::Number(n) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(*n), source_span: None },
            allocator,
        )),
        PooledConstantKind::Boolean(b) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(*b), source_span: None },
            allocator,
        )),
        PooledConstantKind::Array(elements) => {
            // Recursively emit array elements
            let mut entries = OxcVec::with_capacity_in(elements.len(), allocator);
            for elem in elements.iter_mut() {
                entries.push(emit_pooled_constant_value(allocator, elem));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ))
        }
        PooledConstantKind::ArrayPlaceholder => {
            // Placeholder arrays emit as empty arrays
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: OxcVec::new_in(allocator), source_span: None },
                allocator,
            ))
        }
        PooledConstantKind::Object(entries) => {
            // Emit object literal
            let mut map_entries = OxcVec::with_capacity_in(entries.len(), allocator);
            for (key, value) in entries.iter_mut() {
                map_entries.push(LiteralMapEntry {
                    key: key.clone(),
                    value: emit_pooled_constant_value(allocator, value),
                    quoted: false,
                });
            }
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries: map_entries, source_span: None },
                allocator,
            ))
        }
        PooledConstantKind::External(ext) => {
            // Emit external reference as a property read on the module
            OutputExpression::External(Box::new_in(
                crate::output::ast::ExternalExpr {
                    value: crate::output::ast::ExternalReference {
                        module_name: Some(ext.module.clone()),
                        name: Some(ext.name.clone()),
                    },
                    source_span: None,
                },
                allocator,
            ))
        }
        PooledConstantKind::PureFunction(pf) => {
            // Pure functions are emitted as arrow functions with the actual body expression.
            // The body expression may contain PureFunctionParameterExpr nodes that need to be
            // transformed to variable references (a0, a1, etc.).
            let mut params = OxcVec::with_capacity_in(pf.params.len(), allocator);
            for param in pf.params.iter() {
                params.push(FnParam { name: param.clone() });
            }

            // Convert the body expression, transforming PureFunctionParameterExpr to var refs
            let body_expr = convert_pure_function_body(allocator, &pf.body, &pf.params);

            OutputExpression::ArrowFunction(Box::new_in(
                crate::output::ast::ArrowFunctionExpr {
                    params,
                    body: crate::output::ast::ArrowFunctionBody::Expression(Box::new_in(
                        body_expr, allocator,
                    )),
                    source_span: None,
                },
                allocator,
            ))
        }
        PooledConstantKind::RegularExpression(regex) => {
            OutputExpression::RegularExpressionLiteral(Box::new_in(
                crate::output::ast::RegularExpressionLiteralExpr {
                    body: regex.body.clone(),
                    flags: regex.flags.clone(),
                    source_span: None,
                },
                allocator,
            ))
        }
        PooledConstantKind::Literal(literal_expr) => {
            // Literal expressions are stored as-is and cloned for emit.
            // Used by getConstLiteral for array/primitive pooling.
            literal_expr.clone_in(allocator)
        }
    }
}

/// Result of compiling host bindings.
pub struct HostBindingCompilationResult<'a> {
    /// The host binding function, if any.
    pub host_binding_fn: Option<FunctionExpr<'a>>,
    /// Static host attributes to be emitted as `hostAttrs`.
    pub host_attrs: Option<OutputExpression<'a>>,
    /// Number of host variables for change detection.
    /// Only set if > 0.
    pub host_vars: Option<u32>,
    /// Additional declarations (pooled constants like pure functions).
    /// In Angular TS, template and host binding share the same ConstantPool,
    /// so host binding constants get emitted alongside template constants.
    pub declarations: OxcVec<'a, OutputStatement<'a>>,
}

/// Compile host bindings from start to finish.
///
/// This is the main entry point for host binding compilation:
/// 1. Runs all applicable transformation phases
/// 2. Emits the host binding function
/// 3. Extracts hostAttrs and hostVars for component definition
///
/// Ported from Angular's host binding compilation in `emit.ts`.
pub fn compile_host_bindings<'a>(
    job: &mut HostBindingCompilationJob<'a>,
) -> HostBindingCompilationResult<'a> {
    use crate::output::ast::DeclareVarStmt;

    let allocator = job.allocator;

    // Run all transformation phases for host bindings
    transform_host(job);

    // Emit the host binding function
    let host_binding_fn = emit_host_binding_function(job);

    // Extract hostAttrs and hostVars from the job
    // Per Angular compiler.ts lines 525-530:
    //   definitionMap.set('hostAttrs', hostJob.root.attributes);
    //   if (varCount !== null && varCount > 0) {
    //     definitionMap.set('hostVars', o.literal(varCount));
    //   }
    let host_attrs = job.root.attributes.take();
    let host_vars = job.root.vars.filter(|&v| v > 0);

    // Collect declarations from host binding pool constants.
    // In Angular TS, template and host binding share the same ConstantPool,
    // so host binding pure functions get emitted alongside template constants.
    let mut declarations = OxcVec::new_in(allocator);
    for constant in job.pool.constants_mut() {
        let value = emit_pooled_constant_value(allocator, &mut constant.kind);
        declarations.push(OutputStatement::DeclareVar(Box::new_in(
            DeclareVarStmt {
                name: constant.name.clone(),
                value: Some(value),
                modifiers: StmtModifier::FINAL,
                leading_comment: None,
                source_span: None,
            },
            allocator,
        )));
    }
    for stmt in job.pool.statements.iter() {
        declarations.push(clone_output_statement(stmt, allocator));
    }

    HostBindingCompilationResult { host_binding_fn, host_attrs, host_vars, declarations }
}

/// Converts an IR binary operator to an output binary operator.
fn convert_ir_binary_operator(op: crate::ir::expression::IrBinaryOperator) -> BinaryOperator {
    use crate::ir::expression::IrBinaryOperator;

    match op {
        IrBinaryOperator::Plus => BinaryOperator::Plus,
        IrBinaryOperator::Minus => BinaryOperator::Minus,
        IrBinaryOperator::Multiply => BinaryOperator::Multiply,
        IrBinaryOperator::Divide => BinaryOperator::Divide,
        IrBinaryOperator::Modulo => BinaryOperator::Modulo,
        IrBinaryOperator::Exponentiation => BinaryOperator::Exponentiation,
        IrBinaryOperator::Equals => BinaryOperator::Equals,
        IrBinaryOperator::NotEquals => BinaryOperator::NotEquals,
        IrBinaryOperator::Identical => BinaryOperator::Identical,
        IrBinaryOperator::NotIdentical => BinaryOperator::NotIdentical,
        IrBinaryOperator::Lower => BinaryOperator::Lower,
        IrBinaryOperator::LowerEquals => BinaryOperator::LowerEquals,
        IrBinaryOperator::Bigger => BinaryOperator::Bigger,
        IrBinaryOperator::BiggerEquals => BinaryOperator::BiggerEquals,
        IrBinaryOperator::And => BinaryOperator::And,
        IrBinaryOperator::Or => BinaryOperator::Or,
        IrBinaryOperator::NullishCoalesce => BinaryOperator::NullishCoalesce,
        IrBinaryOperator::In => BinaryOperator::In,
        IrBinaryOperator::Instanceof => BinaryOperator::Instanceof,
        IrBinaryOperator::Assign => BinaryOperator::Assign,
        IrBinaryOperator::AdditionAssignment => BinaryOperator::AdditionAssignment,
        IrBinaryOperator::SubtractionAssignment => BinaryOperator::SubtractionAssignment,
        IrBinaryOperator::MultiplicationAssignment => BinaryOperator::MultiplicationAssignment,
        IrBinaryOperator::DivisionAssignment => BinaryOperator::DivisionAssignment,
        IrBinaryOperator::RemainderAssignment => BinaryOperator::RemainderAssignment,
        IrBinaryOperator::ExponentiationAssignment => BinaryOperator::ExponentiationAssignment,
        IrBinaryOperator::AndAssignment => BinaryOperator::AndAssignment,
        IrBinaryOperator::OrAssignment => BinaryOperator::OrAssignment,
        IrBinaryOperator::NullishCoalesceAssignment => BinaryOperator::NullishCoalesceAssignment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_template() {
        let allocator = Allocator::default();
        let mut job = ComponentCompilationJob::new(&allocator, Atom::from("TestComponent"));

        let result = compile_template(&mut job);

        // Per Angular's saveAndRestoreView phase, we eagerly add SavedView
        // variables to all views (they will be optimized away later).
        // So an "empty" template will have a create block with just the SavedView var.
        // The template function should have at most one statement (the create block
        // containing the SavedView variable).
        assert!(result.template_fn.statements.len() <= 1);
    }
}

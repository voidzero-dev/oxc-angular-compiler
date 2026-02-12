//! Query generation for directives.
//!
//! Ported from Angular's `render3/view/query_generation.ts`.
//!
//! Generates query functions for `contentQueries` and `viewQuery` fields
//! in directive definitions.

use oxc_allocator::{Allocator, Box, FromIn, Vec};
use oxc_span::Atom;

use super::metadata::{QueryPredicate, R3QueryMetadata};
use crate::output::ast::{
    BinaryOperator, BinaryOperatorExpr, DeclareVarStmt, ExpressionStatement, FnParam, FunctionExpr,
    IfStmt, InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralValue, OutputExpression,
    OutputStatement, ReadPropExpr, ReadVarExpr, StmtModifier,
};
use crate::pipeline::constant_pool::ConstantPool;
use crate::r3::Identifiers;

// ============================================================================
// Query Flags
// ============================================================================

/// Query flags matching Angular's runtime QueryFlags enum.
///
/// NOTE: Ensure these match `packages/core/src/render3/interfaces/query.ts`
#[derive(Debug, Clone, Copy)]
pub struct QueryFlags(u32);

impl QueryFlags {
    /// No flags.
    pub const NONE: Self = Self(0b0000);

    /// Whether or not the query should descend into children.
    pub const DESCENDANTS: Self = Self(0b0001);

    /// The query can be computed statically and hence can be assigned eagerly.
    pub const IS_STATIC: Self = Self(0b0010);

    /// If the QueryList should fire change event only if actual change to query
    /// was computed.
    pub const EMIT_DISTINCT_CHANGES_ONLY: Self = Self(0b0100);

    /// Get the raw flags value.
    pub fn value(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for QueryFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Convert query metadata to query flags.
fn to_query_flags(query: &R3QueryMetadata<'_>) -> QueryFlags {
    let mut flags = QueryFlags::NONE;

    if query.descendants {
        flags = flags | QueryFlags::DESCENDANTS;
    }

    if query.is_static {
        flags = flags | QueryFlags::IS_STATIC;
    }

    if query.emit_distinct_changes_only {
        flags = flags | QueryFlags::EMIT_DISTINCT_CHANGES_ONLY;
    }

    flags
}

// ============================================================================
// Context Names
// ============================================================================

/// Render flags parameter name.
const RENDER_FLAGS: &str = "rf";

/// Context parameter name.
const CONTEXT_NAME: &str = "ctx";

/// Temporary variable base name.
const TEMPORARY_NAME: &str = "_t";

// ============================================================================
// Render Flags
// ============================================================================

/// Render flags matching Angular's RenderFlags enum.
mod render_flags {
    pub const CREATE: u32 = 1;
    pub const UPDATE: u32 = 2;
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create i0.identifier reference.
fn import_expr<'a>(allocator: &'a Allocator, identifier: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Atom::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Atom::from(identifier),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create a variable reference.
fn variable<'a>(allocator: &'a Allocator, name: &'static str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Atom::from(name), source_span: None },
        allocator,
    ))
}

/// Create a literal number.
fn literal_number<'a>(allocator: &'a Allocator, value: u32) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(f64::from(value)), source_span: None },
        allocator,
    ))
}

/// Create ctx.propertyName.
fn context_prop<'a>(allocator: &'a Allocator, property_name: &Atom<'a>) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(variable(allocator, CONTEXT_NAME), allocator),
            name: property_name.clone(),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create a function call expression.
fn call_fn<'a>(
    allocator: &'a Allocator,
    fn_expr: OutputExpression<'a>,
    args: Vec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create if (rf & flags) { statements }
fn render_flag_check_if_stmt<'a>(
    allocator: &'a Allocator,
    flags: u32,
    statements: Vec<'a, OutputStatement<'a>>,
) -> OutputStatement<'a> {
    // rf & flags
    let condition = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::BitwiseAnd,
            lhs: Box::new_in(variable(allocator, RENDER_FLAGS), allocator),
            rhs: Box::new_in(literal_number(allocator, flags), allocator),
            source_span: None,
        },
        allocator,
    ));

    OutputStatement::If(Box::new_in(
        IfStmt {
            condition,
            true_case: statements,
            false_case: Vec::new_in(allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Create expression statement.
fn expr_stmt<'a>(allocator: &'a Allocator, expr: OutputExpression<'a>) -> OutputStatement<'a> {
    OutputStatement::Expression(Box::new_in(
        ExpressionStatement { expr, source_span: None },
        allocator,
    ))
}

// ============================================================================
// Query Predicate
// ============================================================================

/// Get the query predicate expression.
///
/// For type predicates, returns the type expression.
/// For string selectors, returns a literal array of strings.
///
/// When a constant pool is provided, string selector arrays are pooled to top-level
/// constants (e.g., `const _c0 = ["refName"]`). This matches TypeScript Angular's
/// behavior where query predicates are pooled BEFORE pure functions.
fn get_query_predicate<'a>(
    allocator: &'a Allocator,
    query: &R3QueryMetadata<'a>,
    pool: Option<&mut ConstantPool<'a>>,
) -> OutputExpression<'a> {
    match &query.predicate {
        QueryPredicate::Type(expr) => expr.clone_in(allocator),
        QueryPredicate::Selectors(selectors) => {
            // Convert selectors to literal array
            // Each selector may contain comma-separated refs that need splitting
            let mut entries = Vec::new_in(allocator);

            for selector in selectors.iter() {
                // Split by comma and trim
                for part in selector.as_str().split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        entries.push(OutputExpression::Literal(Box::new_in(
                            LiteralExpr {
                                value: LiteralValue::String(Atom::from(trimmed)),
                                source_span: None,
                            },
                            allocator,
                        )));
                    }
                }
            }

            let array_expr = OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ));

            // Pool the array to a top-level constant if pool is provided
            // This matches TypeScript Angular's behavior where query predicates
            // are pooled BEFORE pure functions, ensuring correct constant ordering.
            if let Some(pool) = pool {
                pool.get_const_literal(array_expr, true)
            } else {
                array_expr
            }
        }
    }
}

// Note: get_query_create_parameters has been replaced by:
// - get_query_create_parameters_with_predicate (for view queries with pre-pooled predicates)
// - get_content_query_create_parameters_with_predicate (for content queries with pre-pooled predicates)
// The pre-pooling approach allows all predicates to be pooled upfront before building the function,
// avoiding borrow checker issues with mutable pool references in loops.

/// Get the parameters for query creation with a pre-computed predicate.
///
/// This variant takes the predicate expression directly instead of computing it,
/// allowing for pre-pooled predicates to be reused.
///
/// Returns [predicate, flags, read?] for view queries.
fn get_query_create_parameters_with_predicate<'a>(
    allocator: &'a Allocator,
    query: &R3QueryMetadata<'a>,
    predicate: OutputExpression<'a>,
) -> Vec<'a, OutputExpression<'a>> {
    let mut parameters = Vec::new_in(allocator);

    // For signal queries, first param is ctx.propertyName
    if query.is_signal {
        parameters.push(context_prop(allocator, &query.property_name));
    }

    // Add pre-computed predicate
    parameters.push(predicate);

    // Add flags
    let flags = to_query_flags(query);
    parameters.push(literal_number(allocator, flags.value()));

    // Add read type if present
    if let Some(read) = &query.read {
        parameters.push(read.clone_in(allocator));
    }

    parameters
}

/// Get the parameters for content query creation with a pre-computed predicate.
///
/// This variant takes the predicate expression directly and prepend parameters,
/// allowing for pre-pooled predicates to be reused.
///
/// Returns [dirIndex, predicate, flags, read?] for content queries.
fn get_content_query_create_parameters_with_predicate<'a>(
    allocator: &'a Allocator,
    query: &R3QueryMetadata<'a>,
    predicate: OutputExpression<'a>,
    prepend_params: Vec<'a, OutputExpression<'a>>,
) -> Vec<'a, OutputExpression<'a>> {
    let mut parameters = Vec::new_in(allocator);

    // Add prepend params (e.g., dirIndex for content queries)
    for param in prepend_params {
        parameters.push(param);
    }

    // For signal queries, first param is ctx.propertyName
    if query.is_signal {
        parameters.push(context_prop(allocator, &query.property_name));
    }

    // Add pre-computed predicate
    parameters.push(predicate);

    // Add flags
    let flags = to_query_flags(query);
    parameters.push(literal_number(allocator, flags.value()));

    // Add read type if present
    if let Some(read) = &query.read {
        parameters.push(read.clone_in(allocator));
    }

    parameters
}

// ============================================================================
// Query Advance Optimization
// ============================================================================

/// Statement that may be a placeholder for query advance.
enum MaybeAdvanceStatement<'a> {
    Statement(OutputStatement<'a>),
    Advance,
}

/// Collapse multiple query advance placeholders into single calls.
///
/// Multiple consecutive advance placeholders are combined into a single
/// ɵɵqueryAdvance(count) call.
fn collapse_advance_statements<'a>(
    allocator: &'a Allocator,
    statements: Vec<'a, MaybeAdvanceStatement<'a>>,
) -> Vec<'a, OutputStatement<'a>> {
    let mut result = Vec::new_in(allocator);
    let mut advance_count = 0u32;

    // Process statements and flush pending advances
    let flush_advance = |result: &mut Vec<'a, OutputStatement<'a>>, count: &mut u32| {
        if *count > 0 {
            // Create ɵɵqueryAdvance() or ɵɵqueryAdvance(count)
            let mut args = Vec::new_in(allocator);
            if *count > 1 {
                args.push(literal_number(allocator, *count));
            }
            let call = call_fn(allocator, import_expr(allocator, Identifiers::QUERY_ADVANCE), args);
            result.push(expr_stmt(allocator, call));
            *count = 0;
        }
    };

    for stmt in statements {
        match stmt {
            MaybeAdvanceStatement::Advance => {
                advance_count += 1;
            }
            MaybeAdvanceStatement::Statement(s) => {
                flush_advance(&mut result, &mut advance_count);
                result.push(s);
            }
        }
    }

    // Flush any remaining advances
    flush_advance(&mut result, &mut advance_count);

    result
}

// ============================================================================
// Temporary Variable Allocator
// ============================================================================

/// Simple temporary variable allocator.
///
/// Creates a single temporary variable on first allocation and returns the same
/// reference on subsequent calls. This matches Angular's `temporaryAllocator` in
/// `render3/view/util.ts` which reuses the same `_t` variable.
struct TempAllocator {
    allocated: bool,
}

impl TempAllocator {
    fn new() -> Self {
        Self { allocated: false }
    }

    fn allocate<'a>(&mut self, allocator: &'a Allocator) -> OutputExpression<'a> {
        // Angular always reuses the same _t variable - see temporaryAllocator in util.ts
        self.allocated = true;
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from(TEMPORARY_NAME), source_span: None },
            allocator,
        ))
    }

    fn needs_declaration(&self) -> bool {
        self.allocated
    }
}

// ============================================================================
// View Query Function
// ============================================================================

/// Create view queries function.
///
/// Generates a function like:
/// ```javascript
/// function MyComponent_Query(rf, ctx) {
///   if (rf & 1) {
///     ɵɵviewQuery(SomeComponent, 5);
///   }
///   if (rf & 2) {
///     let _t;
///     ɵɵqueryRefresh(_t = ɵɵloadQuery()) && (ctx.myQuery = _t.first);
///   }
/// }
/// ```
pub fn create_view_queries_function<'a>(
    allocator: &'a Allocator,
    view_queries: &[R3QueryMetadata<'a>],
    name: Option<&str>,
    pool: Option<&mut ConstantPool<'a>>,
) -> OutputExpression<'a> {
    // Pre-pool all string selector predicates BEFORE building the function.
    // This ensures query predicates are pooled to top-level constants in the correct order
    // (before pure functions), matching TypeScript Angular's behavior.
    let pooled_predicates: std::vec::Vec<OutputExpression<'a>> = if let Some(pool) = pool {
        view_queries.iter().map(|query| get_query_predicate(allocator, query, Some(pool))).collect()
    } else {
        view_queries.iter().map(|query| get_query_predicate(allocator, query, None)).collect()
    };

    let mut create_statements = Vec::new_in(allocator);
    let mut update_statements: Vec<'a, MaybeAdvanceStatement<'a>> = Vec::new_in(allocator);
    let mut temp_allocator = TempAllocator::new();

    for (idx, query) in view_queries.iter().enumerate() {
        // Creation: ɵɵviewQuery(predicate, flags, read) or ɵɵviewQuerySignal(ctx.prop, predicate, flags, read)
        // Use pre-pooled predicate instead of calling get_query_create_parameters
        let params = get_query_create_parameters_with_predicate(
            allocator,
            query,
            pooled_predicates[idx].clone_in(allocator),
        );

        // Emit each query as a separate statement.
        // Angular 20's ɵɵviewQuery returns void, so chaining is not supported.
        if query.is_signal {
            let call =
                call_fn(allocator, import_expr(allocator, Identifiers::VIEW_QUERY_SIGNAL), params);
            create_statements.push(expr_stmt(allocator, call));
        } else {
            let call = call_fn(allocator, import_expr(allocator, Identifiers::VIEW_QUERY), params);
            create_statements.push(expr_stmt(allocator, call));
        }

        // Update phase
        if query.is_signal {
            // Signal queries update lazily, just advance
            update_statements.push(MaybeAdvanceStatement::Advance);
        } else {
            // Regular queries need explicit refresh
            // ɵɵqueryRefresh(_t = ɵɵloadQuery()) && (ctx.prop = _t or _t.first)
            let temp = temp_allocator.allocate(allocator);

            // _t = ɵɵloadQuery()
            let load_query = call_fn(
                allocator,
                import_expr(allocator, Identifiers::LOAD_QUERY),
                Vec::new_in(allocator),
            );
            let temp_set = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(temp.clone_in(allocator), allocator),
                    rhs: Box::new_in(load_query, allocator),
                    source_span: None,
                },
                allocator,
            ));

            // ɵɵqueryRefresh(_t = ɵɵloadQuery())
            let mut refresh_args = Vec::new_in(allocator);
            refresh_args.push(temp_set);
            let refresh = call_fn(
                allocator,
                import_expr(allocator, Identifiers::QUERY_REFRESH),
                refresh_args,
            );

            // ctx.prop = _t or _t.first
            let value = if query.first {
                OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(temp.clone_in(allocator), allocator),
                        name: Atom::from("first"),
                        optional: false,
                        source_span: None,
                    },
                    allocator,
                ))
            } else {
                temp.clone_in(allocator)
            };

            let update_directive = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(context_prop(allocator, &query.property_name), allocator),
                    rhs: Box::new_in(value, allocator),
                    source_span: None,
                },
                allocator,
            ));

            // refresh && (ctx.prop = ...)
            let and_expr = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::And,
                    lhs: Box::new_in(refresh, allocator),
                    rhs: Box::new_in(update_directive, allocator),
                    source_span: None,
                },
                allocator,
            ));

            update_statements
                .push(MaybeAdvanceStatement::Statement(expr_stmt(allocator, and_expr)));
        }
    }

    // Build update statements with temp variable declarations
    let mut final_update_statements = Vec::new_in(allocator);

    // Add temp variable declarations if needed
    if temp_allocator.needs_declaration() {
        final_update_statements.push(OutputStatement::DeclareVar(Box::new_in(
            DeclareVarStmt {
                name: Atom::from(TEMPORARY_NAME),
                value: None,
                modifiers: StmtModifier::NONE,
                leading_comment: None,
                source_span: None,
            },
            allocator,
        )));
    }

    // Collapse advances and add update statements
    for stmt in collapse_advance_statements(allocator, update_statements) {
        final_update_statements.push(stmt);
    }

    // Build function body
    let mut body = Vec::new_in(allocator);
    if !create_statements.is_empty() {
        body.push(render_flag_check_if_stmt(allocator, render_flags::CREATE, create_statements));
    }
    if !final_update_statements.is_empty() {
        body.push(render_flag_check_if_stmt(
            allocator,
            render_flags::UPDATE,
            final_update_statements,
        ));
    }

    // Build function parameters
    let mut params = Vec::new_in(allocator);
    params.push(FnParam { name: Atom::from(RENDER_FLAGS) });
    params.push(FnParam { name: Atom::from(CONTEXT_NAME) });

    // Create function name
    let fn_name = name.map(|n| {
        let formatted = format!("{n}_Query");
        Atom::from_in(formatted.as_str(), allocator)
    });

    OutputExpression::Function(Box::new_in(
        FunctionExpr { name: fn_name, params, statements: body, source_span: None },
        allocator,
    ))
}

// ============================================================================
// Content Queries Function
// ============================================================================

/// Create content queries function.
///
/// Generates a function like:
/// ```javascript
/// function MyDirective_ContentQueries(rf, ctx, dirIndex) {
///   if (rf & 1) {
///     ɵɵcontentQuery(dirIndex, SomeComponent, 5);
///   }
///   if (rf & 2) {
///     let _t;
///     ɵɵqueryRefresh(_t = ɵɵloadQuery()) && (ctx.myQuery = _t.first);
///   }
/// }
/// ```
pub fn create_content_queries_function<'a>(
    allocator: &'a Allocator,
    queries: &[R3QueryMetadata<'a>],
    name: Option<&str>,
    pool: Option<&mut ConstantPool<'a>>,
) -> OutputExpression<'a> {
    // Pre-pool all string selector predicates BEFORE building the function.
    // This ensures query predicates are pooled to top-level constants in the correct order
    // (before pure functions), matching TypeScript Angular's behavior.
    let pooled_predicates: std::vec::Vec<OutputExpression<'a>> = if let Some(pool) = pool {
        queries.iter().map(|query| get_query_predicate(allocator, query, Some(pool))).collect()
    } else {
        queries.iter().map(|query| get_query_predicate(allocator, query, None)).collect()
    };

    let mut create_statements = Vec::new_in(allocator);
    let mut update_statements: Vec<'a, MaybeAdvanceStatement<'a>> = Vec::new_in(allocator);
    let mut temp_allocator = TempAllocator::new();

    for (idx, query) in queries.iter().enumerate() {
        // Prepend dirIndex parameter for content queries
        let mut prepend = Vec::new_in(allocator);
        prepend.push(variable(allocator, "dirIndex"));
        // Use pre-pooled predicate instead of calling get_query_create_parameters
        let params = get_content_query_create_parameters_with_predicate(
            allocator,
            query,
            pooled_predicates[idx].clone_in(allocator),
            prepend,
        );

        // Emit each query as a separate statement.
        // Angular 20's ɵɵcontentQuery returns void, so chaining is not supported.
        if query.is_signal {
            let call = call_fn(
                allocator,
                import_expr(allocator, Identifiers::CONTENT_QUERY_SIGNAL),
                params,
            );
            create_statements.push(expr_stmt(allocator, call));
        } else {
            let call =
                call_fn(allocator, import_expr(allocator, Identifiers::CONTENT_QUERY), params);
            create_statements.push(expr_stmt(allocator, call));
        }

        // Update phase (same as view queries)
        if query.is_signal {
            update_statements.push(MaybeAdvanceStatement::Advance);
        } else {
            let temp = temp_allocator.allocate(allocator);

            let load_query = call_fn(
                allocator,
                import_expr(allocator, Identifiers::LOAD_QUERY),
                Vec::new_in(allocator),
            );
            let temp_set = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(temp.clone_in(allocator), allocator),
                    rhs: Box::new_in(load_query, allocator),
                    source_span: None,
                },
                allocator,
            ));

            let mut refresh_args = Vec::new_in(allocator);
            refresh_args.push(temp_set);
            let refresh = call_fn(
                allocator,
                import_expr(allocator, Identifiers::QUERY_REFRESH),
                refresh_args,
            );

            let value = if query.first {
                OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(temp.clone_in(allocator), allocator),
                        name: Atom::from("first"),
                        optional: false,
                        source_span: None,
                    },
                    allocator,
                ))
            } else {
                temp.clone_in(allocator)
            };

            let update_directive = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(context_prop(allocator, &query.property_name), allocator),
                    rhs: Box::new_in(value, allocator),
                    source_span: None,
                },
                allocator,
            ));

            let and_expr = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::And,
                    lhs: Box::new_in(refresh, allocator),
                    rhs: Box::new_in(update_directive, allocator),
                    source_span: None,
                },
                allocator,
            ));

            update_statements
                .push(MaybeAdvanceStatement::Statement(expr_stmt(allocator, and_expr)));
        }
    }

    // Build update statements with temp variable declarations
    let mut final_update_statements = Vec::new_in(allocator);

    if temp_allocator.needs_declaration() {
        final_update_statements.push(OutputStatement::DeclareVar(Box::new_in(
            DeclareVarStmt {
                name: Atom::from(TEMPORARY_NAME),
                value: None,
                modifiers: StmtModifier::NONE,
                leading_comment: None,
                source_span: None,
            },
            allocator,
        )));
    }

    for stmt in collapse_advance_statements(allocator, update_statements) {
        final_update_statements.push(stmt);
    }

    // Build function body
    let mut body = Vec::new_in(allocator);
    if !create_statements.is_empty() {
        body.push(render_flag_check_if_stmt(allocator, render_flags::CREATE, create_statements));
    }
    if !final_update_statements.is_empty() {
        body.push(render_flag_check_if_stmt(
            allocator,
            render_flags::UPDATE,
            final_update_statements,
        ));
    }

    // Build function parameters (rf, ctx, dirIndex)
    let mut params = Vec::new_in(allocator);
    params.push(FnParam { name: Atom::from(RENDER_FLAGS) });
    params.push(FnParam { name: Atom::from(CONTEXT_NAME) });
    params.push(FnParam { name: Atom::from("dirIndex") });

    // Create function name
    let fn_name = name.map(|n| {
        let formatted = format!("{n}_ContentQueries");
        Atom::from_in(formatted.as_str(), allocator)
    });

    OutputExpression::Function(Box::new_in(
        FunctionExpr { name: fn_name, params, statements: body, source_span: None },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::ReadVarExpr;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_query_flags() {
        let flags = QueryFlags::DESCENDANTS | QueryFlags::IS_STATIC;
        assert_eq!(flags.value(), 0b0011);

        let all_flags = QueryFlags::DESCENDANTS
            | QueryFlags::IS_STATIC
            | QueryFlags::EMIT_DISTINCT_CHANGES_ONLY;
        assert_eq!(all_flags.value(), 0b0111);
    }

    #[test]
    fn test_create_view_queries_function_empty() {
        let allocator = Allocator::default();
        let queries: &[R3QueryMetadata<'_>] = &[];

        let result = create_view_queries_function(&allocator, queries, Some("TestComponent"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        assert!(output.contains("TestComponent_Query"));
        assert!(output.contains("rf"));
        assert!(output.contains("ctx"));
    }

    #[test]
    fn test_create_content_queries_function_empty() {
        let allocator = Allocator::default();
        let queries: &[R3QueryMetadata<'_>] = &[];

        let result =
            create_content_queries_function(&allocator, queries, Some("TestDirective"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        assert!(output.contains("TestDirective_ContentQueries"));
        assert!(output.contains("rf"));
        assert!(output.contains("ctx"));
        assert!(output.contains("dirIndex"));
    }

    /// Test signal view query parameter order: target, predicate, flags
    #[test]
    fn test_signal_view_query_parameter_order() {
        let allocator = Allocator::default();

        // Create a signal query with a type predicate (e.g., SomeComponent)
        let query = R3QueryMetadata {
            property_name: Atom::from("myQuery"),
            first: true,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("SomeComponent"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: false,
            is_static: false,
            is_signal: true,
            read: None,
        };

        let queries = [query];
        let result =
            create_view_queries_function(&allocator, &queries, Some("TestComponent"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        println!("Signal view query output:\n{}", output);

        // Angular's expected order for signal queries:
        // viewQuerySignal(ctx.myQuery, SomeComponent, flags)
        //                 ↑ target    ↑ predicate   ↑ flags
        //
        // The output should contain: viewQuerySignal(ctx.myQuery,SomeComponent,1)
        // (flags=1 because descendants=true gives DESCENDANTS flag)
        assert!(
            output.contains("viewQuerySignal(ctx.myQuery,SomeComponent,1)"),
            "Signal query should have parameters in order: target, predicate, flags.\nGot:\n{}",
            output
        );
    }

    /// Test signal content query parameter order: dirIndex, target, predicate, flags
    #[test]
    fn test_signal_content_query_parameter_order() {
        let allocator = Allocator::default();

        // Create a signal content query with a type predicate
        let query = R3QueryMetadata {
            property_name: Atom::from("myContent"),
            first: true,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("ContentComponent"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: false,
            is_static: false,
            is_signal: true,
            read: None,
        };

        let queries = [query];
        let result =
            create_content_queries_function(&allocator, &queries, Some("TestDirective"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        println!("Signal content query output:\n{}", output);

        // Angular's expected order for signal content queries:
        // contentQuerySignal(dirIndex, ctx.myContent, ContentComponent, flags)
        //                    ↑ prepend ↑ target       ↑ predicate       ↑ flags
        // Remove whitespace for comparison since emitter may format differently
        let normalized = output.replace(['\n', ' '], "");
        assert!(
            normalized.contains("contentQuerySignal(dirIndex,ctx.myContent,ContentComponent,1)"),
            "Signal content query should have parameters: dirIndex, target, predicate, flags.\nGot:\n{}",
            output
        );
    }

    /// Test two chained signal view queries
    #[test]
    fn test_chained_signal_view_queries() {
        let allocator = Allocator::default();

        // Create two signal queries
        let query1 = R3QueryMetadata {
            property_name: Atom::from("query1"),
            first: true,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("Component1"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: false,
            is_static: false,
            is_signal: true,
            read: None,
        };

        let query2 = R3QueryMetadata {
            property_name: Atom::from("query2"),
            first: true,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("Component2"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: false,
            is_static: false,
            is_signal: true,
            read: None,
        };

        let queries = [query1, query2];
        let result =
            create_view_queries_function(&allocator, &queries, Some("TestComponent"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        println!("Chained signal queries output:\n{}", output);

        // Each signal query should be emitted as a separate statement.
        // Angular 20's ɵɵviewQuerySignal returns void, so chaining is not supported.
        let normalized = output.replace(['\n', ' '], "");
        assert!(
            normalized.contains("viewQuerySignal(ctx.query1,Component1,1);")
                && normalized.contains("viewQuerySignal(ctx.query2,Component2,1);"),
            "Each signal query should be a separate statement.\nGot:\n{}",
            output
        );
    }

    /// Regression test: Multiple non-signal view queries must be separate statements.
    ///
    /// Previously, multiple view queries were chained as ɵɵviewQuery(p1)(p2), calling
    /// the result of the first query as a function. Angular 20's ɵɵviewQuery returns void,
    /// so chaining breaks with: TypeError: ɵɵviewQuery(...) is not a function.
    ///
    /// The fix: Emit each query as a separate statement.
    #[test]
    fn test_multiple_non_signal_view_queries_are_separate_statements() {
        let allocator = Allocator::default();

        let query1 = R3QueryMetadata {
            property_name: Atom::from("myChild"),
            first: true,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("ChildComponent"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: true,
            is_static: false,
            is_signal: false,
            read: None,
        };

        let query2 = R3QueryMetadata {
            property_name: Atom::from("myOther"),
            first: false,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("OtherComponent"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: true,
            is_static: false,
            is_signal: false,
            read: None,
        };

        let queries = [query1, query2];
        let result =
            create_view_queries_function(&allocator, &queries, Some("TestComponent"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        let normalized = output.replace(['\n', ' '], "");

        // Each non-signal view query should be a separate statement (ending with ;),
        // NOT chained as ɵɵviewQuery(ChildComponent,5)(OtherComponent,5).
        assert!(
            normalized.contains("i0.ɵɵviewQuery(ChildComponent,5);"),
            "First view query should be a separate statement.\nGot:\n{}",
            output
        );
        assert!(
            normalized.contains("i0.ɵɵviewQuery(OtherComponent,5);"),
            "Second view query should be a separate statement.\nGot:\n{}",
            output
        );

        // Make sure they're NOT chained (the old buggy pattern)
        assert!(
            !normalized.contains("viewQuery(ChildComponent,5)(OtherComponent"),
            "View queries must NOT be chained (Angular 20 returns void).\nGot:\n{}",
            output
        );
    }

    /// Regression test: Multiple content queries must be separate statements.
    ///
    /// Same as the view query chaining bug, but for content queries.
    /// Angular 20's ɵɵcontentQuery also returns void, so chaining breaks.
    #[test]
    fn test_multiple_content_queries_are_separate_statements() {
        let allocator = Allocator::default();

        let query1 = R3QueryMetadata {
            property_name: Atom::from("items"),
            first: false,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("ItemComponent"), source_span: None },
                &allocator,
            ))),
            descendants: true,
            emit_distinct_changes_only: true,
            is_static: false,
            is_signal: false,
            read: None,
        };

        let query2 = R3QueryMetadata {
            property_name: Atom::from("headers"),
            first: true,
            predicate: QueryPredicate::Type(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from("HeaderComponent"), source_span: None },
                &allocator,
            ))),
            descendants: false,
            emit_distinct_changes_only: true,
            is_static: false,
            is_signal: false,
            read: None,
        };

        let queries = [query1, query2];
        let result =
            create_content_queries_function(&allocator, &queries, Some("TestDirective"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        let normalized = output.replace(['\n', ' '], "");

        // Each content query should be a separate statement (ending with ;),
        // NOT chained as ɵɵcontentQuery(dirIndex,ItemComponent,5)(dirIndex,HeaderComponent,4).
        assert!(
            normalized.contains("i0.ɵɵcontentQuery(dirIndex,ItemComponent,5);"),
            "First content query should be a separate statement.\nGot:\n{}",
            output
        );
        assert!(
            normalized.contains("i0.ɵɵcontentQuery(dirIndex,HeaderComponent,4);"),
            "Second content query should be a separate statement.\nGot:\n{}",
            output
        );

        // Make sure they're NOT chained
        assert!(
            !normalized.contains("contentQuery(dirIndex,ItemComponent,5)(dirIndex,HeaderComponent"),
            "Content queries must NOT be chained (Angular 20 returns void).\nGot:\n{}",
            output
        );
    }

    /// Test signal view query with string selector predicate
    #[test]
    fn test_signal_view_query_with_string_selector() {
        let allocator = Allocator::default();

        // Create a signal query with a string selector
        let mut selectors = Vec::new_in(&allocator);
        selectors.push(Atom::from("myRef"));

        let query = R3QueryMetadata {
            property_name: Atom::from("refQuery"),
            first: true,
            predicate: QueryPredicate::Selectors(selectors),
            descendants: true,
            emit_distinct_changes_only: false,
            is_static: false,
            is_signal: true,
            read: None,
        };

        let queries = [query];
        let result =
            create_view_queries_function(&allocator, &queries, Some("TestComponent"), None);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        println!("Signal query with string selector output:\n{}", output);

        // For string selectors, the predicate is an array like ['myRef']
        // Expected: viewQuerySignal(ctx.refQuery, ['myRef'], flags)
        assert!(
            output.contains("ctx.refQuery"),
            "Signal query should have ctx.refQuery as target.\nGot:\n{}",
            output
        );
        // Note: The emitter formats arrays without trailing commas: ["myRef"]
        assert!(
            output.contains(r#"["myRef""#),
            "Signal query should have string selector array as predicate.\nGot:\n{}",
            output
        );
    }
}

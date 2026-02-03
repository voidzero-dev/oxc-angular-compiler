//! Control flow and variable statement generation.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_span::Atom;

use crate::output::ast::{
    DeclareVarStmt, LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadVarExpr,
    StmtModifier,
};
use crate::r3::Identifiers;

use super::super::utils::create_instruction_call_stmt;

/// Creates an ɵɵconditionalCreate() call statement for the first branch in @if/@switch.
///
/// The conditionalCreate instruction takes:
/// - slot: The slot index for this branch
/// - templateFnRef: Reference to the template function for this branch
/// - decls: Number of declaration slots
/// - vars: Number of variable slots
/// - tag: Optional tag name (null for control flow blocks)
/// - constIndex: Optional const array index for attributes
/// - localRefs: Optional local refs index (not implemented yet)
///
/// Ported from Angular's `conditionalCreate()` in `instruction.ts`.
/// Args are trimmed from the end if they are null values.
pub fn create_conditional_create_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    fn_name: Option<Atom<'a>>,
    decls: Option<u32>,
    vars: Option<u32>,
    tag: Option<&Atom<'a>>,
    attributes: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Template function reference
    if let Some(name) = fn_name {
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name, source_span: None },
            allocator,
        )));
    } else {
        // Fallback placeholder
        let placeholder_str = allocator.alloc_str(&format!("_c{slot}"));
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from(placeholder_str), source_span: None },
            allocator,
        )));
    }

    // decls
    let decl_count = decls.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(decl_count as f64), source_span: None },
        allocator,
    )));

    // vars
    let var_count = vars.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(var_count as f64), source_span: None },
        allocator,
    )));

    // tag (string literal or null)
    if let Some(tag_name) = tag {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(tag_name.clone()), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // constIndex (attributes index into consts array, or null)
    if let Some(const_index) = attributes {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(const_index as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Trim trailing null arguments (matching Angular's behavior)
    while let Some(OutputExpression::Literal(lit)) = args.last() {
        if matches!(lit.value, LiteralValue::Null) {
            args.pop();
        } else {
            break;
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::CONDITIONAL_CREATE, args)
}

/// Creates an ɵɵconditional() update call statement.
///
/// The conditional instruction takes:
/// - test: The test expression that determines which branch to take
/// - context_value: Optional expression for alias capture (e.g., `@if (condition as alias)`)
pub fn create_conditional_update_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    test: OutputExpression<'a>,
    context_value: Option<OutputExpression<'a>>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(test);
    if let Some(ctx) = context_value {
        args.push(ctx);
    }
    create_instruction_call_stmt(allocator, Identifiers::CONDITIONAL, args)
}

/// Creates an ɵɵconditionalBranchCreate() call statement for branches after the first in @if/@switch.
///
/// The conditionalBranchCreate instruction takes:
/// - slot: The slot index for this branch
/// - templateFnRef: Reference to the template function for this branch
/// - decls: Number of declaration slots
/// - vars: Number of variable slots
/// - tag: Optional tag name (null for control flow blocks)
/// - constIndex: Optional const array index for attributes
/// - localRefs: Optional local refs index (not implemented yet)
///
/// Ported from Angular's `conditionalBranchCreate()` in `instruction.ts`.
/// Args are trimmed from the end if they are null values.
pub fn create_conditional_branch_create_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    fn_name: Option<Atom<'a>>,
    decls: Option<u32>,
    vars: Option<u32>,
    tag: Option<&Atom<'a>>,
    attributes: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Template function reference
    if let Some(name) = fn_name {
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name, source_span: None },
            allocator,
        )));
    } else {
        // Fallback placeholder
        let placeholder_str = allocator.alloc_str(&format!("_c{slot}"));
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from(placeholder_str), source_span: None },
            allocator,
        )));
    }

    // decls
    let decl_count = decls.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(decl_count as f64), source_span: None },
        allocator,
    )));

    // vars
    let var_count = vars.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(var_count as f64), source_span: None },
        allocator,
    )));

    // tag (string literal or null)
    if let Some(tag_name) = tag {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(tag_name.clone()), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // constIndex (attributes index into consts array, or null)
    if let Some(const_index) = attributes {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(const_index as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Trim trailing null arguments (matching Angular's behavior)
    while let Some(OutputExpression::Literal(lit)) = args.last() {
        if matches!(lit.value, LiteralValue::Null) {
            args.pop();
        } else {
            break;
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::CONDITIONAL_BRANCH_CREATE, args)
}

/// Creates an ɵɵcontrolCreate() call statement for control binding initialization.
///
/// This instruction determines whether a `[control]` binding targets a specialized
/// control directive on a native or custom form control, and if so, adds event
/// listeners to synchronize the bound form field to the form control.
///
/// The controlCreate instruction takes no arguments.
pub fn create_control_create_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    let args = OxcVec::new_in(allocator);
    create_instruction_call_stmt(allocator, Identifiers::CONTROL_CREATE, args)
}

/// Creates an ɵɵadvance() call statement.
pub fn create_advance_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    delta: u32,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    if delta > 1 {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(delta as f64), source_span: None },
            allocator,
        )));
    }
    create_instruction_call_stmt(allocator, Identifiers::ADVANCE, args)
}

/// Creates an ɵɵrepeater() call statement for @for update.
pub fn create_repeater_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    collection: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(collection);
    create_instruction_call_stmt(allocator, Identifiers::REPEATER, args)
}

/// Creates an ɵɵrepeaterCreate() call statement for @for with an OutputExpression for the track function.
///
/// This variant takes the track function as an OutputExpression, which allows
/// passing either a generated track function or a reference to an optimized built-in.
///
/// Ported from Angular's reifyTrackBy() handling in reify.ts and instruction.ts.
#[allow(clippy::too_many_arguments)]
pub fn create_repeater_create_stmt_with_track_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    fn_name: Option<Atom<'a>>,
    body_decl_count: Option<u32>,
    body_var_count: Option<u32>,
    tag: Option<&Atom<'a>>,
    attributes: Option<u32>,
    track_fn_expr: OutputExpression<'a>,
    uses_component_instance: bool,
    empty_fn_name: Option<Atom<'a>>,
    empty_decls: Option<u32>,
    empty_vars: Option<u32>,
    empty_tag: Option<&Atom<'a>>,
    empty_attributes: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Template function reference
    if let Some(name) = fn_name {
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name, source_span: None },
            allocator,
        )));
    } else {
        // Fallback placeholder
        let placeholder_str = allocator.alloc_str(&format!("_r{slot}"));
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from(placeholder_str), source_span: None },
            allocator,
        )));
    }

    // decls: Use the body view's declaration count, or default to 0
    let decls = body_decl_count.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(decls as f64), source_span: None },
        allocator,
    )));
    // vars: Use the body view's variable count
    let vars = body_var_count.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(vars as f64), source_span: None },
        allocator,
    )));

    // Tag (from control flow insertion point, for content projection)
    if let Some(tag_name) = tag {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(tag_name.clone()), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Attributes (const index for content projection)
    if let Some(const_index) = attributes {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(const_index as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Track by function expression
    args.push(track_fn_expr);

    // Uses component instance flag - only included if true OR there's an empty view
    // Per Angular's instruction.ts: if (trackByUsesComponentInstance || emptyViewFnName !== null)
    if uses_component_instance || empty_fn_name.is_some() {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Boolean(uses_component_instance),
                source_span: None,
            },
            allocator,
        )));

        // Empty view arguments (optional)
        if let Some(empty_name) = empty_fn_name {
            args.push(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: empty_name, source_span: None },
                allocator,
            )));
            // Empty decls
            let empty_decl_val = empty_decls.unwrap_or(0);
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(empty_decl_val as f64),
                    source_span: None,
                },
                allocator,
            )));
            // Empty vars
            let empty_var_val = empty_vars.unwrap_or(0);
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(empty_var_val as f64),
                    source_span: None,
                },
                allocator,
            )));

            // Empty tag (from control flow insertion point for @empty block)
            if empty_tag.is_some() || empty_attributes.is_some() {
                if let Some(empty_tag_name) = empty_tag {
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::String(empty_tag_name.clone()),
                            source_span: None,
                        },
                        allocator,
                    )));
                } else {
                    args.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Null, source_span: None },
                        allocator,
                    )));
                }
            }

            // Empty attributes (const index for @empty block)
            if let Some(empty_const_index) = empty_attributes {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::Number(empty_const_index as f64),
                        source_span: None,
                    },
                    allocator,
                )));
            }
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::REPEATER_CREATE, args)
}

/// Creates a variable declaration statement with a value.
/// All Variable ops use `const` (StmtModifier::Final), matching Angular's reify.ts.
pub fn create_variable_decl_stmt_with_value<'a>(
    allocator: &'a oxc_allocator::Allocator,
    name: &Atom<'a>,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    OutputStatement::DeclareVar(Box::new_in(
        DeclareVarStmt {
            name: name.clone(),
            value: Some(value),
            modifiers: StmtModifier::FINAL,
            leading_comment: None,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates an ɵɵdeclareLet() call statement.
pub fn create_declare_let_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));
    create_instruction_call_stmt(allocator, Identifiers::DECLARE_LET, args)
}

// Note: create_store_let_stmt has been removed.
// StoreLet as an update op should have been converted to a StoreLet expression
// during the store_let_optimization phase. If it reaches reify, it's a compiler bug.
// This matches Angular's behavior which throws: "AssertionError: unexpected storeLet"

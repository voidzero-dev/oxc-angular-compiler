//! i18n (internationalization) statement generation.

use oxc_allocator::{Box, Vec as OxcVec};

use crate::output::ast::{LiteralExpr, LiteralValue, OutputExpression, OutputStatement};
use crate::r3::Identifiers;

use super::super::utils::create_instruction_call_stmt;

/// Creates an ɵɵi18nStart() call statement.
///
/// The instruction has the following signature:
/// `ɵɵi18nStart(index: number, messageIndex: number, subTemplateIndex?: number): void`
///
/// - `index`: The slot index for the i18n block
/// - `messageIndex`: Index into the consts array for the i18n message
/// - `subTemplateIndex`: Optional index for nested templates within the i18n block
pub fn create_i18n_start_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    message_index: Option<u32>,
    sub_template_index: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(&allocator);

    // First arg: slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        &allocator,
    )));

    // Second arg: message index (required)
    if let Some(msg_idx) = message_index {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(msg_idx as f64), source_span: None },
            &allocator,
        )));

        // Third arg: sub-template index (optional)
        if let Some(sub_idx) = sub_template_index {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(sub_idx as f64), source_span: None },
                &allocator,
            )));
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::I18N_START, args)
}

/// Creates an ɵɵi18n() call statement for self-closing i18n on elements.
///
/// The instruction has the following signature:
/// `ɵɵi18n(index: number, messageIndex: number, subTemplateIndex?: number): void`
pub fn create_i18n_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    message_index: Option<u32>,
    sub_template_index: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(&allocator);

    // First arg: slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        &allocator,
    )));

    // Second arg: message index (required)
    if let Some(msg_idx) = message_index {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(msg_idx as f64), source_span: None },
            &allocator,
        )));

        // Third arg: sub-template index (optional)
        if let Some(sub_idx) = sub_template_index {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(sub_idx as f64), source_span: None },
                &allocator,
            )));
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::I18N, args)
}

/// Creates an ɵɵi18nEnd() call statement.
pub fn create_i18n_end_stmt<'a>(allocator: &'a oxc_allocator::Allocator) -> OutputStatement<'a> {
    create_instruction_call_stmt(allocator, Identifiers::I18N_END, OxcVec::new_in(&allocator))
}

/// Creates an ɵɵi18nExp() call statement.
pub fn create_i18n_exp_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    value: OutputExpression<'a>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(&allocator);
    args.push(value);
    create_instruction_call_stmt(allocator, Identifiers::I18N_EXP, args)
}

/// Creates an ɵɵi18nApply() call statement.
pub fn create_i18n_apply_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(&allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        &allocator,
    )));
    create_instruction_call_stmt(allocator, Identifiers::I18N_APPLY, args)
}

/// Creates an ɵɵi18nAttributes() call statement.
///
/// The instruction has the following signature:
/// `ɵɵi18nAttributes(index: number, attrsIndex: number): void`
///
/// - `index`: The slot index for the i18n attributes
/// - `attrsIndex`: Index into the consts array for the attribute config array
pub fn create_i18n_attributes_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    attrs_config_index: u32,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(&allocator);

    // First arg: slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        &allocator,
    )));

    // Second arg: attrs config index (index into consts array)
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(attrs_config_index as f64), source_span: None },
        &allocator,
    )));

    create_instruction_call_stmt(allocator, Identifiers::I18N_ATTRIBUTES, args)
}

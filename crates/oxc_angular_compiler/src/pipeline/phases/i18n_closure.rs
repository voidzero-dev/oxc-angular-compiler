//! Closure Compiler i18n support.
//!
//! This module provides helpers for generating Closure Compiler compatible i18n code.
//! The Angular compiler generates dual-mode code that supports both:
//! - Closure Compiler's `goog.getMsg()` translation system
//! - The standard `$localize` API
//!
//! The generated code looks like:
//! ```js
//! var i18n_1;
//! if (typeof ngI18nClosureMode !== "undefined" && ngI18nClosureMode) {
//!     /**
//!      * @desc description
//!      * @meaning meaning
//!      */
//!     var MSG_EXTERNAL_XXX = goog.getMsg(
//!         "Some message with {$interpolation}!",
//!         { "interpolation": "\uFFFD0\uFFFD" }
//!     );
//!     i18n_1 = MSG_EXTERNAL_XXX;
//! } else {
//!     i18n_1 = $localize`Some message with ${'\uFFFD0\uFFFD'}!`;
//! }
//! ```
//!
//! Ported from Angular's `render3/view/i18n/get_msg_utils.ts` and
//! `template/pipeline/src/phases/i18n_const_collection.ts`.

use oxc_allocator::{Box as AllocBox, Vec as AllocVec};
use oxc_str::Ident;

use crate::i18n::serializer::format_i18n_placeholder_name;
use crate::output::ast::{
    BinaryOperator, BinaryOperatorExpr, DeclareVarStmt, ExpressionStatement, IfStmt,
    InvokeFunctionExpr, JsDocComment, LeadingComment, LiteralExpr, LiteralMapEntry, LiteralMapExpr,
    LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr, StmtModifier,
    TypeofExpr,
};

/// Name of the global variable that is used to determine if we use Closure translations.
const NG_I18N_CLOSURE_MODE: &str = "ngI18nClosureMode";

/// Prefix for non-`goog.getMsg` i18n-related vars.
/// Note: the prefix uses lowercase characters intentionally due to a Closure behavior that
/// considers variables like `I18N_0` as constants and throws an error when their value changes.
const TRANSLATION_VAR_PREFIX: &str = "i18n_";

/// Closure variables holding messages must be named `MSG_[A-Z0-9]+`.
const CLOSURE_TRANSLATION_VAR_PREFIX: &str = "MSG_";

/// Creates the closure mode guard expression.
///
/// Generates: `typeof ngI18nClosureMode !== "undefined" && ngI18nClosureMode`
pub fn create_closure_mode_guard<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputExpression<'a> {
    // typeof ngI18nClosureMode
    let typeof_expr = OutputExpression::Typeof(AllocBox::new_in(
        TypeofExpr {
            expr: AllocBox::new_in(
                OutputExpression::ReadVar(AllocBox::new_in(
                    ReadVarExpr { name: Ident::from(NG_I18N_CLOSURE_MODE), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            source_span: None,
        },
        allocator,
    ));

    // "undefined"
    let undefined_literal = OutputExpression::Literal(AllocBox::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from("undefined")), source_span: None },
        allocator,
    ));

    // typeof ngI18nClosureMode !== "undefined"
    let not_undefined = OutputExpression::BinaryOperator(AllocBox::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::NotIdentical,
            lhs: AllocBox::new_in(typeof_expr, allocator),
            rhs: AllocBox::new_in(undefined_literal, allocator),
            source_span: None,
        },
        allocator,
    ));

    // ngI18nClosureMode
    let closure_mode_var = OutputExpression::ReadVar(AllocBox::new_in(
        ReadVarExpr { name: Ident::from(NG_I18N_CLOSURE_MODE), source_span: None },
        allocator,
    ));

    // typeof ngI18nClosureMode !== "undefined" && ngI18nClosureMode
    OutputExpression::BinaryOperator(AllocBox::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::And,
            lhs: AllocBox::new_in(not_undefined, allocator),
            rhs: AllocBox::new_in(closure_mode_var, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// I18n message metadata for JSDoc comments.
pub struct I18nMessageMeta<'a> {
    /// Message description for translators.
    pub description: Option<Ident<'a>>,
    /// Message meaning for disambiguation.
    pub meaning: Option<Ident<'a>>,
}

impl<'a> I18nMessageMeta<'a> {
    /// Creates a new I18n message metadata.
    pub fn new(description: Option<Ident<'a>>, meaning: Option<Ident<'a>>) -> Self {
        Self { description, meaning }
    }
}

/// Creates a JSDoc comment for Closure Compiler.
///
/// Generates:
/// ```js
/// /**
///  * @desc description
///  * @meaning meaning
///  */
/// ```
///
/// If no description is provided, adds `@suppress {msgDescriptions}` to suppress
/// Closure Compiler warnings about missing message descriptions.
pub fn create_i18n_jsdoc<'a>(
    allocator: &'a oxc_allocator::Allocator,
    meta: &I18nMessageMeta<'a>,
) -> LeadingComment<'a> {
    // Convert Option<Ident> to Option<Ident> with arena allocation
    let desc = meta.description.as_ref().map(|d| {
        let s = allocator.alloc_str(d.as_str());
        Ident::from(s)
    });
    let meaning = meta.meaning.as_ref().map(|m| {
        let s = allocator.alloc_str(m.as_str());
        Ident::from(s)
    });

    // Suppress msgDescriptions warning if no description is provided
    let suppress = desc.is_none();

    LeadingComment::JSDoc(JsDocComment {
        description: desc,
        meaning,
        suppress_msg_descriptions: suppress,
    })
}

/// Creates the goog.getMsg() call statements for Closure mode.
///
/// Generates:
/// ```js
/// /**
///  * @desc description
///  * @meaning meaning
///  */
/// var MSG_XXX = goog.getMsg("message", { placeholders });
/// i18n_X = MSG_XXX;
/// ```
pub fn create_goog_get_msg_statements<'a>(
    allocator: &'a oxc_allocator::Allocator,
    i18n_var_name: &Ident<'a>,
    closure_var_name: &Ident<'a>,
    message_string: &str,
    params: &[(String, String)],
    meta: Option<&I18nMessageMeta<'a>>,
) -> AllocVec<'a, OutputStatement<'a>> {
    let mut statements = AllocVec::new_in(allocator);

    // Build goog.getMsg arguments
    let mut goog_args = AllocVec::new_in(allocator);

    // First arg: message string with {$placeholder} format
    let message_str = allocator.alloc_str(message_string);
    goog_args.push(OutputExpression::Literal(AllocBox::new_in(
        LiteralExpr { value: LiteralValue::String(Ident::from(message_str)), source_span: None },
        allocator,
    )));

    // Second arg: placeholder values object (if any)
    if !params.is_empty() {
        let mut entries = AllocVec::new_in(allocator);
        for (name, value) in params {
            // Format placeholder name to camelCase for Closure
            let formatted_name = format_i18n_placeholder_name(name, true);
            let key_str = allocator.alloc_str(&formatted_name);
            let value_str = allocator.alloc_str(value);
            entries.push(LiteralMapEntry::new(
                Ident::from(key_str),
                OutputExpression::Literal(AllocBox::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from(value_str)),
                        source_span: None,
                    },
                    allocator,
                )),
                true,
            ));
        }
        goog_args.push(OutputExpression::LiteralMap(AllocBox::new_in(
            LiteralMapExpr { entries, source_span: None },
            allocator,
        )));
    }

    // goog.getMsg reference
    let goog_var = OutputExpression::ReadVar(AllocBox::new_in(
        ReadVarExpr { name: Ident::from("goog"), source_span: None },
        allocator,
    ));
    let goog_get_msg = OutputExpression::ReadProp(AllocBox::new_in(
        ReadPropExpr {
            receiver: AllocBox::new_in(goog_var, allocator),
            name: Ident::from("getMsg"),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // goog.getMsg(...)
    let goog_call = OutputExpression::InvokeFunction(AllocBox::new_in(
        InvokeFunctionExpr {
            fn_expr: AllocBox::new_in(goog_get_msg, allocator),
            args: goog_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create JSDoc comment if metadata is provided
    let leading_comment = meta.map(|m| create_i18n_jsdoc(allocator, m));

    // var MSG_XXX = goog.getMsg(...)
    statements.push(OutputStatement::DeclareVar(AllocBox::new_in(
        DeclareVarStmt {
            name: closure_var_name.clone(),
            value: Some(goog_call),
            modifiers: StmtModifier::FINAL, // const
            leading_comment,
            source_span: None,
        },
        allocator,
    )));

    // i18n_X = MSG_XXX
    let i18n_var = OutputExpression::ReadVar(AllocBox::new_in(
        ReadVarExpr { name: i18n_var_name.clone(), source_span: None },
        allocator,
    ));
    let closure_var = OutputExpression::ReadVar(AllocBox::new_in(
        ReadVarExpr { name: closure_var_name.clone(), source_span: None },
        allocator,
    ));
    let assignment = OutputExpression::BinaryOperator(AllocBox::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Assign,
            lhs: AllocBox::new_in(i18n_var, allocator),
            rhs: AllocBox::new_in(closure_var, allocator),
            source_span: None,
        },
        allocator,
    ));
    statements.push(OutputStatement::Expression(AllocBox::new_in(
        ExpressionStatement { expr: assignment, source_span: None },
        allocator,
    )));

    statements
}

/// Creates the $localize assignment statements for non-Closure mode.
///
/// Generates:
/// ```js
/// i18n_X = $localize`message`;
/// ```
pub fn create_localize_statements<'a>(
    allocator: &'a oxc_allocator::Allocator,
    i18n_var_name: &Ident<'a>,
    localized_expr: OutputExpression<'a>,
) -> AllocVec<'a, OutputStatement<'a>> {
    let mut statements = AllocVec::new_in(allocator);

    // i18n_X = $localize`...`
    let i18n_var = OutputExpression::ReadVar(AllocBox::new_in(
        ReadVarExpr { name: i18n_var_name.clone(), source_span: None },
        allocator,
    ));
    let assignment = OutputExpression::BinaryOperator(AllocBox::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Assign,
            lhs: AllocBox::new_in(i18n_var, allocator),
            rhs: AllocBox::new_in(localized_expr, allocator),
            source_span: None,
        },
        allocator,
    ));
    statements.push(OutputStatement::Expression(AllocBox::new_in(
        ExpressionStatement { expr: assignment, source_span: None },
        allocator,
    )));

    statements
}

/// Creates the complete translation declaration with dual-mode support.
///
/// Generates:
/// ```js
/// var i18n_X;
/// if (typeof ngI18nClosureMode !== "undefined" && ngI18nClosureMode) {
///     /**
///      * @desc description
///      * @meaning meaning
///      */
///     var MSG_XXX = goog.getMsg("message", { placeholders });
///     i18n_X = MSG_XXX;
/// } else {
///     i18n_X = $localize`message`;
/// }
/// ```
pub fn create_translation_declaration<'a>(
    allocator: &'a oxc_allocator::Allocator,
    i18n_var_name: Ident<'a>,
    closure_var_name: Ident<'a>,
    message_for_closure: &str,
    params: &[(String, String)],
    localized_expr: OutputExpression<'a>,
    meta: Option<&I18nMessageMeta<'a>>,
) -> AllocVec<'a, OutputStatement<'a>> {
    let mut statements = AllocVec::new_in(allocator);

    // var i18n_X;
    statements.push(OutputStatement::DeclareVar(AllocBox::new_in(
        DeclareVarStmt {
            name: i18n_var_name.clone(),
            value: None,
            modifiers: StmtModifier::NONE, // var (not const)
            leading_comment: None,
            source_span: None,
        },
        allocator,
    )));

    // Create the if statement
    let guard = create_closure_mode_guard(allocator);
    let true_case = create_goog_get_msg_statements(
        allocator,
        &i18n_var_name,
        &closure_var_name,
        message_for_closure,
        params,
        meta,
    );
    let false_case = create_localize_statements(allocator, &i18n_var_name, localized_expr);

    statements.push(OutputStatement::If(AllocBox::new_in(
        IfStmt { condition: guard, true_case, false_case, source_span: None },
        allocator,
    )));

    statements
}

/// Generates an i18n variable name for the standard (non-Closure) mode.
///
/// Uses lowercase prefix to avoid Closure treating it as a constant.
pub fn generate_i18n_var_name(index: usize) -> String {
    format!("{TRANSLATION_VAR_PREFIX}{index}")
}

/// Generates a Closure-compatible variable name for goog.getMsg.
///
/// When `use_external_ids` is true, generates: `MSG_EXTERNAL_{sanitized_id}$${suffix}`
/// When `use_external_ids` is false, generates: `MSG_{suffix}_{index}`
pub fn generate_closure_var_name(
    message_id: Option<&str>,
    file_suffix: &str,
    index: usize,
    use_external_ids: bool,
) -> String {
    if use_external_ids {
        if let Some(id) = message_id {
            let sanitized_id = sanitize_identifier(id);
            format!(
                "{CLOSURE_TRANSLATION_VAR_PREFIX}EXTERNAL_{sanitized_id}$${file_suffix}_{index}"
            )
        } else {
            format!("{CLOSURE_TRANSLATION_VAR_PREFIX}{file_suffix}_{index}")
        }
    } else {
        format!("{CLOSURE_TRANSLATION_VAR_PREFIX}{file_suffix}_{index}")
    }
}

/// Sanitizes a string to be used as an identifier.
///
/// Replaces non-alphanumeric characters with underscores.
fn sanitize_identifier(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// Generates a file-based i18n suffix from a file path.
///
/// Replaces non-alphanumeric characters with underscores and converts to uppercase.
pub fn generate_file_based_i18n_suffix(file_path: &str) -> String {
    file_path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect::<String>()
        + "_"
}

/// Serializes an i18n message for goog.getMsg format.
///
/// Converts placeholders to `{$placeholderName}` format (camelCase).
pub fn serialize_message_for_closure(message: &str, params: &[(String, String)]) -> String {
    let mut result = message.to_string();

    // Replace escape sequences with Closure placeholder format
    for (name, _value) in params {
        let formatted_name = format_i18n_placeholder_name(name, true);
        let placeholder = format!("{{${formatted_name}}}");
        // The escape sequence format: \uFFFD{index}\uFFFD
        // For now, we keep the original message and just format placeholders
        result = result.replace(&format!("\u{FFFD}{name}\u{FFFD}"), &placeholder);
    }

    result
}

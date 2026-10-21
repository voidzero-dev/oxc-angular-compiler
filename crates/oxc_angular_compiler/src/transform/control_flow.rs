//! Control flow block parsing utilities.
//!
//! This module provides parsing utilities for Angular control flow blocks:
//! - `@for` loops with `track` expressions and context variables
//! - `@switch` blocks with `@case` and `@default`
//! - `@if`/`@else if`/`@else` conditionals
//!
//! Ported from Angular's `render3/r3_control_flow.ts`.

use oxc_allocator::{Allocator, Vec};
use oxc_span::Span;
use oxc_str::Ident;

use crate::ast::expression::{ASTWithSource, AngularExpression};
use crate::ast::html::HtmlBlockParameter;
use crate::ast::r3::R3Variable;
use crate::parser::expression::BindingParser;

/// Allowed context variables in a @for loop.
pub const ALLOWED_FOR_LOOP_LET_VARIABLES: &[&str] =
    &["$index", "$first", "$last", "$even", "$odd", "$count"];

// =============================================================================
// Pattern matching helpers (replacing regex)
// =============================================================================

/// Parse "as" alias pattern: `^as\s+(.*)`
/// Returns the captured value after "as " if matched.
fn parse_as_alias(s: &str) -> Option<&str> {
    if !s.starts_with("as") {
        return None;
    }
    let after_as = &s[2..];
    if after_as.is_empty() || !after_as.chars().next()?.is_whitespace() {
        return None;
    }
    Some(after_as.trim_start())
}

/// Check if string matches Angular's `ELSE_IF_PATTERN`: `/^else[^\S\r\n]+if/`.
///
/// Any name starting with "else" followed by at least one whitespace character
/// and then "if" is treated as an else-if block. This means names like
/// "else ifx" also match, which is intentional to mirror Angular's behavior.
pub fn is_else_if_pattern(s: &str) -> bool {
    if !s.starts_with("else") {
        return false;
    }
    let after_else = &s[4..];
    // `trim_start` skips newlines too, so `else\nif` matches here where the
    // upstream regex rejects it — `get_block_name` already collapsed the
    // newline to a space, so callers never see it (documented divergence).
    let trimmed = after_else.trim_start();
    // Must have whitespace between "else" and "if"
    trimmed.len() < after_else.len() && trimmed.starts_with("if")
}

/// Check if string is a valid JavaScript identifier: `^[a-zA-Z_$][a-zA-Z0-9_$]*$`
fn is_valid_js_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Parse for-loop expression pattern: `^\s*([0-9A-Za-z_$]+)\s+of\s+([\S\s]+)`
/// Returns (variable_name, collection_expression) if matched.
fn parse_for_of_expression(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();

    // Find the variable name (identifier)
    let var_end = trimmed
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .last()
        .map(|(i, c)| i + c.len_utf8())?;

    if var_end == 0 {
        return None;
    }

    let var_name = &trimmed[..var_end];
    let after_var = &trimmed[var_end..];

    // Must have whitespace, then "of", then whitespace
    let trimmed_after = after_var.trim_start();
    if trimmed_after.len() == after_var.len() {
        return None; // No whitespace after variable
    }

    if !trimmed_after.starts_with("of") {
        return None;
    }

    let after_of = &trimmed_after[2..];
    if after_of.is_empty() || !after_of.chars().next()?.is_whitespace() {
        return None; // No whitespace after "of"
    }

    let expression = after_of.trim_start();
    if expression.is_empty() {
        return None;
    }

    Some((var_name, expression))
}

/// Parse "track" expression pattern: `^track\s+([\S\s]*)`
/// Returns the expression after "track " if matched, which may be empty.
/// An empty expression is valid as a match (Angular handles this by checking EmptyExpr).
/// Reference: r3_control_flow.ts FOR_LOOP_TRACK_PATTERN = /^track\s+([\S\s]*)/
fn parse_track_expression(s: &str) -> Option<&str> {
    if !s.starts_with("track") {
        return None;
    }
    let after_track = &s[5..];
    if after_track.is_empty() || !after_track.chars().next()?.is_whitespace() {
        return None;
    }
    let expr = after_track.trim_start();
    Some(expr)
}

/// Parse "let" expression pattern: `^let\s+([\S\s]+)`
/// Returns the expression after "let " if matched.
fn parse_let_expression(s: &str) -> Option<&str> {
    if !s.starts_with("let") {
        return None;
    }
    let after_let = &s[3..];
    if after_let.is_empty() || !after_let.chars().next()?.is_whitespace() {
        return None;
    }
    let expr = after_let.trim_start();
    if expr.is_empty() {
        return None;
    }
    Some(expr)
}

/// Parse time value pattern: `^\d+\.?\d*(ms|s)?$`
/// Returns true if the string is a valid time value.
fn is_valid_time_pattern(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }

    let bytes = s.as_bytes();
    let mut i = 0;

    // Must start with digit
    if !bytes[i].is_ascii_digit() {
        return false;
    }

    // Consume digits
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }

    // Optional decimal point and more digits
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }

    // Optional unit (ms or s)
    if i < bytes.len() {
        let remaining = &s[i..];
        remaining == "ms" || remaining == "s"
    } else {
        true
    }
}

/// Result of parsing @if/@else if conditional parameters.
pub struct ConditionalParams<'a> {
    /// The condition expression.
    pub expression: Option<ASTWithSource<'a>>,
    /// The alias variable from "as varName".
    pub expression_alias: Option<R3Variable<'a>>,
    /// Parse errors.
    pub errors: std::vec::Vec<String>,
}

/// Parses the parameters of an @if or @else if block.
///
/// Expected format: `@if (condition)` or `@if (condition; as varName)`
///
/// `block_name` is used to validate that "as" expressions are only on @if/@else if blocks.
pub fn parse_conditional_params<'a>(
    allocator: &'a Allocator,
    parameters: &[HtmlBlockParameter<'a>],
    binding_parser: &BindingParser<'a>,
    _block_start_span: Span,
    block_name: &str,
) -> ConditionalParams<'a> {
    let mut errors = std::vec::Vec::new();

    if parameters.is_empty() {
        errors.push("Conditional block does not have an expression".to_string());
        return ConditionalParams { expression: None, expression_alias: None, errors };
    }

    // First parameter is the condition expression
    // Note: The block parameter already contains the expression content from inside
    // the @if (...) - no stripping needed. For @if ((cond.expr)), the parameter
    // is already `(cond.expr)` since the outer parens are part of the @if syntax.
    let first_param = &parameters[0];
    let expr_str = first_param.expression.as_str();

    // Parse the condition expression
    // For conditional blocks, the expression is the full first parameter, so offset is 0
    let expression = Some(parse_expression_to_ast_with_source(
        &allocator,
        binding_parser,
        expr_str,
        first_param.span,
        0, // No prefix in conditional expression
    ));

    // Process additional parameters for "as" alias
    let mut expression_alias: Option<R3Variable<'a>> = None;

    // Start from 1 since we processed the first parameter already.
    for param in parameters.iter().skip(1) {
        let param_str = param.expression.as_str();

        // Check for "as" pattern: ^as\s+(.*)
        if let Some(alias_value) = parse_as_alias(param_str) {
            // Validate that "as" is only allowed on @if and @else if blocks
            if block_name != "if" && !is_else_if_pattern(block_name) {
                errors.push(
                    "\"as\" expression is only allowed on `@if` and `@else if` blocks".to_string(),
                );
                continue;
            }

            // For now conditionals can only have an `as` parameter.
            if expression_alias.is_some() {
                errors.push("Conditional can only have one \"as\" expression".to_string());
                continue;
            }

            let name_str = alias_value.trim();

            if name_str.is_empty() {
                errors
                    .push("\"as\" expression must have a valid JavaScript identifier".to_string());
                continue;
            }

            if is_valid_js_identifier(name_str) {
                // Create the alias variable
                // Calculate span for just the variable name (not including "as ")
                let as_prefix_len = param_str.find(name_str).unwrap_or(0) as u32;
                let name_span = Span::new(
                    param.span.start + as_prefix_len,
                    param.span.start + as_prefix_len + name_str.len() as u32,
                );
                let name_alloc = allocator.alloc_str(name_str);
                expression_alias = Some(R3Variable {
                    name: Ident::from(name_alloc),
                    value: Ident::from(name_alloc), // value same as name for alias
                    source_span: name_span,
                    key_span: name_span,
                    value_span: None, // No value span for expression alias
                });
            } else {
                errors.push(format!(
                    "\"as\" expression must be a valid JavaScript identifier, got \"{}\"",
                    name_str
                ));
            }
        } else {
            errors.push(format!("Unrecognized conditional block parameter \"{}\"", param_str));
        }
    }

    ConditionalParams { expression, expression_alias, errors }
}

/// Result of parsing @for loop parameters.
pub struct ForLoopParams<'a> {
    /// The loop item variable.
    pub item: R3Variable<'a>,
    /// The iterable expression.
    pub expression: ASTWithSource<'a>,
    /// The track expression (required).
    pub track_by: Option<TrackByInfo<'a>>,
    /// Context variables ($index, $first, etc.).
    pub context_variables: Vec<'a, R3Variable<'a>>,
    /// Parse errors.
    pub errors: std::vec::Vec<String>,
    /// Whether the core expression failed to parse (no parameters, unclosed parens, or missing "of").
    /// When true, secondary validations like missing track should be skipped.
    pub expression_parse_failed: bool,
}

/// Track expression info.
pub struct TrackByInfo<'a> {
    /// The track expression.
    pub expression: ASTWithSource<'a>,
    /// The "track" keyword span.
    pub keyword_span: Span,
}

/// Parses the parameters of a @for loop block.
///
/// Expected format: `@for (item of items; track item.id; let i = $index)`
pub fn parse_for_loop_parameters<'a>(
    allocator: &'a Allocator,
    parameters: &[HtmlBlockParameter<'a>],
    binding_parser: &BindingParser<'a>,
    block_start_span: Span,
) -> ForLoopParams<'a> {
    let mut errors = std::vec::Vec::new();

    if parameters.is_empty() {
        errors.push("@for loop does not have an expression".to_string());
        return ForLoopParams {
            item: create_empty_variable(allocator, block_start_span),
            expression: create_empty_ast_with_source(allocator, block_start_span),
            track_by: None,
            context_variables: create_default_context_variables(allocator, block_start_span),
            errors,
            expression_parse_failed: true,
        };
    }

    let expression_param = &parameters[0];
    let secondary_params = &parameters[1..];

    // Parse "item of items"
    let expr_str = expression_param.expression.as_str();
    let Some(stripped) = strip_optional_parentheses(expr_str, &mut errors) else {
        // Unclosed parentheses error was added in strip_optional_parentheses
        return ForLoopParams {
            item: create_empty_variable(allocator, block_start_span),
            expression: create_empty_ast_with_source(allocator, block_start_span),
            track_by: None,
            context_variables: create_default_context_variables(allocator, block_start_span),
            errors,
            expression_parse_failed: true,
        };
    };

    // Pattern: "item of items"
    let Some((item_name_str, raw_expression_str)) = parse_for_of_expression(&stripped) else {
        errors.push(
            "Cannot parse expression. @for loop expression must match the pattern \"<identifier> of <expression>\"".to_string()
        );
        return ForLoopParams {
            item: create_empty_variable(allocator, block_start_span),
            expression: create_empty_ast_with_source(allocator, block_start_span),
            track_by: None,
            context_variables: create_default_context_variables(allocator, block_start_span),
            errors,
            expression_parse_failed: true,
        };
    };

    // Create owned copies for later use
    let item_name_owned = item_name_str.to_string();

    // Validate item name is not a reserved context variable
    if ALLOWED_FOR_LOOP_LET_VARIABLES.contains(&item_name_str) {
        errors.push(format!(
            "@for loop item name cannot be one of {}.",
            ALLOWED_FOR_LOOP_LET_VARIABLES.join(", ")
        ));
    }

    // Create item variable - use the allocator to intern the string
    // Calculate a span that only covers the item name (not "of items.foo.bar")
    let item_name_atom = Ident::from(allocator.alloc_str(&item_name_owned));
    let item_name_span = Span::new(
        expression_param.span.start,
        expression_param.span.start + item_name_str.len() as u32,
    );
    let item = R3Variable {
        name: item_name_atom,
        value: Ident::from("$implicit"),
        source_span: item_name_span,
        key_span: item_name_span,
        value_span: None,
    };

    // Parse the iterable expression - allocate the expression string
    // Calculate the offset where the expression starts within the parameter
    // by finding the position of the expression in the original string
    // Reference: r3_control_flow.ts lines 585-602
    let raw_expression_alloc = allocator.alloc_str(raw_expression_str);
    let expression_offset = expr_str.rfind(raw_expression_str).map(|pos| pos as u32).unwrap_or(0);
    let expression = parse_expression_to_ast_with_source(
        &allocator,
        binding_parser,
        raw_expression_alloc,
        expression_param.span,
        expression_offset,
    );

    // Create default context variables
    let mut context_variables = create_default_context_variables(allocator, block_start_span);

    // Parse secondary parameters (track and let)
    let mut track_by: Option<TrackByInfo<'a>> = None;

    for param in secondary_params {
        let param_str = param.expression.as_str();

        // Check for "track expression"
        if let Some(track_expr_str) = parse_track_expression(param_str) {
            if track_by.is_some() {
                errors.push("@for loop can only have one \"track\" expression".to_string());
            } else {
                // Calculate offset where the expression starts within the parameter
                // by finding the position of the expression in the original string
                // Reference: r3_control_flow.ts lines 369-387
                let track_offset =
                    param_str.rfind(track_expr_str).map(|pos| pos as u32).unwrap_or(0);

                let track_expression = parse_expression_to_ast_with_source(
                    &allocator,
                    binding_parser,
                    track_expr_str,
                    param.span,
                    track_offset,
                );

                // Calculate keyword span (the "track" part)
                let keyword_span = Span::new(param.span.start, param.span.start + 5); // "track" is 5 chars

                // Validate track expression is not empty
                if matches!(&track_expression.ast, AngularExpression::Empty(_)) {
                    errors.push("@for loop must have a \"track\" expression".to_string());
                }

                // Validate no pipes in track expression
                if contains_pipe(&track_expression.ast) {
                    errors.push("Cannot use pipes in track expressions".to_string());
                }

                track_by = Some(TrackByInfo { expression: track_expression, keyword_span });
            }
            continue;
        }

        // Check for "let x = $index, y = $odd"
        if let Some(let_assignments) = parse_let_expression(param_str) {
            // Calculate span offset - skip "let " prefix
            let let_prefix = param_str.find(let_assignments).unwrap_or(0) as u32;
            let assignments_span = Span::new(
                param.span.start + let_prefix,
                param.span.start + let_prefix + let_assignments.len() as u32,
            );
            parse_let_parameter(
                &allocator,
                let_assignments,
                assignments_span,
                &item_name_owned,
                &mut context_variables,
                &mut errors,
            );
            continue;
        }

        // Unrecognized parameter
        errors.push(format!("Unrecognized @for loop parameter \"{}\"", param_str));
    }

    ForLoopParams {
        item,
        expression,
        track_by,
        context_variables,
        errors,
        expression_parse_failed: false,
    }
}

/// Parses the `let` parameter of a @for loop.
///
/// Format: "x = $index, y = $odd"
/// Adds new variables for user aliases (implicit variables are kept unchanged).
fn parse_let_parameter<'a>(
    allocator: &'a Allocator,
    expression: &str,
    span: Span,
    loop_item_name: &str,
    context_variables: &mut Vec<'a, R3Variable<'a>>,
    errors: &mut std::vec::Vec<String>,
) {
    // Track the current position within the expression to calculate individual spans
    let mut current_offset: u32 = 0;

    for part in expression.split(',') {
        // Find the position of this part in the expression
        let part_start_in_expr =
            expression[current_offset as usize..].find(part).map_or(0, |pos| pos as u32);
        let part_start = span.start + current_offset + part_start_in_expr;

        let trimmed = part.trim();
        if trimmed.is_empty() {
            current_offset += part.len() as u32 + 1; // +1 for comma
            continue;
        }

        // Use full split (not splitn) to detect malformed patterns like "a=b=c"
        // which has 3 segments. Angular checks expressionParts.length === 2.
        let parts: std::vec::Vec<&str> = trimmed.split('=').collect();
        if parts.len() != 2 {
            errors.push(
                "Invalid @for loop \"let\" parameter. Parameter should match the pattern \"<name> = <variable name>\"".to_string()
            );
            current_offset += part.len() as u32 + 1;
            continue;
        }

        let name = parts[0].trim();
        let variable_name = parts[1].trim();

        if name.is_empty() || variable_name.is_empty() {
            errors.push(
                "Invalid @for loop \"let\" parameter. Parameter should match the pattern \"<name> = <variable name>\"".to_string()
            );
            current_offset += part.len() as u32 + 1;
            continue;
        }

        if !ALLOWED_FOR_LOOP_LET_VARIABLES.contains(&variable_name) {
            errors.push(format!(
                "Unknown \"let\" parameter variable \"{}\". The allowed variables are: {}",
                variable_name,
                ALLOWED_FOR_LOOP_LET_VARIABLES.join(", ")
            ));
            current_offset += part.len() as u32 + 1;
            continue;
        }

        if name == loop_item_name {
            errors.push(format!(
                "Invalid @for loop \"let\" parameter. Variable cannot be called \"{}\"",
                loop_item_name
            ));
            current_offset += part.len() as u32 + 1;
            continue;
        }

        // Check for duplicate alias name
        // Angular checks all existing names including implicit context vars (e.g. $index).
        // Reference: r3_control_flow.ts line 479
        let already_has_name = context_variables.iter().any(|v| v.name.as_str() == name);
        if already_has_name {
            errors.push(format!("Duplicate \"let\" parameter variable \"{}\"", variable_name));
            current_offset += part.len() as u32 + 1;
            continue;
        }

        // Calculate individual spans for this variable assignment
        // Find positions within the trimmed part
        let trimmed_start_in_part = part.find(trimmed).map_or(0, |pos| pos as u32);
        let name_start = part_start + trimmed_start_in_part;
        let key_span = Span::new(name_start, name_start + name.len() as u32);

        // Find the value position (after '=')
        let eq_pos_in_trimmed = trimmed.find('=').map_or(0, |pos| pos as u32);
        let value_text_start_in_trimmed = eq_pos_in_trimmed + 1; // after '='
        let value_leading_ws = trimmed[(value_text_start_in_trimmed as usize)..].len()
            - trimmed[(value_text_start_in_trimmed as usize)..].trim_start().len();
        let value_start = name_start + eq_pos_in_trimmed + 1 + value_leading_ws as u32;
        let value_span = Span::new(value_start, value_start + variable_name.len() as u32);

        // Source span covers from name to end of value
        let source_span = Span::new(key_span.start, value_span.end);

        // Add a new variable for the user's alias (implicit variables stay unchanged)
        let name_alloc = allocator.alloc_str(name);
        let value_alloc = allocator.alloc_str(variable_name);
        context_variables.push(R3Variable {
            name: Ident::from(name_alloc),
            value: Ident::from(value_alloc),
            source_span,
            key_span,
            value_span: Some(value_span),
        });

        current_offset += part.len() as u32 + 1; // +1 for comma
    }
}

/// Creates the default context variables for a @for loop.
/// Context variables are given empty spans at the end of the block start,
/// since they are not explicitly defined in the template.
/// Reference: r3_control_flow.ts lines 334-347
fn create_default_context_variables<'a>(
    allocator: &'a Allocator,
    block_start_span: Span,
) -> Vec<'a, R3Variable<'a>> {
    let mut vars = Vec::with_capacity_in(ALLOWED_FOR_LOOP_LET_VARIABLES.len(), &allocator);

    // Empty span at the end of the block start - both start and end are the same position
    let empty_span = Span::new(block_start_span.end, block_start_span.end);

    for &var_name in ALLOWED_FOR_LOOP_LET_VARIABLES {
        vars.push(R3Variable {
            name: Ident::from(var_name),
            value: Ident::from(var_name),
            source_span: empty_span,
            key_span: empty_span,
            value_span: None,
        });
    }

    vars
}

/// Creates an empty variable for error recovery.
fn create_empty_variable<'a>(_allocator: &'a Allocator, span: Span) -> R3Variable<'a> {
    // Note: allocator reserved for future use (e.g., allocating default expression).
    R3Variable {
        name: Ident::from(""),
        value: Ident::from("$implicit"),
        source_span: span,
        key_span: span,
        value_span: None,
    }
}

/// Creates an empty ASTWithSource for error recovery.
fn create_empty_ast_with_source<'a>(allocator: &'a Allocator, span: Span) -> ASTWithSource<'a> {
    use crate::ast::expression::{AbsoluteSourceSpan, EmptyExpr, ParseSpan};

    ASTWithSource {
        ast: AngularExpression::Empty(oxc_allocator::Box::new_in(
            EmptyExpr {
                span: ParseSpan { start: span.start, end: span.end },
                source_span: AbsoluteSourceSpan::new(span.start, span.end),
            },
            &allocator,
        )),
        source: None,
        location: Ident::from(""),
        absolute_offset: span.start,
    }
}

/// Parses an expression string to an ASTWithSource.
/// The `expression_start_offset` is the offset from `span.start` where the expression
/// actually starts within the parameter (e.g., 6 for "track " prefix).
/// Reference: r3_control_flow.ts lines 577-604
fn parse_expression_to_ast_with_source<'a>(
    _allocator: &'a Allocator,
    binding_parser: &BindingParser<'a>,
    expr_str: &'a str,
    span: Span,
    expression_start_offset: u32,
) -> ASTWithSource<'a> {
    // Calculate the actual span for the expression by adding the offset
    let expr_span = Span::new(span.start + expression_start_offset, span.end);
    let result = binding_parser.parse_binding(expr_str, expr_span);

    ASTWithSource {
        ast: result.ast,
        source: Some(Ident::from(expr_str)),
        location: Ident::from(""),
        absolute_offset: span.start + expression_start_offset,
    }
}

/// Strips ALL optional parentheses from a for loop expression.
/// E.g., "(item of items)" -> "item of items"
/// E.g., "((  (item of items)  ))" -> "item of items"
///
/// Returns `None` if there are unclosed parentheses (error added to `errors`).
fn strip_optional_parentheses(expr: &str, errors: &mut std::vec::Vec<String>) -> Option<String> {
    let trimmed = expr.trim();
    let chars: std::vec::Vec<char> = trimmed.chars().collect();
    let mut open_parens = 0;
    let mut start = 0;
    let mut end = chars.len();

    // Count leading parens
    for (i, &c) in chars.iter().enumerate() {
        if c == '(' {
            open_parens += 1;
            start = i + 1;
        } else if !c.is_whitespace() {
            break;
        }
    }

    if open_parens == 0 {
        return Some(trimmed.to_string());
    }

    // Find matching closing parens
    for i in (0..chars.len()).rev() {
        if chars[i] == ')' {
            open_parens -= 1;
            end = i;
            if open_parens == 0 {
                break;
            }
        } else if !chars[i].is_whitespace() {
            break;
        }
    }

    if open_parens != 0 {
        // Unbalanced parens, report error
        errors.push("Unclosed parentheses in expression".to_string());
        return None;
    }

    // Collect and trim the inner content
    let inner: String = chars[start..end].iter().collect();
    Some(inner.trim().to_string())
}

/// Checks if an expression contains a pipe (full recursive traversal matching Angular's visitor).
fn contains_pipe(expr: &AngularExpression<'_>) -> bool {
    match expr {
        AngularExpression::BindingPipe(_) => true,
        AngularExpression::Binary(b) => contains_pipe(&b.left) || contains_pipe(&b.right),
        AngularExpression::Conditional(c) => {
            contains_pipe(&c.condition) || contains_pipe(&c.true_exp) || contains_pipe(&c.false_exp)
        }
        AngularExpression::PropertyRead(p) => contains_pipe(&p.receiver),
        AngularExpression::SafePropertyRead(p) => contains_pipe(&p.receiver),
        AngularExpression::KeyedRead(k) => contains_pipe(&k.receiver) || contains_pipe(&k.key),
        AngularExpression::SafeKeyedRead(k) => contains_pipe(&k.receiver) || contains_pipe(&k.key),
        AngularExpression::Call(f) => {
            contains_pipe(&f.receiver) || f.args.iter().any(|a| contains_pipe(a))
        }
        AngularExpression::SafeCall(f) => {
            contains_pipe(&f.receiver) || f.args.iter().any(|a| contains_pipe(a))
        }
        AngularExpression::PrefixNot(p) => contains_pipe(&p.expression),
        AngularExpression::Unary(u) => contains_pipe(&u.expr),
        AngularExpression::TypeofExpression(t) => contains_pipe(&t.expression),
        AngularExpression::LiteralArray(a) => a.expressions.iter().any(|e| contains_pipe(e)),
        AngularExpression::LiteralMap(m) => m.values.iter().any(|v| contains_pipe(v)),
        AngularExpression::Chain(c) => c.expressions.iter().any(|e| contains_pipe(e)),
        AngularExpression::Interpolation(i) => i.expressions.iter().any(|e| contains_pipe(e)),
        AngularExpression::VoidExpression(v) => contains_pipe(&v.expression),
        AngularExpression::NonNullAssert(n) => contains_pipe(&n.expression),
        AngularExpression::ParenthesizedExpression(p) => contains_pipe(&p.expression),
        AngularExpression::SpreadElement(s) => contains_pipe(&s.expression),
        AngularExpression::ArrowFunction(f) => contains_pipe(&f.body),
        AngularExpression::TaggedTemplateLiteral(t) => {
            contains_pipe(&t.tag) || t.template.expressions.iter().any(|e| contains_pipe(e))
        }
        AngularExpression::TemplateLiteral(t) => t.expressions.iter().any(|e| contains_pipe(e)),
        // Leaf nodes that cannot contain pipes
        AngularExpression::Empty(_)
        | AngularExpression::ImplicitReceiver(_)
        | AngularExpression::ThisReceiver(_)
        | AngularExpression::LiteralPrimitive(_)
        | AngularExpression::RegularExpressionLiteral(_) => false,
    }
}

// ============================================================================
// Defer Trigger Parsing
// ============================================================================

use crate::ast::r3::{
    R3BoundDeferredTrigger, R3DeferredBlockTriggers, R3HoverDeferredTrigger, R3IdleDeferredTrigger,
    R3ImmediateDeferredTrigger, R3InteractionDeferredTrigger, R3NeverDeferredTrigger,
    R3TimerDeferredTrigger, R3ViewportDeferredTrigger,
};
use crate::parser::expression::{Lexer, Token, TokenType};
use crate::util::ParseError;

/// Result of parsing defer block triggers.
pub struct DeferTriggerParseResult<'a> {
    /// Regular triggers (load on demand).
    pub triggers: R3DeferredBlockTriggers<'a>,
    /// Prefetch triggers.
    pub prefetch_triggers: R3DeferredBlockTriggers<'a>,
    /// Hydrate triggers.
    pub hydrate_triggers: R3DeferredBlockTriggers<'a>,
    /// Parse errors, in the order Angular reports them.
    pub errors: std::vec::Vec<DeferTriggerError>,
}

/// An error produced while parsing `@defer` triggers.
pub enum DeferTriggerError {
    /// A trigger error.
    Trigger {
        /// Where the error is reported (an absolute template span).
        span: Span,
        /// The error message.
        msg: String,
    },
    /// An error from parsing a trigger expression (`when` condition or `viewport` options).
    Expression(ParseError),
}

impl DeferTriggerError {
    fn new(span: Span, msg: impl Into<String>) -> Self {
        Self::Trigger { span, msg: msg.into() }
    }
}

// Pattern matching functions for Angular's r3_deferred_blocks.ts
// These handle whitespace/newlines between keywords correctly.
//
// The HTML lexer trims block parameters, while Angular matches its patterns
// against the untrimmed text. `has_trailing_ws` restores the trimmed trailing
// whitespace, so that e.g. `on ` matches `^on\s` but `on` does not.

/// Pattern `^keyword\s`.
fn starts_with_keyword(s: &str, keyword: &str, has_trailing_ws: bool) -> bool {
    s.strip_prefix(keyword)
        .is_some_and(|rest| rest.chars().next().map_or(has_trailing_ws, char::is_whitespace))
}

/// Pattern `^first\s+second\s`.
fn starts_with_keyword_pair(s: &str, first: &str, second: &str, has_trailing_ws: bool) -> bool {
    let Some(after_first) = s.strip_prefix(first) else {
        return false;
    };
    let trimmed = after_first.trim_start();
    // Must have at least one whitespace after the first keyword
    trimmed.len() != after_first.len() && starts_with_keyword(trimmed, second, has_trailing_ws)
}

/// Pattern to identify a `hydrate never` trigger: `^hydrate\s+never(\s*)$`.
fn is_hydrate_never_pattern(s: &str) -> bool {
    let Some(after_hydrate) = s.strip_prefix("hydrate") else {
        return false;
    };
    let trimmed = after_hydrate.trim_start();
    if trimmed.len() == after_hydrate.len() {
        return false; // No whitespace after "hydrate"
    }
    trimmed.strip_prefix("never").is_some_and(|rest| rest.trim().is_empty())
}

/// Gets the index within an expression at which the trigger parameters start.
/// Starting at `start_position`, skips to the first non-whitespace character that
/// follows at least one whitespace character.
fn get_trigger_parameters_start(value: &str, start_position: usize) -> Option<usize> {
    let mut has_found_separator = false;

    for (i, ch) in value.char_indices().skip_while(|(i, _)| *i < start_position) {
        if ch.is_whitespace() {
            has_found_separator = true;
        } else if has_found_separator {
            return Some(i);
        }
    }

    None
}

/// Parses all triggers from @defer block parameters.
/// Reference: Angular's r3_deferred_blocks.ts parsePrimaryTriggers function
pub fn parse_defer_triggers<'a>(
    allocator: &'a Allocator,
    parameters: &[HtmlBlockParameter<'a>],
    binding_parser: &BindingParser<'a>,
    block_start_span: Span,
) -> DeferTriggerParseResult<'a> {
    let mut triggers = R3DeferredBlockTriggers::default();
    let mut prefetch_triggers = R3DeferredBlockTriggers::default();
    let mut hydrate_triggers = R3DeferredBlockTriggers::default();
    let mut errors = std::vec::Vec::new();

    for param in parameters {
        let expr = param.expression.as_str();
        let span = param.span;
        // The parameter span still covers the whitespace trimmed off the expression.
        let has_trailing_ws = span.size() as usize > expr.len();
        let param = TriggerParameter { expression: expr, span };

        // Match against patterns in the same order as Angular's TypeScript implementation
        // Reference: r3_deferred_blocks.ts parsePrimaryTriggers
        if starts_with_keyword(expr, "when", has_trailing_ws) {
            parse_when_trigger(&param, binding_parser, &mut triggers, &mut errors);
        } else if starts_with_keyword(expr, "on", has_trailing_ws) {
            parse_on_trigger(allocator, &param, binding_parser, &mut triggers, &mut errors);
        } else if starts_with_keyword_pair(expr, "prefetch", "when", has_trailing_ws) {
            parse_when_trigger(&param, binding_parser, &mut prefetch_triggers, &mut errors);
        } else if starts_with_keyword_pair(expr, "prefetch", "on", has_trailing_ws) {
            parse_on_trigger(
                allocator,
                &param,
                binding_parser,
                &mut prefetch_triggers,
                &mut errors,
            );
        } else if starts_with_keyword_pair(expr, "hydrate", "when", has_trailing_ws) {
            parse_when_trigger(&param, binding_parser, &mut hydrate_triggers, &mut errors);
        } else if starts_with_keyword_pair(expr, "hydrate", "on", has_trailing_ws) {
            parse_on_trigger(allocator, &param, binding_parser, &mut hydrate_triggers, &mut errors);
        } else if is_hydrate_never_pattern(expr) {
            parse_never_trigger(&param, &mut hydrate_triggers, &mut errors);
        } else {
            errors.push(DeferTriggerError::new(span, "Unrecognized trigger"));
        }
    }

    // Validate that `hydrate never` is not combined with other hydrate triggers
    if hydrate_triggers.never.is_some() && has_other_hydrate_triggers(&hydrate_triggers) {
        errors.push(DeferTriggerError::new(
            block_start_span,
            "Cannot specify additional `hydrate` triggers if `hydrate never` is present",
        ));
    }

    DeferTriggerParseResult { triggers, prefetch_triggers, hydrate_triggers, errors }
}

/// Checks if there are any hydrate triggers besides `never`.
fn has_other_hydrate_triggers(triggers: &R3DeferredBlockTriggers<'_>) -> bool {
    triggers.when.is_some()
        || triggers.idle.is_some()
        || triggers.immediate.is_some()
        || triggers.hover.is_some()
        || triggers.timer.is_some()
        || triggers.interaction.is_some()
        || triggers.viewport.is_some()
}

/// A `@defer` block parameter holding a trigger.
struct TriggerParameter<'a> {
    /// The (trimmed) parameter text.
    expression: &'a str,
    /// The parameter span, including trimmed trailing whitespace.
    span: Span,
}

impl TriggerParameter<'_> {
    /// Span of the `prefetch` keyword, if the parameter starts with it.
    fn prefetch_span(&self) -> Option<Span> {
        self.keyword_span("prefetch")
    }

    /// Span of the `hydrate` keyword, if the parameter starts with it.
    fn hydrate_span(&self) -> Option<Span> {
        self.keyword_span("hydrate")
    }

    fn keyword_span(&self, keyword: &str) -> Option<Span> {
        self.expression
            .starts_with(keyword)
            .then(|| Span::new(self.span.start, self.span.start + keyword.len() as u32))
    }

    /// Span of `len` bytes starting `offset` bytes into the parameter.
    fn span_at(&self, offset: usize, len: usize) -> Span {
        let start = self.span.start + offset as u32;
        Span::new(start, start + len as u32)
    }
}

/// Records a trigger, reporting an error if one of the same kind already exists.
fn track_trigger<T>(
    name: &str,
    slot: &mut Option<T>,
    trigger: T,
    source_span: Span,
    errors: &mut std::vec::Vec<DeferTriggerError>,
) {
    if slot.is_some() {
        errors.push(DeferTriggerError::new(
            source_span,
            format!("Duplicate \"{name}\" trigger is not allowed"),
        ));
    } else {
        *slot = Some(trigger);
    }
}

/// Parses a `hydrate never` trigger.
fn parse_never_trigger(
    param: &TriggerParameter<'_>,
    triggers: &mut R3DeferredBlockTriggers<'_>,
    errors: &mut std::vec::Vec<DeferTriggerError>,
) {
    // This is here just to be safe, we shouldn't enter this function
    // in the first place if a block doesn't have the "never" keyword.
    let Some(never_index) = param.expression.find("never") else {
        errors.push(DeferTriggerError::new(
            param.span,
            "Could not find \"never\" keyword in expression",
        ));
        return;
    };

    let never_span = param.span_at(never_index, "never".len());
    let trigger = R3NeverDeferredTrigger {
        source_span: param.span,
        name_span: Some(never_span),
        prefetch_span: param.prefetch_span(),
        when_or_on_source_span: None,
        hydrate_span: param.hydrate_span(),
    };
    track_trigger("never", &mut triggers.never, trigger, param.span, errors);
}

/// Parses a `when` trigger.
fn parse_when_trigger<'a>(
    param: &TriggerParameter<'a>,
    binding_parser: &BindingParser<'a>,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<DeferTriggerError>,
) {
    // This is here just to be safe, we shouldn't enter this function
    // in the first place if a block doesn't have the "when" keyword.
    let Some(when_index) = param.expression.find("when") else {
        errors.push(DeferTriggerError::new(
            param.span,
            "Could not find \"when\" keyword in expression",
        ));
        return;
    };

    // When no condition follows the keyword (`when ` with only whitespace after it),
    // Angular slices from index -1 and parses the trailing whitespace into an empty
    // `when` trigger, which ngtsc then fails to compile ("Unsupported reification of
    // ir.Expression kind: EmptyExpr"). Accept it without a trigger instead.
    let Some(start) = get_trigger_parameters_start(param.expression, when_index + 1) else {
        return;
    };
    let condition = &param.expression[start..];
    let parsed = binding_parser
        .parse_binding(condition, Span::new(param.span.start + start as u32, param.span.end));
    errors.extend(parsed.errors.into_iter().map(DeferTriggerError::Expression));

    let trigger = R3BoundDeferredTrigger {
        value: parsed.ast,
        source_span: param.span,
        prefetch_span: param.prefetch_span(),
        when_source_span: param.span_at(when_index, "when".len()),
        hydrate_span: param.hydrate_span(),
    };
    track_trigger("when", &mut triggers.when, trigger, param.span, errors);
}

/// Parses an `on` trigger.
fn parse_on_trigger<'a>(
    allocator: &'a Allocator,
    param: &TriggerParameter<'a>,
    binding_parser: &BindingParser<'a>,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<DeferTriggerError>,
) {
    // This is here just to be safe, we shouldn't enter this function
    // in the first place if a block doesn't have the "on" keyword.
    let Some(on_index) = param.expression.find("on") else {
        errors.push(DeferTriggerError::new(
            param.span,
            "Could not find \"on\" keyword in expression",
        ));
        return;
    };

    // Without anything after the keyword, Angular tokenizes just the trailing
    // whitespace character, which yields no tokens and therefore no triggers.
    let Some(start) = get_trigger_parameters_start(param.expression, on_index + 1) else {
        return;
    };

    let is_hydration_trigger = param.expression.starts_with("hydrate");
    let mut parser = OnTriggerParser {
        allocator,
        binding_parser,
        expression: param.expression,
        start,
        span: param.span,
        tokens: Lexer::new(allocator, &param.expression[start..]).tokenize(),
        index: 0,
        triggers,
        errors,
        is_hydration_trigger,
        prefetch_span: param.prefetch_span(),
        on_source_span: param.span_at(on_index, "on".len()),
        hydrate_span: param.hydrate_span(),
    };
    parser.parse();
}

/// Reference and options of a `viewport` trigger.
type ViewportReferenceAndOptions<'a> = (Option<Ident<'a>>, Option<AngularExpression<'a>>);

/// Parsed information about a defer trigger parameter.
struct ParsedParameter<'a> {
    /// Expression of the parameter.
    expression: &'a str,
    /// Index within the trigger at which the parameter starts.
    start: u32,
}

/// Port of Angular's `OnTriggerParser`: parses the comma-separated trigger
/// list of an `on`, `prefetch on` or `hydrate on` parameter from the
/// expression lexer's tokens.
struct OnTriggerParser<'p, 'a> {
    allocator: &'a Allocator,
    binding_parser: &'p BindingParser<'a>,
    /// The whole parameter text.
    expression: &'a str,
    /// Index within `expression` at which the trigger list starts.
    start: usize,
    /// Span of the whole parameter.
    span: Span,
    /// Tokens of `expression[start..]`.
    tokens: std::vec::Vec<Token<'a>>,
    index: usize,
    triggers: &'p mut R3DeferredBlockTriggers<'a>,
    errors: &'p mut std::vec::Vec<DeferTriggerError>,
    is_hydration_trigger: bool,
    prefetch_span: Option<Span>,
    on_source_span: Span,
    hydrate_span: Option<Span>,
}

/// Pairs of characters that form syntax that is comma-delimited.
fn comma_delimited_closer(token: &Token<'_>) -> Option<char> {
    if token.token_type != TokenType::Character {
        return None;
    }
    if token.is_character('{') {
        Some('}') // Object literals
    } else if token.is_character('[') {
        Some(']') // Array literals
    } else if token.is_character('(') {
        Some(')') // Function calls
    } else {
        None
    }
}

/// Angular's `Token.toString()`, used in "Unexpected token" errors.
fn token_text(token: &Token<'_>) -> String {
    match token.token_type {
        TokenType::Number => token.num_value.to_string(),
        TokenType::NoSubstitutionTemplate
        | TokenType::TemplateHead
        | TokenType::TemplateMiddle
        | TokenType::TemplateTail => "null".to_string(),
        _ => token.str_value.to_string(),
    }
}

impl<'a> OnTriggerParser<'_, 'a> {
    fn parse(&mut self) {
        while !self.tokens.is_empty() && self.index < self.tokens.len() {
            let token = self.token().clone();

            if !token.is_identifier() {
                self.unexpected_token(&token);
                break;
            }

            // An identifier immediately followed by a comma or the end of
            // the expression cannot have parameters so we can exit early.
            if self.is_followed_by_or_last(',') {
                self.consume_trigger(&token, &[]);
                self.advance();
            } else if self.is_followed_by_or_last('(') {
                self.advance(); // Advance to the opening paren.
                let prev_errors = self.errors.len();
                let parameters = self.consume_parameters();
                if self.errors.len() != prev_errors {
                    break;
                }
                self.consume_trigger(&token, &parameters);
                self.advance(); // Advance past the closing paren.
            } else if self.index < self.tokens.len() - 1 {
                let next = self.tokens[self.index + 1].clone();
                self.unexpected_token(&next);
            }

            self.advance();
        }
    }

    fn advance(&mut self) {
        self.index += 1;
    }

    fn is_followed_by_or_last(&self, ch: char) -> bool {
        if self.index == self.tokens.len() - 1 {
            return true;
        }
        self.tokens[self.index + 1].is_character(ch)
    }

    fn token(&self) -> &Token<'a> {
        &self.tokens[self.index.min(self.tokens.len() - 1)]
    }

    fn consume_trigger(&mut self, identifier: &Token<'a>, parameters: &[ParsedParameter<'a>]) {
        let name_start =
            self.span.start + self.start as u32 + identifier.index - self.tokens[0].index;
        let name_span = Span::new(name_start, name_start + identifier.str_value.len() as u32);
        let end = name_start + (self.token().end - identifier.index);

        // Put the prefetch and on spans with the first trigger
        // This should maybe be refactored to have something like an outer OnGroup AST
        // Since triggers can be grouped with commas "on hover(x), interaction(y)"
        let is_first_trigger = identifier.index == 0;
        let source_span =
            Span::new(if is_first_trigger { self.span.start } else { name_start }, end);

        let result = match identifier.str_value.as_str() {
            "idle" => create_idle_trigger(parameters).map(|timeout| {
                let trigger = R3IdleDeferredTrigger {
                    timeout,
                    source_span,
                    name_span: Some(name_span),
                    prefetch_span: if is_first_trigger { self.prefetch_span } else { None },
                    when_or_on_source_span: is_first_trigger.then_some(self.on_source_span),
                    hydrate_span: if is_first_trigger { self.hydrate_span } else { None },
                };
                track_trigger("idle", &mut self.triggers.idle, trigger, source_span, self.errors);
            }),
            "timer" => create_timer_trigger(parameters).map(|delay| {
                let trigger = R3TimerDeferredTrigger {
                    delay,
                    source_span,
                    name_span,
                    prefetch_span: self.prefetch_span,
                    on_source_span: Some(self.on_source_span),
                    hydrate_span: self.hydrate_span,
                };
                track_trigger("timer", &mut self.triggers.timer, trigger, source_span, self.errors);
            }),
            "interaction" => {
                self.validate_reference_based_trigger("interaction", parameters).map(|()| {
                    let trigger = R3InteractionDeferredTrigger {
                        reference: parameters.first().map(|p| Ident::from(p.expression)),
                        source_span,
                        name_span,
                        prefetch_span: self.prefetch_span,
                        on_source_span: Some(self.on_source_span),
                        hydrate_span: self.hydrate_span,
                    };
                    track_trigger(
                        "interaction",
                        &mut self.triggers.interaction,
                        trigger,
                        source_span,
                        self.errors,
                    );
                })
            }
            "immediate" => create_immediate_trigger(parameters).map(|()| {
                let trigger = R3ImmediateDeferredTrigger {
                    source_span,
                    name_span: Some(name_span),
                    prefetch_span: self.prefetch_span,
                    when_or_on_source_span: Some(self.on_source_span),
                    hydrate_span: self.hydrate_span,
                };
                track_trigger(
                    "immediate",
                    &mut self.triggers.immediate,
                    trigger,
                    source_span,
                    self.errors,
                );
            }),
            "hover" => self.validate_reference_based_trigger("hover", parameters).map(|()| {
                let trigger = R3HoverDeferredTrigger {
                    reference: parameters.first().map(|p| Ident::from(p.expression)),
                    source_span,
                    name_span,
                    prefetch_span: self.prefetch_span,
                    on_source_span: Some(self.on_source_span),
                    hydrate_span: self.hydrate_span,
                };
                track_trigger("hover", &mut self.triggers.hover, trigger, source_span, self.errors);
            }),
            "viewport" => {
                self.create_viewport_trigger(parameters, source_span).map(|(reference, options)| {
                    let trigger = R3ViewportDeferredTrigger {
                        reference,
                        options,
                        source_span,
                        name_span,
                        prefetch_span: self.prefetch_span,
                        on_source_span: Some(self.on_source_span),
                        hydrate_span: self.hydrate_span,
                    };
                    track_trigger(
                        "viewport",
                        &mut self.triggers.viewport,
                        trigger,
                        source_span,
                        self.errors,
                    );
                })
            }
            // `never` is only valid as `hydrate never`, so it is unrecognized here too.
            name => Err(format!("Unrecognized trigger type \"{name}\"")),
        };

        if let Err(msg) = result {
            self.error(identifier, msg);
        }
    }

    fn consume_parameters(&mut self) -> std::vec::Vec<ParsedParameter<'a>> {
        let mut parameters = std::vec::Vec::new();

        if !self.token().is_character('(') {
            let token = self.token().clone();
            self.unexpected_token(&token);
            return parameters;
        }

        self.advance();

        let mut comma_delim_stack: std::vec::Vec<char> = std::vec::Vec::new();
        // Index range (into `self.tokens`) of the current parameter's tokens.
        let mut param_tokens: Option<(usize, usize)> = None;

        while self.index < self.tokens.len() {
            let token = self.token();

            // Stop parsing if we've hit the end character and we're outside of a comma-delimited
            // syntax. Note that we don't need to account for strings here since the lexer already
            // parsed them into string tokens.
            if token.is_character(')') && comma_delim_stack.is_empty() {
                if let Some(range) = param_tokens {
                    parameters.push(self.token_range_parameter(range));
                }
                break;
            }

            // In the `on` microsyntax "top-level" commas (e.g. ones outside of an parameters)
            // separate the different triggers (e.g. `on idle,timer(500)`). This is problematic,
            // because the function-like syntax also implies that multiple parameters can be passed
            // into the individual trigger (e.g. `on foo(a, b)`). To avoid tripping up the parser
            // with commas that are part of other sorts of syntax (object literals, arrays), we
            // treat anything inside a comma-delimited syntax block as plain text.
            if let Some(closer) = comma_delimited_closer(token) {
                comma_delim_stack.push(closer);
            }

            if comma_delim_stack.last().is_some_and(|&closer| token.is_character(closer)) {
                comma_delim_stack.pop();
            }

            // If we hit a comma outside of a comma-delimited syntax, it means
            // that we're at the top level and we're starting a new parameter.
            if comma_delim_stack.is_empty()
                && token.is_character(',')
                && let Some(range) = param_tokens.take()
            {
                parameters.push(self.token_range_parameter(range));
                self.advance();
                continue;
            }

            // Otherwise treat the token as a plain text character in the current parameter.
            param_tokens = Some(match param_tokens {
                Some((first, _)) => (first, self.index),
                None => (self.index, self.index),
            });
            self.advance();
        }

        if !self.token().is_character(')') || !comma_delim_stack.is_empty() {
            let token = self.token().clone();
            self.error(&token, "Unexpected end of expression");
        }

        if self.index + 1 < self.tokens.len() && !self.tokens[self.index + 1].is_character(',') {
            let next = self.tokens[self.index + 1].clone();
            self.unexpected_token(&next);
        }

        parameters
    }

    /// Creates a parameter from the source text covered by a range of tokens.
    fn token_range_parameter(&self, (first, last): (usize, usize)) -> ParsedParameter<'a> {
        let first = &self.tokens[first];
        let last = &self.tokens[last];
        ParsedParameter {
            expression: &self.expression
                [self.start + first.index as usize..self.start + last.end as usize],
            start: first.index,
        }
    }

    fn error(&mut self, token: &Token<'_>, msg: impl Into<String>) {
        let start = self.span.start + self.start as u32 + token.index;
        let span = Span::new(start, start + (token.end - token.index));
        self.errors.push(DeferTriggerError::new(span, msg));
    }

    fn unexpected_token(&mut self, token: &Token<'_>) {
        self.error(token, format!("Unexpected token \"{}\"", token_text(token)));
    }

    /// Checks the structure of a reference-based trigger (`validatePlainReferenceBasedTrigger`
    /// or `validateHydrateReferenceBasedTrigger`, depending on the trigger kind).
    fn validate_reference_based_trigger(
        &self,
        name: &str,
        parameters: &[ParsedParameter<'_>],
    ) -> Result<(), String> {
        if self.is_hydration_trigger {
            if name == "viewport" {
                if parameters.len() > 1 {
                    return Err(format!(
                        "Hydration trigger \"{name}\" cannot have more than one parameter"
                    ));
                }
            } else if !parameters.is_empty() {
                return Err(format!("Hydration trigger \"{name}\" cannot have parameters"));
            }
        } else if parameters.len() > 1 {
            return Err(format!("\"{name}\" trigger can only have zero or one parameters"));
        }
        Ok(())
    }

    /// Returns the `viewport` trigger's reference and options.
    fn create_viewport_trigger(
        &mut self,
        parameters: &[ParsedParameter<'a>],
        source_span: Span,
    ) -> Result<ViewportReferenceAndOptions<'a>, String> {
        use crate::ast::expression::{LiteralMap, LiteralMapKey};

        self.validate_reference_based_trigger("viewport", parameters)?;

        let (reference, options) = match parameters.first() {
            None => (None, None),
            Some(param) if !param.expression.starts_with('{') => {
                (Some(Ident::from(param.expression)), None)
            }
            Some(param) => {
                let offset = source_span.start + self.start as u32 + param.start;
                let parsed = self.binding_parser.parse_binding(
                    param.expression,
                    Span::new(offset, offset + param.expression.len() as u32),
                );
                self.errors.extend(parsed.errors.into_iter().map(DeferTriggerError::Expression));

                let AngularExpression::LiteralMap(map) = parsed.ast else {
                    return Err(
                        "Options parameter of the \"viewport\" trigger must be an object literal"
                            .to_string(),
                    );
                };
                let is_property = |key: &LiteralMapKey<'_>, name: &str| matches!(key, LiteralMapKey::Property(prop) if prop.key == name);
                if map.keys.iter().any(|key| matches!(key, LiteralMapKey::Spread(_))) {
                    return Err("Spread operator are not allowed in this context".to_string());
                }
                if map.keys.iter().any(|key| is_property(key, "root")) {
                    return Err("The \"root\" option is not supported in the options parameter of the \"viewport\" trigger".to_string());
                }

                match map.keys.iter().position(|key| is_property(key, "trigger")) {
                    None => (None, Some(AngularExpression::LiteralMap(map))),
                    Some(trigger_index) => {
                        let reference = match &map.values[trigger_index] {
                            AngularExpression::PropertyRead(read)
                                if matches!(
                                    read.receiver,
                                    AngularExpression::ImplicitReceiver(_)
                                ) =>
                            {
                                read.name
                            }
                            _ => {
                                return Err("\"trigger\" option of the \"viewport\" trigger must be an identifier".to_string());
                            }
                        };

                        let LiteralMap { span, source_span, keys, values } = map.unbox();
                        let mut filtered_keys = Vec::new_in(&self.allocator);
                        let mut filtered_values = Vec::new_in(&self.allocator);
                        for (index, (key, value)) in keys.into_iter().zip(values).enumerate() {
                            if index != trigger_index {
                                filtered_keys.push(key);
                                filtered_values.push(value);
                            }
                        }
                        let options = AngularExpression::LiteralMap(oxc_allocator::Box::new_in(
                            LiteralMap {
                                span,
                                source_span,
                                keys: filtered_keys,
                                values: filtered_values,
                            },
                            &self.allocator,
                        ));
                        (Some(reference), Some(options))
                    }
                }
            }
        };

        if self.is_hydration_trigger && reference.is_some() {
            return Err("\"viewport\" hydration trigger cannot have a \"trigger\"".to_string());
        }
        if let Some(dynamic_node) = options.as_ref().and_then(find_dynamic_node) {
            return Err(format!(
                "Options of the \"viewport\" trigger must be an object literal containing only literal values, but \"{dynamic_node}\" was found"
            ));
        }

        Ok((reference, options))
    }
}

fn create_idle_trigger(parameters: &[ParsedParameter<'_>]) -> Result<Option<f64>, String> {
    if parameters.len() > 1 {
        return Err("\"idle\" trigger can only have zero or one parameters".to_string());
    }
    // v22+: `idle` accepts an optional timeout parameter, e.g. `on idle(100)`.
    parameters
        .first()
        .map(|param| {
            parse_deferred_time(param.expression)
                .ok_or_else(|| "Could not parse time value of trigger \"idle\"".to_string())
        })
        .transpose()
}

fn create_timer_trigger(parameters: &[ParsedParameter<'_>]) -> Result<f64, String> {
    if parameters.len() != 1 {
        return Err("\"timer\" trigger must have exactly one parameter".to_string());
    }
    parse_deferred_time(parameters[0].expression)
        .ok_or_else(|| "Could not parse time value of trigger \"timer\"".to_string())
}

fn create_immediate_trigger(parameters: &[ParsedParameter<'_>]) -> Result<(), String> {
    if !parameters.is_empty() {
        return Err("\"immediate\" trigger cannot have parameters".to_string());
    }
    Ok(())
}

/// Port of Angular's `DynamicAstValidator`: returns the class name of the last
/// non-literal node found in a `viewport` options expression.
fn find_dynamic_node(expr: &AngularExpression<'_>) -> Option<&'static str> {
    match expr {
        AngularExpression::LiteralPrimitive(_) => None,
        AngularExpression::LiteralArray(arr) => {
            arr.expressions.iter().rev().find_map(find_dynamic_node)
        }
        AngularExpression::LiteralMap(map) => map.values.iter().rev().find_map(find_dynamic_node),
        _ => Some(angular_ast_class_name(expr)),
    }
}

/// The name of the Angular AST class an expression corresponds to.
fn angular_ast_class_name(expr: &AngularExpression<'_>) -> &'static str {
    match expr {
        AngularExpression::Empty(_) => "EmptyExpr",
        AngularExpression::ImplicitReceiver(_) => "ImplicitReceiver",
        AngularExpression::ThisReceiver(_) => "ThisReceiver",
        AngularExpression::Chain(_) => "Chain",
        AngularExpression::Conditional(_) => "Conditional",
        AngularExpression::PropertyRead(_) => "PropertyRead",
        AngularExpression::SafePropertyRead(_) => "SafePropertyRead",
        AngularExpression::KeyedRead(_) => "KeyedRead",
        AngularExpression::SafeKeyedRead(_) => "SafeKeyedRead",
        AngularExpression::BindingPipe(_) => "BindingPipe",
        AngularExpression::LiteralPrimitive(_) => "LiteralPrimitive",
        AngularExpression::LiteralArray(_) => "LiteralArray",
        AngularExpression::LiteralMap(_) => "LiteralMap",
        AngularExpression::Interpolation(_) => "Interpolation",
        AngularExpression::Binary(_) => "Binary",
        AngularExpression::Unary(_) => "Unary",
        AngularExpression::PrefixNot(_) => "PrefixNot",
        AngularExpression::TypeofExpression(_) => "TypeofExpression",
        AngularExpression::VoidExpression(_) => "VoidExpression",
        AngularExpression::NonNullAssert(_) => "NonNullAssert",
        AngularExpression::Call(_) => "Call",
        AngularExpression::SafeCall(_) => "SafeCall",
        AngularExpression::TaggedTemplateLiteral(_) => "TaggedTemplateLiteral",
        AngularExpression::TemplateLiteral(_) => "TemplateLiteral",
        AngularExpression::ParenthesizedExpression(_) => "ParenthesizedExpression",
        AngularExpression::RegularExpressionLiteral(_) => "RegularExpressionLiteral",
        AngularExpression::SpreadElement(_) => "SpreadElement",
        AngularExpression::ArrowFunction(_) => "ArrowFunction",
    }
}

/// Parses a time value like "500ms" or "1.5s" to milliseconds.
/// Returns f64 to preserve fractional precision (matching Angular's parseFloat behavior).
fn parse_deferred_time(value: &str) -> Option<f64> {
    let value = value.trim();

    if !is_valid_time_pattern(value) {
        return None;
    }

    // Extract numeric part and unit
    let (num_str, unit) = if value.ends_with("ms") {
        (&value[..value.len() - 2], "ms")
    } else if value.ends_with('s') {
        (&value[..value.len() - 1], "s")
    } else {
        (value, "ms") // Default to milliseconds
    };

    let num: f64 = num_str.parse().ok()?;
    let millis = if unit == "s" { num * 1000.0 } else { num };

    Some(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_for_loop_expression_pattern() {
        let (var, expr) = parse_for_of_expression("item of items").unwrap();
        assert_eq!(var, "item");
        assert_eq!(expr, "items");

        let (var, expr) = parse_for_of_expression("  user   of   users  ").unwrap();
        assert_eq!(var, "user");
        assert_eq!(expr, "users  ");
    }

    #[test]
    fn test_for_loop_track_pattern() {
        let expr = parse_track_expression("track item.id").unwrap();
        assert_eq!(expr, "item.id");

        let expr = parse_track_expression("track $index").unwrap();
        assert_eq!(expr, "$index");
    }

    #[test]
    fn test_for_loop_let_pattern() {
        let expr = parse_let_expression("let i = $index").unwrap();
        assert_eq!(expr, "i = $index");

        let expr = parse_let_expression("let i = $index, odd = $odd").unwrap();
        assert_eq!(expr, "i = $index, odd = $odd");
    }

    #[test]
    fn test_strip_optional_parentheses() {
        let mut errors = std::vec::Vec::new();
        assert_eq!(
            strip_optional_parentheses("item of items", &mut errors),
            Some("item of items".to_string())
        );
        assert_eq!(
            strip_optional_parentheses("(item of items)", &mut errors),
            Some("item of items".to_string())
        );
        assert_eq!(
            strip_optional_parentheses("  (  item of items  )  ", &mut errors),
            Some("item of items".to_string())
        );
        assert!(errors.is_empty());

        // Test unclosed parentheses
        let mut errors2 = std::vec::Vec::new();
        assert_eq!(strip_optional_parentheses("((item of items)", &mut errors2), None);
        assert_eq!(errors2.len(), 1);
        assert!(errors2[0].contains("Unclosed parentheses"));
    }

    #[test]
    fn test_parse_deferred_time() {
        // Milliseconds
        assert_eq!(parse_deferred_time("500ms"), Some(500.0));
        assert_eq!(parse_deferred_time("100ms"), Some(100.0));
        assert_eq!(parse_deferred_time("0ms"), Some(0.0));

        // Fractional milliseconds (must preserve precision)
        assert_eq!(parse_deferred_time("1.5ms"), Some(1.5));

        // Seconds
        assert_eq!(parse_deferred_time("1s"), Some(1000.0));
        assert_eq!(parse_deferred_time("2s"), Some(2000.0));
        assert_eq!(parse_deferred_time("1.5s"), Some(1500.0));

        // No unit defaults to ms
        assert_eq!(parse_deferred_time("500"), Some(500.0));

        // With whitespace
        assert_eq!(parse_deferred_time(" 500ms "), Some(500.0));

        // Invalid
        assert_eq!(parse_deferred_time("abc"), None);
        assert_eq!(parse_deferred_time(""), None);
    }
}

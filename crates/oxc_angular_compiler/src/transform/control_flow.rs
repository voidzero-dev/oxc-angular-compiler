//! Control flow block parsing utilities.
//!
//! This module provides parsing utilities for Angular control flow blocks:
//! - `@for` loops with `track` expressions and context variables
//! - `@switch` blocks with `@case` and `@default`
//! - `@if`/`@else if`/`@else` conditionals
//!
//! Ported from Angular's `render3/r3_control_flow.ts`.

use oxc_allocator::{Allocator, Vec};
use oxc_span::{Atom, Span};

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
        allocator,
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
                    name: Atom::from(name_alloc),
                    value: Atom::from(name_alloc), // value same as name for alias
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
    let item_name_atom = Atom::from(allocator.alloc_str(&item_name_owned));
    let item_name_span = Span::new(
        expression_param.span.start,
        expression_param.span.start + item_name_str.len() as u32,
    );
    let item = R3Variable {
        name: item_name_atom,
        value: Atom::from("$implicit"),
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
        allocator,
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
                    allocator,
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
                allocator,
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
            name: Atom::from(name_alloc),
            value: Atom::from(value_alloc),
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
    let mut vars = Vec::with_capacity_in(ALLOWED_FOR_LOOP_LET_VARIABLES.len(), allocator);

    // Empty span at the end of the block start - both start and end are the same position
    let empty_span = Span::new(block_start_span.end, block_start_span.end);

    for &var_name in ALLOWED_FOR_LOOP_LET_VARIABLES {
        vars.push(R3Variable {
            name: Atom::from(var_name),
            value: Atom::from(var_name),
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
        name: Atom::from(""),
        value: Atom::from("$implicit"),
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
            allocator,
        )),
        source: None,
        location: Atom::from(""),
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
        source: Some(Atom::from(expr_str)),
        location: Atom::from(""),
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

/// Result of parsing defer block triggers.
pub struct DeferTriggerParseResult<'a> {
    /// Regular triggers (load on demand).
    pub triggers: R3DeferredBlockTriggers<'a>,
    /// Prefetch triggers.
    pub prefetch_triggers: R3DeferredBlockTriggers<'a>,
    /// Hydrate triggers.
    pub hydrate_triggers: R3DeferredBlockTriggers<'a>,
    /// Parse errors.
    pub errors: std::vec::Vec<String>,
}

// Pattern matching functions for Angular's r3_deferred_blocks.ts
// These handle whitespace/newlines between keywords correctly

/// Check if string starts with a keyword followed by whitespace, then another keyword followed by whitespace.
fn starts_with_keyword_pair(s: &str, first: &str, second: &str) -> bool {
    if !s.starts_with(first) {
        return false;
    }
    let after_first = &s[first.len()..];
    // Must have at least one whitespace
    let trimmed = after_first.trim_start();
    if trimmed.len() == after_first.len() {
        return false; // No whitespace after first keyword
    }
    if !trimmed.starts_with(second) {
        return false;
    }
    let after_second = &trimmed[second.len()..];
    // Must have at least one whitespace after second keyword (or be at end for "never")
    !after_second.is_empty() && after_second.chars().next().map_or(false, |c| c.is_whitespace())
}

/// Pattern to identify a `prefetch when` trigger.
fn is_prefetch_when_pattern(s: &str) -> bool {
    starts_with_keyword_pair(s, "prefetch", "when")
}

/// Pattern to identify a `prefetch on` trigger.
fn is_prefetch_on_pattern(s: &str) -> bool {
    starts_with_keyword_pair(s, "prefetch", "on")
}

/// Pattern to identify a `hydrate when` trigger.
fn is_hydrate_when_pattern(s: &str) -> bool {
    starts_with_keyword_pair(s, "hydrate", "when")
}

/// Pattern to identify a `hydrate on` trigger.
fn is_hydrate_on_pattern(s: &str) -> bool {
    starts_with_keyword_pair(s, "hydrate", "on")
}

/// Pattern to identify a `hydrate never` trigger.
fn is_hydrate_never_pattern(s: &str) -> bool {
    if !s.starts_with("hydrate") {
        return false;
    }
    let after_hydrate = &s["hydrate".len()..];
    let trimmed = after_hydrate.trim_start();
    if trimmed.len() == after_hydrate.len() {
        return false; // No whitespace after "hydrate"
    }
    // Check for "never" followed by optional whitespace to end
    trimmed.strip_prefix("never").map_or(false, |rest| rest.trim().is_empty())
}

/// Pattern to identify a `when` parameter in a block.
/// Matches "when" followed by whitespace, or bare "when" (which can occur after trimming).
fn is_when_pattern(s: &str) -> bool {
    s.starts_with("when")
        && (s.len() == 4 || (s.len() > 4 && s.as_bytes()[4].is_ascii_whitespace()))
}

/// Pattern to identify an `on` parameter in a block.
/// Matches "on" followed by whitespace, or bare "on" (which can occur after trimming).
fn is_on_pattern(s: &str) -> bool {
    s.starts_with("on") && (s.len() == 2 || (s.len() > 2 && s.as_bytes()[2].is_ascii_whitespace()))
}

/// Gets the index within an expression at which the trigger parameters start.
/// After finding the keyword (e.g., "when", "on"), this function skips any
/// separator characters (whitespace/newlines) to find where the actual content begins.
fn get_trigger_parameters_start(value: &str, start_position: usize) -> Option<usize> {
    let chars: std::vec::Vec<char> = value.chars().collect();
    let mut has_found_separator = false;

    for i in start_position..chars.len() {
        if chars[i].is_whitespace() {
            has_found_separator = true;
        } else if has_found_separator {
            return Some(i);
        }
    }

    None
}

/// Parses all triggers from @defer block parameters.
/// Reference: Angular's r3_deferred_blocks.ts parseTriggers function
pub fn parse_defer_triggers<'a>(
    allocator: &'a Allocator,
    parameters: &[HtmlBlockParameter<'a>],
    binding_parser: &BindingParser<'a>,
) -> DeferTriggerParseResult<'a> {
    let mut triggers = R3DeferredBlockTriggers::default();
    let mut prefetch_triggers = R3DeferredBlockTriggers::default();
    let mut hydrate_triggers = R3DeferredBlockTriggers::default();
    let mut errors = std::vec::Vec::new();

    for param in parameters {
        let expr = param.expression.as_str();
        let span = param.span;

        // Match against patterns in the same order as Angular's TypeScript implementation
        // Reference: r3_deferred_blocks.ts lines 279-295
        if is_when_pattern(expr) {
            parse_when_trigger_from_expr(
                allocator,
                binding_parser,
                expr,
                span,
                None,
                None,
                &mut triggers,
                &mut errors,
            );
        } else if is_on_pattern(expr) {
            parse_on_trigger_from_expr(
                allocator,
                expr,
                span,
                None,
                None,
                &mut triggers,
                &mut errors,
                binding_parser,
            );
        } else if is_prefetch_when_pattern(expr) {
            let prefetch_span = Some(Span::new(span.start, span.start + 8)); // "prefetch"
            parse_when_trigger_from_expr(
                allocator,
                binding_parser,
                expr,
                span,
                prefetch_span,
                None,
                &mut prefetch_triggers,
                &mut errors,
            );
        } else if is_prefetch_on_pattern(expr) {
            let prefetch_span = Some(Span::new(span.start, span.start + 8)); // "prefetch"
            parse_on_trigger_from_expr(
                allocator,
                expr,
                span,
                prefetch_span,
                None,
                &mut prefetch_triggers,
                &mut errors,
                binding_parser,
            );
        } else if is_hydrate_when_pattern(expr) {
            let hydrate_span = Some(Span::new(span.start, span.start + 7)); // "hydrate"
            parse_when_trigger_from_expr(
                allocator,
                binding_parser,
                expr,
                span,
                None,
                hydrate_span,
                &mut hydrate_triggers,
                &mut errors,
            );
        } else if is_hydrate_on_pattern(expr) {
            let hydrate_span = Some(Span::new(span.start, span.start + 7)); // "hydrate"
            parse_on_trigger_from_expr(
                allocator,
                expr,
                span,
                None,
                hydrate_span,
                &mut hydrate_triggers,
                &mut errors,
                binding_parser,
            );
        } else if is_hydrate_never_pattern(expr) {
            let hydrate_span = Some(Span::new(span.start, span.start + 7)); // "hydrate"
            parse_never_trigger(expr, span, hydrate_span, &mut hydrate_triggers, &mut errors);
        } else if !expr.trim().is_empty() {
            errors.push(format!("Unrecognized trigger: \"{}\"", expr.trim()));
        }
    }

    // Validate that `hydrate never` is not combined with other hydrate triggers
    if hydrate_triggers.never.is_some() && has_other_hydrate_triggers(&hydrate_triggers) {
        errors.push(
            "Cannot specify additional `hydrate` triggers if `hydrate never` is present"
                .to_string(),
        );
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

/// Parses a "never" trigger (only valid for hydrate).
fn parse_never_trigger(
    expr: &str,
    span: Span,
    hydrate_span: Option<Span>,
    triggers: &mut R3DeferredBlockTriggers<'_>,
    errors: &mut std::vec::Vec<String>,
) {
    let never_index = expr.find("never");

    if let Some(idx) = never_index {
        let never_start = span.start + idx as u32;
        let never_span = Span::new(never_start, never_start + 5); // "never" is 5 chars

        if triggers.never.is_some() {
            errors.push("Duplicate 'never' trigger is not allowed".to_string());
            return;
        }

        triggers.never = Some(R3NeverDeferredTrigger {
            source_span: span,
            name_span: Some(never_span),
            prefetch_span: None,
            when_or_on_source_span: Some(never_span),
            hydrate_span,
        });
    } else {
        errors.push("Could not find \"never\" keyword in expression".to_string());
    }
}

/// Parses a "when" trigger from the full expression.
fn parse_when_trigger_from_expr<'a>(
    allocator: &'a Allocator,
    binding_parser: &BindingParser<'a>,
    expr: &'a str,
    span: Span,
    prefetch_span: Option<Span>,
    hydrate_span: Option<Span>,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<String>,
) {
    let when_index = expr.find("when");

    if let Some(idx) = when_index {
        let when_start = span.start + idx as u32;
        let when_source_span = Span::new(when_start, when_start + 4); // "when" is 4 chars

        // Find where the actual condition starts (after "when" + whitespace)
        let start = get_trigger_parameters_start(expr, idx + 4);

        if let Some(start_idx) = start {
            let condition_str = &expr[start_idx..];
            // The full source span should cover from the start of the parameter
            // (including prefetch/hydrate/when keywords) to the end
            let full_source_span = span;
            parse_when_trigger(
                allocator,
                binding_parser,
                condition_str,
                Span::new(span.start + start_idx as u32, span.end),
                full_source_span,
                prefetch_span,
                hydrate_span,
                when_source_span,
                triggers,
                errors,
            );
        }
        // If no condition found (e.g., bare "when" after trimming),
        // Angular silently accepts it with no triggers. Match that behavior.
    } else {
        errors.push("Could not find \"when\" keyword in expression".to_string());
    }
}

/// Parses an "on" trigger from the full expression.
fn parse_on_trigger_from_expr<'a>(
    allocator: &'a Allocator,
    expr: &'a str,
    span: Span,
    prefetch_span: Option<Span>,
    hydrate_span: Option<Span>,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<String>,
    binding_parser: &BindingParser<'a>,
) {
    let on_index = expr.find("on");

    if let Some(idx) = on_index {
        // Find where the actual trigger names start (after "on" + whitespace)
        let start = get_trigger_parameters_start(expr, idx + 2);

        if let Some(start_idx) = start {
            let triggers_str = &expr[start_idx..];
            // Calculate span for the triggers portion
            let triggers_span = Span::new(span.start + start_idx as u32, span.end);
            // The full span starts from the beginning (including prefetch/hydrate/on keywords)
            let full_source_span_start = span.start;
            parse_on_triggers(
                allocator,
                triggers_str,
                triggers_span,
                full_source_span_start,
                prefetch_span,
                hydrate_span,
                triggers,
                errors,
                binding_parser,
            );
        }
        // If no trigger parameters found (e.g., bare "on" after trimming),
        // Angular silently accepts it with no triggers. Match that behavior.
    } else {
        errors.push("Could not find \"on\" keyword in expression".to_string());
    }
}

/// Parses a "when condition" trigger.
fn parse_when_trigger<'a>(
    _allocator: &'a Allocator,
    binding_parser: &BindingParser<'a>,
    condition_str: &'a str,
    span: Span,
    full_source_span: Span,
    prefetch_span: Option<Span>,
    hydrate_span: Option<Span>,
    when_source_span: Span,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<String>,
) {
    if condition_str.is_empty() {
        errors.push("@defer 'when' trigger requires a condition expression".to_string());
        return;
    }

    if triggers.when.is_some() {
        errors.push("Duplicate 'when' trigger is not allowed".to_string());
        return;
    }

    // Parse the condition expression
    let parsed = binding_parser.parse_binding(condition_str, span);

    // source_span should cover the entire trigger clause including keywords
    triggers.when = Some(R3BoundDeferredTrigger {
        value: parsed.ast,
        source_span: full_source_span,
        prefetch_span,
        when_source_span,
        hydrate_span,
    });
}

/// Splits a string by top-level commas, respecting nested parentheses, braces, and brackets.
fn split_by_top_level_comma(s: &str) -> std::vec::Vec<&str> {
    let mut parts = std::vec::Vec::new();
    let mut start = 0;
    let mut depth_paren: i32 = 0;
    let mut depth_brace: i32 = 0;
    let mut depth_bracket: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut string_char = '"';

    for (i, ch) in s.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        if ch == '\\' {
            escape_next = true;
            continue;
        }

        if in_string {
            if ch == string_char {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                in_string = true;
                string_char = ch;
            }
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            '{' => depth_brace += 1,
            '}' => depth_brace = depth_brace.saturating_sub(1),
            '[' => depth_bracket += 1,
            ']' => depth_bracket = depth_bracket.saturating_sub(1),
            ',' if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    // Push the last part
    if start < s.len() {
        parts.push(&s[start..]);
    } else if start == s.len() && !s.is_empty() && s.ends_with(',') {
        parts.push("");
    }

    parts
}

/// Parses "on <type>" triggers (can have multiple comma-separated).
/// The span passed here should already point to the start of the triggers portion
/// (after "on" and its trailing whitespace).
fn parse_on_triggers<'a>(
    allocator: &'a Allocator,
    triggers_str: &'a str,
    span: Span,
    full_source_span_start: u32,
    prefetch_span: Option<Span>,
    hydrate_span: Option<Span>,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<String>,
    binding_parser: &BindingParser<'a>,
) {
    // The span now starts at the triggers portion (after "on" + whitespace)
    let triggers_str_start = span.start;

    // Split by top-level commas only, respecting nested structures
    // E.g., "idle, viewport({trigger: foo, margin: 1})" splits into:
    // - "idle"
    // - " viewport({trigger: foo, margin: 1})"
    let parts = split_by_top_level_comma(triggers_str);

    let mut current_pos = 0u32;
    let mut is_first_trigger = true;

    for trigger_part_raw in parts {
        // Calculate position of this trigger in the original string
        let original_len = trigger_part_raw.len() as u32;
        let leading_whitespace = trigger_part_raw.len() - trigger_part_raw.trim_start().len();
        let trigger_part = trigger_part_raw.trim();
        if trigger_part.is_empty() {
            // Account for the empty part and comma separator
            current_pos += original_len + 1;
            continue;
        }

        // Calculate the actual span for this trigger
        let trigger_start = triggers_str_start + current_pos + leading_whitespace as u32;
        let trigger_end = trigger_start + trigger_part.len() as u32;
        let trigger_span = Span::new(trigger_start, trigger_end);

        // For the first trigger, source_span includes the "on"/"prefetch on"/"hydrate on" prefix
        // For subsequent triggers, source_span is just the trigger name
        let source_span = if is_first_trigger {
            Span::new(full_source_span_start, trigger_end)
        } else {
            trigger_span
        };

        // Parse individual trigger
        parse_single_on_trigger(
            allocator,
            trigger_part,
            trigger_span,
            source_span,
            prefetch_span,
            hydrate_span,
            triggers,
            errors,
            binding_parser,
        );

        // Move past this trigger and the comma separator
        current_pos += original_len + 1;
        is_first_trigger = false;
    }
}

/// Parses a single "on" trigger like "idle", "timer(500ms)", "hover(ref)".
/// `trigger_span` is the span of just the trigger (e.g., "idle" or "hover(button)")
/// `source_span` is the full span including any prefix (e.g., "on hover(button)")
fn parse_single_on_trigger<'a>(
    allocator: &'a Allocator,
    trigger_str: &'a str,
    trigger_span: Span,
    source_span: Span,
    prefetch_span: Option<Span>,
    hydrate_span: Option<Span>,
    triggers: &mut R3DeferredBlockTriggers<'a>,
    errors: &mut std::vec::Vec<String>,
    binding_parser: &BindingParser<'a>,
) {
    // Extract trigger name and optional parameters
    let (name, params) = if let Some(paren_start) = trigger_str.find('(') {
        let name = trigger_str[..paren_start].trim();
        let params_end = trigger_str.rfind(')').unwrap_or(trigger_str.len());
        let params_str = trigger_str[paren_start + 1..params_end].trim();
        // Empty parentheses like `idle()` should be treated as zero parameters,
        // matching Angular's consumeParameters() which returns an empty array for `()`.
        (name, if params_str.is_empty() { None } else { Some(params_str) })
    } else {
        (trigger_str.trim(), None)
    };

    match name {
        "idle" => {
            if params.is_some() {
                errors.push("'idle' trigger cannot have parameters".to_string());
                return;
            }
            if triggers.idle.is_some() {
                errors.push("Duplicate 'idle' trigger is not allowed".to_string());
                return;
            }
            triggers.idle = Some(R3IdleDeferredTrigger {
                source_span,
                name_span: Some(trigger_span),
                prefetch_span,
                when_or_on_source_span: Some(trigger_span),
                hydrate_span,
            });
        }

        "immediate" => {
            if params.is_some() {
                errors.push("'immediate' trigger cannot have parameters".to_string());
                return;
            }
            if triggers.immediate.is_some() {
                errors.push("Duplicate 'immediate' trigger is not allowed".to_string());
                return;
            }
            triggers.immediate = Some(R3ImmediateDeferredTrigger {
                source_span,
                name_span: Some(trigger_span),
                prefetch_span,
                when_or_on_source_span: Some(trigger_span),
                hydrate_span,
            });
        }

        "timer" => {
            let delay = if let Some(param_str) = params {
                match parse_deferred_time(param_str) {
                    Some(d) => d,
                    None => {
                        errors.push(format!(
                            "Could not parse time value '{}' for 'timer' trigger",
                            param_str
                        ));
                        return;
                    }
                }
            } else {
                errors.push("'timer' trigger must have exactly one parameter".to_string());
                return;
            };

            if triggers.timer.is_some() {
                errors.push("Duplicate 'timer' trigger is not allowed".to_string());
                return;
            }
            triggers.timer = Some(R3TimerDeferredTrigger {
                delay,
                source_span,
                name_span: trigger_span,
                prefetch_span,
                on_source_span: Some(trigger_span),
                hydrate_span,
            });
        }

        "hover" => {
            if triggers.hover.is_some() {
                errors.push("Duplicate 'hover' trigger is not allowed".to_string());
                return;
            }
            // Hydration triggers for hover cannot have parameters
            if hydrate_span.is_some() && params.is_some() {
                errors.push("Hydration trigger \"hover\" cannot have parameters".to_string());
                return;
            }
            // Validate zero or one parameter (matching Angular's validatePlainReferenceBasedTrigger)
            if let Some(p) = params {
                let param_parts: std::vec::Vec<&str> =
                    p.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                if param_parts.len() > 1 {
                    errors
                        .push("\"hover\" trigger can only have zero or one parameters".to_string());
                    return;
                }
            }
            let reference = params.map(|s| Atom::from(s.trim()));
            triggers.hover = Some(R3HoverDeferredTrigger {
                reference,
                source_span,
                name_span: trigger_span,
                prefetch_span,
                on_source_span: Some(trigger_span),
                hydrate_span,
            });
        }

        "interaction" => {
            if triggers.interaction.is_some() {
                errors.push("Duplicate 'interaction' trigger is not allowed".to_string());
                return;
            }
            // Hydration triggers for interaction cannot have parameters
            if hydrate_span.is_some() && params.is_some() {
                errors.push("Hydration trigger \"interaction\" cannot have parameters".to_string());
                return;
            }
            // Validate zero or one parameter (matching Angular's validatePlainReferenceBasedTrigger)
            if let Some(p) = params {
                let param_parts: std::vec::Vec<&str> =
                    p.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                if param_parts.len() > 1 {
                    errors.push(
                        "\"interaction\" trigger can only have zero or one parameters".to_string(),
                    );
                    return;
                }
            }
            let reference = params.map(|s| Atom::from(s.trim()));
            triggers.interaction = Some(R3InteractionDeferredTrigger {
                reference,
                source_span,
                name_span: trigger_span,
                prefetch_span,
                on_source_span: Some(trigger_span),
                hydrate_span,
            });
        }

        "viewport" => {
            if triggers.viewport.is_some() {
                errors.push("Duplicate 'viewport' trigger is not allowed".to_string());
                return;
            }

            // Validate parameter count before parsing (matching Angular's validator).
            // Non-hydrate: validatePlainReferenceBasedTrigger → max 1 parameter
            // Hydrate: validateHydrateReferenceBasedTrigger → max 1 parameter
            // Use top-level comma splitting to respect nested {}, [], () in object literals.
            if let Some(p) = params {
                let param_parts: std::vec::Vec<&str> = split_by_top_level_comma(p)
                    .into_iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if param_parts.len() > 1 {
                    if hydrate_span.is_some() {
                        errors.push(
                            "Hydration trigger \"viewport\" cannot have more than one parameter"
                                .to_string(),
                        );
                    } else {
                        errors.push(
                            "\"viewport\" trigger can only have zero or one parameters".to_string(),
                        );
                    }
                    return;
                }
            }

            let (reference, options) = if let Some(param_str) = params {
                let trimmed = param_str.trim();
                if trimmed.starts_with('{') {
                    // Parse as object literal for options
                    let parsed = binding_parser.parse_binding(trimmed, trigger_span);
                    // Extract reference from 'trigger' key if present and filter it from options
                    let result =
                        extract_viewport_trigger_and_options(allocator, parsed.ast, trigger_span);
                    // Add any errors from extraction
                    errors.extend(result.errors);
                    (result.reference, result.options)
                } else {
                    // Simple reference name
                    (Some(Atom::from(trimmed)), None)
                }
            } else {
                (None, None)
            };

            // Hydration viewport triggers cannot have a reference
            if hydrate_span.is_some() && reference.is_some() {
                errors.push("\"viewport\" hydration trigger cannot have a \"trigger\"".to_string());
                return;
            }

            triggers.viewport = Some(R3ViewportDeferredTrigger {
                reference,
                options,
                source_span,
                name_span: trigger_span,
                prefetch_span,
                on_source_span: Some(trigger_span),
                hydrate_span,
            });
        }

        _ => {
            // "never" is only valid as "hydrate never" (top-level pattern), not as an on-trigger.
            // Angular's OnTriggerParser switch has no case for NEVER, so it falls to default.
            errors.push(format!("Unrecognized trigger type \"{}\"", name));
        }
    }
}

/// Result type for viewport trigger extraction.
struct ViewportTriggerResult<'a> {
    reference: Option<Atom<'a>>,
    options: Option<AngularExpression<'a>>,
    errors: std::vec::Vec<String>,
}

/// Checks if an expression contains dynamic nodes (not literal values).
fn contains_dynamic_node(expr: &AngularExpression<'_>) -> bool {
    match expr {
        AngularExpression::LiteralPrimitive(_) => false,
        AngularExpression::LiteralArray(arr) => arr.expressions.iter().any(contains_dynamic_node),
        AngularExpression::LiteralMap(map) => map.values.iter().any(contains_dynamic_node),
        // All other expression types are considered dynamic
        _ => true,
    }
}

/// Extracts the trigger reference and options from a viewport trigger object literal.
///
/// Given `{trigger: foo, rootMargin: "123px", threshold: [1, 2, 3]}`, returns:
/// - reference: `Some("foo")`
/// - options: `LiteralMap({rootMargin: "123px", threshold: [1, 2, 3]})`
fn extract_viewport_trigger_and_options<'a>(
    allocator: &'a Allocator,
    expr: AngularExpression<'a>,
    _span: Span,
) -> ViewportTriggerResult<'a> {
    use crate::ast::expression::{LiteralMap, LiteralMapKey, LiteralMapPropertyKey};

    let mut errors = std::vec::Vec::new();

    if let AngularExpression::LiteralMap(map) = expr {
        let mut trigger_ref: Option<Atom<'a>> = None;
        let mut trigger_idx: Option<usize> = None;

        // First pass: find the trigger key, check for "root" key, and extract trigger value
        for (idx, (key, value)) in map.keys.iter().zip(map.values.iter()).enumerate() {
            let key_str = match key {
                LiteralMapKey::Property(prop) => prop.key.as_str(),
                LiteralMapKey::Spread(_) => continue, // Skip spread keys
            };
            if key_str == "trigger" {
                trigger_idx = Some(idx);
                // Extract reference from the trigger value
                if let AngularExpression::PropertyRead(prop) = value {
                    // It's a simple identifier like `foo`
                    trigger_ref = Some(prop.name.clone());
                } else {
                    errors.push(
                        "\"trigger\" option of the \"viewport\" trigger must be an identifier"
                            .to_string(),
                    );
                }
            } else if key_str == "root" {
                errors.push(
                    "The \"root\" option is not supported in the options parameter of the \"viewport\" trigger".to_string()
                );
            }
        }

        // If no trigger key found, return the original expression as options
        let trigger_idx = match trigger_idx {
            Some(idx) => idx,
            None => {
                // Check for dynamic nodes in options
                if map.values.iter().any(contains_dynamic_node) {
                    errors.push(
                        "Options of the \"viewport\" trigger must be an object literal containing only literal values".to_string()
                    );
                }
                return ViewportTriggerResult {
                    reference: None,
                    options: Some(AngularExpression::LiteralMap(map)),
                    errors,
                };
            }
        };

        // Unbox the map to get ownership of keys and values
        let LiteralMap { span: map_span, source_span: map_source_span, keys, values } = map.unbox();

        // Second pass: filter out the trigger key
        let mut filtered_keys = Vec::new_in(allocator);
        let mut filtered_values = Vec::new_in(allocator);

        for (idx, (key, value)) in keys.into_iter().zip(values.into_iter()).enumerate() {
            if idx != trigger_idx {
                // Check for dynamic values
                if contains_dynamic_node(&value) {
                    errors.push(
                        "Options of the \"viewport\" trigger must be an object literal containing only literal values".to_string()
                    );
                }
                // Only include property keys, skip spread keys
                if let LiteralMapKey::Property(prop) = key {
                    filtered_keys.push(LiteralMapKey::Property(LiteralMapPropertyKey {
                        key: prop.key,
                        quoted: prop.quoted,
                        is_shorthand_initialized: prop.is_shorthand_initialized,
                    }));
                    filtered_values.push(value);
                }
            }
        }

        // Always create a LiteralMap for options, even if empty (matching Angular behavior).
        // Angular keeps an empty LiteralMap {} when only the trigger key was present.
        let options = Some(AngularExpression::LiteralMap(oxc_allocator::Box::new_in(
            LiteralMap {
                span: map_span,
                source_span: map_source_span,
                keys: filtered_keys,
                values: filtered_values,
            },
            allocator,
        )));

        ViewportTriggerResult { reference: trigger_ref, options, errors }
    } else {
        // Not a LiteralMap
        errors.push(
            "Options parameter of the \"viewport\" trigger must be an object literal".to_string(),
        );
        ViewportTriggerResult { reference: None, options: Some(expr), errors }
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

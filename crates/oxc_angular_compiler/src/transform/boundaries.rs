//! `@boundary` / `@error` block parsing.
//!
//! Ported from Angular's `render3/r3_boundaries.ts` and the shared
//! `parseLetParameters` helper in `render3/util.ts`.

use oxc_allocator::{Allocator, Vec};
use oxc_span::Span;
use oxc_str::Ident;

use crate::ast::expression::AngularExpression;
use crate::ast::html::HtmlBlockParameter;
use crate::ast::r3::R3Variable;
use crate::parser::expression::BindingParser;
use crate::util::ParseError;

/// An error reported while parsing `@error` block parameters.
pub struct BoundaryParamError {
    /// The message.
    pub msg: String,
    /// Where it is reported (the parameter's span).
    pub span: Span,
}

/// The parsed parameters of one `@error` block.
pub struct BoundaryErrorParams<'a> {
    /// `$error`, `$reset` and the aliases declared with `let`.
    pub context_variables: Vec<'a, R3Variable<'a>>,
    /// The `when` condition.
    pub expression: Option<AngularExpression<'a>>,
    /// Parameter errors.
    pub errors: std::vec::Vec<BoundaryParamError>,
    /// Errors from parsing the `when` expression.
    pub expression_errors: std::vec::Vec<ParseError>,
}

/// Matches JavaScript's `\s`, which is also what `String.prototype.trim` strips. Unlike
/// `char::is_whitespace`, it includes U+FEFF and leaves out U+0085.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// Matches `IDENTIFIER_PATTERN` (`/^[$A-Z_][0-9A-Z_$]*$/i`).
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Matches `LET_PATTERN` (`/^let\s+([\S\s]*)/`), returning the offset and text of the
/// captured group.
fn match_let(s: &str) -> Option<(usize, &str)> {
    let rest = s.strip_prefix("let")?;
    let trimmed = rest.trim_start_matches(is_js_whitespace);
    if trimmed.len() == rest.len() {
        return None;
    }
    Some((s.len() - trimmed.len(), trimmed))
}

/// Matches `/^\s*[$A-Z_][0-9A-Z_$]*\s*=/i`.
fn is_alias_match(s: &str) -> bool {
    let s = s.trim_start_matches(is_js_whitespace);
    let mut chars = s.char_indices();
    if !matches!(chars.next(), Some((_, c)) if c.is_ascii_alphabetic() || c == '_' || c == '$') {
        return false;
    }
    let ident_end = chars
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .map_or(s.len(), |(i, _)| i);
    s[ident_end..].trim_start_matches(is_js_whitespace).starts_with('=')
}

/// Matches `WHEN_PATTERN` (`/^(when\s+)(.*)/`). `.` stops at a line terminator.
fn match_when(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("when")?;
    let trimmed = rest.trim_start_matches(is_js_whitespace);
    if trimmed.len() == rest.len() {
        return None;
    }
    let end = trimmed.find(['\n', '\r', '\u{2028}', '\u{2029}']).unwrap_or(trimmed.len());
    Some(&trimmed[..end])
}

/// Matches `/(\s*)(\S+)(\s*)/` and returns the lengths of the leading whitespace and
/// of the first non-whitespace run.
fn surrounding_whitespace(s: &str) -> Option<(usize, usize)> {
    let start = s.find(|c: char| !is_js_whitespace(c))?;
    let len = s[start..].find(is_js_whitespace).unwrap_or(s.len() - start);
    Some((start, len))
}

/// Parses the `let` parameter of an `@error` block, e.g. `err, retry = $reset`.
///
/// Port of Angular's `parseLetParameters` with the `@error` block's validation and
/// `$error` as the default variable.
fn parse_let_parameters<'a>(
    allocator: &'a Allocator,
    source_span: Span,
    expression: &str,
    span_start: u32,
    context: &mut Vec<'a, R3Variable<'a>>,
    errors: &mut std::vec::Vec<BoundaryParamError>,
) {
    let mut error = |msg: String| errors.push(BoundaryParamError { msg, span: source_span });
    let mut start = span_start;
    for part in expression.split(',') {
        let expression_parts: std::vec::Vec<&str> = part.split('=').collect();
        let name = js_trim(expression_parts[0]);
        let variable_name =
            if expression_parts.len() == 2 { js_trim(expression_parts[1]) } else { "$error" };

        if name.is_empty() || variable_name.is_empty() {
            error(
                "Invalid @error block \"let\" parameter. Parameter should match the pattern \"<name> = <variable name>\"".to_string(),
            );
        } else if !is_identifier(name) {
            error("\"let\" parameter must be a valid JavaScript identifier".to_string());
        } else {
            if variable_name != "$error" && variable_name != "$reset" {
                error(format!(
                    "Unknown context variable \"{variable_name}\". Only \"$error\" and \"$reset\" are allowed"
                ));
            } else if context.iter().any(|v| v.name.as_str() == name) {
                error(format!("Duplicate \"let\" parameter variable \"{name}\""));
            }

            let key_span = surrounding_whitespace(expression_parts[0])
                .map_or(Span::new(span_start, source_span.end), |(lead, len)| {
                    Span::new(start + lead as u32, start + (lead + len) as u32)
                });
            let value_span = if expression_parts.len() == 2 {
                surrounding_whitespace(expression_parts[1]).map(|(lead, len)| {
                    let value_start = start + (expression_parts[0].len() + 1 + lead) as u32;
                    Span::new(value_start, value_start + len as u32)
                })
            } else {
                None
            };
            let variable_span =
                Span::new(key_span.start, value_span.map_or(key_span.end, |s| s.end));
            context.push(R3Variable {
                name: Ident::from(allocator.alloc_str(name)),
                value: Ident::from(allocator.alloc_str(variable_name)),
                source_span: variable_span,
                key_span,
                value_span,
            });
        }
        start += part.len() as u32 + 1;
    }
}

/// Parses the parameters of an `@error` block connected to a `@boundary`.
///
/// `source_text` is the template the parameter spans point into.
///
/// Port of the per-block loop in Angular's `createBoundaryBlock`.
pub fn parse_boundary_error_params<'a>(
    allocator: &'a Allocator,
    source_text: &'a str,
    parameters: &[HtmlBlockParameter<'a>],
    binding_parser: &BindingParser<'a>,
    start_span: Span,
) -> BoundaryErrorParams<'a> {
    let mut errors = std::vec::Vec::new();
    let mut expression_errors = std::vec::Vec::new();
    let empty_span = Span::new(start_span.end, start_span.end);
    let implicit = |name: &'static str| R3Variable {
        name: Ident::from(name),
        value: Ident::from(name),
        source_span: empty_span,
        key_span: empty_span,
        value_span: Some(empty_span),
    };
    let mut context_variables = Vec::new_in(&allocator);
    context_variables.push(implicit("$error"));
    context_variables.push(implicit("$reset"));
    let mut expression = None;

    for param in parameters {
        // Angular's parameter token keeps the whitespace before the next `;` or `)`, which
        // the lexer trims here; take the raw text so `when ` and `let ` match as upstream.
        let param_str = source_text
            .get(param.span.start as usize..param.span.end as usize)
            .filter(|raw| js_trim(raw) == param.expression.as_str())
            .unwrap_or(param.expression.as_str());
        let let_match = match_let(param_str);
        if let_match.is_some() || is_alias_match(param_str) {
            let (offset, to_parse) = let_match.unwrap_or((0, param_str));
            parse_let_parameters(
                allocator,
                param.span,
                to_parse,
                param.span.start + offset as u32,
                &mut context_variables,
                &mut errors,
            );
            continue;
        }

        if let Some(when_expr) = match_when(param_str) {
            if expression.is_some() {
                errors.push(BoundaryParamError {
                    msg: "@error block can only have one \"when\" expression".to_string(),
                    span: param.span,
                });
            } else {
                let start = param_str.find(when_expr).unwrap_or(0);
                let text = allocator.alloc_str(&param_str[start..start + when_expr.len()]);
                let result = binding_parser.parse_binding(
                    text,
                    Span::new(param.span.start + start as u32, param.span.end),
                );
                expression_errors.extend(result.errors);
                expression = Some(result.ast);
            }
            continue;
        }

        errors.push(BoundaryParamError {
            msg: format!("Unrecognized @error block parameter \"{param_str}\""),
            span: param.span,
        });
    }

    BoundaryErrorParams { context_variables, expression, errors, expression_errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(match_let("let err"), Some((4, "err")));
        assert_eq!(match_let("let  a = $error, b = $reset"), Some((5, "a = $error, b = $reset")));
        assert_eq!(match_let("letter"), None);
        assert!(is_alias_match("retry = $reset"));
        assert!(is_alias_match(" $r=$reset"));
        assert!(!is_alias_match("when a === b"));
        assert!(!is_alias_match("1a = b"));
        assert_eq!(match_when("when e.code === 1"), Some("e.code === 1"));
        assert_eq!(match_when("when\n  a\n && b"), Some("a"));
        assert_eq!(match_when("whenever"), None);
        // JavaScript's `\s` has U+FEFF but not U+0085.
        assert_eq!(match_let("let\u{FEFF}e"), Some((6, "e")));
        assert_eq!(match_let("let\u{85}e"), None);
        assert_eq!(match_when("when\u{FEFF}x"), Some("x"));
        assert_eq!(match_when("when\u{85}x"), None);
    }
}

//! Angular expression parser.
//!
//! Parses Angular binding expressions including:
//! - Interpolations: `{{ expression }}`
//! - Property bindings: `[property]="expression"`
//! - Event bindings: `(event)="handler($event)"`
//! - Two-way bindings: `[(ngModel)]="property"`
//! - Pipes: `value | pipeName:arg1:arg2`
//! - Safe navigation: `object?.property`
//! - Template microsyntax: `*ngFor="let item of items"`

mod lexer;
mod parser;
mod simple_checker;

pub use lexer::*;
pub use parser::*;
pub use simple_checker::*;

use oxc_allocator::Allocator;
use oxc_span::Span;

use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, TemplateBinding, TemplateBindingIdentifier,
};

/// Result of parsing template bindings (microsyntax).
pub struct TemplateBindingParseResult<'a> {
    /// The parsed bindings.
    pub bindings: oxc_allocator::Vec<'a, TemplateBinding<'a>>,
    /// Any parsing errors.
    pub errors: std::vec::Vec<String>,
    /// Any warnings.
    pub warnings: std::vec::Vec<String>,
}

/// Result of stripping comments from an expression.
pub struct StripCommentsResult<'a> {
    /// The stripped expression (without comment).
    pub stripped: &'a str,
    /// Whether a comment was found.
    pub has_comments: bool,
    /// The position where the comment starts (if any).
    pub comment_start: Option<usize>,
}

/// Finds the start of a `//` comment, respecting string quotes.
///
/// Returns `None` if no comment is found, or the byte position of `//`.
pub fn find_comment_start(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut outer_quote: Option<u8> = None;

    for i in 0..bytes.len().saturating_sub(1) {
        let ch = bytes[i];
        let next_ch = bytes[i + 1];

        // Check for // outside of quotes
        if ch == b'/' && next_ch == b'/' && outer_quote.is_none() {
            return Some(i);
        }

        // Track quote state
        if outer_quote == Some(ch) {
            outer_quote = None;
        } else if outer_quote.is_none() && (ch == b'\'' || ch == b'"' || ch == b'`') {
            outer_quote = Some(ch);
        }
    }

    None
}

/// Strips `//` comments from an expression, respecting string quotes.
pub fn strip_comments(input: &str) -> StripCommentsResult<'_> {
    match find_comment_start(input) {
        Some(pos) => StripCommentsResult {
            stripped: &input[..pos],
            has_comments: true,
            comment_start: Some(pos),
        },
        None => StripCommentsResult { stripped: input, has_comments: false, comment_start: None },
    }
}

/// A piece of text with start and end positions.
#[derive(Debug, Clone)]
pub struct InterpolationPiece {
    /// The text content.
    pub text: String,
    /// The start position in the input.
    pub start: usize,
    /// The end position in the input.
    pub end: usize,
}

/// Result of splitting interpolation from text.
///
/// This splits text like `"{{a}}  {{b}}  {{c}}"` into:
/// - `strings`: Text between interpolations
/// - `expressions`: Expression text inside `{{...}}`
/// - `offsets`: Start positions of each expression (after `{{`)
#[derive(Debug)]
pub struct SplitInterpolation {
    /// Text pieces between interpolations.
    pub strings: std::vec::Vec<InterpolationPiece>,
    /// Expression pieces inside interpolations.
    pub expressions: std::vec::Vec<InterpolationPiece>,
    /// Start positions of each expression (position of first character after `{{`).
    pub offsets: std::vec::Vec<usize>,
}

/// A high-level parser for Angular bindings that wraps the expression parser.
///
/// This provides convenient methods for parsing different types of Angular
/// template expressions: property bindings, event handlers, interpolations,
/// and template microsyntax.
pub struct BindingParser<'a> {
    allocator: &'a Allocator,
}

impl<'a> BindingParser<'a> {
    /// Creates a new binding parser.
    pub fn new(allocator: &'a Allocator) -> Self {
        Self { allocator }
    }

    /// Parses a property binding expression.
    ///
    /// Used for `[property]="expression"` and `bind-property="expression"`.
    ///
    /// # Arguments
    /// * `value` - The binding expression text (without the attribute delimiters)
    /// * `span` - The source span for error reporting
    ///
    /// # Returns
    /// The parsed expression and any errors.
    pub fn parse_binding(&self, value: &'a str, span: Span) -> ParseResult<'a> {
        let parser = Parser::with_offset(self.allocator, value, span.start);
        parser.parse_simple_binding()
    }

    /// Parses an event handler expression.
    ///
    /// Used for `(event)="handler($event)"` and `on-event="handler($event)"`.
    /// Event handlers can contain statement chains (separated by `;`).
    ///
    /// # Arguments
    /// * `value` - The handler expression text
    /// * `span` - The source span for error reporting
    ///
    /// # Returns
    /// The parsed expression and any errors.
    pub fn parse_event(&self, value: &'a str, span: Span) -> ParseResult<'a> {
        let parser = Parser::with_offset(self.allocator, value, span.start);
        parser.parse_action()
    }

    /// Parses an interpolation expression.
    ///
    /// Used for `{{ expression }}` within text nodes.
    ///
    /// # Arguments
    /// * `value` - The full text containing interpolations
    /// * `span` - The source span for error reporting
    /// * `start_delimiter` - The interpolation start marker (usually `{{`)
    /// * `end_delimiter` - The interpolation end marker (usually `}}`)
    ///
    /// # Returns
    /// The parsed interpolation expression, or None if no interpolations found.
    pub fn parse_interpolation(
        &self,
        value: &'a str,
        span: Span,
        start_delimiter: &str,
        end_delimiter: &str,
    ) -> Option<ParseResult<'a>> {
        // Use for_interpolation to avoid tokenizing the full input including
        // literal text like "/ " before interpolations.
        let parser = Parser::for_interpolation(self.allocator, value, span.start);
        parser.parse_interpolation(start_delimiter, end_delimiter)
    }

    /// Parses an interpolation with default delimiters (`{{` and `}}`).
    pub fn parse_default_interpolation(
        &self,
        value: &'a str,
        span: Span,
    ) -> Option<ParseResult<'a>> {
        self.parse_interpolation(value, span, "{{", "}}")
    }

    /// Extracts the expression from a parse result.
    ///
    /// Returns the AST expression, ignoring any parse errors.
    /// This is useful when you want to continue compilation even with errors.
    pub fn extract_expression(result: ParseResult<'a>) -> AngularExpression<'a> {
        result.ast
    }

    /// Parses template bindings (microsyntax) like `*ngFor="let item of items"`.
    ///
    /// # Arguments
    /// * `template_key` - The directive name (e.g., "ngFor", "ngIf")
    /// * `template_value` - The microsyntax expression (e.g., "let item of items")
    /// * `key_span` - The span of the directive name
    /// * `value_span` - The span of the expression
    ///
    /// # Returns
    /// The parsed template bindings.
    ///
    /// # Examples
    /// ```text
    /// *ngFor="let item of items; let i = index; trackBy: trackByFn"
    /// *ngIf="condition | async as result"
    /// ```
    pub fn parse_template_bindings(
        &self,
        template_key: &'a str,
        template_value: &'a str,
        key_span: Span,
        value_span: Span,
    ) -> TemplateBindingParseResult<'a> {
        let parser = Parser::with_offset(self.allocator, template_value, value_span.start);
        let key_identifier = TemplateBindingIdentifier {
            source: oxc_span::Ident::from(template_key),
            span: AbsoluteSourceSpan::new(key_span.start, key_span.end),
        };
        parser.parse_template_bindings(key_identifier)
    }
}

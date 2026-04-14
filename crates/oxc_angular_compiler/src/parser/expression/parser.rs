//! Angular expression parser.
//!
//! Parses Angular binding expressions using the tokens from the lexer.
//!
//! Ported from Angular's `expression_parser/parser.ts`.

use oxc_allocator::{Allocator, Box, FromIn, Vec};
use oxc_str::Ident;

use crate::ast::expression::{
    ASTWithSource, AbsoluteSourceSpan, AngularExpression, ArrowFunction, ArrowFunctionParameter,
    Binary, BinaryOperator, BindingPipe, BindingPipeType, Call, Chain, Conditional, EmptyExpr,
    ExpressionBinding, ImplicitReceiver, Interpolation, KeyedRead, LiteralArray, LiteralMap,
    LiteralMapKey, LiteralMapPropertyKey, LiteralMapSpreadKey, LiteralPrimitive, LiteralValue,
    NonNullAssert, ParenthesizedExpression, ParseSpan, PrefixNot, PropertyRead,
    RegularExpressionLiteral, SafeCall, SafeKeyedRead, SafePropertyRead, SpreadElement,
    TaggedTemplateLiteral, TemplateBinding, TemplateBindingIdentifier, TemplateLiteral,
    TemplateLiteralElement, ThisReceiver, TypeofExpression, Unary, UnaryOperator, VariableBinding,
    VoidExpression,
};
use crate::util::ParseError;

use super::TemplateBindingParseResult;
use super::lexer::{Lexer, Token};

/// Helper to capitalize the first character of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Result of parsing an expression.
pub struct ParseResult<'a> {
    /// The parsed expression.
    pub ast: AngularExpression<'a>,
    /// Parsing errors.
    pub errors: std::vec::Vec<ParseError>,
}

/// Parsing context flags for error recovery.
///
/// These flags indicate what context we're in, which affects
/// what tokens are considered recovery points.
#[derive(Clone, Copy, Default)]
struct ParseContextFlags(u8);

impl ParseContextFlags {
    /// No special context.
    const NONE: Self = Self(0);
    /// In a writable context where assignment is possible.
    /// This allows `=` to be a recovery point.
    const WRITABLE: Self = Self(1);

    fn contains(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }
}

impl std::ops::BitOr for ParseContextFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for ParseContextFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitXorAssign for ParseContextFlags {
    fn bitxor_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}

/// Angular expression parser.
pub struct Parser<'a> {
    /// The allocator.
    allocator: &'a Allocator,
    /// The tokens to parse.
    tokens: std::vec::Vec<Token<'a>>,
    /// Current token index.
    index: usize,
    /// The source text (original, before comment stripping).
    source: &'a str,
    /// Parsing errors.
    errors: std::vec::Vec<ParseError>,
    /// Absolute offset in the template.
    absolute_offset: u32,
    /// Count of expected right parentheses for error recovery.
    rparens_expected: u32,
    /// Count of expected right brackets for error recovery.
    rbrackets_expected: u32,
    /// Count of expected right braces for error recovery.
    rbraces_expected: u32,
    /// Current parsing context for error recovery.
    context: ParseContextFlags,
    /// Whether we're in action mode (event handlers, where assignments are allowed).
    action_mode: bool,
}

impl<'a> Parser<'a> {
    /// Creates a new parser.
    pub fn new(allocator: &'a Allocator, source: &'a str) -> Self {
        // Strip comments from the source before tokenizing
        let stripped_source = Self::strip_comments(source);
        // Allocate the stripped source in the allocator to get 'a lifetime
        let stripped_ref: &'a str = allocator.alloc_str(&stripped_source);
        let lexer = Lexer::new(allocator, stripped_ref);
        let tokens = lexer.tokenize();

        // Collect error tokens from the lexer
        let mut errors = std::vec::Vec::new();
        for token in &tokens {
            if token.is_error() {
                errors.push(ParseError::new(Default::default(), token.str_value.to_string()));
            }
        }

        Self {
            allocator,
            tokens,
            index: 0,
            source,
            errors,
            absolute_offset: 0,
            rparens_expected: 0,
            rbrackets_expected: 0,
            rbraces_expected: 0,
            context: ParseContextFlags::NONE,
            action_mode: false,
        }
    }

    /// Creates a new parser with absolute offset.
    pub fn with_offset(allocator: &'a Allocator, source: &'a str, offset: u32) -> Self {
        let mut parser = Self::new(allocator, source);
        parser.absolute_offset = offset;
        parser
    }

    /// Creates a parser for interpolation parsing without tokenizing.
    ///
    /// Interpolation parsing doesn't need tokenization of the full input because
    /// it manually splits the text to find `{{` and `}}` boundaries, then creates
    /// sub-parsers for each expression part.
    pub fn for_interpolation(allocator: &'a Allocator, source: &'a str, offset: u32) -> Self {
        Self {
            allocator,
            tokens: std::vec::Vec::new(),
            index: 0,
            source,
            errors: std::vec::Vec::new(),
            absolute_offset: offset,
            rparens_expected: 0,
            rbrackets_expected: 0,
            rbraces_expected: 0,
            context: ParseContextFlags::NONE,
            action_mode: false,
        }
    }

    /// Strips single-line comments (`//`) from the input.
    ///
    /// Comments are only recognized outside of quoted strings.
    /// This matches Angular's behavior in `_stripComments`.
    fn strip_comments(input: &str) -> String {
        let comment_start = Self::find_comment_start(input);
        if let Some(idx) = comment_start { input[..idx].to_string() } else { input.to_string() }
    }

    /// Finds the start index of a `//` comment, respecting quoted strings.
    ///
    /// Returns `None` if no comment is found.
    fn find_comment_start(input: &str) -> Option<usize> {
        let bytes = input.as_bytes();
        let mut outer_quote: Option<u8> = None;

        for i in 0..input.len().saturating_sub(1) {
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

    /// Checks for interpolation syntax (`{{` and `}}`) in the input.
    ///
    /// If found, adds an error: "Got interpolation ({{}}) where expression was expected".
    /// This is called for binding expressions where interpolation is not allowed.
    fn check_no_interpolation(&mut self) {
        let bytes = self.source.as_bytes();
        let mut outer_quote: Option<u8> = None;
        let mut start_index: Option<usize> = None;
        let mut escape_count = 0;

        for i in 0..self.source.len() {
            let ch = bytes[i];

            // Track escape sequences
            if ch == b'\\' {
                escape_count += 1;
                continue;
            }

            // Track quote state (respecting escapes)
            if (ch == b'\'' || ch == b'"' || ch == b'`')
                && (outer_quote.is_none() || outer_quote == Some(ch))
                && escape_count % 2 == 0
            {
                if outer_quote == Some(ch) {
                    outer_quote = None;
                } else {
                    outer_quote = Some(ch);
                }
                escape_count = 0;
                continue;
            }

            escape_count = 0;

            // Only check for {{ and }} outside of quotes
            if outer_quote.is_some() {
                continue;
            }

            if start_index.is_none() {
                // Look for {{
                if i + 1 < self.source.len() && ch == b'{' && bytes[i + 1] == b'{' {
                    start_index = Some(i);
                }
            } else if let Some(idx) = start_index {
                // Look for }}
                if i + 1 < self.source.len() && ch == b'}' && bytes[i + 1] == b'}' {
                    // Found complete interpolation
                    // Note: {{{{ and }}}} are needed to produce literal {{ and }} in format!
                    let error_msg = format!(
                        "Got interpolation ({{{{}}}}) where expression was expected at column {}",
                        idx
                    );
                    self.errors.push(ParseError::new(Default::default(), error_msg));
                    return;
                }
            }
        }
    }

    /// Parses a simple binding expression.
    ///
    /// This parses the full expression including pipes. For host bindings
    /// where pipes are not allowed, use `SimpleExpressionChecker::check()`
    /// to validate the result.
    pub fn parse_simple_binding(mut self) -> ParseResult<'a> {
        // Check for interpolation syntax which is not allowed in bindings
        self.check_no_interpolation();
        let ast = self.parse_pipe();

        // Check for chain expressions (semicolons) which are not allowed in bindings
        if self.peek().is_some_and(|t| t.is_character(';')) {
            self.errors.push(ParseError::new(
                Default::default(),
                "Binding expression cannot contain chained expression".to_string(),
            ));
        } else if !self.at_end() {
            // There are unconsumed tokens - report an unexpected token error
            if let Some(token) = self.peek() {
                self.error(&format!("Unexpected token '{}'", token.str_value));
            }
        }

        ParseResult { ast, errors: self.errors }
    }

    /// Parses an event handler expression (allows assignment).
    ///
    /// In action mode, assignments like `a = b`, `obj.prop = value`, and `arr[i] = value`
    /// are allowed, as well as compound assignments like `+=`, `-=`, `||=`, etc.
    pub fn parse_action(mut self) -> ParseResult<'a> {
        // Check for interpolation syntax which is not allowed in actions
        self.check_no_interpolation();
        self.action_mode = true;
        let ast = self.parse_chain();
        ParseResult { ast, errors: self.errors }
    }

    /// Wraps a literal primitive value in an AST.
    ///
    /// This is used for creating AST nodes for literal attribute values
    /// without parsing them as expressions.
    pub fn wrap_literal_primitive(
        &self,
        input: Option<&'a str>,
        location: &'a str,
        absolute_offset: u32,
    ) -> ASTWithSource<'a> {
        let len = input.map(|s| s.len() as u32).unwrap_or(0);
        let span = ParseSpan::new(0, len);
        let source_span = span.to_absolute(absolute_offset);

        let value = match input {
            Some(s) => LiteralValue::String(Ident::from_in(s, self.allocator)),
            None => LiteralValue::Null,
        };

        let literal = LiteralPrimitive { span, source_span, value };
        let ast = AngularExpression::LiteralPrimitive(Box::new_in(literal, self.allocator));

        ASTWithSource {
            ast,
            source: input.map(|s| Ident::from_in(s, self.allocator)),
            location: Ident::from_in(location, self.allocator),
            absolute_offset,
        }
    }

    /// Splits an input string into interpolation parts.
    ///
    /// This method extracts text pieces and expression pieces from a string
    /// containing interpolations like `"{{a}}  {{b}}  {{c}}"`.
    ///
    /// # Arguments
    /// * `input` - The input string to split
    /// * `interp_start` - The interpolation start marker (usually `{{`)
    /// * `interp_end` - The interpolation end marker (usually `}}`)
    ///
    /// # Returns
    /// A `SplitInterpolation` containing strings, expressions, and offsets.
    pub fn split_interpolation(
        input: &str,
        interp_start: &str,
        interp_end: &str,
    ) -> super::SplitInterpolation {
        let mut strings = std::vec::Vec::new();
        let mut expressions = std::vec::Vec::new();
        let mut offsets = std::vec::Vec::new();

        let mut i = 0;
        let mut at_interpolation = false;
        let mut extend_last_string = false;

        while i < input.len() {
            if !at_interpolation {
                // Parse until starting {{
                let start = i;
                match input[i..].find(interp_start) {
                    Some(pos) => i += pos,
                    None => i = input.len(),
                }
                let text = input[start..i].to_string();
                strings.push(super::InterpolationPiece { text, start, end: i });
                at_interpolation = true;
            } else {
                // Parse from starting {{ to ending }}
                let full_start = i;
                let expr_start = full_start + interp_start.len();

                // Find the end of the interpolation
                match Self::get_interpolation_end_index(input, interp_end, expr_start) {
                    Some(expr_end) => {
                        let full_end = expr_end + interp_end.len();
                        let text = input[expr_start..expr_end].to_string();
                        expressions.push(super::InterpolationPiece {
                            text,
                            start: full_start,
                            end: full_end,
                        });
                        offsets.push(expr_start);
                        i = full_end;
                        at_interpolation = false;
                    }
                    None => {
                        // Could not find the end of the interpolation
                        at_interpolation = false;
                        extend_last_string = true;
                        break;
                    }
                }
            }
        }

        // Handle remaining content
        if !at_interpolation {
            if extend_last_string {
                if let Some(piece) = strings.last_mut() {
                    piece.text.push_str(&input[i..]);
                    piece.end = input.len();
                }
            } else {
                strings.push(super::InterpolationPiece {
                    text: input[i..].to_string(),
                    start: i,
                    end: input.len(),
                });
            }
        }

        super::SplitInterpolation { strings, expressions, offsets }
    }

    /// Finds the end of an interpolation expression.
    ///
    /// Looks for `interp_end` (e.g., `}}`) starting from `start` position,
    /// while handling quoted content.
    fn get_interpolation_end_index(input: &str, interp_end: &str, start: usize) -> Option<usize> {
        let mut current_quote: Option<char> = None;
        let chars: std::vec::Vec<char> = input.chars().collect();
        let mut i = start;

        while i < chars.len() {
            let char = chars[i];

            // Handle quote state
            if current_quote.is_none() {
                // Check for start of interpolation end marker
                if input[i..].starts_with(interp_end) {
                    return Some(i);
                }

                // Check for comment start - skip to end marker
                if input[i..].starts_with("//") {
                    return input[i..].find(interp_end).map(|pos| i + pos);
                }

                // Check for quotes
                if char == '\'' || char == '"' || char == '`' {
                    current_quote = Some(char);
                }
            } else if Some(char) == current_quote {
                // Check if this quote is escaped
                let mut escape_count = 0;
                let mut j = i;
                while j > start && chars[j - 1] == '\\' {
                    escape_count += 1;
                    j -= 1;
                }
                // If even number of escapes, the quote closes
                if escape_count % 2 == 0 {
                    current_quote = None;
                }
            }

            i += 1;
        }

        None
    }

    /// Parses template bindings (microsyntax).
    ///
    /// Handles expressions like:
    /// - `*ngFor="let item of items; let i = index; trackBy: trackByFn"`
    /// - `*ngIf="condition | async as result"`
    ///
    /// The `template_key` is the directive name (e.g., `ngFor`, `ngIf`).
    pub fn parse_template_bindings(
        mut self,
        template_key: TemplateBindingIdentifier<'a>,
    ) -> TemplateBindingParseResult<'a> {
        let mut bindings = Vec::new_in(self.allocator);
        let mut errors = std::vec::Vec::new();
        let warnings = std::vec::Vec::new();

        // First, check if we start with `let` keyword
        if self.peek_keyword_let() {
            // `*ngFor="let item..."` - create expression binding with null value for directive
            let key = self.make_template_binding_identifier(
                &template_key.source,
                template_key.span.start,
                template_key.span.end,
            );
            let expr_binding =
                ExpressionBinding { source_span: template_key.span, key, value: None };
            bindings.push(TemplateBinding::Expression(expr_binding));
        } else {
            // Parse the primary expression (e.g., `condition` in `*ngIf="condition"`)
            let start = self.peek().map(|t| t.index).unwrap_or(0);
            let value = self.parse_pipe_for_template_binding();
            // Value source is just the expression (up to current_end_index)
            let value_end = self.current_end_index();
            // Source span extends to start of next token (includes trailing whitespace)
            let source_end = self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32);

            let value_source = &self.source[start as usize..value_end as usize];

            let key = self.make_template_binding_identifier(
                &template_key.source,
                template_key.span.start,
                template_key.span.end,
            );
            let source_span =
                AbsoluteSourceSpan::new(template_key.span.start, self.absolute_offset + source_end);

            // Angular expects None for empty values (e.g., `*a=""`)
            let expr_value = if value_source.is_empty() {
                None
            } else {
                Some(ASTWithSource {
                    ast: value,
                    source: Some(Ident::from_in(value_source, self.allocator)),
                    location: Ident::from_in("", self.allocator),
                    absolute_offset: self.absolute_offset + start,
                })
            };

            let expr_binding = ExpressionBinding { source_span, key, value: expr_value };
            bindings.push(TemplateBinding::Expression(expr_binding));

            // Check for `as` binding after the primary expression (e.g., `*ngIf="cond | async as result"`)
            if self.optional_keyword("as") {
                let as_binding = self.parse_as_binding(
                    &template_key.source,
                    template_key.span.start,
                    template_key.span,
                );
                if let Some(binding) = as_binding {
                    bindings.push(binding);
                }
            }
        }

        // Now process remaining bindings
        loop {
            // Check for statement terminator
            if !self.consume_statement_terminator() && !self.at_end() {
                // If we can't consume a terminator and we're not at end, something's wrong
                if !self.peek_keyword_let() && !self.peek_identifier_or_keyword() {
                    break;
                }
            }

            if self.at_end() {
                break;
            }

            // Check for `as` binding (e.g., `*ngIf="cond | async as result"`)
            // This is handled inside parseDirectiveKeywordBindings

            // Check for `let` keyword
            if self.peek_keyword_let() {
                let let_binding = self.parse_let_binding(&template_key.source);
                if let Some(binding) = let_binding {
                    bindings.push(binding);
                }
                continue;
            }

            // Parse directive keyword binding (e.g., `of items`, `trackBy: func`)
            let keyword_bindings = self.parse_directive_keyword_bindings(&template_key.source);
            for binding in keyword_bindings {
                bindings.push(binding);
            }
        }

        // Collect any parse errors
        for err in &self.errors {
            errors.push(err.msg.clone());
        }

        TemplateBindingParseResult { bindings, errors, warnings }
    }

    // ========================================================================
    // Template binding helpers
    // ========================================================================

    /// Checks if the current token is the `let` keyword.
    fn peek_keyword_let(&self) -> bool {
        self.peek().map(|t| t.is_keyword_value("let")).unwrap_or(false)
    }

    /// Checks if the current token is an identifier or keyword.
    fn peek_identifier_or_keyword(&self) -> bool {
        self.peek().map(|t| t.is_identifier() || t.is_keyword()).unwrap_or(false)
    }

    /// Creates a `TemplateBindingIdentifier`.
    fn make_template_binding_identifier(
        &self,
        source: &str,
        start: u32,
        end: u32,
    ) -> TemplateBindingIdentifier<'a> {
        TemplateBindingIdentifier {
            source: Ident::from_in(source, self.allocator),
            span: AbsoluteSourceSpan::new(start, end),
        }
    }

    /// Consumes a statement terminator (`;` or `,`).
    fn consume_statement_terminator(&mut self) -> bool {
        self.optional_character(';') || self.optional_character(',')
    }

    /// Parses a `let` binding: `let item` or `let i = index`.
    fn parse_let_binding(&mut self, _template_key: &str) -> Option<TemplateBinding<'a>> {
        // Record the position of 'let' keyword before consuming it
        let let_token = self.peek()?.clone();
        let let_start = self.absolute_offset + let_token.index;

        // Consume `let` keyword
        if !self.optional_keyword("let") {
            return None;
        }

        // Get variable name
        let key_token = self.peek()?.clone();
        if !key_token.is_identifier() && !key_token.is_keyword() {
            self.error("Expected identifier after 'let'");
            return None;
        }
        let key_start = self.absolute_offset + key_token.index;
        let key_end = self.absolute_offset + key_token.end;
        let key_source = key_token.str_value.clone();
        self.advance();

        let key = self.make_template_binding_identifier(key_source.as_str(), key_start, key_end);

        // Check for `=` (e.g., `let i = index`)
        let (value, source_end) = if self.optional_operator("=") {
            let value_token = self.peek()?.clone();
            if !value_token.is_identifier() && !value_token.is_keyword() {
                self.error("Expected identifier after '='");
                return None;
            }
            let value_start = self.absolute_offset + value_token.index;
            let value_end = self.absolute_offset + value_token.end;

            // Use the value as-is (e.g., `let i = index` -> value is "index")
            let value_source = value_token.str_value.as_str();
            self.advance();
            // Source span extends to include the value
            (
                Some(self.make_template_binding_identifier(value_source, value_start, value_end)),
                value_end,
            )
        } else {
            // For bare `let item`, Angular expects no value (None)
            // Source span includes trailing space after the variable name
            (None, key_end + 1)
        };

        // Source span starts from 'let' keyword
        let source_span = AbsoluteSourceSpan::new(let_start, source_end);
        Some(TemplateBinding::Variable(VariableBinding { source_span, key, value }))
    }

    /// Parses directive keyword bindings: `of items`, `trackBy: func`.
    ///
    /// Per TypeScript's parser, we need to check for `as` binding BEFORE transforming
    /// the keyword to the full key. For example:
    /// - `first as isFirst` -> value is `first`, NOT `ngForFirst`
    /// - `index as i` -> value is `index`, NOT `ngForIndex`
    ///
    /// The context object (NgForOfContext) has properties like `first`, `last`, `index`,
    /// not `ngForFirst`, `ngForLast`, `ngForIndex`.
    fn parse_directive_keyword_bindings(
        &mut self,
        template_key: &str,
    ) -> std::vec::Vec<TemplateBinding<'a>> {
        let mut result = std::vec::Vec::new();

        // Get the keyword (e.g., `of`, `trackBy`, `first`, `index`)
        let keyword_token = match self.peek() {
            Some(t) if t.is_identifier() || t.is_keyword() => t.clone(),
            _ => return result,
        };
        let keyword_start = self.absolute_offset + keyword_token.index;
        let keyword_end = self.absolute_offset + keyword_token.end;
        let keyword = keyword_token.str_value.clone();
        self.advance();

        // Check for `as` binding BEFORE constructing the full key.
        // Per TypeScript (parser.ts lines 1369-1385):
        // - Read keyword first (`first`, `index`, etc.)
        // - If followed by `as`, this is a variable binding with the ORIGINAL keyword as value
        // - If NOT followed by `as`, transform keyword to full key (`ngForFirst`, `ngForIndex`, etc.)
        if self.optional_keyword("as") {
            // This is a `value as alias` pattern (e.g., `first as isFirst`)
            // The value should be the ORIGINAL keyword (e.g., `first`), not transformed
            let value_key_span = AbsoluteSourceSpan::new(keyword_start, keyword_end);
            // Pass the original keyword (e.g., "first"), NOT the full key (e.g., "ngForFirst")
            let as_binding = self.parse_as_binding(keyword.as_str(), keyword_start, value_key_span);
            if let Some(binding) = as_binding {
                result.push(binding);
            }
            return result;
        }

        // Consume optional `:` for directive keywords like `trackBy: func`
        self.optional_character(':');

        // Now construct the full key (e.g., `ngForOf`, `ngForTrackBy`)
        let full_key = format!("{}{}", template_key, capitalize_first(keyword.as_str()));
        let key = self.make_template_binding_identifier(&full_key, keyword_start, keyword_end);

        // Parse the value expression
        let expr_start = self.peek().map(|t| t.index).unwrap_or(0);

        // Check if there's actually an expression to parse
        if self.at_end()
            || self.peek().map(|t| t.is_character(';') || t.is_character(',')).unwrap_or(false)
        {
            // No value - `*ngIf="cond; else elseBlock"` where `else` has no value
            let source_span = AbsoluteSourceSpan::new(keyword_start, keyword_end);
            result.push(TemplateBinding::Expression(ExpressionBinding {
                source_span,
                key: key.clone(),
                value: None,
            }));

            return result;
        }

        let value = self.parse_pipe_for_template_binding();
        // Value source is just the expression (up to current_end_index)
        let value_end = self.current_end_index();

        // Source span extends to start of next binding (includes trailing separator and whitespace)
        // If the next token is `;` or `,`, consume it and use the start of the following token
        let source_end =
            if self.peek().map(|t| t.is_character(';') || t.is_character(',')).unwrap_or(false) {
                // Consume the separator
                self.advance();
                // Source span extends to start of next token (after separator)
                self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32)
            } else {
                // No separator, source span extends to start of next token
                self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32)
            };

        let value_source = &self.source[expr_start as usize..value_end as usize];
        let ast_with_source = ASTWithSource {
            ast: value,
            source: Some(Ident::from_in(value_source, self.allocator)),
            location: Ident::from_in("", self.allocator),
            absolute_offset: self.absolute_offset + expr_start,
        };

        let source_span = AbsoluteSourceSpan::new(keyword_start, self.absolute_offset + source_end);
        result.push(TemplateBinding::Expression(ExpressionBinding {
            source_span,
            key,
            value: Some(ast_with_source),
        }));

        // Check for `as` binding after the expression
        if self.optional_keyword("as") {
            // The value_key_span points to the keyword (e.g., "of" in "of items")
            let value_key_span = AbsoluteSourceSpan::new(keyword_start, keyword_end);
            let as_binding = self.parse_as_binding(&full_key, keyword_start, value_key_span);
            if let Some(binding) = as_binding {
                result.push(binding);
            }
        }

        result
    }

    /// Parses an `as` binding: `... as alias`.
    /// `source_span_start` is the start of the source span (directive name start).
    /// `value_key_span` is the span of the directive name for the value.
    fn parse_as_binding(
        &mut self,
        value_key: &str,
        source_span_start: u32,
        value_key_span: AbsoluteSourceSpan,
    ) -> Option<TemplateBinding<'a>> {
        // Get alias name
        let alias_token = self.peek()?.clone();
        if !alias_token.is_identifier() && !alias_token.is_keyword() {
            self.error("Expected identifier after 'as'");
            return None;
        }
        let alias_start = self.absolute_offset + alias_token.index;
        let alias_end = self.absolute_offset + alias_token.end;
        let alias_source = alias_token.str_value.clone();
        self.advance();

        let key =
            self.make_template_binding_identifier(alias_source.as_str(), alias_start, alias_end);
        // The value's span should point to the directive name, not the alias
        let value = self.make_template_binding_identifier(
            value_key,
            value_key_span.start,
            value_key_span.end,
        );

        // Source span extends from directive name start to alias end
        let source_span = AbsoluteSourceSpan::new(source_span_start, alias_end);
        Some(TemplateBinding::Variable(VariableBinding { source_span, key, value: Some(value) }))
    }

    /// Parses a pipe expression for template bindings.
    /// This is similar to `parse_pipe` but stops at template binding terminators.
    fn parse_pipe_for_template_binding(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut result = self.parse_conditional();

        while self.optional_operator("|") {
            // Check if this is actually a pipe or just end of expression
            if let Some(token) = self.peek().cloned() {
                if token.is_identifier() {
                    let name = token.str_value.clone();
                    let name_start = token.index;
                    let name_end = token.end;
                    self.advance();

                    // Parse pipe arguments (stop at `;`, `,`, or `as`)
                    let mut args = Vec::new_in(self.allocator);
                    while self.optional_character(':') {
                        // Check for terminators
                        if self
                            .peek()
                            .map(|t| {
                                t.is_character(';')
                                    || t.is_character(',')
                                    || t.is_keyword_value("as")
                            })
                            .unwrap_or(true)
                        {
                            break;
                        }
                        args.push(self.parse_conditional());
                    }

                    let end = self.current_end_index();
                    let span = ParseSpan::new(start, end);
                    let source_span = span.to_absolute(self.absolute_offset);
                    let name_span = AbsoluteSourceSpan::new(
                        self.absolute_offset + name_start,
                        self.absolute_offset + name_end,
                    );

                    let pipe = BindingPipe {
                        span,
                        source_span,
                        name_span,
                        exp: result,
                        name,
                        args,
                        pipe_type: BindingPipeType::ReferencedByName,
                    };
                    result = AngularExpression::BindingPipe(Box::new_in(pipe, self.allocator));
                } else {
                    self.error("expected identifier or keyword");
                    break;
                }
            } else {
                self.error("expected identifier or keyword");
                break;
            }
        }

        result
    }

    /// Finds the end of an interpolation, respecting quotes, escapes, and comments.
    /// Returns the index of the end delimiter (relative to the start of the input),
    /// or None if no valid end is found.
    fn find_interpolation_end(input: &str, end_delimiter: &str) -> Option<usize> {
        let mut current_quote: Option<char> = None;
        let mut escape_count = 0;
        let mut in_single_line_comment = false;
        let mut in_multi_line_comment = false;
        let end_len = end_delimiter.len();
        let mut byte_pos = 0;
        let input_bytes = input.as_bytes();

        while byte_pos < input.len() {
            // Get current char
            let remaining = &input[byte_pos..];
            // Safety: byte_pos < input.len() guarantees at least one char
            let Some(ch) = remaining.chars().next() else { break };
            let ch_len = ch.len_utf8();

            // Get next char if available
            let next_ch = remaining.chars().nth(1);

            // Handle single-line comment
            // Note: In Angular interpolations, we still check for end delimiter inside comments
            // because the interpolation end `}}` takes precedence over comment continuation
            if in_single_line_comment {
                if ch == '\n' || ch == '\r' {
                    in_single_line_comment = false;
                }
                // Check for end delimiter even inside single-line comment
                // This is Angular-specific behavior for interpolations
                if byte_pos + end_len <= input.len()
                    && &input_bytes[byte_pos..byte_pos + end_len] == end_delimiter.as_bytes()
                {
                    return Some(byte_pos);
                }
                byte_pos += ch_len;
                continue;
            }

            // Handle multi-line comment end
            if in_multi_line_comment {
                if ch == '*' && next_ch == Some('/') {
                    in_multi_line_comment = false;
                    byte_pos += 2; // '*' and '/' are both 1 byte
                    continue;
                }
                byte_pos += ch_len;
                continue;
            }

            // Track escape sequences by counting consecutive backslashes
            if ch == '\\' {
                escape_count += 1;
                byte_pos += ch_len;
                continue;
            }

            // Check for quotes (only if not escaped: even number of backslashes)
            if (ch == '"' || ch == '\'' || ch == '`')
                && (current_quote.is_none() || current_quote == Some(ch))
                && escape_count % 2 == 0
            {
                current_quote = if current_quote.is_none() { Some(ch) } else { None };
            }

            // Check for comment start (only when not in quotes)
            if current_quote.is_none() && escape_count % 2 == 0 {
                if ch == '/' && next_ch == Some('/') {
                    in_single_line_comment = true;
                    byte_pos += 2; // '//' is 2 bytes
                    escape_count = 0;
                    continue;
                }
                if ch == '/' && next_ch == Some('*') {
                    in_multi_line_comment = true;
                    byte_pos += 2; // '/*' is 2 bytes
                    escape_count = 0;
                    continue;
                }
            }

            // If not inside quotes or comments, check for end delimiter
            if current_quote.is_none()
                && !in_single_line_comment
                && !in_multi_line_comment
                && byte_pos + end_len <= input.len()
                && &input_bytes[byte_pos..byte_pos + end_len] == end_delimiter.as_bytes()
            {
                return Some(byte_pos);
            }

            escape_count = 0;
            byte_pos += ch_len;
        }

        None
    }

    /// Parses an interpolation expression.
    pub fn parse_interpolation(
        mut self,
        start_delimiter: &str,
        end_delimiter: &str,
    ) -> Option<ParseResult<'a>> {
        // Find interpolation boundaries
        let start_len = start_delimiter.len();
        let end_len = end_delimiter.len();

        let mut strings = Vec::new_in(self.allocator);
        let mut expressions = Vec::new_in(self.allocator);

        let mut current_pos = 0;

        while current_pos < self.source.len() {
            // Find next start delimiter
            if let Some(start_idx) = self.source[current_pos..].find(start_delimiter) {
                let abs_start = current_pos + start_idx;

                // Find end delimiter (respecting quotes and escapes)
                let expr_start = abs_start + start_len;
                if let Some(end_idx) =
                    Self::find_interpolation_end(&self.source[expr_start..], end_delimiter)
                {
                    // Add text before the interpolation
                    let text = &self.source[current_pos..abs_start];
                    strings.push(Ident::from_in(text, self.allocator));

                    let expr_text = &self.source[expr_start..expr_start + end_idx];

                    // Parse the expression
                    let expr_parser = Parser::with_offset(
                        self.allocator,
                        expr_text,
                        self.absolute_offset + expr_start as u32,
                    );
                    let result = expr_parser.parse_simple_binding();
                    self.errors.extend(result.errors);
                    expressions.push(result.ast);

                    current_pos = expr_start + end_idx + end_len;
                } else {
                    // No end delimiter found for this {{
                    // Treat the {{ and everything after as literal text
                    // by adding all remaining text to strings and exiting
                    let text = &self.source[current_pos..];
                    strings.push(Ident::from_in(text, self.allocator));
                    break;
                }
            } else {
                // No more interpolations - add remaining text and exit
                let text = &self.source[current_pos..];
                strings.push(Ident::from_in(text, self.allocator));
                break;
            }
        }

        // If the loop exited because current_pos >= source.len() (i.e., we consumed
        // all input including the last "}}"), we still need to add the trailing string.
        // This ensures strings.len() == expressions.len() + 1 for proper interleaving.
        if !expressions.is_empty() && strings.len() == expressions.len() {
            strings.push(Ident::from_in("", self.allocator));
        }

        if expressions.is_empty() {
            return None;
        }

        let span = ParseSpan::new(0, self.source.len() as u32);
        let source_span = span.to_absolute(self.absolute_offset);
        let interpolation = Interpolation { span, source_span, strings, expressions };
        let ast = AngularExpression::Interpolation(Box::new_in(interpolation, self.allocator));

        Some(ParseResult { ast, errors: self.errors })
    }

    // ========================================================================
    // Token utilities
    // ========================================================================

    /// Returns the current token.
    fn peek(&self) -> Option<&Token<'a>> {
        self.tokens.get(self.index)
    }

    /// Returns the end position of the previous token.
    /// This is used for ImplicitReceiver spans which should cover the whitespace
    /// between the previous token and the current identifier.
    fn previous_end(&self) -> u32 {
        if self.index > 0 { self.tokens.get(self.index - 1).map(|t| t.end).unwrap_or(0) } else { 0 }
    }

    /// Advances to the next token.
    fn advance(&mut self) {
        if self.index < self.tokens.len() {
            self.index += 1;
        }
    }

    /// Returns true if at end of tokens.
    fn at_end(&self) -> bool {
        self.index >= self.tokens.len()
    }

    /// Returns the end position of the previous token (like Angular's currentEndIndex).
    /// This is used for calculating spans - it represents the end of what has been consumed.
    fn current_end_index(&self) -> u32 {
        if self.index > 0 {
            self.tokens[self.index - 1].end
        } else {
            // No tokens have been processed yet; return the next token's start or the length of the input
            self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32)
        }
    }

    /// Creates a span, swapping start and end if necessary (like Angular's span workaround).
    /// This handles cases where an empty expression is created after advancing past tokens.
    fn make_span(&self, mut start: u32, mut end: u32) -> ParseSpan {
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        ParseSpan::new(start, end)
    }

    /// Creates absolute span and source_span together.
    /// The spans use absolute positions (with absolute_offset added).
    fn make_absolute_spans(&self, start: u32, end: u32) -> (ParseSpan, AbsoluteSourceSpan) {
        let abs_start = self.absolute_offset + start;
        let abs_end = self.absolute_offset + end;
        let span = ParseSpan::new(abs_start, abs_end);
        let source_span = AbsoluteSourceSpan::new(abs_start, abs_end);
        (span, source_span)
    }

    /// Expects a character token.
    fn expect_character(&mut self, ch: char) -> bool {
        if let Some(token) = self.peek() {
            if token.is_character(ch) {
                self.advance();
                return true;
            }
        }
        self.error(&format!("expected {ch}"));
        // Skip to recover - this will stop at valid recovery points like `)`, `]`, `}`
        self.skip();
        // Try to consume the expected character after skipping
        if let Some(token) = self.peek() {
            if token.is_character(ch) {
                self.advance();
            }
        }
        false
    }

    /// Optionally consumes a character token.
    fn optional_character(&mut self, ch: char) -> bool {
        if let Some(token) = self.peek() {
            if token.is_character(ch) {
                self.advance();
                return true;
            }
        }
        false
    }

    /// Optionally consumes an operator token.
    fn optional_operator(&mut self, op: &str) -> bool {
        if let Some(token) = self.peek() {
            if token.is_operator(op) {
                self.advance();
                return true;
            }
        }
        false
    }

    /// Expects an operator token.
    fn expect_operator(&mut self, op: &str) {
        if let Some(token) = self.peek() {
            if token.is_operator(op) {
                self.advance();
                return;
            }
        }
        self.error(&format!("Missing expected {op}"));
    }

    /// Optionally consumes a keyword token.
    fn optional_keyword(&mut self, keyword: &str) -> bool {
        if let Some(token) = self.peek() {
            if token.is_keyword_value(keyword) {
                self.advance();
                return true;
            }
        }
        false
    }

    /// Records an error and skips tokens until reaching a recovery point.
    ///
    /// See `skip()` for details on recovery points.
    fn error(&mut self, message: &str) {
        self.error_at_index(message, None);
    }

    /// Records an error without skipping tokens.
    /// Use this when you want to report an error but continue parsing normally.
    fn record_error(&mut self, message: &str) {
        // Calculate column (1-indexed)
        let column = if self.index < self.tokens.len() {
            self.tokens[self.index].index + 1
        } else {
            0 // At end
        };

        let at_end = self.index >= self.tokens.len();
        let location =
            if at_end { "at end of expression".to_string() } else { format!("at column {column}") };
        let error_msg = format!("{message} {location}");
        self.errors.push(ParseError::new(Default::default(), error_msg));
    }

    /// Records an error at a specific byte position and skips to recovery point.
    fn error_at(&mut self, message: &str, byte_pos: u32) {
        self.error_at_index(message, Some(byte_pos));
    }

    /// Records an error at a specific position (byte position or current token).
    fn error_at_index(&mut self, message: &str, byte_pos: Option<u32>) {
        // Angular-compatible error format:
        // - For "expected X" messages at EOF: "Missing expected X at the end of the expression [source]"
        // - For "expected X" messages not at EOF: "Missing expected X at column N"
        // - For "Unexpected token" messages: "{message} at column N in [source]"
        // - For other messages at EOF: "{message} at end of expression"
        // - For other messages: "{message} at column N"

        // Calculate column (1-indexed)
        let column = if let Some(pos) = byte_pos {
            pos + 1
        } else if self.index < self.tokens.len() {
            self.tokens[self.index].index + 1
        } else {
            0 // At end
        };

        let at_end = byte_pos.is_none() && self.index >= self.tokens.len();

        let error_msg = if message.starts_with("expected ") {
            // Transform "expected X" to "Missing expected X"
            let expected_what = &message["expected ".len()..];
            if at_end {
                format!(
                    "Missing expected {} at the end of the expression [{}]",
                    expected_what, self.source
                )
            } else {
                format!("Missing expected {} at column {}", expected_what, column)
            }
        } else if message.starts_with("Unexpected token") {
            // "Unexpected token X" -> "Unexpected token X at column N in [source]"
            if at_end {
                format!("{} at end of expression in [{}]", message, self.source)
            } else {
                format!("{} at column {} in [{}]", message, column, self.source)
            }
        } else {
            let location = if at_end {
                "at the end of the expression".to_string()
            } else {
                format!("at column {column}")
            };
            format!("{message} {location}")
        };
        self.errors.push(ParseError::new(Default::default(), error_msg));

        // Skip to next recovery point
        self.skip();
    }

    /// Skips tokens until reaching a recovery point.
    ///
    /// Recovery points:
    /// - Unconditional: end of input, `;`, `|`
    /// - Conditional: `)`, `]`, `}` if the corresponding counter > 0
    /// - Assignment: `=` operator in Writable context
    ///
    /// This allows error recovery to preserve more of the AST.
    /// For example, `(a.) + 1` can recover at `)` to parse the rest.
    fn skip(&mut self) {
        while self.index < self.tokens.len() {
            if let Some(token) = self.peek() {
                // Unconditional recovery points
                if token.is_character(';') || token.is_operator("|") {
                    break;
                }

                // Conditional recovery points - only stop if we're expecting this bracket
                if self.rparens_expected > 0 && token.is_character(')') {
                    break;
                }
                if self.rbrackets_expected > 0 && token.is_character(']') {
                    break;
                }
                if self.rbraces_expected > 0 && token.is_character('}') {
                    break;
                }

                // Assignment in writable context
                if self.context.contains(ParseContextFlags::WRITABLE)
                    && self.is_assignment_operator(token)
                {
                    break;
                }
            } else {
                break;
            }

            self.advance();
        }
    }

    /// Checks if a token is an assignment operator.
    fn is_assignment_operator(&self, token: &Token<'_>) -> bool {
        // Single `=` and compound assignments are all operator tokens
        token.is_operator("=")
            || token.is_operator("+=")
            || token.is_operator("-=")
            || token.is_operator("*=")
            || token.is_operator("/=")
            || token.is_operator("%=")
            || token.is_operator("**=")
            || token.is_operator("&&=")
            || token.is_operator("||=")
            || token.is_operator("??=")
    }

    /// Converts an assignment operator string to a BinaryOperator.
    fn string_to_binary_operator(&self, op: &str) -> BinaryOperator {
        match op {
            "=" => BinaryOperator::Assign,
            "+=" => BinaryOperator::AddAssign,
            "-=" => BinaryOperator::SubtractAssign,
            "*=" => BinaryOperator::MultiplyAssign,
            "/=" => BinaryOperator::DivideAssign,
            "%=" => BinaryOperator::ModuloAssign,
            "**=" => BinaryOperator::PowerAssign,
            "&&=" => BinaryOperator::AndAssign,
            "||=" => BinaryOperator::OrAssign,
            "??=" => BinaryOperator::NullishCoalescingAssign,
            // Fallback (shouldn't happen if is_assignment_operator is correct)
            _ => BinaryOperator::Assign,
        }
    }

    // ========================================================================
    // Expression parsing
    // ========================================================================

    /// Parses a chain of expressions.
    fn parse_chain(&mut self) -> AngularExpression<'a> {
        let mut expressions = Vec::new_in(self.allocator);
        let start = self.peek().map(|t| t.index).unwrap_or(0);

        loop {
            // Use parse_pipe to allow pipes at top level
            let expr = self.parse_pipe();
            expressions.push(expr);

            // Check for semicolon (statement separator)
            if self.optional_character(';') {
                // Skip any extra semicolons
                while self.optional_character(';') {}
            } else if !self.at_end() {
                // There's an unconsumed token that's not a semicolon
                let error_index = self.index;
                if let Some(token) = self.peek() {
                    // Use quotes around identifiers/keywords and assignment operators
                    if token.is_identifier()
                        || token.is_keyword()
                        || self.is_assignment_operator(token)
                    {
                        self.error(&format!("Unexpected token '{}'", token.str_value));
                    } else {
                        self.error(&format!("Unexpected token {}", token.str_value));
                    }
                }
                // Skip to recover, but break if we didn't make progress
                if self.index == error_index {
                    break;
                }
            }

            if self.at_end() {
                break;
            }
        }

        if expressions.len() == 1 {
            // Safe: we just checked len() == 1, so pop() will return Some
            if let Some(expr) = expressions.pop() {
                return expr;
            }
            // Fallback: should never happen, but return an Empty expression
            let span = ParseSpan::new(start, self.peek().map(|t| t.end).unwrap_or(start));
            let source_span = span.to_absolute(self.absolute_offset);
            return AngularExpression::Empty(Box::new_in(
                EmptyExpr { span, source_span },
                self.allocator,
            ));
        }

        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let chain = Chain { span, source_span, expressions };
        AngularExpression::Chain(Box::new_in(chain, self.allocator))
    }

    /// Parses an expression.
    fn parse_expression(&mut self) -> AngularExpression<'a> {
        self.parse_conditional()
    }

    /// Parses a conditional expression.
    fn parse_conditional(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let condition = self.parse_logical_or();

        if self.optional_character('?') {
            // Check for incomplete ternary: true ?<EOF>
            if self.peek().is_none() {
                self.error("Unexpected end of expression");
                // Create a Conditional with empty expressions to preserve the '?'
                let end = self.source.len() as u32;
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let empty_span = ParseSpan::new(end, end);
                let empty_source_span = empty_span.to_absolute(self.absolute_offset);
                let true_exp = AngularExpression::Empty(Box::new_in(
                    EmptyExpr { span: empty_span, source_span: empty_source_span },
                    self.allocator,
                ));
                let false_exp = AngularExpression::Empty(Box::new_in(
                    EmptyExpr { span: empty_span, source_span: empty_source_span },
                    self.allocator,
                ));
                let cond = Conditional { span, source_span, condition, true_exp, false_exp };
                return AngularExpression::Conditional(Box::new_in(cond, self.allocator));
            }

            let true_exp = self.parse_pipe();
            let false_exp = if self.optional_character(':') {
                self.parse_pipe()
            } else {
                // Missing colon - emit error with the expression
                let end = self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32);
                let expression = &self.source[start as usize..end as usize];
                self.error(&format!(
                    "Conditional expression {expression} requires all 3 expressions"
                ));
                let empty_span = ParseSpan::new(end, end);
                let empty_source_span = empty_span.to_absolute(self.absolute_offset);
                AngularExpression::Empty(Box::new_in(
                    EmptyExpr { span: empty_span, source_span: empty_source_span },
                    self.allocator,
                ))
            };

            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let cond = Conditional { span, source_span, condition, true_exp, false_exp };
            return AngularExpression::Conditional(Box::new_in(cond, self.allocator));
        }

        condition
    }

    /// Parses nullish coalescing (??) expression.
    fn parse_nullish_coalescing(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_equality();

        while self.optional_operator("??") {
            let right = self.parse_equality();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let binary = Binary {
                span,
                source_span,
                operation: BinaryOperator::NullishCoalescing,
                left,
                right,
            };
            left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
        }

        left
    }

    /// Parses logical OR (||) expression.
    fn parse_logical_or(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_logical_and();

        while self.optional_operator("||") {
            let right = self.parse_logical_and();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let binary = Binary { span, source_span, operation: BinaryOperator::Or, left, right };
            left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
        }

        left
    }

    /// Parses logical AND (&&) expression.
    fn parse_logical_and(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_nullish_coalescing();

        while self.optional_operator("&&") {
            let right = self.parse_nullish_coalescing();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let binary = Binary { span, source_span, operation: BinaryOperator::And, left, right };
            left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
        }

        left
    }

    /// Parses equality expressions.
    fn parse_equality(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_relational();

        loop {
            let op = if self.optional_operator("===") {
                Some(BinaryOperator::StrictEqual)
            } else if self.optional_operator("!==") {
                Some(BinaryOperator::StrictNotEqual)
            } else if self.optional_operator("==") {
                Some(BinaryOperator::Equal)
            } else if self.optional_operator("!=") {
                Some(BinaryOperator::NotEqual)
            } else {
                None
            };

            if let Some(operation) = op {
                let right = self.parse_relational();
                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let binary = Binary { span, source_span, operation, left, right };
                left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
            } else {
                break;
            }
        }

        left
    }

    /// Parses relational expressions.
    fn parse_relational(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_additive();

        loop {
            let op = if self.optional_operator("<=") {
                Some(BinaryOperator::LessThanOrEqual)
            } else if self.optional_operator(">=") {
                Some(BinaryOperator::GreaterThanOrEqual)
            } else if self.optional_operator("<") {
                Some(BinaryOperator::LessThan)
            } else if self.optional_operator(">") {
                Some(BinaryOperator::GreaterThan)
            } else if self.optional_keyword("in") {
                Some(BinaryOperator::In)
            } else if self.optional_keyword("instanceof") {
                Some(BinaryOperator::Instanceof)
            } else {
                None
            };

            if let Some(operation) = op {
                let right = self.parse_additive();
                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let binary = Binary { span, source_span, operation, left, right };
                left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
            } else {
                break;
            }
        }

        left
    }

    /// Parses additive expressions.
    fn parse_additive(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_multiplicative();

        loop {
            let op = if self.optional_operator("+") {
                Some(BinaryOperator::Add)
            } else if self.optional_operator("-") {
                Some(BinaryOperator::Subtract)
            } else {
                None
            };

            if let Some(operation) = op {
                // Check for incomplete expression: 1 +<EOF> or 1 +<unexpected token>
                if self.peek().is_none() {
                    self.error("Unexpected end of expression");
                } else if let Some(token) = self.peek() {
                    // Check if next token can start an expression
                    // If it's a closing bracket/paren, assignment, separator, or pipe, it's an incomplete expression
                    if token.is_character(']')
                        || token.is_character(')')
                        || token.is_character('}')
                        || token.is_character(',')
                        || token.is_character(':')
                        || token.is_character(';')
                        || token.is_operator("|")
                        || self.is_assignment_operator(token)
                    {
                        let token_str = &token.str_value;
                        self.error(&format!("Unexpected token {token_str}"));
                    }
                }
                let right = self.parse_multiplicative();
                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let binary = Binary { span, source_span, operation, left, right };
                left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
            } else {
                break;
            }
        }

        left
    }

    /// Parses multiplicative expressions.
    fn parse_multiplicative(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut left = self.parse_exponentiation();

        loop {
            let op = if self.optional_operator("*") {
                Some(BinaryOperator::Multiply)
            } else if self.optional_operator("/") {
                Some(BinaryOperator::Divide)
            } else if self.optional_operator("%") {
                Some(BinaryOperator::Modulo)
            } else {
                None
            };

            if let Some(operation) = op {
                let right = self.parse_exponentiation();
                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let binary = Binary { span, source_span, operation, left, right };
                left = AngularExpression::Binary(Box::new_in(binary, self.allocator));
            } else {
                break;
            }
        }

        left
    }

    /// Parses exponentiation expressions (`**`).
    ///
    /// Exponentiation is right-associative: `2 ** 3 ** 2` = `2 ** (3 ** 2)` = 512.
    /// JavaScript requires unary operators before exponentiation to be parenthesized.
    fn parse_exponentiation(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut result = self.parse_prefix();

        while self.optional_operator("**") {
            // Check if the base is a unary expression that needs parentheses
            // This aligns with JavaScript semantics which require any unary operator
            // preceding the exponentiation operation to be explicitly grouped
            if matches!(
                result,
                AngularExpression::Unary(_)
                    | AngularExpression::PrefixNot(_)
                    | AngularExpression::TypeofExpression(_)
                    | AngularExpression::VoidExpression(_)
            ) {
                self.error(
                    "Unary operator used immediately before exponentiation expression. \
                     Parenthesis must be used to disambiguate operator precedence",
                );
            }

            // Right-associative: recursively parse the right side
            let right = self.parse_exponentiation();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let binary =
                Binary { span, source_span, operation: BinaryOperator::Power, left: result, right };
            result = AngularExpression::Binary(Box::new_in(binary, self.allocator));
        }

        result
    }

    /// Parses prefix expressions.
    fn parse_prefix(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);

        if self.optional_operator("+") {
            let expr = self.parse_prefix();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let unary = Unary { span, source_span, operator: UnaryOperator::Plus, expr };
            return AngularExpression::Unary(Box::new_in(unary, self.allocator));
        }

        if self.optional_operator("-") {
            let expr = self.parse_prefix();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let unary = Unary { span, source_span, operator: UnaryOperator::Minus, expr };
            return AngularExpression::Unary(Box::new_in(unary, self.allocator));
        }

        if self.optional_operator("!") {
            let expression = self.parse_prefix();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let prefix_not = PrefixNot { span, source_span, expression };
            return AngularExpression::PrefixNot(Box::new_in(prefix_not, self.allocator));
        }

        // Handle typeof keyword
        if self.optional_keyword("typeof") {
            let expression = self.parse_prefix();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let typeof_expr = TypeofExpression { span, source_span, expression };
            return AngularExpression::TypeofExpression(Box::new_in(typeof_expr, self.allocator));
        }

        // Handle void keyword
        if self.optional_keyword("void") {
            let expression = self.parse_prefix();
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let void_expr = VoidExpression { span, source_span, expression };
            return AngularExpression::VoidExpression(Box::new_in(void_expr, self.allocator));
        }

        self.parse_call_chain()
    }

    /// Parses call chains (property access, method calls, etc.).
    fn parse_call_chain(&mut self) -> AngularExpression<'a> {
        // Capture start BEFORE parse_primary() to match Angular's behavior
        // This ensures the span covers the entire chain from the first token
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut result = self.parse_primary();

        loop {
            if self.optional_character('.') {
                result = self.parse_access_member(result, start, false);
            } else if self.optional_operator("?.") {
                // After `?.`, check for `(` (safe call), `[` (safe keyed read), or property
                if self.optional_character('(') {
                    result = self.parse_call(result, start, true);
                } else if self.optional_character('[') {
                    result = self.parse_keyed_read(result, start, true);
                } else {
                    result = self.parse_access_member(result, start, true);
                }
            } else if self.optional_character('[') {
                result = self.parse_keyed_read(result, start, false);
            } else if self.optional_character('(') {
                result = self.parse_call(result, start, false);
            } else if self.optional_operator("!") {
                // Postfix non-null assertion: expr!
                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let assert = NonNullAssert { span, source_span, expression: result };
                result = AngularExpression::NonNullAssert(Box::new_in(assert, self.allocator));
            } else if let Some(token) = self.peek() {
                // Handle tagged template literals: tag`template`
                // Only NoSubstitutionTemplate and TemplateHead can start a tagged template
                // TemplateMiddle and TemplateTail are continuation tokens within template literals
                if token.is_no_substitution_template() || token.is_template_head() {
                    result = self.parse_tagged_template_literal(result, start);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Check for assignment operator on PropertyRead (simple variable assignment like `a = b`)
        // Note: PropertyRead with property access (a.b = c) is handled in parse_access_member
        // and KeyedRead (a[b] = c) is handled in parse_keyed_read
        // Only check if result is a valid lvalue (PropertyRead or KeyedRead) - not literals, binaries, etc.
        let is_valid_lvalue =
            matches!(&result, AngularExpression::PropertyRead(_) | AngularExpression::KeyedRead(_));

        if is_valid_lvalue {
            if let Some(token) = self.peek() {
                if self.is_assignment_operator(token) {
                    // Clone the operation first to avoid borrow issues
                    let operation = token.str_value.clone();
                    let result_span = result.span();

                    // In binding mode, still parse the full expression but emit an error
                    // This ensures spans cover the entire expression
                    // Use record_error to avoid skipping tokens since we still want to parse RHS
                    if !self.action_mode {
                        self.record_error("Bindings cannot contain assignments");
                    }

                    self.advance();

                    // Check for empty rvalue
                    if self.peek().is_none() {
                        self.error("Unexpected end of expression");
                    }

                    // Parse the value expression
                    let right = self.parse_conditional();

                    // Create Binary expression for assignment
                    let end = self.current_end_index();
                    let assign_span = ParseSpan::new(result_span.start, end);
                    let assign_source_span = assign_span.to_absolute(self.absolute_offset);
                    let binary_op = self.string_to_binary_operator(&operation);
                    let binary = Binary {
                        span: assign_span,
                        source_span: assign_source_span,
                        operation: binary_op,
                        left: result,
                        right,
                    };
                    return AngularExpression::Binary(Box::new_in(binary, self.allocator));
                }
            }
        }

        result
    }

    /// Parses member access.
    fn parse_access_member(
        &mut self,
        receiver: AngularExpression<'a>,
        start: u32,
        safe: bool,
    ) -> AngularExpression<'a> {
        if let Some(token) = self.peek().cloned() {
            // Check for private identifier - not supported in Angular expressions
            if token.is_private_identifier() {
                let name = token.str_value.clone();
                self.error(&format!(
                    "Private identifiers are not supported. Unexpected private identifier: {name}, expected identifier or keyword"
                ));
                self.advance();
                // Create a PropertyRead/SafePropertyRead with empty name to preserve the access
                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let name_span = source_span;
                let empty_name = Ident::from_in("", self.allocator);
                if safe {
                    let read = SafePropertyRead {
                        span,
                        source_span,
                        name_span,
                        receiver,
                        name: empty_name,
                    };
                    return AngularExpression::SafePropertyRead(Box::new_in(read, self.allocator));
                }
                let read =
                    PropertyRead { span, source_span, name_span, receiver, name: empty_name };
                return AngularExpression::PropertyRead(Box::new_in(read, self.allocator));
            }

            if token.is_identifier() || token.is_keyword() {
                let name = token.str_value.clone();
                let name_start = token.index;
                let name_end = token.end;
                self.advance();

                // Check for method call
                if self.optional_character('(') {
                    return self.parse_call_after_access(
                        receiver, start, name, name_start, name_end, safe,
                    );
                }

                let end = self.current_end_index();
                let span = ParseSpan::new(start, end);
                let source_span = span.to_absolute(self.absolute_offset);
                let name_span = AbsoluteSourceSpan::new(
                    self.absolute_offset + name_start,
                    self.absolute_offset + name_end,
                );

                if safe {
                    // Check for assignment on safe property access (not allowed)
                    if let Some(next_token) = self.peek() {
                        if self.is_assignment_operator(next_token) {
                            self.advance();
                            self.error("The '?.' operator cannot be used in the assignment");
                            return AngularExpression::Empty(Box::new_in(
                                EmptyExpr { span, source_span },
                                self.allocator,
                            ));
                        }
                    }
                    let read = SafePropertyRead { span, source_span, name_span, receiver, name };
                    return AngularExpression::SafePropertyRead(Box::new_in(read, self.allocator));
                }

                // Check for assignment operator (in action mode only)
                if let Some(next_token) = self.peek() {
                    if self.is_assignment_operator(next_token) {
                        let operation = next_token.str_value.clone();

                        // In binding mode, still parse the full expression but emit an error
                        // This ensures spans cover the entire expression
                        // Use record_error to avoid skipping tokens since we still want to parse RHS
                        if !self.action_mode {
                            self.record_error("Bindings cannot contain assignments");
                        }

                        // Create PropertyRead as the assignment target
                        let property_read =
                            PropertyRead { span, source_span, name_span, receiver, name };
                        let left = AngularExpression::PropertyRead(Box::new_in(
                            property_read,
                            self.allocator,
                        ));

                        // Consume the assignment operator
                        self.advance();

                        // Check for empty rvalue
                        if self.peek().is_none() {
                            self.error("Unexpected end of expression");
                        }

                        // Parse the value expression
                        let right = self.parse_conditional();

                        // Create Binary expression for assignment
                        let end = self.current_end_index();
                        let assign_span = ParseSpan::new(start, end);
                        let assign_source_span = assign_span.to_absolute(self.absolute_offset);
                        let binary_op = self.string_to_binary_operator(&operation);
                        let binary = Binary {
                            span: assign_span,
                            source_span: assign_source_span,
                            operation: binary_op,
                            left,
                            right,
                        };
                        return AngularExpression::Binary(Box::new_in(binary, self.allocator));
                    }
                }
                let read = PropertyRead { span, source_span, name_span, receiver, name };
                return AngularExpression::PropertyRead(Box::new_in(read, self.allocator));
            }
        }

        // No identifier found after dot - emit errors but preserve the incomplete access
        // Set WRITABLE context so skip() will stop at assignment operators
        // Report error at the receiver's span end (right after the dot)
        let receiver_end = receiver.span().end;
        self.context |= ParseContextFlags::WRITABLE;

        // First, emit error about what unexpected token was found (matches Angular's expectIdentifierOrKeyword)
        if let Some(token) = self.peek() {
            self.error(&format!(
                "Unexpected token {}, expected identifier or keyword",
                token.str_value
            ));
        } else {
            self.error("Unexpected end of input, expected identifier or keyword");
        }

        // Then emit the "Expected identifier for property access" error
        self.error_at("Expected identifier for property access", receiver_end);
        self.context ^= ParseContextFlags::WRITABLE;

        // Create a PropertyRead/SafePropertyRead with empty name to preserve the trailing dot
        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let name_span = source_span; // Empty name span at end
        let empty_name = Ident::from_in("", self.allocator);

        if safe {
            let read =
                SafePropertyRead { span, source_span, name_span, receiver, name: empty_name };
            return AngularExpression::SafePropertyRead(Box::new_in(read, self.allocator));
        }

        // Check for assignment operator (in action mode only) - error recovery
        if let Some(next_token) = self.peek() {
            if self.is_assignment_operator(next_token) && self.action_mode {
                let operation = next_token.str_value.clone();

                // Create PropertyRead as the assignment target
                let property_read =
                    PropertyRead { span, source_span, name_span, receiver, name: empty_name };
                let left =
                    AngularExpression::PropertyRead(Box::new_in(property_read, self.allocator));

                // Consume the assignment operator
                self.advance();

                // Parse the value expression
                let right = self.parse_conditional();

                // Create Binary expression for assignment
                let end = self.current_end_index();
                let assign_span = ParseSpan::new(start, end);
                let assign_source_span = assign_span.to_absolute(self.absolute_offset);
                let binary_op = self.string_to_binary_operator(&operation);
                let binary = Binary {
                    span: assign_span,
                    source_span: assign_source_span,
                    operation: binary_op,
                    left,
                    right,
                };
                return AngularExpression::Binary(Box::new_in(binary, self.allocator));
            }
        }

        let read = PropertyRead { span, source_span, name_span, receiver, name: empty_name };
        AngularExpression::PropertyRead(Box::new_in(read, self.allocator))
    }

    /// Parses keyed read with an explicit start position for span calculation.
    /// Note: The opening `[` was already consumed before calling this.
    fn parse_keyed_read(
        &mut self,
        receiver: AngularExpression<'a>,
        start: u32,
        safe: bool,
    ) -> AngularExpression<'a> {
        // Set WRITABLE context so = is a recovery point for unterminated keys
        self.context |= ParseContextFlags::WRITABLE;
        self.rbrackets_expected += 1;

        // Check for empty key access: a[]
        if self.peek().map(|t| t.is_character(']')).unwrap_or(false) {
            self.error("Key access cannot be empty");
        }

        // Parse the key expression - pipes are allowed inside keyed access
        let key = self.parse_pipe();

        // Decrement rbrackets_expected BEFORE expect_character so ] isn't a recovery point
        // during skip() - this allows = to be the recovery point for unterminated keys
        self.rbrackets_expected -= 1;
        self.expect_character(']');

        // Restore context
        self.context ^= ParseContextFlags::WRITABLE;

        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);

        // Check for assignment operator after keyed access
        if let Some(next_token) = self.peek() {
            if self.is_assignment_operator(next_token) {
                let operation = next_token.str_value.clone();

                if safe {
                    self.advance();
                    self.error("The '?.' operator cannot be used in the assignment");
                    return AngularExpression::Empty(Box::new_in(
                        EmptyExpr { span, source_span },
                        self.allocator,
                    ));
                }

                // In binding mode, still parse the full expression but emit an error
                // This ensures spans cover the entire expression
                // Use record_error to avoid skipping tokens since we still want to parse RHS
                if !self.action_mode {
                    self.record_error("Bindings cannot contain assignments");
                }

                // Create KeyedRead as the assignment target
                let keyed_read = KeyedRead { span, source_span, receiver, key };
                let left = AngularExpression::KeyedRead(Box::new_in(keyed_read, self.allocator));

                // Consume the assignment operator
                self.advance();

                // Check for empty rvalue
                if self.peek().is_none() {
                    self.error("Unexpected end of expression");
                }

                // Parse the value expression
                let right = self.parse_conditional();

                // Create Binary expression for assignment
                let end = self.current_end_index();
                let assign_span = ParseSpan::new(start, end);
                let assign_source_span = assign_span.to_absolute(self.absolute_offset);
                let binary_op = self.string_to_binary_operator(&operation);
                let binary = Binary {
                    span: assign_span,
                    source_span: assign_source_span,
                    operation: binary_op,
                    left,
                    right,
                };
                return AngularExpression::Binary(Box::new_in(binary, self.allocator));
            }
        }

        if safe {
            let read = SafeKeyedRead { span, source_span, receiver, key };
            AngularExpression::SafeKeyedRead(Box::new_in(read, self.allocator))
        } else {
            let read = KeyedRead { span, source_span, receiver, key };
            AngularExpression::KeyedRead(Box::new_in(read, self.allocator))
        }
    }

    /// Parses a function call with an explicit start position for span calculation.
    fn parse_call(
        &mut self,
        receiver: AngularExpression<'a>,
        start: u32,
        safe: bool,
    ) -> AngularExpression<'a> {
        let arg_start = self.peek().map(|t| t.index).unwrap_or(0);
        let (args, arg_end) = self.parse_call_arguments();
        let end = self.current_end_index();

        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let argument_span = AbsoluteSourceSpan::new(
            self.absolute_offset + arg_start,
            self.absolute_offset + arg_end,
        );

        if safe {
            let call = SafeCall { span, source_span, receiver, args, argument_span };
            AngularExpression::SafeCall(Box::new_in(call, self.allocator))
        } else {
            let call = Call { span, source_span, receiver, args, argument_span };
            AngularExpression::Call(Box::new_in(call, self.allocator))
        }
    }

    /// Parses call after member access.
    fn parse_call_after_access(
        &mut self,
        receiver: AngularExpression<'a>,
        start: u32,
        name: Ident<'a>,
        name_start: u32,
        name_end: u32,
        safe: bool,
    ) -> AngularExpression<'a> {
        // Create the property read for the method receiver (e.g., "foo.bar" in "foo.bar()")
        // The span covers from start to name_end (before the "(")
        let span = ParseSpan::new(start, name_end);
        let source_span = span.to_absolute(self.absolute_offset);
        let name_span = AbsoluteSourceSpan::new(
            self.absolute_offset + name_start,
            self.absolute_offset + name_end,
        );

        let method = if safe {
            let read = SafePropertyRead { span, source_span, name_span, receiver, name };
            AngularExpression::SafePropertyRead(Box::new_in(read, self.allocator))
        } else {
            let read = PropertyRead { span, source_span, name_span, receiver, name };
            AngularExpression::PropertyRead(Box::new_in(read, self.allocator))
        };

        // Then parse the call, using the receiver's start for the call's span
        self.parse_call(method, start, false)
    }

    /// Parses call arguments and returns the end position (before the closing paren).
    /// Note: The opening `(` was already consumed before calling this.
    fn parse_call_arguments(&mut self) -> (Vec<'a, AngularExpression<'a>>, u32) {
        self.rparens_expected += 1;
        let mut args = Vec::new_in(self.allocator);

        if !self.peek().map(|t| t.is_character(')')).unwrap_or(true) {
            loop {
                // Parse spread elements or regular arguments
                if self.peek().map(|t| t.is_operator("...")).unwrap_or(false) {
                    args.push(self.parse_spread_element());
                } else {
                    // Parse pipes inside function call arguments (e.g., `a(b | c)`)
                    args.push(self.parse_pipe());
                }
                if !self.optional_character(',') {
                    break;
                }
            }
        }

        // Capture the position before consuming ")"
        let arg_end = self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32);

        self.expect_character(')');
        self.rparens_expected -= 1;
        (args, arg_end)
    }

    /// Checks if the current position is the start of an arrow function.
    ///
    /// This performs lookahead without advancing the parser position.
    /// Arrow functions can be:
    /// - `identifier => expr` (single parameter without parens)
    /// - `() => expr` (no parameters)
    /// - `(a, b, c) => expr` (parenthesized parameters)
    fn is_arrow_function(&self) -> bool {
        let start = self.index;
        let tokens = &self.tokens;

        // Need at least 2 tokens from start position for any arrow function
        if start + 2 > tokens.len() {
            return false;
        }

        // One parameter and no parens: `a => ...`
        if tokens[start].is_identifier() && tokens[start + 1].is_operator("=>") {
            return true;
        }

        // Multiple parenthesized params: `(a, b, ...) => ...`
        if tokens[start].is_character('(') {
            let mut i = start + 1;

            // Scan through identifiers and commas
            while i < tokens.len() {
                if !tokens[i].is_identifier() && !tokens[i].is_character(',') {
                    break;
                }
                i += 1;
            }

            // Check if we have `) => ...` - need at least 2 more tokens
            return i + 1 < tokens.len()
                && tokens[i].is_character(')')
                && tokens[i + 1].is_operator("=>");
        }

        false
    }

    /// Parses an arrow function expression.
    fn parse_arrow_function(&mut self, start: u32) -> AngularExpression<'a> {
        let params: Vec<'a, ArrowFunctionParameter<'a>>;

        if self.peek().is_some_and(|t| t.is_identifier()) {
            // Single parameter without parens: `a => ...`
            let token = self.peek().cloned();
            self.advance();
            if let Some(token) = token {
                let mut vec = Vec::new_in(self.allocator);
                vec.push(self.get_arrow_function_identifier_arg(&token));
                params = vec;
            } else {
                params = Vec::new_in(self.allocator);
            }
        } else if self.peek().is_some_and(|t| t.is_character('(')) {
            // Parenthesized parameters: `() => ...` or `(a, b) => ...`
            self.rparens_expected += 1;
            self.advance(); // consume '('
            params = self.parse_arrow_function_parameters();
            self.rparens_expected -= 1;
        } else {
            // Error case
            params = Vec::new_in(self.allocator);
            let token_str =
                self.peek().map(|t| t.str_value.to_string()).unwrap_or_else(|| "EOF".to_string());
            self.error(&format!("Unexpected token {token_str}"));
        }

        self.expect_operator("=>");
        let body: AngularExpression<'a>;

        if self.peek().is_some_and(|t| t.is_character('{')) {
            // Multi-line arrow function body with braces
            self.error("Multi-line arrow functions are not supported. If you meant to return an object literal, wrap it with parentheses.");
            let span = ParseSpan::new(start, self.current_end_index());
            let source_span = span.to_absolute(self.absolute_offset);
            body = AngularExpression::Empty(Box::new_in(
                EmptyExpr { span, source_span },
                self.allocator,
            ));
        } else {
            // Arrow function can contain assignments even in a binding context
            let prev_action_mode = self.action_mode;
            self.action_mode = true;
            body = self.parse_expression();
            self.action_mode = prev_action_mode;
        }

        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let arrow_fn = ArrowFunction { span, source_span, parameters: params, body };
        AngularExpression::ArrowFunction(Box::new_in(arrow_fn, self.allocator))
    }

    /// Parses arrow function parameters inside parentheses.
    /// Note: The opening `(` was already consumed.
    fn parse_arrow_function_parameters(&mut self) -> Vec<'a, ArrowFunctionParameter<'a>> {
        let mut params = Vec::new_in(self.allocator);

        if !self.optional_character(')') {
            loop {
                if self.peek().is_some_and(|t| t.is_identifier()) {
                    let token = self.peek().cloned();
                    self.advance();
                    if let Some(token) = token {
                        params.push(self.get_arrow_function_identifier_arg(&token));
                    }

                    if self.optional_character(')') {
                        break;
                    }
                    self.expect_character(',');
                } else {
                    let token_str = self
                        .peek()
                        .map(|t| t.str_value.to_string())
                        .unwrap_or_else(|| "EOF".to_string());
                    self.error(&format!("Unexpected token {token_str}"));
                    break;
                }
            }
        }

        params
    }

    /// Creates an arrow function parameter from an identifier token.
    fn get_arrow_function_identifier_arg(&self, token: &Token<'a>) -> ArrowFunctionParameter<'a> {
        let span = ParseSpan::new(token.index, token.end);
        let source_span = span.to_absolute(self.absolute_offset);
        ArrowFunctionParameter { name: token.str_value.clone(), span, source_span }
    }

    /// Parses a primary expression.
    fn parse_primary(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);

        // Check for arrow function first (before parenthesized expression)
        if self.is_arrow_function() {
            return self.parse_arrow_function(start);
        }

        // Parenthesized expression
        if self.optional_character('(') {
            self.rparens_expected += 1;
            // Parse pipes inside parenthesized expressions (e.g., `(a | b)`)
            let expression = self.parse_pipe();
            if !self.optional_character(')') {
                // Use "Missing closing parentheses" (not "expected )")
                // to match Angular's error message format
                self.error("Missing closing parentheses");
                // Try to consume the closing paren if error recovery found one
                self.optional_character(')');
            }
            self.rparens_expected -= 1;
            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let paren = ParenthesizedExpression { span, source_span, expression };
            return AngularExpression::ParenthesizedExpression(Box::new_in(paren, self.allocator));
        }

        // Array literal
        if self.optional_character('[') {
            return self.parse_array_literal(start);
        }

        // Object literal
        if self.optional_character('{') {
            return self.parse_object_literal(start);
        }

        // Check for token
        if let Some(token) = self.peek().cloned() {
            // Number literal
            if token.is_number() {
                self.advance();
                let span = ParseSpan::new(token.index, token.end);
                let source_span = span.to_absolute(self.absolute_offset);
                let lit = LiteralPrimitive {
                    span,
                    source_span,
                    value: LiteralValue::Number(token.num_value),
                };
                return AngularExpression::LiteralPrimitive(Box::new_in(lit, self.allocator));
            }

            // String literal
            if token.is_string() {
                self.advance();
                let span = ParseSpan::new(token.index, token.end);
                let source_span = span.to_absolute(self.absolute_offset);
                let lit = LiteralPrimitive {
                    span,
                    source_span,
                    value: LiteralValue::String(token.str_value.clone()),
                };
                return AngularExpression::LiteralPrimitive(Box::new_in(lit, self.allocator));
            }

            // Template literal (no substitutions)
            if token.is_no_substitution_template() {
                self.advance();
                let span = ParseSpan::new(token.index, token.end);
                let source_span = span.to_absolute(self.absolute_offset);
                let mut elements = Vec::new_in(self.allocator);
                elements.push(TemplateLiteralElement {
                    span,
                    source_span,
                    text: token.str_value.clone(),
                });
                let tpl = TemplateLiteral {
                    span,
                    source_span,
                    elements,
                    expressions: Vec::new_in(self.allocator),
                };
                return AngularExpression::TemplateLiteral(Box::new_in(tpl, self.allocator));
            }

            // Template literal (with substitutions)
            if token.is_template_head() {
                return self.parse_template_literal(start, token.clone());
            }

            // Regex literal (starts with RegExpBody token)
            if token.is_regexp_body() {
                return self.parse_regex_literal(token.clone());
            }

            // Keywords
            if token.is_keyword() {
                self.advance();
                let span = ParseSpan::new(token.index, token.end);
                let source_span = span.to_absolute(self.absolute_offset);

                match token.str_value.as_str() {
                    "true" => {
                        let lit = LiteralPrimitive {
                            span,
                            source_span,
                            value: LiteralValue::Boolean(true),
                        };
                        return AngularExpression::LiteralPrimitive(Box::new_in(
                            lit,
                            self.allocator,
                        ));
                    }
                    "false" => {
                        let lit = LiteralPrimitive {
                            span,
                            source_span,
                            value: LiteralValue::Boolean(false),
                        };
                        return AngularExpression::LiteralPrimitive(Box::new_in(
                            lit,
                            self.allocator,
                        ));
                    }
                    "null" => {
                        let lit = LiteralPrimitive { span, source_span, value: LiteralValue::Null };
                        return AngularExpression::LiteralPrimitive(Box::new_in(
                            lit,
                            self.allocator,
                        ));
                    }
                    "undefined" => {
                        let lit =
                            LiteralPrimitive { span, source_span, value: LiteralValue::Undefined };
                        return AngularExpression::LiteralPrimitive(Box::new_in(
                            lit,
                            self.allocator,
                        ));
                    }
                    "this" => {
                        let receiver = ThisReceiver { span, source_span };
                        return AngularExpression::ThisReceiver(Box::new_in(
                            receiver,
                            self.allocator,
                        ));
                    }
                    _ => {
                        // Treat as identifier
                        // ImplicitReceiver spans from previous token end to current token start
                        // This covers any whitespace between tokens (Angular behavior)
                        let prev_end = self.previous_end();
                        let implicit_span = ParseSpan::new(prev_end, token.index);
                        let implicit_source_span = implicit_span.to_absolute(self.absolute_offset);
                        let implicit = ImplicitReceiver {
                            span: implicit_span,
                            source_span: implicit_source_span,
                        };
                        let receiver = AngularExpression::ImplicitReceiver(Box::new_in(
                            implicit,
                            self.allocator,
                        ));
                        let name_span = source_span;
                        let read = PropertyRead {
                            span,
                            source_span,
                            name_span,
                            receiver,
                            name: token.str_value.clone(),
                        };
                        return AngularExpression::PropertyRead(Box::new_in(read, self.allocator));
                    }
                }
            }

            // Private identifier - not supported in Angular expressions
            if token.is_private_identifier() {
                let name = token.str_value.to_string();
                self.error(&format!(
                    "Private identifiers are not supported. Unexpected private identifier: {name}"
                ));
                self.advance();
                let span = ParseSpan::new(token.index, token.end);
                let source_span = span.to_absolute(self.absolute_offset);
                let empty = EmptyExpr { span, source_span };
                return AngularExpression::Empty(Box::new_in(empty, self.allocator));
            }

            // Identifier
            if token.is_identifier() {
                // Get prev_end before consuming the token
                let prev_end = self.previous_end();
                self.advance();
                let (span, source_span) = self.make_absolute_spans(token.index, token.end);
                // ImplicitReceiver spans from previous token end to current token start
                // This covers any whitespace between tokens (Angular behavior)
                let (implicit_span, implicit_source_span) =
                    self.make_absolute_spans(prev_end, token.index);
                let implicit =
                    ImplicitReceiver { span: implicit_span, source_span: implicit_source_span };
                let receiver =
                    AngularExpression::ImplicitReceiver(Box::new_in(implicit, self.allocator));
                let name_span = source_span;
                let read = PropertyRead {
                    span,
                    source_span,
                    name_span,
                    receiver,
                    name: token.str_value.clone(),
                };
                return AngularExpression::PropertyRead(Box::new_in(read, self.allocator));
            }
        }

        // Empty expression - use make_span to handle the case where start > current_end_index
        // This matches Angular's workaround for empty expressions after commas, etc.
        let span = self.make_span(start, self.current_end_index());
        let source_span = span.to_absolute(self.absolute_offset);
        let empty = EmptyExpr { span, source_span };
        AngularExpression::Empty(Box::new_in(empty, self.allocator))
    }

    /// Parses an array literal.
    /// Note: The opening `[` was already consumed before calling this.
    fn parse_array_literal(&mut self, start: u32) -> AngularExpression<'a> {
        self.rbrackets_expected += 1;
        let mut expressions = Vec::new_in(self.allocator);

        loop {
            if self.peek().map(|t| t.is_operator("...")).unwrap_or(false) {
                expressions.push(self.parse_spread_element());
            } else if !self.peek().map(|t| t.is_character(']')).unwrap_or(true) {
                // Pipes are allowed inside array literal elements
                expressions.push(self.parse_pipe());
            } else {
                break;
            }
            if !self.optional_character(',') {
                break;
            }
        }

        self.rbrackets_expected -= 1;
        self.expect_character(']');
        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let arr = LiteralArray { span, source_span, expressions };
        AngularExpression::LiteralArray(Box::new_in(arr, self.allocator))
    }

    /// Parses a template literal with substitutions.
    fn parse_template_literal(&mut self, start: u32, head: Token<'a>) -> AngularExpression<'a> {
        self.advance(); // Consume the TemplateHead token

        let mut elements = Vec::new_in(self.allocator);
        let mut expressions = Vec::new_in(self.allocator);

        // Add the first element from TemplateHead
        let head_span = ParseSpan::new(head.index, head.end);
        let head_source_span = head_span.to_absolute(self.absolute_offset);
        elements.push(TemplateLiteralElement {
            span: head_span,
            source_span: head_source_span,
            text: head.str_value.clone(),
        });

        // Parse expressions and template parts
        loop {
            // The lexer emits ${ as an operator token - skip it
            self.optional_operator("${");

            // Increment rbraces_expected so that error recovery stops at }
            self.rbraces_expected += 1;

            // Parse the expression inside ${...}
            // Use parse_pipe to allow pipes inside template interpolations
            let expr = self.parse_pipe();

            self.rbraces_expected -= 1;

            // Check for empty interpolation
            if matches!(expr, AngularExpression::Empty(_)) {
                self.errors.push(ParseError::new(
                    Default::default(),
                    "Template literal interpolation cannot be empty".to_string(),
                ));
            }

            expressions.push(expr);

            // The lexer emits } as a character token - skip it
            self.optional_character('}');

            // Check what comes next - look for TemplateMiddle/TemplateTail
            if let Some(token) = self.peek().cloned() {
                if token.is_template_middle() {
                    // More parts to come
                    self.advance();
                    let middle_span = ParseSpan::new(token.index, token.end);
                    let middle_source_span = middle_span.to_absolute(self.absolute_offset);
                    elements.push(TemplateLiteralElement {
                        span: middle_span,
                        source_span: middle_source_span,
                        text: token.str_value.clone(),
                    });
                    // Continue to next expression
                    continue;
                } else if token.is_template_tail() {
                    // Last part
                    self.advance();
                    let tail_span = ParseSpan::new(token.index, token.end);
                    let tail_source_span = tail_span.to_absolute(self.absolute_offset);
                    elements.push(TemplateLiteralElement {
                        span: tail_span,
                        source_span: tail_source_span,
                        text: token.str_value.clone(),
                    });
                    // Done
                    break;
                }
            }

            // If we didn't find a template part, exit
            break;
        }

        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let tpl = TemplateLiteral { span, source_span, elements, expressions };
        AngularExpression::TemplateLiteral(Box::new_in(tpl, self.allocator))
    }

    /// Parses a tagged template literal with an explicit start position for span calculation.
    fn parse_tagged_template_literal(
        &mut self,
        tag: AngularExpression<'a>,
        start: u32,
    ) -> AngularExpression<'a> {
        // Get the template token
        if let Some(token) = self.peek().cloned() {
            // Parse the template part
            let template_expr = if token.is_no_substitution_template() {
                // Simple template without interpolations
                self.advance();
                let span = ParseSpan::new(token.index, token.end);
                let source_span = span.to_absolute(self.absolute_offset);

                let mut elements = Vec::new_in(self.allocator);
                elements.push(TemplateLiteralElement {
                    span,
                    source_span,
                    text: token.str_value.clone(),
                });

                let expressions = Vec::new_in(self.allocator);
                TemplateLiteral { span, source_span, elements, expressions }
            } else if token.is_template_head() {
                // Template with interpolations - reuse the same logic as parse_template_literal
                // but extract the TemplateLiteral directly
                self.advance();

                let mut elements = Vec::new_in(self.allocator);
                let mut expressions = Vec::new_in(self.allocator);

                // Add the first element from TemplateHead
                let head_span = ParseSpan::new(token.index, token.end);
                let head_source_span = head_span.to_absolute(self.absolute_offset);
                elements.push(TemplateLiteralElement {
                    span: head_span,
                    source_span: head_source_span,
                    text: token.str_value.clone(),
                });

                // Parse expressions and template parts
                loop {
                    // The lexer emits ${ as an operator token - skip it
                    self.optional_operator("${");

                    // Parse the expression inside ${...}
                    // Use parse_pipe to allow pipes inside template interpolations
                    let expr = self.parse_pipe();
                    expressions.push(expr);

                    // The lexer emits } as a character token - skip it
                    self.optional_character('}');

                    // Check what comes next
                    if let Some(next_token) = self.peek().cloned() {
                        if next_token.is_template_middle() {
                            self.advance();
                            let middle_span = ParseSpan::new(next_token.index, next_token.end);
                            let middle_source_span = middle_span.to_absolute(self.absolute_offset);
                            elements.push(TemplateLiteralElement {
                                span: middle_span,
                                source_span: middle_source_span,
                                text: next_token.str_value.clone(),
                            });
                            continue;
                        } else if next_token.is_template_tail() {
                            self.advance();
                            let tail_span = ParseSpan::new(next_token.index, next_token.end);
                            let tail_source_span = tail_span.to_absolute(self.absolute_offset);
                            elements.push(TemplateLiteralElement {
                                span: tail_span,
                                source_span: tail_source_span,
                                text: next_token.str_value.clone(),
                            });
                            break;
                        }
                    }
                    break;
                }

                let end = self.current_end_index();
                let template_span = ParseSpan::new(token.index, end);
                let template_source_span = template_span.to_absolute(self.absolute_offset);
                TemplateLiteral {
                    span: template_span,
                    source_span: template_source_span,
                    elements,
                    expressions,
                }
            } else {
                // This shouldn't happen since we checked is_template() in the caller
                self.error("Expected template literal after tag");
                return tag;
            };

            let end = self.current_end_index();
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);

            let tagged = TaggedTemplateLiteral { span, source_span, tag, template: template_expr };
            return AngularExpression::TaggedTemplateLiteral(Box::new_in(tagged, self.allocator));
        }

        // No template token found, just return the tag
        tag
    }

    /// Parses a regex literal.
    /// The lexer emits separate RegExpBody and RegExpFlags tokens.
    fn parse_regex_literal(&mut self, token: Token<'a>) -> AngularExpression<'a> {
        let start = token.index;
        let body = token.str_value.clone();
        self.advance();

        // Check for optional flags token
        let (flags, end) = match self.peek() {
            Some(t) if t.is_regexp_flags() => {
                let flags_str = t.str_value.clone();
                let flags_end = t.end;
                self.advance();
                (Some(flags_str), flags_end)
            }
            _ => (None, token.end),
        };

        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);

        let regex = RegularExpressionLiteral { span, source_span, body, flags };
        AngularExpression::RegularExpressionLiteral(Box::new_in(regex, self.allocator))
    }

    /// Parses an object literal.
    /// Note: The opening `{` was already consumed before calling this.
    fn parse_object_literal(&mut self, start: u32) -> AngularExpression<'a> {
        self.rbraces_expected += 1;
        let mut keys = Vec::new_in(self.allocator);
        let mut values = Vec::new_in(self.allocator);

        if !self.peek().map(|t| t.is_character('}')).unwrap_or(true) {
            loop {
                let (key, value) = self.parse_object_property();
                keys.push(key);
                values.push(value);
                if !self.optional_character(',') {
                    break;
                }
                // Handle trailing comma: if next token is '}', break (don't add empty entry)
                if self.peek().map(|t| t.is_character('}')).unwrap_or(true) {
                    break;
                }
            }
        }

        self.expect_character('}');
        self.rbraces_expected -= 1;
        let end = self.current_end_index();
        let span = ParseSpan::new(start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let obj = LiteralMap { span, source_span, keys, values };
        AngularExpression::LiteralMap(Box::new_in(obj, self.allocator))
    }

    /// Parses an object property.
    fn parse_object_property(&mut self) -> (LiteralMapKey<'a>, AngularExpression<'a>) {
        // Check for spread: `...expr`
        if self.peek().map(|t| t.is_operator("...")).unwrap_or(false) {
            let key_start = self.peek().map(|t| t.index).unwrap_or(0);
            self.advance(); // consume '...'
            let key_end = self.current_end_index();
            let span = ParseSpan::new(key_start, key_end);
            let source_span = span.to_absolute(self.absolute_offset);
            let key = LiteralMapKey::Spread(LiteralMapSpreadKey { span, source_span });
            let value = self.parse_pipe();
            return (key, value);
        }

        if let Some(token) = self.peek().cloned() {
            // String key
            if token.is_string() {
                self.advance();
                let key = LiteralMapKey::Property(LiteralMapPropertyKey {
                    key: token.str_value.clone(),
                    quoted: true,
                    is_shorthand_initialized: false,
                });
                self.expect_character(':');
                let value = self.parse_pipe();
                return (key, value);
            }

            // Check for private identifier - not supported
            if token.is_private_identifier() {
                let name = token.str_value.clone();
                let token_index = token.index;
                let token_end = token.end;
                self.error(&format!(
                    "Private identifiers are not supported. Unexpected private identifier: {name}, expected identifier, keyword or string"
                ));
                self.advance();
                let key = LiteralMapKey::Property(LiteralMapPropertyKey {
                    key: Ident::from_in("", self.allocator),
                    quoted: false,
                    is_shorthand_initialized: false,
                });
                let span = ParseSpan::new(token_index, token_end);
                let source_span = span.to_absolute(self.absolute_offset);
                let empty = EmptyExpr { span, source_span };
                return (key, AngularExpression::Empty(Box::new_in(empty, self.allocator)));
            }

            // Identifier key
            if token.is_identifier() || token.is_keyword() {
                // Capture prev_end before advancing
                let prev_end = self.previous_end();
                self.advance();
                let key_name = token.str_value.clone();

                // Check for shorthand
                if !self.peek().map(|t| t.is_character(':')).unwrap_or(false) {
                    let key = LiteralMapKey::Property(LiteralMapPropertyKey {
                        key: key_name.clone(),
                        quoted: false,
                        is_shorthand_initialized: true,
                    });
                    // Create property read for shorthand
                    let span = ParseSpan::new(token.index, token.end);
                    let source_span = span.to_absolute(self.absolute_offset);
                    // ImplicitReceiver spans from previous token end to current token start
                    // This covers any whitespace between tokens (Angular behavior)
                    let implicit_span = ParseSpan::new(prev_end, token.index);
                    let implicit_source_span = implicit_span.to_absolute(self.absolute_offset);
                    let implicit =
                        ImplicitReceiver { span: implicit_span, source_span: implicit_source_span };
                    let receiver =
                        AngularExpression::ImplicitReceiver(Box::new_in(implicit, self.allocator));
                    let name_span = source_span;
                    let read =
                        PropertyRead { span, source_span, name_span, receiver, name: key_name };
                    let value = AngularExpression::PropertyRead(Box::new_in(read, self.allocator));
                    return (key, value);
                }

                let key = LiteralMapKey::Property(LiteralMapPropertyKey {
                    key: key_name,
                    quoted: false,
                    is_shorthand_initialized: false,
                });
                self.expect_character(':');
                let value = self.parse_pipe();
                return (key, value);
            }
        }

        self.error("Missing expected identifier, keyword, or string");
        let key = LiteralMapKey::Property(LiteralMapPropertyKey {
            key: Ident::from_in("", self.allocator),
            quoted: false,
            is_shorthand_initialized: false,
        });
        let span = ParseSpan::new(0, 0);
        let source_span = span.to_absolute(self.absolute_offset);
        let empty = EmptyExpr { span, source_span };
        (key, AngularExpression::Empty(Box::new_in(empty, self.allocator)))
    }

    /// Parses a spread element: `...expr`.
    fn parse_spread_element(&mut self) -> AngularExpression<'a> {
        let spread_start = self.peek().map(|t| t.index).unwrap_or(0);

        if !self.optional_operator("...") {
            self.error("Spread element must start with '...' operator");
        }

        let expression = self.parse_pipe();
        let end = self.current_end_index();
        let span = ParseSpan::new(spread_start, end);
        let source_span = span.to_absolute(self.absolute_offset);
        let spread = SpreadElement { span, source_span, expression };
        AngularExpression::SpreadElement(Box::new_in(spread, self.allocator))
    }

    /// Parses a pipe expression.
    fn parse_pipe(&mut self) -> AngularExpression<'a> {
        let start = self.peek().map(|t| t.index).unwrap_or(0);
        let mut result = self.parse_expression();

        // Check if we're about to consume a pipe with an empty left-hand expression
        // This handles cases like " | a | b" where the expression starts with a pipe
        if let Some(token) = self.peek() {
            if token.is_operator("|") && matches!(result, AngularExpression::Empty(_)) {
                self.error("Unexpected token |");
            }
        }

        if !self.optional_operator("|") {
            return result;
        }

        // Pipes are not allowed in action expressions
        if self.action_mode {
            self.error("Cannot have a pipe in an action expression");
        }

        loop {
            let name_start = self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32);
            let (name, name_end, full_span_end) = if let Some(token) = self.peek().cloned() {
                if token.is_identifier() || token.is_keyword() {
                    let name = token.str_value.clone();
                    let name_end = token.end;
                    self.advance();
                    (name, name_end, None)
                } else {
                    // No valid identifier was found, so we'll assume an empty pipe name ('')
                    // Report error but continue parsing
                    if token.is_operator("|") {
                        self.error(&format!(
                            "Unexpected token {}, expected identifier or keyword",
                            token.str_value
                        ));
                    } else {
                        self.error("expected identifier or keyword");
                    }
                    // The fullSpanEnd tracks whitespace after the pipe character
                    let full_span_end =
                        self.peek().map(|t| t.index).unwrap_or(self.source.len() as u32);
                    (Ident::from_in("", self.allocator), full_span_end, Some(full_span_end))
                }
            } else {
                // End of input - create empty pipe name
                self.error("Unexpected end of input, expected identifier or keyword");
                let end_pos = self.source.len() as u32;
                (Ident::from_in("", self.allocator), end_pos, Some(end_pos))
            };

            // Parse pipe arguments
            let mut args = Vec::new_in(self.allocator);
            while self.optional_character(':') {
                let arg = self.parse_expression();
                // Check for empty argument followed by | or EOF
                if matches!(arg, AngularExpression::Empty(_)) {
                    if let Some(next_token) = self.peek() {
                        if next_token.is_operator("|") {
                            self.error("Unexpected token |");
                        }
                    } else {
                        // EOF after colon
                        self.error("Unexpected end of expression");
                    }
                }
                args.push(arg);
            }

            // Calculate spans
            let end = full_span_end.unwrap_or_else(|| self.current_end_index());
            let span = ParseSpan::new(start, end);
            let source_span = span.to_absolute(self.absolute_offset);
            let name_span = AbsoluteSourceSpan::new(
                self.absolute_offset + name_start,
                self.absolute_offset + name_end,
            );

            let pipe = BindingPipe {
                span,
                source_span,
                name_span,
                exp: result,
                name,
                args,
                pipe_type: BindingPipeType::ReferencedByName,
            };
            result = AngularExpression::BindingPipe(Box::new_in(pipe, self.allocator));

            // Continue if there's another pipe
            if !self.optional_operator("|") {
                break;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_identifier() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "foo");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::PropertyRead(_)));
    }

    #[test]
    fn test_parse_number() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "42");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::LiteralPrimitive(_)));
    }

    #[test]
    fn test_parse_binary() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "1 + 2");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
    }

    #[test]
    fn test_parse_in_operator() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "key in object");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::In);
        }
    }

    #[test]
    fn test_parse_property_read() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "foo.bar");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::PropertyRead(_)));
    }

    #[test]
    fn test_parse_method_call() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "foo.bar()");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::Call(_)));
    }

    #[test]
    fn test_parse_array() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "[1, 2, 3]");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::LiteralArray(_)));
    }

    #[test]
    fn test_parse_object() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "{a: 1, b: 2}");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::LiteralMap(_)));
    }

    #[test]
    fn test_parse_keyed_read() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "obj[key]");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::KeyedRead(_)));
    }

    #[test]
    fn test_parse_safe_keyed_read() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "obj?.[key]");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::SafeKeyedRead(_)));
    }

    #[test]
    fn test_parse_safe_call() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "fn?.()");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::SafeCall(_)));
    }

    #[test]
    fn test_parse_safe_property_read() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "obj?.prop");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::SafePropertyRead(_)));
    }

    #[test]
    fn test_parse_typeof() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "typeof value");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::TypeofExpression(_)));
    }

    #[test]
    fn test_parse_void() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "void 0");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::VoidExpression(_)));
    }

    #[test]
    fn test_parse_nested_typeof() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "typeof obj.prop");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::TypeofExpression(_)));
    }

    #[test]
    fn test_parse_simple_template_literal() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "`hello world`");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::TemplateLiteral(_)));
        if let AngularExpression::TemplateLiteral(tpl) = result.ast {
            assert_eq!(tpl.elements.len(), 1);
            assert_eq!(tpl.expressions.len(), 0);
            assert_eq!(tpl.elements[0].text.as_str(), "hello world");
        }
    }

    #[test]
    fn test_parse_template_literal_with_expression() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "`hello ${name}`");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::TemplateLiteral(_)));
        if let AngularExpression::TemplateLiteral(tpl) = result.ast {
            assert_eq!(tpl.elements.len(), 2);
            assert_eq!(tpl.expressions.len(), 1);
            assert_eq!(tpl.elements[0].text.as_str(), "hello ");
            assert_eq!(tpl.elements[1].text.as_str(), "");
        }
    }

    #[test]
    fn test_parse_template_literal_with_multiple_expressions() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "`${a} + ${b} = ${c}`");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::TemplateLiteral(_)));
        if let AngularExpression::TemplateLiteral(tpl) = result.ast {
            assert_eq!(tpl.elements.len(), 4);
            assert_eq!(tpl.expressions.len(), 3);
            assert_eq!(tpl.elements[0].text.as_str(), "");
            assert_eq!(tpl.elements[1].text.as_str(), " + ");
            assert_eq!(tpl.elements[2].text.as_str(), " = ");
            assert_eq!(tpl.elements[3].text.as_str(), "");
        }
    }

    // ========================================================================
    // Template binding (microsyntax) tests
    // ========================================================================

    #[test]
    fn test_parse_template_bindings_simple_ngif() {
        // *ngIf="condition"
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "condition");
        let key = TemplateBindingIdentifier {
            source: Ident::from("ngIf"),
            span: AbsoluteSourceSpan::new(0, 4),
        };
        let result = parser.parse_template_bindings(key);

        assert_eq!(result.bindings.len(), 1);
        assert!(result.errors.is_empty());

        // First binding: ngIf = condition
        match &result.bindings[0] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngIf");
                assert!(expr.value.is_some());
            }
            _ => panic!("Expected expression binding"),
        }
    }

    #[test]
    fn test_parse_template_bindings_ngfor_let() {
        // *ngFor="let item of items"
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "let item of items");
        let key = TemplateBindingIdentifier {
            source: Ident::from("ngFor"),
            span: AbsoluteSourceSpan::new(0, 5),
        };
        let result = parser.parse_template_bindings(key);

        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.bindings.len(), 3);

        // First binding: ngFor (no value since starts with let)
        match &result.bindings[0] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngFor");
                assert!(expr.value.is_none());
            }
            _ => panic!("Expected expression binding"),
        }

        // Second binding: let item (variable binding with no value - Angular behavior)
        match &result.bindings[1] {
            TemplateBinding::Variable(var) => {
                assert_eq!(var.key.source.as_str(), "item");
                assert!(var.value.is_none());
            }
            _ => panic!("Expected variable binding"),
        }

        // Third binding: ngForOf = items
        match &result.bindings[2] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngForOf");
                assert!(expr.value.is_some());
            }
            _ => panic!("Expected expression binding"),
        }
    }

    #[test]
    fn test_parse_template_bindings_ngfor_with_index() {
        // *ngFor="let item of items; let i = index"
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "let item of items; let i = index");
        let key = TemplateBindingIdentifier {
            source: Ident::from("ngFor"),
            span: AbsoluteSourceSpan::new(0, 5),
        };
        let result = parser.parse_template_bindings(key);

        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.bindings.len(), 4);

        // First: ngFor (no value)
        match &result.bindings[0] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngFor");
            }
            _ => panic!("Expected expression binding"),
        }

        // Second: let item (no value)
        match &result.bindings[1] {
            TemplateBinding::Variable(var) => {
                assert_eq!(var.key.source.as_str(), "item");
                assert!(var.value.is_none());
            }
            _ => panic!("Expected variable binding"),
        }

        // Third: ngForOf = items
        match &result.bindings[2] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngForOf");
            }
            _ => panic!("Expected expression binding"),
        }

        // Fourth: let i = index (value as-is, not prefixed with directive name)
        match &result.bindings[3] {
            TemplateBinding::Variable(var) => {
                assert_eq!(var.key.source.as_str(), "i");
                assert!(var.value.is_some());
                assert_eq!(var.value.as_ref().unwrap().source.as_str(), "index");
            }
            _ => panic!("Expected variable binding"),
        }
    }

    #[test]
    fn test_parse_template_bindings_ngfor_with_trackby() {
        // *ngFor="let item of items; trackBy: trackByFn"
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "let item of items; trackBy: trackByFn");
        let key = TemplateBindingIdentifier {
            source: Ident::from("ngFor"),
            span: AbsoluteSourceSpan::new(0, 5),
        };
        let result = parser.parse_template_bindings(key);

        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.bindings.len(), 4);

        // Fourth: ngForTrackBy = trackByFn
        match &result.bindings[3] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngForTrackBy");
                assert!(expr.value.is_some());
            }
            _ => panic!("Expected expression binding"),
        }
    }

    #[test]
    fn test_parse_template_bindings_ngif_with_as() {
        // *ngIf="obs$ | async as result"
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "obs$ | async as result");
        let key = TemplateBindingIdentifier {
            source: Ident::from("ngIf"),
            span: AbsoluteSourceSpan::new(0, 4),
        };
        let result = parser.parse_template_bindings(key);

        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.bindings.len(), 2);

        // First: ngIf = obs$ | async
        match &result.bindings[0] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngIf");
                assert!(expr.value.is_some());
                // The value should be a BindingPipe
                if let Some(ast_with_source) = &expr.value {
                    assert!(matches!(ast_with_source.ast, AngularExpression::BindingPipe(_)));
                }
            }
            _ => panic!("Expected expression binding"),
        }

        // Second: result = ngIf (variable binding from 'as')
        match &result.bindings[1] {
            TemplateBinding::Variable(var) => {
                assert_eq!(var.key.source.as_str(), "result");
                assert!(var.value.is_some());
                assert_eq!(var.value.as_ref().unwrap().source.as_str(), "ngIf");
            }
            _ => panic!("Expected variable binding"),
        }
    }

    #[test]
    fn test_parse_template_bindings_ngfor_index_as_i() {
        // *ngFor="let item of items; index as i"
        // This tests the case where a context variable (index) uses "as" to create an alias
        // Per TypeScript, this produces 4 bindings (not 5):
        // 1. ngFor (no value)
        // 2. let item (no value)
        // 3. ngForOf = items
        // 4. i = index (variable binding)
        // Note: We do NOT produce an ngForIndex expression binding - the `as` pattern
        // directly produces a variable binding with the original keyword as value.
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "let item of items; index as i");
        let key = TemplateBindingIdentifier {
            source: Ident::from("ngFor"),
            span: AbsoluteSourceSpan::new(0, 5),
        };
        let result = parser.parse_template_bindings(key);

        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.bindings.len(), 4, "Bindings: {:?}", result.bindings);

        // First: ngFor (no value)
        match &result.bindings[0] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngFor");
                assert!(expr.value.is_none());
            }
            _ => panic!("Expected expression binding for ngFor"),
        }

        // Second: let item (no value)
        match &result.bindings[1] {
            TemplateBinding::Variable(var) => {
                assert_eq!(var.key.source.as_str(), "item");
                assert!(var.value.is_none());
            }
            _ => panic!("Expected variable binding for item"),
        }

        // Third: ngForOf = items
        match &result.bindings[2] {
            TemplateBinding::Expression(expr) => {
                assert_eq!(expr.key.source.as_str(), "ngForOf");
                assert!(expr.value.is_some());
            }
            _ => panic!("Expected expression binding for ngForOf"),
        }

        // Fourth: i = index (variable binding from 'as')
        // Per TypeScript, when `index as i` is parsed, we get a variable binding
        // with value="index" (the original keyword), NOT "ngForIndex".
        // The NgForOfContext class has a property `index`, not `ngForIndex`.
        match &result.bindings[3] {
            TemplateBinding::Variable(var) => {
                assert_eq!(var.key.source.as_str(), "i");
                assert!(var.value.is_some());
                // IMPORTANT: value is the original keyword "index", not "ngForIndex"
                assert_eq!(var.value.as_ref().unwrap().source.as_str(), "index");
            }
            _ => panic!("Expected variable binding for i"),
        }
    }

    // ========================================================================
    // Regex literal tests
    // ========================================================================

    #[test]
    fn test_parse_regex_literal() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "/pattern/gi");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::RegularExpressionLiteral(_)));
        if let AngularExpression::RegularExpressionLiteral(regex) = result.ast {
            assert_eq!(regex.body.as_str(), "pattern");
            assert_eq!(regex.flags.as_ref().map(|f| f.as_str()), Some("gi"));
        }
    }

    #[test]
    fn test_parse_regex_without_flags() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "/abc/");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::RegularExpressionLiteral(_)));
        if let AngularExpression::RegularExpressionLiteral(regex) = result.ast {
            assert_eq!(regex.body.as_str(), "abc");
            assert!(regex.flags.is_none());
        }
    }

    #[test]
    fn test_parse_regex_with_escape() {
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, r"/\d+/");
        let result = parser.parse_simple_binding();
        assert!(matches!(result.ast, AngularExpression::RegularExpressionLiteral(_)));
        if let AngularExpression::RegularExpressionLiteral(regex) = result.ast {
            assert_eq!(regex.body.as_str(), r"\d+");
        }
    }

    // ========================================================================
    // Error recovery tests
    // ========================================================================

    #[test]
    fn test_error_recovery_in_parenthesized_expr() {
        // Test that an error inside parentheses allows recovery
        // The parser should skip to ')' and continue parsing
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "(a.) + 1");
        let result = parser.parse_simple_binding();
        // Should produce an expression (with errors) but not panic
        assert!(!result.errors.is_empty()); // Should have an error
    }

    #[test]
    fn test_error_recovery_in_array_literal() {
        // Test error recovery in array literals
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "[1, , 3]");
        let result = parser.parse_simple_binding();
        // Should parse as an array despite the empty element
        assert!(matches!(result.ast, AngularExpression::LiteralArray(_)));
    }

    #[test]
    fn test_error_recovery_in_call() {
        // Test error recovery in function calls
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "foo(a.)");
        let result = parser.parse_simple_binding();
        // Should produce a call expression with errors
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_error_recovery_at_semicolon() {
        // Test that skip() stops at semicolons
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a.; b");
        let result = parser.parse_action();
        // Should parse as a chain with errors
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_error_recovery_at_pipe() {
        // Test that skip() stops at pipe operators
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a. | uppercase");
        let result = parser.parse_simple_binding();
        // Should have errors but still be parseable
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_nested_error_recovery() {
        // Test error recovery in nested grouping constructs
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "f(a[.])");
        let result = parser.parse_simple_binding();
        // Should handle nested brackets with errors
        assert!(!result.errors.is_empty());
    }

    // ========================================================================
    // Assignment expression tests (action mode only)
    // ========================================================================

    #[test]
    fn test_parse_assignment_in_action() {
        // Simple assignment: a = b
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a = b");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::Assign);
            assert!(matches!(bin.left, AngularExpression::PropertyRead(_)));
            assert!(matches!(bin.right, AngularExpression::PropertyRead(_)));
        }
    }

    #[test]
    fn test_parse_property_assignment_in_action() {
        // Property assignment: a.b = c
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a.b = c");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::Assign);
            assert!(matches!(bin.left, AngularExpression::PropertyRead(_)));
        }
    }

    #[test]
    fn test_parse_keyed_assignment_in_action() {
        // Keyed assignment: a[b] = c
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a[b] = c");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::Assign);
            assert!(matches!(bin.left, AngularExpression::KeyedRead(_)));
        }
    }

    #[test]
    fn test_parse_compound_assignment_in_action() {
        // Compound assignment: a += b
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a += b");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::AddAssign);
        }
    }

    #[test]
    fn test_parse_nullish_coalescing_assignment() {
        // Nullish coalescing assignment: a.b ??= c
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a.b ??= c");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::NullishCoalescingAssign);
        }
    }

    #[test]
    fn test_parse_or_assignment() {
        // Logical OR assignment: a[b] ||= c
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a[b] ||= c");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
        if let AngularExpression::Binary(bin) = result.ast {
            assert_eq!(bin.operation, BinaryOperator::OrAssign);
        }
    }

    #[test]
    fn test_assignment_not_allowed_in_binding() {
        // Assignment should not be allowed in simple binding
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a = b");
        let result = parser.parse_simple_binding();
        // Should have errors - bindings cannot contain assignments
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_safe_property_assignment_error() {
        // Safe property access with assignment should error
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a?.b = c");
        let result = parser.parse_action();
        // Should have an error about ?. in assignment
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_chain_with_assignment() {
        // Multiple expressions with semicolons: a = 1; b = 2
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a = 1; b = 2");
        let result = parser.parse_action();
        assert!(matches!(result.ast, AngularExpression::Chain(_)));
        if let AngularExpression::Chain(chain) = result.ast {
            assert_eq!(chain.expressions.len(), 2);
            assert!(matches!(chain.expressions[0], AngularExpression::Binary(_)));
            assert!(matches!(chain.expressions[1], AngularExpression::Binary(_)));
        }
    }

    // ========================================================================
    // Comment stripping tests
    // ========================================================================

    #[test]
    fn test_comment_stripping_single_line() {
        // Comments should be stripped: a // comment
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a // comment");
        let result = parser.parse_simple_binding();
        assert!(result.errors.is_empty());
        assert!(matches!(result.ast, AngularExpression::PropertyRead(_)));
    }

    #[test]
    fn test_comment_stripping_preserves_expression() {
        // Expression before comment should be preserved: a + b // comment
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a + b // plus operation");
        let result = parser.parse_simple_binding();
        assert!(result.errors.is_empty());
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
    }

    #[test]
    fn test_comment_stripping_in_string_ignored() {
        // Slashes inside strings should not be treated as comments
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "'http://example.com'");
        let result = parser.parse_simple_binding();
        assert!(result.errors.is_empty());
        assert!(matches!(result.ast, AngularExpression::LiteralPrimitive(_)));
        if let AngularExpression::LiteralPrimitive(lit) = result.ast {
            if let LiteralValue::String(s) = &lit.value {
                assert_eq!(s.as_str(), "http://example.com");
            } else {
                panic!("Expected string literal");
            }
        }
    }

    #[test]
    fn test_comment_stripping_double_quote_string() {
        // Double-quoted strings should also preserve //
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "\"http://example.com\"");
        let result = parser.parse_simple_binding();
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_no_comment_without_double_slash() {
        // Single slash should not trigger comment stripping
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "a / b");
        let result = parser.parse_simple_binding();
        assert!(result.errors.is_empty());
        assert!(matches!(result.ast, AngularExpression::Binary(_)));
    }

    #[test]
    fn test_comment_at_start_produces_empty() {
        // Comment at the start should produce empty expression
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "// just a comment");
        let result = parser.parse_simple_binding();
        // Empty input after stripping should produce EmptyExpr
        assert!(matches!(result.ast, AngularExpression::Empty(_)));
    }

    // ========================================================================
    // Interpolation validation tests
    // ========================================================================

    #[test]
    fn test_interpolation_in_binding_produces_error() {
        // {{ }} in binding expression should produce an error
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "{{name}}");
        let result = parser.parse_simple_binding();
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].msg.contains("interpolation"));
    }

    #[test]
    fn test_interpolation_in_action_produces_error() {
        // {{ }} in action expression should also produce an error
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "{{onClick()}}");
        let result = parser.parse_action();
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].msg.contains("interpolation"));
    }

    #[test]
    fn test_interpolation_in_string_ignored() {
        // {{ }} inside strings should not trigger the error
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "'{{name}}'");
        let result = parser.parse_simple_binding();
        // Should not produce interpolation error (the expression is valid - it's a string literal)
        let has_interpolation_error = result.errors.iter().any(|e| e.msg.contains("interpolation"));
        assert!(!has_interpolation_error);
    }

    #[test]
    fn test_no_interpolation_error_for_regular_braces() {
        // Single braces should not trigger interpolation error
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "{a: 1}");
        let result = parser.parse_simple_binding();
        // Should not have interpolation error
        let has_interpolation_error = result.errors.iter().any(|e| e.msg.contains("interpolation"));
        assert!(!has_interpolation_error);
    }

    #[test]
    fn test_interpolation_error_with_expression_inside() {
        // Interpolation with expression: {{a + b}}
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "{{a + b}}");
        let result = parser.parse_simple_binding();
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].msg.contains("interpolation"));
    }

    #[test]
    fn test_incomplete_interpolation_no_error() {
        // Incomplete interpolation {{ without }} should not trigger interpolation error
        let allocator = Allocator::default();
        let parser = Parser::new(&allocator, "{{name");
        let result = parser.parse_simple_binding();
        // Should not have interpolation error (no closing }})
        let has_interpolation_error = result.errors.iter().any(|e| e.msg.contains("interpolation"));
        assert!(!has_interpolation_error);
    }
}

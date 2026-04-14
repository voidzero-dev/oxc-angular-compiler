//! Angular expression lexer.
//!
//! Tokenizes Angular binding expressions including pipes and safe navigation.
//!
//! Ported from Angular's `expression_parser/lexer.ts`.

use oxc_allocator::{Allocator, FromIn};
use oxc_str::Ident;

use crate::util::chars;

/// Token types for Angular expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    // Literals
    /// A character literal.
    Character,
    /// An identifier.
    Identifier,
    /// A private identifier (#name).
    PrivateIdentifier,
    /// A keyword (true, false, null, undefined, etc.).
    Keyword,
    /// A string literal.
    String,
    /// A number literal.
    Number,
    /// A regular expression body (the pattern between /.../).
    RegExpBody,
    /// Regular expression flags (after the closing /).
    RegExpFlags,

    // Template literal parts
    /// A template literal with no substitutions: `simple string`
    NoSubstitutionTemplate,
    /// The first part of a template literal before ${: `hello ${
    TemplateHead,
    /// A middle part of a template literal between } and ${: } world ${
    TemplateMiddle,
    /// The last part of a template literal after }: } end`
    TemplateTail,

    // Operators
    /// An operator (+, -, *, /, etc.).
    Operator,

    // Structural
    /// An error token.
    Error,
}

/// String token kinds for distinguishing different string types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringTokenKind {
    /// A plain string literal.
    Plain,
    /// A template literal part (before/between interpolations).
    TemplateLiteralPart,
    /// The end of a template literal (after last interpolation or no substitution).
    TemplateLiteralEnd,
}

/// A token in an Angular expression.
#[derive(Debug, Clone)]
pub struct Token<'a> {
    /// The token type.
    pub token_type: TokenType,
    /// The token index in the source.
    pub index: u32,
    /// The end index in the source.
    pub end: u32,
    /// The numeric value (for Number tokens).
    pub num_value: f64,
    /// The string value (for String/Identifier tokens).
    pub str_value: Ident<'a>,
    /// The string token kind (for String/Template tokens).
    pub str_kind: StringTokenKind,
}

impl<'a> Token<'a> {
    /// Creates a new token.
    fn new(
        token_type: TokenType,
        index: u32,
        end: u32,
        num_value: f64,
        str_value: Ident<'a>,
        str_kind: StringTokenKind,
    ) -> Self {
        Self { token_type, index, end, num_value, str_value, str_kind }
    }

    /// Creates a character token.
    fn new_character(index: u32, end: u32, code: char, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::Character,
            index,
            end,
            code as u32 as f64,
            Ident::from_in(String::from(code), allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates an identifier token.
    fn new_identifier(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::Identifier,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates a private identifier token.
    fn new_private_identifier(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::PrivateIdentifier,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates a keyword token.
    fn new_keyword(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::Keyword,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates an operator token.
    fn new_operator(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::Operator,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates a string token.
    fn new_string(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::String,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates a number token.
    fn new_number(index: u32, end: u32, value: f64, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::Number,
            index,
            end,
            value,
            Ident::from_in("", allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates an error token.
    fn new_error(index: u32, end: u32, message: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::Error,
            index,
            end,
            0.0,
            Ident::from_in(message, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates a template token with no substitutions.
    fn new_no_substitution_template(
        index: u32,
        end: u32,
        text: &str,
        allocator: &'a Allocator,
    ) -> Self {
        Self::new(
            TokenType::NoSubstitutionTemplate,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::TemplateLiteralEnd,
        )
    }

    /// Creates a template head token (first part before ${).
    fn new_template_head(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::TemplateHead,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::TemplateLiteralPart,
        )
    }

    /// Creates a template middle token (part between } and ${).
    fn new_template_middle(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::TemplateMiddle,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::TemplateLiteralPart,
        )
    }

    /// Creates a template tail token (last part after }).
    fn new_template_tail(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::TemplateTail,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::TemplateLiteralEnd,
        )
    }

    /// Creates a regexp body token.
    fn new_regexp_body(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::RegExpBody,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Creates a regexp flags token.
    fn new_regexp_flags(index: u32, end: u32, text: &str, allocator: &'a Allocator) -> Self {
        Self::new(
            TokenType::RegExpFlags,
            index,
            end,
            0.0,
            Ident::from_in(text, allocator),
            StringTokenKind::Plain,
        )
    }

    /// Returns true if this is a character token with the given character.
    pub fn is_character(&self, code: char) -> bool {
        self.token_type == TokenType::Character && self.num_value == code as u32 as f64
    }

    /// Returns true if this is an operator token with the given operator.
    pub fn is_operator(&self, op: &str) -> bool {
        self.token_type == TokenType::Operator && self.str_value.as_str() == op
    }

    /// Returns true if this is an identifier.
    pub fn is_identifier(&self) -> bool {
        self.token_type == TokenType::Identifier
    }

    /// Returns true if this is a private identifier.
    pub fn is_private_identifier(&self) -> bool {
        self.token_type == TokenType::PrivateIdentifier
    }

    /// Returns true if this is a keyword.
    pub fn is_keyword(&self) -> bool {
        self.token_type == TokenType::Keyword
    }

    /// Returns true if this is a keyword with the given value.
    pub fn is_keyword_value(&self, value: &str) -> bool {
        self.token_type == TokenType::Keyword && self.str_value.as_str() == value
    }

    /// Returns true if this is a string literal.
    pub fn is_string(&self) -> bool {
        self.token_type == TokenType::String
    }

    /// Returns true if this is a number literal.
    pub fn is_number(&self) -> bool {
        self.token_type == TokenType::Number
    }

    /// Returns true if this is an error token.
    pub fn is_error(&self) -> bool {
        self.token_type == TokenType::Error
    }

    /// Returns true if this is a template literal token (any type).
    pub fn is_template(&self) -> bool {
        matches!(
            self.token_type,
            TokenType::NoSubstitutionTemplate
                | TokenType::TemplateHead
                | TokenType::TemplateMiddle
                | TokenType::TemplateTail
        )
    }

    /// Returns true if this is a template head token.
    pub fn is_template_head(&self) -> bool {
        self.token_type == TokenType::TemplateHead
    }

    /// Returns true if this is a template middle token.
    pub fn is_template_middle(&self) -> bool {
        self.token_type == TokenType::TemplateMiddle
    }

    /// Returns true if this is a template tail token.
    pub fn is_template_tail(&self) -> bool {
        self.token_type == TokenType::TemplateTail
    }

    /// Returns true if this is a no-substitution template.
    pub fn is_no_substitution_template(&self) -> bool {
        self.token_type == TokenType::NoSubstitutionTemplate
    }

    /// Returns true if this is a regex literal (body or flags).
    pub fn is_regex(&self) -> bool {
        self.is_regexp_body() || self.is_regexp_flags()
    }

    /// Returns true if this is a regexp body token.
    pub fn is_regexp_body(&self) -> bool {
        self.token_type == TokenType::RegExpBody
    }

    /// Returns true if this is a regexp flags token.
    pub fn is_regexp_flags(&self) -> bool {
        self.token_type == TokenType::RegExpFlags
    }

    /// Converts the token to a string representation.
    #[expect(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        match self.token_type {
            TokenType::Character => format!("Character {}", self.str_value),
            TokenType::Identifier => format!("Identifier {}", self.str_value),
            TokenType::PrivateIdentifier => format!("PrivateIdentifier {}", self.str_value),
            TokenType::Keyword => format!("Keyword {}", self.str_value),
            TokenType::Operator => format!("Operator {}", self.str_value),
            TokenType::String => format!("String {}", self.str_value),
            TokenType::Number => format!("Number {}", self.num_value),
            TokenType::NoSubstitutionTemplate => {
                format!("NoSubstitutionTemplate {}", self.str_value)
            }
            TokenType::TemplateHead => format!("TemplateHead {}", self.str_value),
            TokenType::TemplateMiddle => format!("TemplateMiddle {}", self.str_value),
            TokenType::TemplateTail => format!("TemplateTail {}", self.str_value),
            TokenType::RegExpBody => format!("RegExpBody {}", self.str_value),
            TokenType::RegExpFlags => format!("RegExpFlags {}", self.str_value),
            TokenType::Error => format!("Error {}", self.str_value),
        }
    }
}

/// Keywords recognized by the Angular expression parser.
const KEYWORDS: &[&str] = &[
    "var",
    "let",
    "as",
    "null",
    "undefined",
    "true",
    "false",
    "if",
    "else",
    "this",
    "typeof",
    "void",
    "in",
    "instanceof",
];

/// Angular expression lexer.
pub struct Lexer<'a> {
    /// The allocator.
    allocator: &'a Allocator,
    /// The source text.
    input: &'a str,
    /// The input length (in bytes).
    length: u32,
    /// The current byte position.
    index: u32,
    /// The generated tokens.
    tokens: std::vec::Vec<Token<'a>>,
}

impl<'a> Lexer<'a> {
    /// Creates a new lexer.
    pub fn new(allocator: &'a Allocator, input: &'a str) -> Self {
        Self {
            allocator,
            input,
            length: input.len() as u32,
            index: 0,
            tokens: std::vec::Vec::new(),
        }
    }

    /// Tokenizes the input and returns the tokens.
    pub fn tokenize(mut self) -> std::vec::Vec<Token<'a>> {
        while self.index < self.length {
            let start = self.index;
            self.scan_token(start);
        }
        self.tokens
    }

    /// Peeks at the current character.
    /// Returns the next UTF-8 character or EOF if at end of input.
    fn peek(&self) -> char {
        if self.index >= self.length {
            chars::EOF
        } else {
            self.input[self.index as usize..].chars().next().unwrap_or(chars::EOF)
        }
    }

    /// Peeks at a character at offset from current position.
    /// Note: offset is in characters, not bytes. For multi-byte UTF-8 characters,
    /// this iterates through the characters to find the one at the given offset.
    fn peek_at(&self, offset: u32) -> char {
        if self.index >= self.length {
            return chars::EOF;
        }
        self.input[self.index as usize..].chars().nth(offset as usize).unwrap_or(chars::EOF)
    }

    /// Advances the index by the current character's byte length and returns the character.
    fn advance(&mut self) -> char {
        let ch = self.peek();
        if ch != chars::EOF {
            self.index += ch.len_utf8() as u32;
        }
        ch
    }

    /// Returns true if at end of input.
    fn is_eof(&self) -> bool {
        self.index >= self.length
    }

    /// Skips whitespace characters.
    fn skip_whitespace(&mut self) {
        while !self.is_eof() && chars::is_whitespace(self.peek()) {
            self.advance();
        }
    }

    /// Scans a single token.
    fn scan_token(&mut self, start: u32) {
        let ch = self.advance();

        // Whitespace
        if chars::is_whitespace(ch) {
            return;
        }

        // Identifier or keyword
        if chars::is_identifier_start(ch) {
            self.scan_identifier(start);
            return;
        }

        // Private identifier or invalid hash
        if ch == chars::HASH {
            if chars::is_identifier_start(self.peek()) {
                self.scan_private_identifier(start);
            } else {
                // Invalid hash character - emit an error matching Angular's format
                let message = format!(
                    "Lexer Error: Invalid character [#] at column {} in expression [{}]",
                    start, self.input
                );
                self.error(start, &message);
            }
            return;
        }

        // Number
        if chars::is_digit(ch) {
            self.scan_number(start);
            return;
        }

        match ch {
            // String literals
            chars::SQ | chars::DQ => {
                self.scan_string(start, ch);
            }
            // Template literal
            chars::BT => {
                self.scan_template_string(start);
            }
            // Operators and punctuation
            chars::PLUS => {
                if self.peek() == chars::PLUS {
                    self.error(start, "++ is not allowed");
                } else if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "+=");
                } else {
                    self.scan_operator(start, "+");
                }
            }
            chars::MINUS => {
                if self.peek() == chars::MINUS {
                    self.error(start, "-- is not allowed");
                } else if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "-=");
                } else {
                    self.scan_operator(start, "-");
                }
            }
            chars::STAR => {
                if self.peek() == chars::STAR {
                    self.advance();
                    // Check for **= (exponentiation assignment)
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "**=");
                    } else {
                        self.scan_operator(start, "**");
                    }
                } else if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "*=");
                } else {
                    self.scan_operator(start, "*");
                }
            }
            chars::SLASH => {
                // Check if this is a regex or division
                if self.last_token_allows_regex() {
                    // In regex context, scan as regex (errors will be reported if unterminated)
                    self.scan_regex(start);
                } else if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "/=");
                } else {
                    self.scan_operator(start, "/");
                }
            }
            chars::PERCENT => {
                if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "%=");
                } else {
                    self.scan_operator(start, "%");
                }
            }
            chars::CARET => {
                self.scan_operator(start, "^");
            }
            chars::QUESTION => {
                if self.peek() == chars::QUESTION {
                    self.advance();
                    // Check for ??= (nullish coalescing assignment)
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "??=");
                    } else {
                        self.scan_operator(start, "??");
                    }
                } else if self.peek() == chars::PERIOD {
                    self.advance();
                    self.scan_operator(start, "?.");
                } else {
                    self.scan_character(start, ch);
                }
            }
            chars::LT => {
                if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "<=");
                } else {
                    self.scan_operator(start, "<");
                }
            }
            chars::GT => {
                if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, ">=");
                } else {
                    self.scan_operator(start, ">");
                }
            }
            chars::BANG => {
                if self.peek() == chars::EQ {
                    self.advance();
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "!==");
                    } else {
                        self.scan_operator(start, "!=");
                    }
                } else {
                    // Standalone ! is an operator (negation/non-null assertion)
                    self.scan_operator(start, "!");
                }
            }
            chars::EQ => {
                if self.peek() == chars::EQ {
                    self.advance();
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "===");
                    } else {
                        self.scan_operator(start, "==");
                    }
                } else if self.peek() == chars::GT {
                    // Arrow function =>
                    self.advance();
                    self.scan_operator(start, "=>");
                } else {
                    // Standalone = is an operator (assignment)
                    self.scan_operator(start, "=");
                }
            }
            chars::AMPERSAND => {
                if self.peek() == chars::AMPERSAND {
                    self.advance();
                    // Check for &&= (logical AND assignment)
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "&&=");
                    } else {
                        self.scan_operator(start, "&&");
                    }
                } else {
                    self.scan_character(start, ch);
                }
            }
            chars::BAR => {
                if self.peek() == chars::BAR {
                    self.advance();
                    // Check for ||= (logical OR assignment)
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "||=");
                    } else {
                        self.scan_operator(start, "||");
                    }
                } else if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "|=");
                } else {
                    // Single | is an operator (used for pipes in Angular)
                    self.scan_operator(start, "|");
                }
            }
            chars::PERIOD => {
                if chars::is_digit(self.peek()) {
                    self.index = start;
                    self.advance(); // re-consume the period
                    self.scan_number(start);
                } else if self.peek() == chars::PERIOD {
                    // Could be spread operator ...
                    self.advance(); // Second period
                    if self.peek() == chars::PERIOD {
                        self.advance(); // Third period
                        self.scan_operator(start, "...");
                    } else {
                        // Two dots is an error - Angular uses `peek` from start (the first period)
                        // Error position should be at current index (after second period)
                        let error_pos = self.index;
                        let message = format!(
                            "Lexer Error: Unexpected character [{ch}] at column {} in expression [{}]",
                            error_pos, self.input
                        );
                        self.error(error_pos, &message);
                    }
                } else {
                    self.scan_character(start, ch);
                }
            }
            // Other characters
            _ => {
                self.scan_character(start, ch);
            }
        }
    }

    /// Scans a character token.
    fn scan_character(&mut self, start: u32, ch: char) {
        let token = Token::new_character(start, self.index, ch, self.allocator);
        self.tokens.push(token);
    }

    /// Scans an operator token.
    fn scan_operator(&mut self, start: u32, op: &str) {
        let token = Token::new_operator(start, self.index, op, self.allocator);
        self.tokens.push(token);
    }

    /// Scans an identifier or keyword.
    fn scan_identifier(&mut self, start: u32) {
        while chars::is_identifier_part(self.peek()) {
            self.advance();
        }

        let text = &self.input[start as usize..self.index as usize];
        let token = if KEYWORDS.contains(&text) {
            Token::new_keyword(start, self.index, text, self.allocator)
        } else {
            Token::new_identifier(start, self.index, text, self.allocator)
        };
        self.tokens.push(token);
    }

    /// Scans a private identifier.
    fn scan_private_identifier(&mut self, start: u32) {
        self.advance(); // consume the identifier start character
        while chars::is_identifier_part(self.peek()) {
            self.advance();
        }

        // Include the # in the str_value to match Angular's TypeScript implementation
        let text = &self.input[start as usize..self.index as usize];
        let token = Token::new_private_identifier(start, self.index, text, self.allocator);
        self.tokens.push(token);
    }

    /// Scans a number literal, including support for numeric separators.
    fn scan_number(&mut self, start: u32) {
        // Check if number starts with a period (e.g., .5)
        let start_char = self.input[start as usize..].chars().next();
        let mut is_float = start_char == Some('.');

        // Check for hex, octal, or binary
        if start_char == Some('0') {
            let next = self.peek();
            if next == 'x' || next == 'X' {
                self.advance();
                return self.scan_hex_number(start);
            }
            if next == 'o' || next == 'O' {
                self.advance();
                return self.scan_octal_number(start);
            }
            if next == 'b' || next == 'B' {
                self.advance();
                return self.scan_binary_number(start);
            }
        }

        // Scan integer part (with numeric separators)
        let mut last_underscore_pos: Option<u32> = None;
        while chars::is_digit(self.peek()) || self.peek() == '_' {
            if self.peek() == '_' {
                if let Some(error_pos) = last_underscore_pos {
                    // Double underscore is invalid - report at the first underscore
                    let message = format!(
                        "Lexer Error: Invalid numeric separator at column {} in expression [{}]",
                        error_pos, self.input
                    );
                    self.error_at(error_pos, error_pos, &message);
                    return;
                }
                last_underscore_pos = Some(self.index);
            } else {
                last_underscore_pos = None;
            }
            self.advance();
        }

        // Check for trailing underscore before decimal point
        if let Some(underscore_pos) = last_underscore_pos {
            let message = format!(
                "Lexer Error: Invalid numeric separator at column {} in expression [{}]",
                underscore_pos, self.input
            );
            self.error_at(underscore_pos, underscore_pos, &message);
            return;
        }

        // Scan decimal part
        if self.peek() == chars::PERIOD {
            is_float = true;
            self.advance();

            // Check for leading underscore after decimal point
            if self.peek() == '_' {
                let error_pos = self.index;
                let message = format!(
                    "Lexer Error: Invalid numeric separator at column {} in expression [{}]",
                    error_pos, self.input
                );
                self.error_at(error_pos, error_pos, &message);
                return;
            }

            last_underscore_pos = None;
            while chars::is_digit(self.peek()) || self.peek() == '_' {
                if self.peek() == '_' {
                    if let Some(error_pos) = last_underscore_pos {
                        let message = format!(
                            "Lexer Error: Invalid numeric separator at column {} in expression [{}]",
                            error_pos, self.input
                        );
                        self.error_at(error_pos, error_pos, &message);
                        return;
                    }
                    last_underscore_pos = Some(self.index);
                } else {
                    last_underscore_pos = None;
                }
                self.advance();
            }

            // Check for trailing underscore
            if let Some(underscore_pos) = last_underscore_pos {
                let message = format!(
                    "Lexer Error: Invalid numeric separator at column {} in expression [{}]",
                    underscore_pos, self.input
                );
                self.error_at(underscore_pos, underscore_pos, &message);
                return;
            }
        }

        // Scan exponent
        if self.peek() == 'e' || self.peek() == 'E' {
            is_float = true;
            self.advance();
            if self.peek() == chars::PLUS || self.peek() == chars::MINUS {
                self.advance();
            }
            // Check for valid exponent start (not an underscore)
            if self.peek() == '_' || !chars::is_digit(self.peek()) {
                let error_pos = self.index;
                let message = format!(
                    "Lexer Error: Invalid exponent at column {} in expression [{}]",
                    error_pos - 1,
                    self.input
                );
                self.error_at(error_pos - 1, error_pos, &message);
                return;
            }
            while chars::is_digit(self.peek()) {
                self.advance();
            }
        }

        let text = &self.input[start as usize..self.index as usize];
        // Remove underscores for parsing
        let cleaned_text = text.replace('_', "");
        let value = if is_float {
            cleaned_text.parse::<f64>().unwrap_or(0.0)
        } else {
            cleaned_text.parse::<i64>().unwrap_or(0) as f64
        };

        let token = Token::new_number(start, self.index, value, self.allocator);
        self.tokens.push(token);
    }

    /// Scans a hexadecimal number.
    fn scan_hex_number(&mut self, start: u32) {
        if !chars::is_ascii_hex_digit(self.peek()) {
            self.error(start, "Invalid hexadecimal number");
            return;
        }
        while chars::is_ascii_hex_digit(self.peek()) || self.peek() == '_' {
            self.advance();
        }
        let text = &self.input[(start + 2) as usize..self.index as usize];
        let text = text.replace('_', "");
        let value = i64::from_str_radix(&text, 16).unwrap_or(0) as f64;
        let token = Token::new_number(start, self.index, value, self.allocator);
        self.tokens.push(token);
    }

    /// Scans an octal number.
    fn scan_octal_number(&mut self, start: u32) {
        if !chars::is_octal_digit(self.peek()) {
            self.error(start, "Invalid octal number");
            return;
        }
        while chars::is_octal_digit(self.peek()) || self.peek() == '_' {
            self.advance();
        }
        let text = &self.input[(start + 2) as usize..self.index as usize];
        let text = text.replace('_', "");
        let value = i64::from_str_radix(&text, 8).unwrap_or(0) as f64;
        let token = Token::new_number(start, self.index, value, self.allocator);
        self.tokens.push(token);
    }

    /// Scans a binary number.
    fn scan_binary_number(&mut self, start: u32) {
        if self.peek() != '0' && self.peek() != '1' {
            self.error(start, "Invalid binary number");
            return;
        }
        while self.peek() == '0' || self.peek() == '1' || self.peek() == '_' {
            self.advance();
        }
        let text = &self.input[(start + 2) as usize..self.index as usize];
        let text = text.replace('_', "");
        let value = i64::from_str_radix(&text, 2).unwrap_or(0) as f64;
        let token = Token::new_number(start, self.index, value, self.allocator);
        self.tokens.push(token);
    }

    /// Scans a string literal.
    fn scan_string(&mut self, start: u32, quote: char) {
        let mut result = String::new();

        loop {
            let ch = self.peek();
            if ch == chars::EOF {
                self.error(start, "Unterminated string");
                return;
            }
            if ch == quote {
                self.advance();
                break;
            }
            if ch == chars::BACKSLASH {
                let escape_start = self.index;
                self.advance();
                match self.scan_escape_sequence_with_error(escape_start) {
                    Ok(escaped) => result.push(escaped),
                    Err((err_index, err_end, message)) => {
                        self.error_at(err_index, err_end, &message);
                        return;
                    }
                }
            } else {
                result.push(self.advance());
            }
        }

        let token = Token::new_string(start, self.index, &result, self.allocator);
        self.tokens.push(token);
    }

    /// Scans a template string, handling ${...} interpolations.
    /// This method handles the full template including embedded expressions.
    fn scan_template_string(&mut self, start: u32) {
        let mut result = String::new();
        let part_start = start;

        loop {
            let ch = self.peek();
            if ch == chars::EOF {
                // Error at the end of input where we expected more
                let error_pos = self.index;
                let message = format!(
                    "Lexer Error: Unterminated template literal at column {} in expression [{}]",
                    error_pos, self.input
                );
                self.error_at(error_pos, error_pos, &message);
                return;
            }
            if ch == chars::BT {
                self.advance();
                // No substitutions - complete template
                let token = Token::new_no_substitution_template(
                    part_start,
                    self.index,
                    &result,
                    self.allocator,
                );
                self.tokens.push(token);
                return;
            }
            if ch == chars::DOLLAR && self.peek_at(1) == chars::LBRACE {
                // Start of substitution - create TemplateHead ending at current position
                let head_end = self.index;
                let token = Token::new_template_head(part_start, head_end, &result, self.allocator);
                self.tokens.push(token);

                // Emit the ${ as an operator token
                let interp_start = self.index;
                self.advance(); // $
                self.advance(); // {
                let interp_token =
                    Token::new_operator(interp_start, self.index, "${", self.allocator);
                self.tokens.push(interp_token);

                // Scan the expression tokens inside ${...}
                self.scan_template_expression();

                // Only continue scanning if we haven't hit EOF or an error
                // (An error token would have been created inside if EOF was reached)
                if !self.is_eof() {
                    // Continue scanning the rest of the template
                    self.scan_template_continuation_internal();
                }
                return;
            }
            if ch == chars::BACKSLASH {
                self.advance();
                let escaped = self.scan_escape_sequence();
                result.push(escaped);
            } else {
                result.push(self.advance());
            }
        }
    }

    /// Scans tokens inside a template expression ${...}.
    /// Stops when it sees a closing } at the right nesting level.
    fn scan_template_expression(&mut self) {
        let mut brace_depth = 1; // We already consumed the opening {

        while !self.is_eof() && brace_depth > 0 {
            self.skip_whitespace();
            if self.is_eof() {
                break;
            }

            let start = self.index;
            let ch = self.peek();

            // Track brace nesting
            if ch == chars::LBRACE {
                brace_depth += 1;
                self.advance();
                self.scan_character(start, ch);
                continue;
            }
            if ch == chars::RBRACE {
                brace_depth -= 1;
                if brace_depth == 0 {
                    // End of template expression - emit the closing } as a Character token
                    self.advance();
                    self.scan_character(start, ch);
                    return;
                }
                self.advance();
                self.scan_character(start, ch);
                continue;
            }

            // Scan regular token
            self.scan_token_at(start, ch);
        }
    }

    /// Internal helper for scanning template continuation after an expression.
    fn scan_template_continuation_internal(&mut self) {
        let mut result = String::new();
        let part_start = self.index;

        loop {
            let ch = self.peek();
            if ch == chars::EOF {
                // Error at the end of input where we expected more
                let error_pos = self.index;
                let message = format!(
                    "Lexer Error: Unterminated template literal at column {} in expression [{}]",
                    error_pos, self.input
                );
                self.error_at(error_pos, error_pos, &message);
                return;
            }
            if ch == chars::BT {
                self.advance();
                // End of template - create TemplateTail
                let token =
                    Token::new_template_tail(part_start, self.index, &result, self.allocator);
                self.tokens.push(token);
                return;
            }
            if ch == chars::DOLLAR && self.peek_at(1) == chars::LBRACE {
                // Another substitution - create TemplateMiddle ending at current position
                let middle_end = self.index;
                let token =
                    Token::new_template_middle(part_start, middle_end, &result, self.allocator);
                self.tokens.push(token);

                // Emit the ${ as an operator token
                let interp_start = self.index;
                self.advance(); // $
                self.advance(); // {
                let interp_token =
                    Token::new_operator(interp_start, self.index, "${", self.allocator);
                self.tokens.push(interp_token);

                // Scan the expression tokens
                self.scan_template_expression();

                // Continue recursively
                self.scan_template_continuation_internal();
                return;
            }
            if ch == chars::BACKSLASH {
                self.advance();
                let escaped = self.scan_escape_sequence();
                result.push(escaped);
            } else {
                result.push(self.advance());
            }
        }
    }

    /// Scans a single token at the current position.
    /// This is used by scan_template_expression to tokenize embedded expressions.
    fn scan_token_at(&mut self, start: u32, ch: char) {
        // Advance past the first character (like scan_token does)
        self.advance();

        // Identifier or keyword
        if chars::is_identifier_start(ch) {
            self.scan_identifier(start);
            return;
        }

        // Number
        if chars::is_digit(ch) {
            self.scan_number(start);
            return;
        }

        match ch {
            // String literals
            chars::SQ | chars::DQ => {
                self.scan_string(start, ch);
            }
            // Nested template literal - recursively scan it
            chars::BT => {
                self.scan_template_string(start);
            }
            // Operators and punctuation - reuse existing scanning logic
            chars::PLUS => {
                if self.peek() == chars::PLUS {
                    self.error(start, "++ is not allowed");
                } else {
                    self.scan_operator(start, "+");
                }
            }
            chars::MINUS => {
                if self.peek() == chars::MINUS {
                    self.error(start, "-- is not allowed");
                } else {
                    self.scan_operator(start, "-");
                }
            }
            chars::STAR => {
                if self.peek() == chars::STAR {
                    self.advance();
                    self.scan_operator(start, "**");
                } else {
                    self.scan_operator(start, "*");
                }
            }
            chars::SLASH => self.scan_operator(start, "/"),
            chars::PERCENT => self.scan_operator(start, "%"),
            chars::CARET => self.scan_operator(start, "^"),
            chars::QUESTION => {
                if self.peek() == chars::QUESTION {
                    self.advance();
                    self.scan_operator(start, "??");
                } else if self.peek() == chars::PERIOD {
                    self.advance();
                    self.scan_operator(start, "?.");
                } else {
                    self.scan_character(start, ch);
                }
            }
            chars::LT => {
                if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, "<=");
                } else {
                    self.scan_operator(start, "<");
                }
            }
            chars::GT => {
                if self.peek() == chars::EQ {
                    self.advance();
                    self.scan_operator(start, ">=");
                } else {
                    self.scan_operator(start, ">");
                }
            }
            chars::BANG => {
                if self.peek() == chars::EQ {
                    self.advance();
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "!==");
                    } else {
                        self.scan_operator(start, "!=");
                    }
                } else {
                    self.scan_operator(start, "!");
                }
            }
            chars::EQ => {
                if self.peek() == chars::EQ {
                    self.advance();
                    if self.peek() == chars::EQ {
                        self.advance();
                        self.scan_operator(start, "===");
                    } else {
                        self.scan_operator(start, "==");
                    }
                } else {
                    self.scan_character(start, ch);
                }
            }
            chars::AMPERSAND => {
                if self.peek() == chars::AMPERSAND {
                    self.advance();
                    self.scan_operator(start, "&&");
                } else {
                    self.scan_operator(start, "&");
                }
            }
            chars::BAR => {
                if self.peek() == chars::BAR {
                    self.advance();
                    self.scan_operator(start, "||");
                } else {
                    self.scan_operator(start, "|");
                }
            }
            // Single characters
            _ => {
                self.scan_character(start, ch);
            }
        }
    }

    /// Scans an escape sequence and returns the escaped character.
    /// Used for template literals where we don't need error recovery.
    fn scan_escape_sequence(&mut self) -> char {
        // For template literals, we don't do error recovery - just return the best effort
        match self.scan_escape_sequence_with_error(self.index - 1) {
            Ok(ch) => ch,
            Err(_) => '\u{FFFD}',
        }
    }

    /// Scans an escape sequence with proper error handling.
    /// Returns Ok(char) on success, or Err((index, end, message)) on error.
    fn scan_escape_sequence_with_error(
        &mut self,
        escape_start: u32,
    ) -> Result<char, (u32, u32, String)> {
        let ch = self.advance();
        match ch {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'b' => Ok('\u{0008}'),
            'f' => Ok('\u{000C}'),
            'v' => Ok('\u{000B}'),
            '0' => Ok('\0'),
            'x' => self.scan_hex_escape_with_error(2, escape_start),
            'u' => {
                if self.peek() == chars::LBRACE {
                    self.advance();
                    let result = self.scan_unicode_escape_with_error(escape_start);
                    if self.peek() == chars::RBRACE {
                        self.advance();
                    }
                    result
                } else {
                    self.scan_hex_escape_with_error(4, escape_start)
                }
            }
            _ => Ok(ch),
        }
    }

    /// Scans a hex escape sequence of the given length with error handling.
    fn scan_hex_escape_with_error(
        &mut self,
        len: u32,
        escape_start: u32,
    ) -> Result<char, (u32, u32, String)> {
        let mut value = 0u32;
        let mut count = 0u32;

        for _ in 0..len {
            let ch = self.peek();
            if let Some(digit) = ch.to_digit(16) {
                value = value * 16 + digit;
                self.advance();
                count += 1;
            } else {
                break;
            }
        }

        if count < len {
            // Build the invalid escape sequence for the error message
            // Read ahead to find what the invalid sequence looks like
            let mut invalid_seq = String::new();
            invalid_seq.push_str(&self.input[escape_start as usize..self.index as usize]);
            // Continue reading non-hex chars to show in the error (like Angular does)
            let mut ahead = 0u32;
            while ahead < (len - count) && !self.is_eof() {
                let next = self.peek_at(ahead);
                if next == chars::EOF {
                    break;
                }
                invalid_seq.push(next);
                ahead += 1;
            }

            let error_index = escape_start + 1; // Position of the 'u' in \u
            let message = format!(
                "Lexer Error: Invalid unicode escape [{}] at column {} in expression [{}]",
                invalid_seq, error_index, self.input
            );
            return Err((error_index, error_index, message));
        }

        Ok(char::from_u32(value).unwrap_or('\u{FFFD}'))
    }

    /// Scans a unicode escape sequence (for \u{...} syntax).
    fn scan_unicode_escape_with_error(
        &mut self,
        escape_start: u32,
    ) -> Result<char, (u32, u32, String)> {
        let mut value = 0u32;
        let mut count = 0;

        while chars::is_ascii_hex_digit(self.peek()) {
            let ch = self.advance();
            if let Some(digit) = ch.to_digit(16) {
                value = value * 16 + digit;
                count += 1;
            }
        }

        if count == 0 {
            let error_index = escape_start + 1;
            let invalid_seq = &self.input[escape_start as usize..self.index as usize];
            let message = format!(
                "Lexer Error: Invalid unicode escape [{}] at column {} in expression [{}]",
                invalid_seq, error_index, self.input
            );
            return Err((error_index, error_index, message));
        }

        Ok(char::from_u32(value).unwrap_or('\u{FFFD}'))
    }

    /// Records an error with the current position as the end.
    fn error(&mut self, start: u32, message: &str) {
        let token = Token::new_error(start, self.index, message, self.allocator);
        self.tokens.push(token);
    }

    /// Records an error at specific index and end positions.
    fn error_at(&mut self, index: u32, end: u32, message: &str) {
        let token = Token::new_error(index, end, message, self.allocator);
        self.tokens.push(token);
    }

    /// Scans a regex literal starting from the current position.
    /// Assumes the opening `/` has already been consumed.
    ///
    /// Returns true if a valid regex was found, false otherwise.
    pub fn scan_regex(&mut self, start: u32) -> bool {
        let mut in_class = false;
        let mut pattern = String::new();

        // Scan the regex body
        loop {
            let ch = self.peek();

            if ch == chars::EOF || ch == '\n' || ch == '\r' {
                // Error at the end of input where we expected more
                let error_pos = self.index;
                let message = format!(
                    "Lexer Error: Unterminated regular expression at column {} in expression [{}]",
                    error_pos, self.input
                );
                self.error_at(error_pos, error_pos, &message);
                return false;
            }

            if in_class {
                // Inside character class [...]
                if ch == ']' {
                    in_class = false;
                    pattern.push(self.advance());
                } else if ch == chars::BACKSLASH {
                    pattern.push(self.advance());
                    if !self.is_eof() {
                        pattern.push(self.advance());
                    }
                } else {
                    pattern.push(self.advance());
                }
            } else {
                // Outside character class
                if ch == '/' {
                    self.advance(); // consume closing /
                    break;
                } else if ch == '[' {
                    in_class = true;
                    pattern.push(self.advance());
                } else if ch == chars::BACKSLASH {
                    pattern.push(self.advance());
                    if !self.is_eof() {
                        pattern.push(self.advance());
                    }
                } else {
                    pattern.push(self.advance());
                }
            }
        }

        // The body ends at the closing / (which we already advanced past)
        let body_end = self.index;

        // Create the body token (just the pattern, not including slashes)
        let body_token = Token::new_regexp_body(start, body_end, &pattern, self.allocator);
        self.tokens.push(body_token);

        // Scan flags
        let flags_start = self.index;
        while self.is_regex_flag(self.peek()) {
            self.advance();
        }

        // Only create flags token if there are flags
        if self.index > flags_start {
            let flags = &self.input[flags_start as usize..self.index as usize];
            let flags_token =
                Token::new_regexp_flags(flags_start, self.index, flags, self.allocator);
            self.tokens.push(flags_token);

            // Validate flags (this reports errors but doesn't prevent token creation)
            self.validate_regex_flags(flags, flags_start);
        }

        true
    }

    /// Returns true if the character could be a regex flag (any ASCII letter).
    /// Validation of actual flag values is done in validate_regex_flags.
    fn is_regex_flag(&self, ch: char) -> bool {
        ch.is_ascii_alphabetic()
    }

    /// Validates regex flags and reports errors for duplicates or invalid combinations.
    fn validate_regex_flags(&mut self, flags: &str, start: u32) -> bool {
        let mut seen = [false; 8];

        for ch in flags.chars() {
            let idx = match ch {
                'd' => 0,
                'g' => 1,
                'i' => 2,
                'm' => 3,
                's' => 4,
                'u' => 5,
                'v' => 6,
                'y' => 7,
                _ => {
                    self.error(start, &format!("Unsupported regular expression flag \"{ch}\""));
                    return false;
                }
            };

            if seen[idx] {
                self.error(start, &format!("Duplicate regular expression flag \"{ch}\""));
                return false;
            }
            seen[idx] = true;
        }

        // Check for invalid flag combinations
        // 'u' and 'v' are mutually exclusive
        if seen[5] && seen[6] {
            self.error(start, "Regular expression flags 'u' and 'v' cannot be used together");
            return false;
        }

        true
    }

    /// Checks if the last token suggests that `/` should be interpreted as regex start.
    /// This is used to disambiguate between division and regex.
    pub fn last_token_allows_regex(&self) -> bool {
        if let Some(last) = self.tokens.last() {
            match last.token_type {
                // After these operators, `/` is more likely to be regex
                TokenType::Operator => {
                    let op = last.str_value.as_str();
                    // `!` can be non-null assertion (postfix) or negation (prefix)
                    // Check the token before `!` to determine:
                    // - If preceded by value-like token (identifier, ), ], keyword), it's non-null assertion
                    // - Otherwise, it's negation and `/` is regex
                    if op == "!" {
                        if self.tokens.len() >= 2 {
                            let before_bang = &self.tokens[self.tokens.len() - 2];
                            return match before_bang.token_type {
                                TokenType::Identifier
                                | TokenType::PrivateIdentifier
                                | TokenType::Number
                                | TokenType::String => false, // non-null assertion, division
                                TokenType::Character => {
                                    let ch = before_bang.num_value as u32;
                                    // After ) or ], `!` is non-null assertion
                                    if matches!(char::from_u32(ch), Some(')' | ']')) {
                                        false // non-null assertion, division
                                    } else {
                                        true // negation, regex
                                    }
                                }
                                TokenType::Keyword => false, // non-null assertion
                                _ => true,                   // negation, regex
                            };
                        }
                        // `!` at start of expression is negation
                        return true;
                    }
                    // After other operators like =, +, -, *, etc., `/` is regex
                    true
                }
                TokenType::Character => {
                    // After (, [, {, ,, :, ;, etc.
                    let ch = last.num_value as u32;
                    matches!(char::from_u32(ch), Some('(' | '[' | '{' | ',' | ':' | ';'))
                }
                TokenType::Keyword => {
                    // After keywords like return, typeof, void, in, etc.
                    let kw = last.str_value.as_str();
                    matches!(kw, "return" | "typeof" | "void" | "in" | "else" | "if" | "case")
                }
                _ => false,
            }
        } else {
            // At start of expression, `/` is regex
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identifier() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "foo");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_identifier());
        assert_eq!(tokens[0].str_value.as_str(), "foo");
    }

    #[test]
    fn test_keywords() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "true false null undefined");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 4);
        assert!(tokens[0].is_keyword_value("true"));
        assert!(tokens[1].is_keyword_value("false"));
        assert!(tokens[2].is_keyword_value("null"));
        assert!(tokens[3].is_keyword_value("undefined"));
    }

    #[test]
    fn test_numbers() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "42 3.14 0xff");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 3);
        assert!(tokens[0].is_number());
        assert_eq!(tokens[0].num_value, 42.0);
        assert!(tokens[1].is_number());
        assert_eq!(tokens[1].num_value, 3.14);
        assert!(tokens[2].is_number());
        assert_eq!(tokens[2].num_value, 255.0);
    }

    #[test]
    fn test_strings() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "'hello' \"world\"");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 2);
        assert!(tokens[0].is_string());
        assert_eq!(tokens[0].str_value.as_str(), "hello");
        assert!(tokens[1].is_string());
        assert_eq!(tokens[1].str_value.as_str(), "world");
    }

    #[test]
    fn test_operators() {
        let allocator = Allocator::default();
        // Use % instead of / to avoid triggering regex context after *
        let lexer = Lexer::new(&allocator, "+ - * % === !== && || ??");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 9);
        assert!(tokens[0].is_operator("+"));
        assert!(tokens[1].is_operator("-"));
        assert!(tokens[2].is_operator("*"));
        assert!(tokens[3].is_operator("%"));
        assert!(tokens[4].is_operator("==="));
        assert!(tokens[5].is_operator("!=="));
        assert!(tokens[6].is_operator("&&"));
        assert!(tokens[7].is_operator("||"));
        assert!(tokens[8].is_operator("??"));
    }

    #[test]
    fn test_safe_navigation() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "a?.b");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 3);
        assert!(tokens[0].is_identifier());
        assert!(tokens[1].is_operator("?."));
        assert!(tokens[2].is_identifier());
    }

    #[test]
    fn test_regex_simple() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "/pattern/");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_regexp_body());
        assert_eq!(tokens[0].str_value.as_str(), "pattern");
    }

    #[test]
    fn test_regex_with_flags() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "/pattern/gim");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 2);
        assert!(tokens[0].is_regexp_body());
        assert_eq!(tokens[0].str_value.as_str(), "pattern");
        assert!(tokens[1].is_regexp_flags());
        assert_eq!(tokens[1].str_value.as_str(), "gim");
    }

    #[test]
    fn test_regex_with_escape() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, r"/\d+/");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_regexp_body());
        assert_eq!(tokens[0].str_value.as_str(), r"\d+");
    }

    #[test]
    fn test_regex_with_character_class() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "/[a-z]/");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_regexp_body());
        assert_eq!(tokens[0].str_value.as_str(), "[a-z]");
    }

    #[test]
    fn test_regex_with_slash_in_class() {
        // A slash inside a character class should not end the regex
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "/[/]/");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_regexp_body());
        assert_eq!(tokens[0].str_value.as_str(), "[/]");
    }

    #[test]
    fn test_division_after_identifier() {
        // After an identifier, / should be division, not regex
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "a / b");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 3);
        assert!(tokens[0].is_identifier());
        assert!(tokens[1].is_operator("/"));
        assert!(tokens[2].is_identifier());
    }

    #[test]
    fn test_regex_after_operator() {
        // After an operator, / should be regex
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "a = /pattern/g");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 4);
        assert!(tokens[0].is_identifier());
        assert!(tokens[1].is_operator("="));
        assert!(tokens[2].is_regexp_body());
        assert_eq!(tokens[2].str_value.as_str(), "pattern");
        assert!(tokens[3].is_regexp_flags());
        assert_eq!(tokens[3].str_value.as_str(), "g");
    }

    #[test]
    fn test_regex_after_parenthesis() {
        // After (, / should be regex
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "(/abc/)");
        let tokens = lexer.tokenize();
        assert_eq!(tokens.len(), 3);
        assert!(tokens[0].is_character('('));
        assert!(tokens[1].is_regexp_body());
        assert_eq!(tokens[1].str_value.as_str(), "abc");
        assert!(tokens[2].is_character(')'));
    }

    #[test]
    fn test_template_literal_with_pipe() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "`hello ${name | capitalize}!!!`");
        let tokens = lexer.tokenize();
        // Expected tokens: TemplateHead, ${, Identifier, |, Identifier, }, TemplateTail
        assert_eq!(tokens.len(), 7);
        assert!(tokens[0].is_template_head());
        assert!(tokens[1].is_operator("${"));
        assert!(tokens[2].is_identifier());
        assert!(tokens[3].is_operator("|"));
        assert!(tokens[4].is_identifier());
        assert!(tokens[5].is_character('}'));
        assert!(tokens[6].is_template_tail());
    }

    #[test]
    fn test_simple_pipe() {
        let allocator = Allocator::default();
        let lexer = Lexer::new(&allocator, "a | b");
        let tokens = lexer.tokenize();
        // Debug: print all tokens
        for (i, token) in tokens.iter().enumerate() {
            eprintln!("{}: {:?} '{}'", i, token.token_type, token.str_value);
        }
        assert_eq!(tokens.len(), 3);
        assert!(tokens[0].is_identifier());
        assert!(
            tokens[1].is_operator("|"),
            "Expected '|' operator, got {:?}",
            tokens[1].token_type
        );
        assert!(tokens[2].is_identifier());
    }

    #[test]
    fn test_nested_template_in_object() {
        let allocator = Allocator::default();
        // Corrected input: the interpolation contains an object literal {"b": `hello`}
        let input = r#"{"a": `hello ${{"b": `hello`}}`}"#;
        let lexer = Lexer::new(&allocator, input);
        let tokens = lexer.tokenize();
        // Debug: print all tokens
        for (i, token) in tokens.iter().enumerate() {
            eprintln!("{}: {:?} '{}'", i, token.token_type, token.str_value);
        }
        // Expected: {, "a", :, TemplateHead, ${, {, "b", :, NoSubstitutionTemplate, }, }, TemplateTail, }
        assert_eq!(tokens.len(), 13);
        assert!(tokens[0].is_character('{'));
        assert!(tokens[1].is_string());
        assert!(tokens[2].is_character(':'));
        assert!(tokens[3].is_template_head());
        assert!(tokens[4].is_operator("${"));
        assert!(tokens[5].is_character('{'));
        assert!(tokens[6].is_string());
        assert!(tokens[7].is_character(':'));
        assert!(tokens[8].is_no_substitution_template());
        assert!(tokens[9].is_character('}'));
        assert!(tokens[10].is_character('}'));
        assert!(tokens[11].is_template_tail());
        assert!(tokens[12].is_character('}'));
    }
}

//! HTML template lexer.
//!
//! Tokenizes Angular HTML templates including control flow blocks.
//!
//! Ported from Angular's `ml_parser/lexer.ts`.

use super::entities::{decode_entity, get_named_entities};
use super::tags::{TagContentType, get_html_tag_definition};
use crate::util::chars;

/// Supported block keywords for Angular control flow.
/// Matches Angular's SUPPORTED_BLOCKS array from lexer.ts.
const SUPPORTED_BLOCKS: &[&str] = &[
    "if",
    "else", // Covers `@else if` as well
    "for",
    "switch",
    "case",
    "default",
    "empty",
    "defer",
    "placeholder",
    "loading",
    "error",
];

/// Token types for HTML templates.
/// Matches Angular's `TokenType` enum from `ml_parser/tokens.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlTokenType {
    /// Tag open start: `<tag` - parts: [prefix, name]
    TagOpenStart,
    /// Tag open end: `>`
    TagOpenEnd,
    /// Tag open end void (self-closing): `/>`
    TagOpenEndVoid,
    /// Tag close: `</tag>` - parts: [prefix, name]
    TagClose,
    /// Incomplete tag open (terminated early)
    IncompleteTagOpen,
    /// Text content
    Text,
    /// Escapable raw text (inside script/style)
    EscapableRawText,
    /// Raw text
    RawText,
    /// Interpolation: `{{expr}}` - parts: [startMarker, expr, endMarker]
    Interpolation,
    /// Encoded entity: `&amp;` - parts: [decoded, encoded]
    EncodedEntity,
    /// Comment start: `<!--`
    CommentStart,
    /// Comment end: `-->`
    CommentEnd,
    /// CDATA start: `<![CDATA[`
    CdataStart,
    /// CDATA end: `]]>`
    CdataEnd,
    /// Attribute name: parts: [prefix, name]
    AttrName,
    /// Attribute quote: parts: [quote char]
    AttrQuote,
    /// Attribute value text: parts: [value]
    AttrValueText,
    /// Attribute value interpolation
    AttrValueInterpolation,
    /// DOCTYPE
    DocType,
    /// Expansion form start: `{`
    ExpansionFormStart,
    /// Expansion case value
    ExpansionCaseValue,
    /// Expansion case expression start: `{`
    ExpansionCaseExpStart,
    /// Expansion case expression end: `}`
    ExpansionCaseExpEnd,
    /// Expansion form end: `}`
    ExpansionFormEnd,
    /// Block open start: `@if` - parts: [name]
    BlockOpenStart,
    /// Block open end: `{`
    BlockOpenEnd,
    /// Block close: `}`
    BlockClose,
    /// Block parameter: parts: [expression]
    BlockParameter,
    /// Incomplete block open
    IncompleteBlockOpen,
    /// Let start: `@let` - parts: [name]
    LetStart,
    /// Let value: parts: [value]
    LetValue,
    /// Let end: `;`
    LetEnd,
    /// Incomplete let
    IncompleteLet,
    /// Component open start (selectorless)
    ComponentOpenStart,
    /// Component open end
    ComponentOpenEnd,
    /// Component open end void
    ComponentOpenEndVoid,
    /// Component close
    ComponentClose,
    /// Incomplete component open
    IncompleteComponentOpen,
    /// Directive name
    DirectiveName,
    /// Directive open
    DirectiveOpen,
    /// Directive close
    DirectiveClose,
    /// End of file
    Eof,
}

/// A token in an HTML template.
#[derive(Debug, Clone)]
pub struct HtmlToken {
    /// The token type.
    pub token_type: HtmlTokenType,
    /// The token parts (for composite tokens).
    pub parts: Vec<String>,
    /// The start offset (after leading trivia).
    pub start: u32,
    /// The end offset.
    pub end: u32,
    /// The full start offset (before leading trivia, for source maps).
    /// If None, same as start (no trivia skipped).
    pub full_start: Option<u32>,
}

impl HtmlToken {
    /// Creates a new token with multiple parts.
    pub fn new(token_type: HtmlTokenType, parts: Vec<String>, start: u32, end: u32) -> Self {
        Self { token_type, parts, start, end, full_start: None }
    }

    /// Creates a new token with full_start tracking.
    pub fn new_with_full_start(
        token_type: HtmlTokenType,
        parts: Vec<String>,
        start: u32,
        end: u32,
        full_start: Option<u32>,
    ) -> Self {
        Self { token_type, parts, start, end, full_start }
    }

    /// Creates a token with no parts.
    pub fn empty(token_type: HtmlTokenType, start: u32, end: u32) -> Self {
        Self { token_type, parts: vec![], start, end, full_start: None }
    }

    /// Creates a token with no parts and full_start tracking.
    pub fn empty_with_full_start(
        token_type: HtmlTokenType,
        start: u32,
        end: u32,
        full_start: Option<u32>,
    ) -> Self {
        Self { token_type, parts: vec![], start, end, full_start }
    }

    /// Creates a token with one part.
    pub fn with_part(token_type: HtmlTokenType, part: &str, start: u32, end: u32) -> Self {
        Self { token_type, parts: vec![part.to_string()], start, end, full_start: None }
    }

    /// Creates a token with two parts (prefix, name).
    pub fn with_prefix_name(
        token_type: HtmlTokenType,
        prefix: &str,
        name: &str,
        start: u32,
        end: u32,
    ) -> Self {
        Self {
            token_type,
            parts: vec![prefix.to_string(), name.to_string()],
            start,
            end,
            full_start: None,
        }
    }

    /// Returns the first part (main value).
    pub fn value(&self) -> &str {
        self.parts.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// Returns the name part for tag tokens (second part, or first if no prefix).
    pub fn name(&self) -> &str {
        if self.parts.len() >= 2 { &self.parts[1] } else { self.value() }
    }

    /// Returns the prefix part for tag tokens (first part).
    pub fn prefix(&self) -> &str {
        if self.parts.len() >= 2 { &self.parts[0] } else { "" }
    }

    /// Returns the effective full_start (or start if not set).
    pub fn effective_full_start(&self) -> u32 {
        self.full_start.unwrap_or(self.start)
    }
}

/// Normalizes line endings in text content.
/// Per HTML5 spec, CRLF and standalone CR are converted to LF.
fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Converts a byte offset to line:column position.
fn offset_to_position(input: &str, offset: u32) -> (u32, u32) {
    let mut line: u32 = 0;
    let mut col: u32 = 0;
    let mut byte_pos = 0u32;

    for ch in input.chars() {
        if byte_pos >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else if ch == '\r' {
            // Standalone CR or CRLF
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
        byte_pos += ch.len_utf8() as u32;
    }
    (line, col)
}

/// Processes escape sequences in text when in "escaped strings" mode.
/// This is used when tokenize_expansion_forms is false (Angular's default for inline templates).
/// Returns (processed_text, errors, encountered_null).
/// When a null character (\0) is encountered, everything after it is discarded (EOF behavior).
fn process_escape_sequences(text: &str) -> (String, Vec<(String, usize)>, bool) {
    let mut result = String::with_capacity(text.len());
    let mut errors = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut encountered_null = false;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            match next {
                // Standard escape sequences
                '\'' | '"' | '`' | '\\' => {
                    result.push(next);
                    i += 2;
                }
                'n' => {
                    result.push('\n');
                    i += 2;
                }
                'r' => {
                    // \r gets normalized to \n
                    result.push('\n');
                    i += 2;
                }
                'v' => {
                    result.push('\x0B');
                    i += 2;
                }
                't' => {
                    result.push('\t');
                    i += 2;
                }
                'b' => {
                    result.push('\x08');
                    i += 2;
                }
                'f' => {
                    result.push('\x0C');
                    i += 2;
                }
                '0' => {
                    // \0 is null character - treat as EOF (everything after is discarded)
                    // But if followed by more octal digits (1-7), it might produce a non-null value
                    if i + 2 < chars.len() && ('0'..='7').contains(&chars[i + 2]) {
                        // Could be octal like \001, \007, \077
                        let mut octal = String::new();
                        let mut j = i + 1; // Start from the first '0'
                        while j < chars.len() && octal.len() < 3 && ('0'..='7').contains(&chars[j])
                        {
                            octal.push(chars[j]);
                            j += 1;
                        }
                        if let Ok(code) = u8::from_str_radix(&octal, 8) {
                            if code > 0 {
                                result.push(code as char);
                            } else {
                                // Result is null - treat as EOF
                                encountered_null = true;
                                break;
                            }
                        }
                        i = j;
                    } else if i + 2 < chars.len() && chars[i + 2].is_ascii_digit() {
                        // \0 followed by 8 or 9 - \0 is null (EOF), stop processing
                        encountered_null = true;
                        break;
                    } else {
                        // Just \0 - null character, treat as EOF
                        encountered_null = true;
                        break;
                    }
                }
                '1'..='7' => {
                    // Octal escape sequence
                    let mut octal = String::new();
                    let mut j = i + 1;
                    while j < chars.len() && octal.len() < 3 && ('0'..='7').contains(&chars[j]) {
                        octal.push(chars[j]);
                        j += 1;
                    }
                    if let Ok(code) = u8::from_str_radix(&octal, 8) {
                        if code > 0 {
                            result.push(code as char);
                        } else {
                            // Null character - treat as EOF
                            encountered_null = true;
                            break;
                        }
                    }
                    i = j;
                }
                '8' | '9' => {
                    // Invalid octal - treat as literal
                    result.push(next);
                    i += 2;
                }
                'x' => {
                    // Hex escape sequence: \xNN
                    if i + 3 < chars.len()
                        && chars[i + 2].is_ascii_hexdigit()
                        && chars[i + 3].is_ascii_hexdigit()
                    {
                        let hex: String = chars[i + 2..i + 4].iter().collect();
                        if let Ok(code) = u8::from_str_radix(&hex, 16) {
                            if code > 0 {
                                result.push(code as char);
                            } else {
                                // Null character - treat as EOF
                                encountered_null = true;
                                break;
                            }
                        }
                        i += 4;
                    } else if i + 2 >= chars.len() {
                        // Hit EOF after \x
                        errors.push(("Unexpected character \"EOF\"".to_string(), i + 2));
                        result.push('\\');
                        result.push('x');
                        i += 2;
                    } else {
                        // Invalid hex characters after \x
                        errors.push(("Invalid hexadecimal escape sequence".to_string(), i + 2));
                        result.push('\\');
                        result.push('x');
                        i += 2;
                    }
                }
                'u' => {
                    // Unicode escape sequence: \uNNNN or \u{N...}
                    if i + 2 < chars.len() && chars[i + 2] == '{' {
                        // Variable length Unicode: \u{N...}
                        let mut j = i + 3;
                        let mut hex = String::new();
                        while j < chars.len() && chars[j] != '}' && chars[j].is_ascii_hexdigit() {
                            hex.push(chars[j]);
                            j += 1;
                        }
                        if j < chars.len() && chars[j] == '}' && !hex.is_empty() {
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if code == 0 {
                                    // Null character - treat as EOF
                                    encountered_null = true;
                                    break;
                                }
                                if let Some(ch) = char::from_u32(code) {
                                    result.push(ch);
                                }
                            }
                            i = j + 1;
                        } else if j >= chars.len() {
                            // Hit EOF
                            errors.push(("Unexpected character \"EOF\"".to_string(), i + 3));
                            result.push('\\');
                            result.push('u');
                            i += 2;
                        } else {
                            // Invalid characters (like \u{GG})
                            errors.push(("Invalid hexadecimal escape sequence".to_string(), i + 3));
                            result.push('\\');
                            result.push('u');
                            i += 2;
                        }
                    } else if i + 5 < chars.len() {
                        // Fixed length Unicode: \uNNNN
                        let valid = chars[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit());
                        if valid {
                            let hex: String = chars[i + 2..i + 6].iter().collect();
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if code == 0 {
                                    // Null character - treat as EOF
                                    encountered_null = true;
                                    break;
                                }
                                if let Some(ch) = char::from_u32(code) {
                                    result.push(ch);
                                }
                            }
                            i += 6;
                        } else {
                            // Invalid characters (like \uGGGG)
                            errors.push(("Invalid hexadecimal escape sequence".to_string(), i + 2));
                            result.push('\\');
                            result.push('u');
                            i += 2;
                        }
                    } else {
                        // Incomplete sequence (hit EOF)
                        errors.push(("Unexpected character \"EOF\"".to_string(), i + 2));
                        result.push('\\');
                        result.push('u');
                        i += 2;
                    }
                }
                '\n' => {
                    // Line continuation - skip both backslash and newline
                    i += 2;
                }
                '\r' => {
                    // Line continuation with CR or CRLF
                    i += 2;
                    if i < chars.len() && chars[i] == '\n' {
                        i += 1;
                    }
                }
                _ => {
                    // Unknown escape - just use the character after backslash
                    result.push(next);
                    i += 2;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    (result, errors, encountered_null)
}

/// Merges adjacent text tokens and adjacent attribute value text tokens.
/// This matches Angular's `mergeTextTokens()` function.
fn merge_text_tokens(src_tokens: Vec<HtmlToken>) -> Vec<HtmlToken> {
    let mut dst_tokens: Vec<HtmlToken> = Vec::new();

    for token in src_tokens {
        let should_merge = match dst_tokens.last() {
            Some(last) => {
                (last.token_type == HtmlTokenType::Text && token.token_type == HtmlTokenType::Text)
                    || (last.token_type == HtmlTokenType::AttrValueText
                        && token.token_type == HtmlTokenType::AttrValueText)
            }
            None => false,
        };

        if should_merge {
            // Merge with the last token
            // Safety: should_merge is only true if dst_tokens.last() returned Some
            if let Some(last) = dst_tokens.last_mut() {
                if let Some(last_part) = last.parts.first_mut() {
                    if let Some(token_part) = token.parts.first() {
                        last_part.push_str(token_part);
                    }
                }
                last.end = token.end;
            }
        } else {
            dst_tokens.push(token);
        }
    }

    dst_tokens
}

/// Result of tokenizing HTML.
pub struct HtmlTokenizeResult {
    /// The tokens.
    pub tokens: Vec<HtmlToken>,
    /// Any errors.
    pub errors: Vec<HtmlTokenError>,
}

/// A tokenization error.
#[derive(Debug, Clone)]
pub struct HtmlTokenError {
    /// The error message.
    pub msg: String,
    /// The position (line, column) where the error occurred.
    pub position: (u32, u32),
}

/// HTML template lexer.
pub struct HtmlLexer<'a> {
    /// The source text.
    input: &'a str,
    /// The input length (in bytes).
    length: u32,
    /// The current position.
    index: u32,
    /// Current line number (0-based).
    line: u32,
    /// Current column number (0-based).
    column: u32,
    /// The generated tokens.
    tokens: Vec<HtmlToken>,
    /// Errors.
    errors: Vec<HtmlTokenError>,
    /// Interpolation config.
    interpolation_start: &'a str,
    /// Interpolation config.
    interpolation_end: &'a str,
    /// Block nesting depth.
    block_depth: u32,
    /// Enable selectorless components/directives.
    selectorless_enabled: bool,
    /// Enable ICU/expansion form tokenization (for i18n plural/select).
    tokenize_icu: bool,
    /// Stack to track expansion form/case nesting.
    /// Values are HtmlTokenType::ExpansionFormStart or HtmlTokenType::ExpansionCaseExpStart.
    expansion_case_stack: Vec<HtmlTokenType>,
    /// Enable escape sequence processing (for inline template strings).
    escaped_string: bool,
    /// Enable block tokenization (default: true).
    /// When enabled, standalone `}` characters become BLOCK_CLOSE tokens.
    tokenize_blocks: bool,
    /// Enable @let tokenization (default: true).
    /// When disabled, @let is treated as text or incomplete block.
    tokenize_let: bool,
    /// Characters to consider as leading trivia (for source map optimization).
    leading_trivia_chars: Option<Vec<char>>,
    /// Range start position (for processing a sub-range of input).
    range_start_pos: u32,
    /// Range end position.
    range_end_pos: u32,
    /// Line offset for range mode.
    range_line_offset: u32,
    /// Column offset for range mode.
    range_col_offset: u32,
}

impl<'a> HtmlLexer<'a> {
    /// Creates a new HTML lexer.
    pub fn new(input: &'a str) -> Self {
        let length = input.len() as u32;
        Self {
            input,
            length,
            index: 0,
            line: 0,
            column: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
            interpolation_start: "{{",
            interpolation_end: "}}",
            block_depth: 0,
            selectorless_enabled: false,
            tokenize_icu: false,
            expansion_case_stack: Vec::new(),
            escaped_string: false,
            tokenize_blocks: true, // default to true like Angular
            tokenize_let: true,    // default to true like Angular
            leading_trivia_chars: None,
            range_start_pos: 0,
            range_end_pos: length,
            range_line_offset: 0,
            range_col_offset: 0,
        }
    }

    /// Sets the interpolation delimiters.
    pub fn with_interpolation(mut self, start: &'a str, end: &'a str) -> Self {
        self.interpolation_start = start;
        self.interpolation_end = end;
        self
    }

    /// Enables selectorless components/directives.
    pub fn with_selectorless(mut self, enabled: bool) -> Self {
        self.selectorless_enabled = enabled;
        self
    }

    /// Enables ICU/expansion form tokenization (for i18n plural/select).
    pub fn with_expansion_forms(mut self, enabled: bool) -> Self {
        self.tokenize_icu = enabled;
        self
    }

    /// Enables escape sequence processing (for inline template strings).
    pub fn with_escaped_string(mut self, enabled: bool) -> Self {
        self.escaped_string = enabled;
        self
    }

    /// Enables or disables block tokenization.
    pub fn with_blocks(mut self, enabled: bool) -> Self {
        self.tokenize_blocks = enabled;
        self
    }

    /// Enables or disables @let tokenization.
    pub fn with_let(mut self, enabled: bool) -> Self {
        self.tokenize_let = enabled;
        self
    }

    /// Sets the leading trivia characters for source map optimization.
    pub fn with_leading_trivia_chars(mut self, chars: Vec<char>) -> Self {
        self.leading_trivia_chars = Some(chars);
        self
    }

    /// Sets the range of input to process.
    /// This allows tokenizing a sub-range of the input with correct line/column tracking.
    pub fn with_range(
        mut self,
        start_pos: u32,
        end_pos: u32,
        start_line: u32,
        start_col: u32,
    ) -> Self {
        self.range_start_pos = start_pos;
        self.range_end_pos = end_pos.min(self.length);
        self.range_line_offset = start_line;
        self.range_col_offset = start_col;
        // Set initial position to start of range
        self.index = start_pos;
        self.line = start_line;
        self.column = start_col;
        // Update length to range end
        self.length = self.range_end_pos;
        self
    }

    /// Calculates the start position after skipping leading trivia characters.
    /// Returns (adjusted_start, full_start) where:
    /// - adjusted_start: position after skipping trivia (used for actual token start)
    /// - full_start: original position before trivia (for source maps), None if same as adjusted
    fn calculate_start_with_trivia(&self, original_start: u32, end: u32) -> (u32, Option<u32>) {
        if let Some(ref trivia_chars) = self.leading_trivia_chars {
            let mut adjusted_start = original_start;
            // Advance past trivia characters from the token start
            for ch in self.input[original_start as usize..end as usize].chars() {
                if trivia_chars.contains(&ch) {
                    adjusted_start += ch.len_utf8() as u32;
                } else {
                    break;
                }
            }
            if adjusted_start != original_start {
                return (adjusted_start, Some(original_start));
            }
        }
        (original_start, None)
    }

    /// Checks if a character can start a selectorless name (uppercase letter or underscore).
    fn is_selectorless_name_start(ch: char) -> bool {
        ch == '_' || ch.is_ascii_uppercase()
    }

    /// Checks if a character can be part of a selectorless name.
    fn is_selectorless_name_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }

    /// Tokenizes the input.
    pub fn tokenize(mut self) -> HtmlTokenizeResult {
        while self.index < self.length {
            self.scan_token();
        }

        // If we hit EOF while still in an expansion form, report an unescaped `{` error
        // (only when ICU tokenization is enabled)
        if self.tokenize_icu && !self.expansion_case_stack.is_empty() {
            self.errors.push(HtmlTokenError {
                msg: "Unexpected character \"EOF\" (Do you have an unescaped \"{\" in your template? Use \"{{ '{' }}\") to escape it.)".to_string(),
                position: (self.line, self.column),
            });
        }

        self.tokens.push(HtmlToken::empty(HtmlTokenType::Eof, self.index, self.index));

        // Merge adjacent text tokens (Angular does this to combine empty text tokens with adjacent text)
        let mut tokens = merge_text_tokens(self.tokens);

        // Process escape sequences if enabled
        if self.escaped_string {
            let mut errors_to_add = Vec::new();
            let mut null_encountered = false;
            let mut null_token_idx = None;

            for (token_idx, token) in tokens.iter_mut().enumerate() {
                if null_encountered {
                    break;
                }

                // Process escape sequences in token parts
                for part in &mut token.parts {
                    let (processed, errs, has_null) = process_escape_sequences(part);
                    *part = processed;
                    for (msg, offset) in errs {
                        // Convert offset to line:column using token start
                        let error_pos = token.start + offset as u32;
                        let (line, col) = offset_to_position(self.input, error_pos);
                        errors_to_add.push(HtmlTokenError { msg, position: (line, col) });
                    }
                    if has_null {
                        null_encountered = true;
                        null_token_idx = Some(token_idx);
                        break;
                    }
                }
            }

            // If null was encountered, truncate tokens after that point and ensure EOF is present
            if let Some(idx) = null_token_idx {
                // Keep tokens up to and including the null token
                tokens.truncate(idx + 1);

                // If the last token has empty parts after null processing, and it's a TEXT token,
                // we might need to remove it
                if let Some(last) = tokens.last() {
                    if last.token_type == HtmlTokenType::Text
                        && last.parts.iter().all(|p| p.is_empty())
                    {
                        tokens.pop();
                    }
                }

                // Add EOF if not already present
                if tokens.last().is_none_or(|t| t.token_type != HtmlTokenType::Eof) {
                    let eof_start = tokens.last().map(|t| t.end).unwrap_or(0);
                    tokens.push(HtmlToken::empty(HtmlTokenType::Eof, eof_start, eof_start));
                }
            }

            self.errors.extend(errors_to_add);
        }

        HtmlTokenizeResult { tokens, errors: self.errors }
    }

    /// Peeks at the raw current character (no escape processing).
    fn raw_peek(&self) -> char {
        if self.index >= self.length {
            chars::EOF
        } else {
            self.input[self.index as usize..].chars().next().unwrap_or(chars::EOF)
        }
    }

    /// Peeks at the current character (handles escape sequences when escaped_string is true).
    fn peek(&self) -> char {
        let ch = self.raw_peek();
        if self.escaped_string && ch == '\\' {
            // Look ahead to see what character is being escaped
            let next = self.input[(self.index as usize + 1)..].chars().next().unwrap_or(chars::EOF);
            Self::unescape_char(next)
        } else {
            ch
        }
    }

    /// Returns the unescaped character for an escape sequence.
    fn unescape_char(ch: char) -> char {
        match ch {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\x0B', // vertical tab
            'b' => '\x08', // backspace
            'f' => '\x0C', // form feed
            // For quotes and backslash, just return the character itself
            '"' | '\'' | '\\' => ch,
            // For any other escaped character, return it as-is
            _ => ch,
        }
    }

    /// Peeks at a character at offset from current position.
    /// Note: offset is in characters, not bytes.
    /// When escaped_string is true, this accounts for escape sequences.
    fn peek_at(&self, offset: u32) -> char {
        if self.index >= self.length {
            return chars::EOF;
        }
        if self.escaped_string {
            // Need to iterate through escape sequences properly
            let mut pos = self.index as usize;
            let mut remaining = offset;
            while remaining > 0 && pos < self.length as usize {
                let ch = self.input[pos..].chars().next().unwrap_or(chars::EOF);
                if ch == '\\' {
                    // Skip past escape sequence
                    pos += 1;
                    if pos < self.length as usize {
                        let escaped = self.input[pos..].chars().next().unwrap_or(chars::EOF);
                        pos += escaped.len_utf8();
                    }
                } else {
                    pos += ch.len_utf8();
                }
                remaining -= 1;
            }
            if pos >= self.length as usize {
                chars::EOF
            } else {
                let ch = self.input[pos..].chars().next().unwrap_or(chars::EOF);
                if ch == '\\' {
                    let next = self.input[(pos + 1)..].chars().next().unwrap_or(chars::EOF);
                    Self::unescape_char(next)
                } else {
                    ch
                }
            }
        } else {
            self.input[self.index as usize..].chars().nth(offset as usize).unwrap_or(chars::EOF)
        }
    }

    /// Advances the index and returns the current character.
    fn advance(&mut self) -> char {
        if self.index >= self.length {
            return chars::EOF;
        }

        let raw_ch = self.raw_peek();

        // Handle escape sequences when escaped_string is true
        if self.escaped_string && raw_ch == '\\' {
            let next_pos = self.index as usize + 1;
            if next_pos >= self.length as usize {
                // Backslash at end of input - just advance past it
                self.index += 1;
                self.column += 1;
                return '\\';
            }

            let next = self.input[next_pos..].chars().next().unwrap_or(chars::EOF);

            // Check for line continuation (backslash followed by newline)
            if next == '\n' || next == '\r' {
                // Skip the backslash
                self.index += 1;
                self.column += 1;
                // Now handle the newline
                if next == '\r' {
                    let after_pos = self.index as usize + 1;
                    let after = if after_pos < self.length as usize {
                        self.input[after_pos..].chars().next()
                    } else {
                        None
                    };
                    if after == Some('\n') {
                        self.index += 2; // Skip CR and LF
                    } else {
                        self.index += 1;
                    }
                } else {
                    self.index += 1;
                }
                self.line += 1;
                self.column = 0;
                // Line continuation - return the next actual character (or EOF if at end)
                if self.index >= self.length {
                    return chars::EOF;
                }
                return self.advance();
            }

            // For unicode escapes (\uXXXX or \u{XXXX}) and hex (\xXX), we need to handle specially
            // For now, just skip the backslash and escaped char
            let unescaped = Self::unescape_char(next);

            // Skip backslash
            self.index += 1;
            self.column += 1;

            // Skip escaped character
            self.index += next.len_utf8() as u32;
            self.column += 1;

            return unescaped;
        }

        // Normal character handling
        if raw_ch == '\n' {
            self.line += 1;
            self.column = 0;
        } else if raw_ch == '\r' {
            // CR can be standalone or part of CRLF
            let next_pos = self.index as usize + 1;
            let next = if next_pos < self.length as usize {
                self.input[next_pos..].chars().next()
            } else {
                None
            };
            if next == Some('\n') {
                // CRLF - consume both, count as single newline
                self.index += 2;
                self.line += 1;
                self.column = 0;
                return '\n';
            } else {
                // Standalone CR - treat as newline
                self.index += 1;
                self.line += 1;
                self.column = 0;
                return '\r';
            }
        } else {
            self.column += 1;
        }
        // Advance by the byte length of the current character
        self.index += raw_ch.len_utf8() as u32;
        raw_ch
    }

    /// Checks if the input starts with the given string at current position.
    fn starts_with(&self, s: &str) -> bool {
        self.input[self.index as usize..].starts_with(s)
    }

    /// Reports an error at the current position.
    fn error(&mut self, msg: &str) {
        self.errors
            .push(HtmlTokenError { msg: msg.to_string(), position: (self.line, self.column) });
    }

    /// Checks if the current position is the start of a supported block.
    /// A block starts with `@` followed by a supported block keyword.
    fn is_block_start(&self) -> bool {
        if self.peek() != '@' {
            return false;
        }
        // Check if followed by a supported block keyword
        for &block_name in SUPPORTED_BLOCKS {
            let check_str = format!("@{block_name}");
            if self.starts_with(&check_str) {
                // Make sure the block name is not followed by an identifier char
                // (e.g., "@iffy" should not match "@if")
                let next_char_index = self.index as usize + check_str.len();
                if next_char_index >= self.input.len() {
                    return true; // At end of input, it's a match
                }
                let next_char = self.input[next_char_index..].chars().next().unwrap_or(chars::EOF);
                if !chars::is_identifier_part(next_char) || chars::is_whitespace(next_char) {
                    return true;
                }
            }
        }
        false
    }

    /// Scans a single token.
    fn scan_token(&mut self) {
        let start = self.index;

        // Check for block close (`}`) when tokenizing blocks (but not in expansion form)
        // Angular behavior: standalone `}` becomes BLOCK_CLOSE when:
        // - tokenize_blocks is true (default), AND
        // - We're not inside an expansion case or expansion form
        // - We're not in escaped_string mode (where `}` may be part of `\u{...}` escape)
        if self.peek() == '}'
            && self.tokenize_blocks
            && !self.escaped_string
            && !self.is_in_expansion_case()
            && !self.is_in_expansion_form()
        {
            self.advance();
            self.tokens.push(HtmlToken::empty(HtmlTokenType::BlockClose, start, self.index));
            if self.block_depth > 0 {
                self.block_depth -= 1;
            }
            return;
        }

        // Check for @let declarations (only if tokenize_let is enabled)
        if self.tokenize_let && self.peek() == '@' && self.starts_with("@let") {
            // Make sure "@let" is followed by whitespace (not "@letter")
            let next_char_index = self.index as usize + 4;
            if next_char_index < self.input.len() {
                let next_char = self.input[next_char_index..].chars().next().unwrap_or(chars::EOF);
                if chars::is_whitespace(next_char) {
                    self.scan_let_start(start);
                    return;
                }
                // @let not followed by whitespace - emit INCOMPLETE_LET and continue
                // This handles cases like "@letFoo" where @let is immediately followed by identifier
                if chars::is_identifier_part(next_char) {
                    // Consume "@let"
                    for _ in 0..4 {
                        self.advance();
                    }
                    self.tokens.push(HtmlToken::with_part(
                        HtmlTokenType::IncompleteLet,
                        "@let",
                        start,
                        self.index,
                    ));
                    return;
                }
            }
        }

        // Check for block start (@if, @for, etc.)
        // Only match supported block keywords - `@` followed by non-keyword is text
        if self.tokenize_blocks && self.is_block_start() {
            self.scan_block(start);
            return;
        }

        // Check for interpolation
        if self.starts_with(self.interpolation_start) {
            // Emit empty Text token before interpolation if the previous token is NOT Text
            // This ensures interpolations are always surrounded by Text tokens for proper parsing
            // (Angular's lexer behavior - empty tokens get filtered at parser level)
            let needs_text_before = match self.tokens.last() {
                Some(t) => {
                    t.token_type != HtmlTokenType::Text
                        && t.token_type != HtmlTokenType::RawText
                        && t.token_type != HtmlTokenType::EscapableRawText
                }
                None => true,
            };
            if needs_text_before {
                self.tokens.push(HtmlToken::with_part(HtmlTokenType::Text, "", start, start));
            }
            self.scan_interpolation(start);
            // Emit empty Text token after interpolation
            let after_interp = self.index;
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::Text,
                "",
                after_interp,
                after_interp,
            ));
            return;
        }

        // Check for tag - but only if followed by valid tag start character
        // A `<` followed by whitespace or other non-tag characters is just text
        if self.peek() == '<' {
            let next = self.peek_at(1);
            // Valid tag start: `/` (close tag), `!` (comment/doctype/cdata), or letter/underscore
            if next == '/' || next == '!' || next.is_ascii_alphabetic() || next == '_' {
                self.scan_tag(start);
                return;
            }
            // Otherwise, treat `<` as start of text
        }

        // Try to tokenize expansion forms (ICU messages) if enabled
        if self.tokenize_icu && self.scan_expansion_form() {
            return;
        }

        // Otherwise, scan text
        self.scan_text(start);
    }

    /// Gets the block name from current position.
    /// This allows capturing names like `@else if`, but not `@ if`.
    /// Matches Angular's `_getBlockName()`.
    fn get_block_name(&mut self) -> String {
        let name_start = self.index;
        let mut spaces_in_name_allowed = false;

        while self.index < self.length {
            let ch = self.peek();
            if chars::is_whitespace(ch) {
                if !spaces_in_name_allowed {
                    break;
                }
                // Whitespace allowed in name - continue
                self.advance();
            } else if chars::is_identifier_part(ch) {
                spaces_in_name_allowed = true;
                self.advance();
            } else {
                // Not whitespace and not identifier char - stop
                break;
            }
        }

        // Normalize whitespace: collapse multiple spaces/tabs into single space
        let raw = self.input[name_start as usize..self.index as usize].trim();
        let mut result = String::with_capacity(raw.len());
        let mut prev_was_whitespace = false;
        for ch in raw.chars() {
            if ch.is_whitespace() {
                if !prev_was_whitespace {
                    result.push(' ');
                    prev_was_whitespace = true;
                }
            } else {
                result.push(ch);
                prev_was_whitespace = false;
            }
        }
        result
    }

    /// Scans a block (@if, @for, etc.).
    fn scan_block(&mut self, start: u32) {
        self.advance(); // consume @

        // Read block/keyword name (can include spaces like "else if")
        let name = self.get_block_name();

        // Track the token index so we can modify it to IncompleteBlockOpen if needed
        let token_index = self.tokens.len();
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::BlockOpenStart,
            &name,
            start,
            self.index,
        ));

        // Skip whitespace
        self.skip_whitespace();

        // Check for parameters
        let mut params_unclosed = false;
        if self.peek() == '(' {
            self.advance(); // consume (
            self.scan_block_parameters();
            // Skip whitespace after parameters
            self.skip_whitespace();
            // Expect )
            if self.peek() == ')' {
                self.advance();
            } else {
                // Parameters were not properly closed
                params_unclosed = true;
            }
        }

        // Skip whitespace before {
        self.skip_whitespace();

        // v22: `@default never;` (optionally `@default never(expr);`) is a switch
        // exhaustive check terminated by `;` rather than a `{ ... }` body. Emit
        // BlockOpenEnd + BlockClose so it parses as an empty, self-closed block.
        if !params_unclosed && name == "default never" && self.peek() == ';' {
            let semi = self.index;
            self.advance(); // consume ;
            self.tokens.push(HtmlToken::empty(HtmlTokenType::BlockOpenEnd, semi, self.index));
            self.tokens.push(HtmlToken::empty(HtmlTokenType::BlockClose, self.index, self.index));
            return;
        }

        // Expect { to end the block header
        if !params_unclosed && self.peek() == '{' {
            let brace_start = self.index;
            self.advance();
            self.tokens.push(HtmlToken::empty(
                HtmlTokenType::BlockOpenEnd,
                brace_start,
                self.index,
            ));
            self.block_depth += 1;
        } else if !params_unclosed && self.is_block_start() && (name == "case" || name == "default")
        {
            // Special handling for consecutive @case/@default blocks without braces.
            // Angular allows `@case ('foo') @case ('bar') { ... }` where the first
            // case has no body. We emit BLOCK_OPEN_END and BLOCK_CLOSE to close it.
            let pos = self.index;
            self.tokens.push(HtmlToken::empty(HtmlTokenType::BlockOpenEnd, pos, pos));
            self.tokens.push(HtmlToken::empty(HtmlTokenType::BlockClose, pos, pos));
        } else {
            // Block is incomplete - change token type to IncompleteBlockOpen
            if let Some(token) = self.tokens.get_mut(token_index) {
                token.token_type = HtmlTokenType::IncompleteBlockOpen;
            }
        }
    }

    /// Scans a @let declaration: `@let name = value;`
    fn scan_let_start(&mut self, start: u32) {
        // Consume "@let"
        for _ in 0..4 {
            self.advance();
        }
        // Skip whitespace (but not newlines for detecting invalid @let)
        while self.peek() == ' ' || self.peek() == '\t' {
            self.advance();
        }

        // Check if we have valid identifier start
        let first_char = self.peek();
        let is_valid_name_start =
            first_char.is_ascii_alphabetic() || first_char == '_' || first_char == '$';

        // Invalid name start (digit, #, etc.) - emit INCOMPLETE_LET with empty name
        if !is_valid_name_start {
            // Emit INCOMPLETE_LET with empty name
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::IncompleteLet,
                "",
                start,
                self.index,
            ));
            return;
        }

        // Read variable name
        let var_name_start = self.index;
        while chars::is_identifier_part(self.peek()) {
            self.advance();
        }
        let var_name = self.input[var_name_start as usize..self.index as usize].to_string();

        // Check for invalid characters after name (backslash, #, newline without =)
        let next = self.peek();
        if next == '\\' || next == '#' {
            // Invalid character after name - emit INCOMPLETE_LET
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::IncompleteLet,
                &var_name,
                start,
                self.index,
            ));
            return;
        }

        // Check for newline without = (skip whitespace first to see if there's an =)
        if next == '\n' || next == '\r' {
            let saved_index = self.index;

            // Skip the newline
            self.advance();
            self.skip_whitespace();
            if self.peek() != '=' {
                // No = after whitespace - incomplete
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::IncompleteLet,
                    &var_name,
                    start,
                    saved_index,
                ));
                return;
            }
            // Continue - we found = after newline
        }

        // Save start position for the name token
        let name_end = self.index;

        // Skip whitespace
        self.skip_whitespace();

        // Expect =
        if self.peek() == '=' {
            self.advance();
        } else {
            // No equals sign - incomplete
            return;
        }

        // Skip whitespace after =
        self.skip_whitespace();

        // Read value expression until `;` - need to handle quotes
        let value_start = self.index;
        let mut in_quote: Option<char> = None;
        let mut found_semicolon = false;

        while self.index < self.length {
            let ch = self.peek();

            // Handle escape sequences in quotes
            if in_quote.is_some() && ch == '\\' {
                self.advance(); // skip backslash
                if self.index < self.length {
                    self.advance(); // skip escaped char
                }
                continue;
            }

            // Track quote state
            if let Some(quote_char) = in_quote {
                if ch == quote_char {
                    in_quote = None;
                }
                self.advance();
                continue;
            }

            // Check for quote start
            if ch == '"' || ch == '\'' || ch == '`' {
                in_quote = Some(ch);
                self.advance();
                continue;
            }

            // Semicolon ALWAYS ends the value (even if brackets are unbalanced)
            // But only if we're not inside a string
            if ch == ';' && in_quote.is_none() {
                found_semicolon = true;
                break;
            }

            // EOF or tag start ends the value
            if ch == chars::EOF || ch == '<' {
                break;
            }

            self.advance();
        }

        // Check for unclosed quotes - report error
        if in_quote.is_some() {
            self.error("Unexpected character \"EOF\"");
        }

        let value_end = self.index;
        // Preserve whitespace in value (don't trim)
        let value = self.input[value_start as usize..value_end as usize].to_string();

        // Emit tokens based on whether we found a semicolon
        if found_semicolon {
            // Complete declaration: LET_START, LET_VALUE, LET_END
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::LetStart,
                &var_name,
                start,
                name_end,
            ));
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::LetValue,
                &value,
                value_start,
                value_end,
            ));
            let semi_start = self.index;
            self.advance();
            self.tokens.push(HtmlToken::empty(HtmlTokenType::LetEnd, semi_start, self.index));
        } else {
            // Incomplete declaration: INCOMPLETE_LET, LET_VALUE
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::IncompleteLet,
                &var_name,
                start,
                name_end,
            ));
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::LetValue,
                &value,
                value_start,
                value_end,
            ));
        }
    }

    /// Scans block parameters inside `(...)`.
    fn scan_block_parameters(&mut self) {
        self.skip_whitespace();

        while self.peek() != ')' && self.peek() != chars::EOF {
            let param_start = self.index;

            let mut in_quote: Option<char> = None;
            let mut paren_depth = 0;

            // Consume the parameter until the next semicolon or closing paren.
            // Note that we skip over semicolons inside of strings.
            while self.index < self.length {
                let ch = self.peek();

                // Handle escape sequences - skip the escaped char
                if ch == '\\' {
                    self.advance();
                    if self.index < self.length {
                        self.advance();
                    }
                    continue;
                }

                // If in a quote, check for quote end
                if let Some(quote_char) = in_quote {
                    if ch == quote_char {
                        in_quote = None;
                    }
                    self.advance();
                    continue;
                }

                // Not in quote - check for quote start
                if ch == '"' || ch == '\'' {
                    in_quote = Some(ch);
                    self.advance();
                    continue;
                }

                // Track parens
                if ch == '(' {
                    paren_depth += 1;
                } else if ch == ')' {
                    if paren_depth == 0 {
                        break;
                    }
                    paren_depth -= 1;
                } else if ch == ';' && paren_depth == 0 {
                    break;
                }

                self.advance();
            }

            // If we hit EOF while still in a quote, report an error
            if in_quote.is_some() && self.index >= self.length {
                self.errors.push(HtmlTokenError {
                    msg: "Unexpected character \"EOF\"".to_string(),
                    position: (self.line, self.column),
                });
            }

            let param_text = self.input[param_start as usize..self.index as usize].trim();
            if !param_text.is_empty() {
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::BlockParameter,
                    param_text,
                    param_start,
                    self.index,
                ));
            }

            if self.peek() == ';' {
                self.advance();
            }

            self.skip_whitespace();
        }
    }

    /// Skips whitespace characters.
    fn skip_whitespace(&mut self) {
        while chars::is_whitespace(self.peek()) {
            self.advance();
        }
    }

    /// Checks if the current position is the start of a tag
    /// (opening/closing/comments/cdata/etc).
    fn is_tag_start(&self) -> bool {
        if self.peek() == '<' {
            let next = self.peek_at(1);
            // If the next character is alphabetic, ! or / then it is a tag start
            next.is_ascii_alphabetic() || next == '/' || next == '!'
        } else {
            false
        }
    }

    /// Scans an interpolation.
    fn scan_interpolation(&mut self, start: u32) {
        // Skip start delimiter
        for _ in 0..self.interpolation_start.len() {
            self.advance();
        }

        // Find end delimiter, handling quoted strings and comments
        // Angular behavior:
        // - `}}` ends the interpolation unless we're inside a quoted string
        // - `//` starts a comment that lasts until `}}` (quotes in comments are ignored)
        // - Mismatched quotes will capture everything up to EOF
        let expr_start = self.index;
        let mut in_quote: Option<char> = None;
        let mut in_comment = false;

        while self.index < self.length {
            let ch = self.peek();

            // Check for tag start - break out of interpolation
            // (This is for backward compatibility with Angular's behavior)
            if self.is_tag_start() {
                // End interpolation token without the closing marker
                let expr = &self.input[expr_start as usize..self.index as usize];
                let normalized_expr = normalize_line_endings(expr);
                let parts = vec![self.interpolation_start.to_string(), normalized_expr];
                self.tokens.push(HtmlToken::new(
                    HtmlTokenType::Interpolation,
                    parts,
                    start,
                    self.index,
                ));
                return;
            }

            // When not in a quote, check for end delimiter first
            // (comments don't prevent }} from ending the interpolation)
            if in_quote.is_none() && self.starts_with(self.interpolation_end) {
                break;
            }

            // Handle escape sequences - skip the escaped char
            if ch == '\\' {
                self.advance(); // skip backslash
                if self.index < self.length {
                    self.advance(); // skip escaped char
                }
                continue;
            }

            // If we're in a quote, check for quote end
            if let Some(quote_char) = in_quote {
                if ch == quote_char {
                    in_quote = None;
                }
                self.advance();
                continue;
            }

            // Not in a quote - check for // comment start
            // (once in comment, quotes are ignored until we hit }})
            if !in_comment && self.starts_with("//") {
                in_comment = true;
                self.advance();
                self.advance();
                continue;
            }

            // Check for quote start (only when not in a comment)
            if !in_comment && (ch == '"' || ch == '\'' || ch == '`') {
                in_quote = Some(ch);
                self.advance();
                continue;
            }

            self.advance();
        }

        let expr = &self.input[expr_start as usize..self.index as usize];

        // Skip end delimiter if present
        let has_end = self.starts_with(self.interpolation_end);
        if has_end {
            for _ in 0..self.interpolation_end.len() {
                self.advance();
            }
        }

        // Create interpolation token with parts: [startMarker, expression, endMarker]
        // Normalize line endings in the expression (CRLF -> LF, CR -> LF)
        let normalized_expr = normalize_line_endings(expr);
        let parts = if has_end {
            vec![
                self.interpolation_start.to_string(),
                normalized_expr,
                self.interpolation_end.to_string(),
            ]
        } else {
            vec![self.interpolation_start.to_string(), normalized_expr]
        };

        self.tokens.push(HtmlToken::new(HtmlTokenType::Interpolation, parts, start, self.index));
    }

    /// Scans a tag.
    fn scan_tag(&mut self, start: u32) {
        self.advance(); // consume <

        // Check for closing tag
        if self.peek() == '/' {
            self.advance();
            self.scan_close_tag(start);
            return;
        }

        // Check for comment
        if self.starts_with("!--") {
            self.scan_comment(start);
            return;
        }

        // Check for DOCTYPE
        if self.starts_with("!DOCTYPE") || self.starts_with("!doctype") {
            self.scan_doctype(start);
            return;
        }

        // Check for CDATA
        if self.starts_with("![CDATA[") {
            self.scan_cdata(start);
            return;
        }

        // Check for incomplete `<!` sequences - must report error
        if self.peek() == '!' {
            self.advance(); // consume !

            if self.peek() == '-' {
                // `<!-` but not `<!--` - report error for the next char
                self.advance(); // consume -
                if self.peek() == chars::EOF {
                    self.error("Unexpected character \"EOF\"");
                } else {
                    self.error(&format!("Unexpected character \"{}\"", self.peek()));
                }
            } else if self.peek() == '[' {
                // `<![` but not `<![CDATA[` - report error for the next char
                self.advance(); // consume [
                if self.peek() == chars::EOF {
                    self.error("Unexpected character \"EOF\"");
                } else {
                    self.error(&format!("Unexpected character \"{}\"", self.peek()));
                }
            } else if self.peek() == chars::EOF {
                // Just `<!` at EOF
                self.error("Unexpected character \"EOF\"");
            } else {
                // `<!` followed by something unexpected
                self.error(&format!("Unexpected character \"{}\"", self.peek()));
            }
            // Emit as text
            let text = &self.input[start as usize..self.index as usize];
            self.tokens.push(HtmlToken::with_part(HtmlTokenType::Text, text, start, self.index));
            return;
        }

        // Regular opening tag
        self.scan_open_tag(start);
    }

    /// Scans an opening tag: `<tagname attrs>`
    fn scan_open_tag(&mut self, start: u32) {
        // Check for selectorless component tag
        if self.selectorless_enabled && Self::is_selectorless_name_start(self.peek()) {
            self.scan_component_open(start);
            return;
        }

        // Scan tag name
        let (prefix, name) = self.scan_tag_name();

        if name.is_empty() {
            // Incomplete tag
            self.tokens.push(HtmlToken::with_prefix_name(
                HtmlTokenType::IncompleteTagOpen,
                &prefix,
                &name,
                start,
                self.index,
            ));
            return;
        }

        // TAG_OPEN_START token spans from < to end of tag name
        self.tokens.push(HtmlToken::with_prefix_name(
            HtmlTokenType::TagOpenStart,
            &prefix,
            &name,
            start,
            self.index,
        ));

        // Scan attributes (with directive support if selectorless is enabled)
        self.scan_attributes();

        // Check for tag close
        self.skip_whitespace();

        let is_void = self.peek() == '/' && self.peek_at(1) == '>';
        if is_void {
            let close_start = self.index;
            self.advance();
            self.advance();
            self.tokens.push(HtmlToken::empty(
                HtmlTokenType::TagOpenEndVoid,
                close_start,
                self.index,
            ));
        } else if self.peek() == '>' {
            let close_start = self.index;
            self.advance();
            self.tokens.push(HtmlToken::empty(HtmlTokenType::TagOpenEnd, close_start, self.index));

            // Check if this tag has raw or escapable raw text content
            // Use get_html_tag_definition().get_content_type(prefix) to handle
            // namespace-aware content types (e.g., svg:title is parsable, html:title is escapable raw)
            let lower_name = name.to_lowercase();
            let ns_prefix = if prefix.is_empty() { None } else { Some(prefix.as_str()) };
            let content_type = get_html_tag_definition(&lower_name).get_content_type(ns_prefix);
            match content_type {
                TagContentType::RawText => {
                    self.scan_raw_text_with_tag_close(&lower_name, false);
                }
                TagContentType::EscapableRawText => {
                    self.scan_raw_text_with_tag_close(&lower_name, true);
                }
                TagContentType::Parsable => {
                    // Normal parsable content, no special handling needed
                }
            }
        } else if self.peek() == '/' {
            // `/` without `>` following - Angular consumes the `/` and fails on `>`
            // which marks the tag as incomplete
            self.advance(); // consume the `/`
            // Mark tag as incomplete
            for token in self.tokens.iter_mut().rev() {
                if token.token_type == HtmlTokenType::TagOpenStart {
                    token.token_type = HtmlTokenType::IncompleteTagOpen;
                    break;
                }
                if token.token_type == HtmlTokenType::ComponentOpenStart {
                    token.token_type = HtmlTokenType::IncompleteComponentOpen;
                    break;
                }
            }
        } else if self.peek() == '<' || self.peek() == chars::EOF {
            // Incomplete tag - find the most recent TAG_OPEN_START token and modify it
            // We need to search backwards because there may be attribute tokens after it
            for token in self.tokens.iter_mut().rev() {
                if token.token_type == HtmlTokenType::TagOpenStart {
                    token.token_type = HtmlTokenType::IncompleteTagOpen;
                    break;
                }
            }
        }
    }

    /// Scans raw text content until the closing tag.
    /// For RAW_TEXT (script/style): entities are NOT decoded.
    /// For ESCAPABLE_RAW_TEXT (title/textarea): entities ARE decoded.
    fn scan_raw_text_with_tag_close(&mut self, tag_name: &str, consume_entities: bool) {
        let token_type =
            if consume_entities { HtmlTokenType::EscapableRawText } else { HtmlTokenType::RawText };

        let content_start = self.index;
        let mut text_start = self.index;

        loop {
            // Check if we're at the closing tag
            if self.peek() == '<' && self.peek_at(1) == '/' {
                // Check if the closing tag matches our tag name (case-insensitive)
                let saved_index = self.index;
                let saved_line = self.line;
                let saved_column = self.column;

                self.advance(); // <
                self.advance(); // /
                self.skip_whitespace();

                let close_tag_start = self.index;
                while !chars::is_whitespace(self.peek())
                    && self.peek() != '>'
                    && self.peek() != chars::EOF
                {
                    self.advance();
                }
                let close_tag_name =
                    self.input[close_tag_start as usize..self.index as usize].to_lowercase();

                self.skip_whitespace();

                if close_tag_name == tag_name && self.peek() == '>' {
                    // Found the closing tag - emit any accumulated content
                    if consume_entities {
                        // For escapable raw text, Angular ALWAYS emits a text token, even if empty.
                        // This ensures entities are surrounded by text tokens.
                        let text = &self.input[text_start as usize..saved_index as usize];
                        let normalized = normalize_line_endings(text);
                        self.tokens.push(HtmlToken::with_part(
                            token_type,
                            &normalized,
                            text_start,
                            saved_index,
                        ));
                    } else {
                        // For raw text, only emit if there is actual content
                        if content_start < saved_index {
                            let content = &self.input[content_start as usize..saved_index as usize];
                            let normalized = normalize_line_endings(content);
                            self.tokens.push(HtmlToken::with_part(
                                token_type,
                                &normalized,
                                content_start,
                                saved_index,
                            ));
                        }
                    }

                    // Emit the closing tag
                    self.advance(); // consume >
                    self.tokens.push(HtmlToken::with_prefix_name(
                        HtmlTokenType::TagClose,
                        "",
                        tag_name,
                        saved_index,
                        self.index,
                    ));
                    return;
                } else {
                    // Not the matching closing tag - revert and include in content
                    self.index = saved_index;
                    self.line = saved_line;
                    self.column = saved_column;
                }
            }

            if self.peek() == chars::EOF {
                // End of input - emit what we have (if non-empty)
                if consume_entities {
                    if text_start < self.index {
                        let text = &self.input[text_start as usize..self.index as usize];
                        let normalized = normalize_line_endings(text);
                        self.tokens.push(HtmlToken::with_part(
                            token_type,
                            &normalized,
                            text_start,
                            self.index,
                        ));
                    }
                } else if content_start < self.index {
                    let content = &self.input[content_start as usize..self.index as usize];
                    let normalized = normalize_line_endings(content);
                    self.tokens.push(HtmlToken::with_part(
                        token_type,
                        &normalized,
                        content_start,
                        self.index,
                    ));
                }
                return;
            }

            // Handle entities for escapable raw text
            if consume_entities && self.peek() == '&' {
                // Angular's lexer ALWAYS emits a text token before a valid entity, even if empty.
                // This ensures entities are surrounded by text tokens (like interpolations).
                // We emit the text token first, then try to scan the entity.
                // If entity scanning fails, we pop the text token and treat & as regular text.
                let text = &self.input[text_start as usize..self.index as usize];
                let normalized = normalize_line_endings(text);
                self.tokens.push(HtmlToken::with_part(
                    token_type,
                    &normalized,
                    text_start,
                    self.index,
                ));

                // Try to scan entity
                if self.scan_entity() {
                    // Entity was emitted, start new token position after entity
                    text_start = self.index;
                } else {
                    // Not a valid entity - pop the text token we just emitted and
                    // treat & as regular text. scan_entity() has reverted index to &.
                    self.tokens.pop();
                    // Just advance past & and continue accumulating text
                    self.advance();
                }
            } else {
                self.advance();
            }
        }
    }

    /// Scans a selectorless component opening tag: `<MyComp attrs>`
    fn scan_component_open(&mut self, start: u32) {
        // Scan component name (starts with uppercase or underscore)
        let (component_name, prefix, tag_name) = self.scan_component_name();

        if component_name.is_empty() {
            // Incomplete component
            self.tokens.push(HtmlToken::new(
                HtmlTokenType::IncompleteComponentOpen,
                vec![component_name, prefix, tag_name],
                start,
                self.index,
            ));
            return;
        }

        // COMPONENT_OPEN_START token
        self.tokens.push(HtmlToken::new(
            HtmlTokenType::ComponentOpenStart,
            vec![component_name.clone(), prefix.clone(), tag_name.clone()],
            start,
            self.index,
        ));

        // Scan attributes (with directive support)
        self.scan_attributes();

        // Check for component close
        self.skip_whitespace();

        if self.peek() == '/' && self.peek_at(1) == '>' {
            let close_start = self.index;
            self.advance();
            self.advance();
            self.tokens.push(HtmlToken::empty(
                HtmlTokenType::ComponentOpenEndVoid,
                close_start,
                self.index,
            ));
        } else if self.peek() == '>' {
            let close_start = self.index;
            self.advance();
            self.tokens.push(HtmlToken::empty(
                HtmlTokenType::ComponentOpenEnd,
                close_start,
                self.index,
            ));

            // Check if the tag suffix indicates raw or escapable raw text
            // For component tags like <MyComp:script> or <MyComp:title>
            // But NOT when there's a namespace prefix like svg: or math:
            // Use get_html_tag_definition().get_content_type(prefix) to handle
            // namespace-aware content types (e.g., svg:title is parsable, html:title is escapable raw)
            let lower_tag = tag_name.to_lowercase();
            let ns_prefix = if prefix.is_empty() { None } else { Some(prefix.as_str()) };
            let content_type = get_html_tag_definition(&lower_tag).get_content_type(ns_prefix);
            match content_type {
                TagContentType::RawText => {
                    self.scan_component_raw_text(&component_name, &prefix, &tag_name, false);
                }
                TagContentType::EscapableRawText => {
                    self.scan_component_raw_text(&component_name, &prefix, &tag_name, true);
                }
                TagContentType::Parsable => {
                    // Normal parsable content, no special handling needed
                }
            }
        } else if self.peek() == '<' || self.peek() == chars::EOF {
            // Incomplete component - find the most recent COMPONENT_OPEN_START token
            for token in self.tokens.iter_mut().rev() {
                if token.token_type == HtmlTokenType::ComponentOpenStart {
                    token.token_type = HtmlTokenType::IncompleteComponentOpen;
                    break;
                }
            }
        }
    }

    /// Scans raw text content for a component until its closing tag.
    fn scan_component_raw_text(
        &mut self,
        component_name: &str,
        prefix: &str,
        tag_name: &str,
        consume_entities: bool,
    ) {
        let token_type =
            if consume_entities { HtmlTokenType::EscapableRawText } else { HtmlTokenType::RawText };

        let content_start = self.index;

        loop {
            // Check if we're at the closing tag
            if self.peek() == '<' && self.peek_at(1) == '/' {
                let saved_index = self.index;

                self.advance(); // <
                self.advance(); // /
                self.skip_whitespace();

                // Check if closing tag matches: </ComponentName:prefix:tag> or </ComponentName:tag>
                let close_name_start = self.index;
                while !chars::is_whitespace(self.peek())
                    && self.peek() != '>'
                    && self.peek() != chars::EOF
                {
                    self.advance();
                }
                let close_name = &self.input[close_name_start as usize..self.index as usize];

                // Build the expected closing tag name
                let expected_close = if prefix.is_empty() {
                    format!("{}:{}", component_name, tag_name)
                } else {
                    format!("{}:{}:{}", component_name, prefix, tag_name)
                };

                self.skip_whitespace();

                if close_name == expected_close && self.peek() == '>' {
                    // Found the closing tag - emit content
                    let content = &self.input[content_start as usize..saved_index as usize];
                    let normalized = normalize_line_endings(content);
                    self.tokens.push(HtmlToken::with_part(
                        token_type,
                        &normalized,
                        content_start,
                        saved_index,
                    ));

                    // Emit the closing component tag
                    self.advance(); // consume >
                    self.tokens.push(HtmlToken::new(
                        HtmlTokenType::ComponentClose,
                        vec![component_name.to_string(), prefix.to_string(), tag_name.to_string()],
                        saved_index,
                        self.index,
                    ));
                    return;
                }

                // Not our closing tag - continue scanning
            }

            if self.peek() == chars::EOF {
                // EOF reached - emit whatever content we have
                let content = &self.input[content_start as usize..self.index as usize];
                let normalized = normalize_line_endings(content);
                self.tokens.push(HtmlToken::with_part(
                    token_type,
                    &normalized,
                    content_start,
                    self.index,
                ));
                return;
            }

            self.advance();
        }
    }

    /// Scans a component name with optional namespace and tag name.
    /// Returns (component_name, prefix, tag_name).
    /// Format: ComponentName[:prefix:tagname] or ComponentName[:tagname]
    fn scan_component_name(&mut self) -> (String, String, String) {
        self.skip_whitespace();

        // Scan component name (uppercase/underscore start, alphanumeric/_)
        let name_start = self.index;
        while Self::is_selectorless_name_char(self.peek()) {
            self.advance();
        }
        let component_name = self.input[name_start as usize..self.index as usize].to_string();

        // Check for colon (indicates prefix:tagname or tagname)
        if self.peek() == ':' {
            self.advance(); // consume first ':'

            // Scan next part
            let part1_start = self.index;
            while !chars::is_whitespace(self.peek())
                && self.peek() != '>'
                && self.peek() != '/'
                && self.peek() != '<'
                && self.peek() != ':'
                && self.peek() != chars::EOF
            {
                self.advance();
            }
            let part1 = self.input[part1_start as usize..self.index as usize].to_string();

            // Check for another colon (prefix:tagname)
            if self.peek() == ':' {
                self.advance(); // consume second ':'

                let part2_start = self.index;
                while !chars::is_whitespace(self.peek())
                    && self.peek() != '>'
                    && self.peek() != '/'
                    && self.peek() != '<'
                    && self.peek() != chars::EOF
                {
                    self.advance();
                }
                let part2 = self.input[part2_start as usize..self.index as usize].to_string();

                (component_name, part1, part2)
            } else {
                // Just tagname, no prefix
                (component_name, String::new(), part1)
            }
        } else {
            (component_name, String::new(), String::new())
        }
    }

    /// Scans a closing tag: `</tagname>`
    fn scan_close_tag(&mut self, start: u32) {
        // Check for selectorless component close
        if self.selectorless_enabled && Self::is_selectorless_name_start(self.peek()) {
            let (component_name, prefix, tag_name) = self.scan_component_name();

            // Skip whitespace
            self.skip_whitespace();

            // Check for missing >
            if self.peek() != '>' {
                self.errors.push(HtmlTokenError {
                    msg: "Unexpected character \"EOF\"".to_string(),
                    position: (self.line, self.column),
                });
            } else {
                self.advance();
            }

            self.tokens.push(HtmlToken::new(
                HtmlTokenType::ComponentClose,
                vec![component_name, prefix, tag_name],
                start,
                self.index,
            ));
            return;
        }

        // Scan tag name
        let (prefix, name) = self.scan_tag_name();

        // Check for missing tag name
        if name.is_empty() && prefix.is_empty() {
            self.errors.push(HtmlTokenError {
                msg: "Unexpected character \"EOF\"".to_string(),
                position: (self.line, self.column),
            });
            return;
        }

        // Skip whitespace
        self.skip_whitespace();

        // Check for missing >
        if self.peek() != '>' {
            self.errors.push(HtmlTokenError {
                msg: "Unexpected character \"EOF\"".to_string(),
                position: (self.line, self.column),
            });
        } else {
            self.advance();
        }

        self.tokens.push(HtmlToken::with_prefix_name(
            HtmlTokenType::TagClose,
            &prefix,
            &name,
            start,
            self.index,
        ));
    }

    /// Scans a tag name (with optional namespace prefix).
    fn scan_tag_name(&mut self) -> (String, String) {
        self.skip_whitespace();

        let name_start = self.index;
        while !chars::is_whitespace(self.peek())
            && self.peek() != '>'
            && self.peek() != '/'
            && self.peek() != '<'
            && self.peek() != chars::EOF
        {
            self.advance();
        }

        let full_name = &self.input[name_start as usize..self.index as usize];

        // Split on colon for namespace prefix
        if let Some(colon_pos) = full_name.find(':') {
            let prefix = &full_name[..colon_pos];
            let name = &full_name[colon_pos + 1..];
            (prefix.to_string(), name.to_string())
        } else {
            (String::new(), full_name.to_string())
        }
    }

    /// Scans a selectorless directive: @DirectiveName or @DirectiveName(attrs)
    fn scan_directive(&mut self) {
        let start = self.index;
        self.advance(); // consume '@'

        // Scan directive name
        let name_start = self.index;
        while Self::is_selectorless_name_char(self.peek()) {
            self.advance();
        }
        let directive_name = self.input[name_start as usize..self.index as usize].to_string();

        // Emit DirectiveName token
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::DirectiveName,
            &directive_name,
            start,
            self.index,
        ));

        // Check for directive attributes in parentheses
        if self.peek() == '(' {
            let open_start = self.index;
            self.advance(); // consume '('
            self.tokens.push(HtmlToken::empty(
                HtmlTokenType::DirectiveOpen,
                open_start,
                self.index,
            ));

            // Scan attributes within directive (until closing paren)
            self.scan_directive_attributes();

            // Check for closing paren
            if self.peek() == ')' {
                let close_start = self.index;
                self.advance();
                self.tokens.push(HtmlToken::empty(
                    HtmlTokenType::DirectiveClose,
                    close_start,
                    self.index,
                ));
            }
        }
    }

    /// Scans attributes within a directive (inside parentheses).
    /// This handles Angular binding syntax like `[prop]="value"` and `(event)="handler"`.
    fn scan_directive_attributes(&mut self) {
        loop {
            self.skip_whitespace();

            // Check for end of directive - only stop at a closing paren at depth 0
            if self.peek() == ')' || self.peek() == chars::EOF {
                break;
            }

            // Skip comma separators
            if self.peek() == ',' {
                self.advance();
                continue;
            }

            // Scan attribute name - handle nested brackets for Angular bindings
            let attr_start = self.index;
            let mut bracket_depth = 0;
            let mut paren_depth = 0;
            while self.peek() != chars::EOF {
                let ch = self.peek();

                // Track bracket and parenthesis depth for nested content
                if ch == '[' {
                    bracket_depth += 1;
                } else if ch == ']' {
                    if bracket_depth > 0 {
                        bracket_depth -= 1;
                    } else {
                        // Unmatched ], stop
                        break;
                    }
                } else if ch == '(' {
                    paren_depth += 1;
                } else if ch == ')' {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    } else {
                        // Unmatched ) at depth 0 means end of directive
                        break;
                    }
                }

                // Only stop at delimiters when not inside brackets/parens
                if bracket_depth == 0 && paren_depth == 0 {
                    if chars::is_whitespace(ch) || ch == '=' || ch == ',' {
                        break;
                    }
                }

                self.advance();
            }

            let attr_name = &self.input[attr_start as usize..self.index as usize];
            if attr_name.is_empty() {
                break;
            }

            // Split on colon for namespace prefix (but not for Angular bindings)
            let first_char = attr_name.chars().next();
            let is_angular_binding = matches!(first_char, Some('(' | '[' | '*' | '#'));
            let (prefix, name) = if !is_angular_binding {
                if let Some(colon_pos) = attr_name.find(':') {
                    (&attr_name[..colon_pos], &attr_name[colon_pos + 1..])
                } else {
                    ("", attr_name)
                }
            } else {
                ("", attr_name)
            };

            self.tokens.push(HtmlToken::with_prefix_name(
                HtmlTokenType::AttrName,
                prefix,
                name,
                attr_start,
                self.index,
            ));

            self.skip_whitespace();

            // Check for value
            if self.peek() == '=' {
                self.advance();
                self.skip_whitespace();
                self.scan_attribute_value();
            }
        }
    }

    /// Scans attributes.
    fn scan_attributes(&mut self) {
        loop {
            self.skip_whitespace();

            // Check for end of tag
            if self.peek() == '>'
                || (self.peek() == '/' && self.peek_at(1) == '>')
                || self.peek() == '<'
                || self.peek() == chars::EOF
            {
                break;
            }

            // Check for quote character in place of attribute name
            // This marks the tag as incomplete and we stop attribute scanning
            if self.peek() == '"' || self.peek() == '\'' {
                // Mark the tag as incomplete by modifying the most recent TAG_OPEN_START
                for token in self.tokens.iter_mut().rev() {
                    if token.token_type == HtmlTokenType::TagOpenStart {
                        token.token_type = HtmlTokenType::IncompleteTagOpen;
                        break;
                    }
                    if token.token_type == HtmlTokenType::ComponentOpenStart {
                        token.token_type = HtmlTokenType::IncompleteComponentOpen;
                        break;
                    }
                }
                break;
            }

            // Check for selectorless directive attribute: @DirectiveName
            if self.selectorless_enabled
                && self.peek() == '@'
                && Self::is_selectorless_name_start(self.peek_at(1))
            {
                self.scan_directive();
                continue;
            }

            // Scan attribute name - Angular's permissive bracket parsing for Tailwind-style classes
            let attr_start = self.index;
            let starts_with_bracket = self.peek() == '[';
            let mut open_brackets: i32 = 0;

            while self.peek() != chars::EOF {
                let ch = self.peek();

                // Track bracket depth (can go negative for mismatched brackets)
                if ch == '[' {
                    open_brackets += 1;
                } else if ch == ']' {
                    open_brackets -= 1;
                }

                // Check stopping conditions based on bracket state
                if starts_with_bracket {
                    // Angular's permissive mode for square-bracketed attributes:
                    // - When brackets are balanced or mismatched (open_brackets <= 0):
                    //   stop at name-end characters (whitespace, =, >, <, /, ', ", EOF)
                    // - When brackets are unbalanced with more opens (open_brackets > 0):
                    //   only stop on newline (interrupts matching)
                    if open_brackets <= 0 {
                        if chars::is_whitespace(ch)
                            || ch == '='
                            || ch == '>'
                            || ch == '<'
                            || ch == '/'
                            || ch == '\''
                            || ch == '"'
                        {
                            break;
                        }
                    } else {
                        // Inside unbalanced brackets, only newline interrupts
                        if ch == '\n' || ch == '\r' {
                            break;
                        }
                    }
                } else {
                    // Normal attribute name - stop at standard name-end characters
                    if chars::is_whitespace(ch)
                        || ch == '='
                        || ch == '>'
                        || ch == '<'
                        || ch == '/'
                        || ch == '\''
                        || ch == '"'
                    {
                        break;
                    }
                }

                self.advance();
            }

            let attr_full_name = &self.input[attr_start as usize..self.index as usize];
            if attr_full_name.is_empty() {
                break;
            }

            // If we started with `[` but have unbalanced brackets (open_brackets > 0),
            // the tag is incomplete because a newline interrupted the attribute name
            if starts_with_bracket && open_brackets > 0 {
                // Mark the tag as incomplete
                for token in self.tokens.iter_mut().rev() {
                    if token.token_type == HtmlTokenType::TagOpenStart {
                        token.token_type = HtmlTokenType::IncompleteTagOpen;
                        break;
                    }
                    if token.token_type == HtmlTokenType::ComponentOpenStart {
                        token.token_type = HtmlTokenType::IncompleteComponentOpen;
                        break;
                    }
                }
            }

            // Split on colon for namespace prefix
            // But NOT if the colon is inside parentheses or brackets (Angular bindings)
            let (prefix, name) = {
                let first_char = attr_full_name.chars().next();
                let is_angular_binding = matches!(first_char, Some('(' | '[' | '*' | '#'));
                if !is_angular_binding {
                    if let Some(colon_pos) = attr_full_name.find(':') {
                        (&attr_full_name[..colon_pos], &attr_full_name[colon_pos + 1..])
                    } else {
                        ("", attr_full_name)
                    }
                } else {
                    // Angular bindings like (click), [value], *ngIf, #ref - don't split on colon
                    ("", attr_full_name)
                }
            };

            self.tokens.push(HtmlToken::with_prefix_name(
                HtmlTokenType::AttrName,
                prefix,
                name,
                attr_start,
                self.index,
            ));

            self.skip_whitespace();

            // Check for value
            if self.peek() == '=' {
                self.advance();
                self.skip_whitespace();
                self.scan_attribute_value();
            }
        }
    }

    /// Scans an attribute value.
    fn scan_attribute_value(&mut self) {
        if self.peek() == '"' || self.peek() == '\'' {
            let quote = self.peek();
            let quote_start = self.index;
            self.advance();

            // Emit quote token
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::AttrQuote,
                if quote == '"' { "\"" } else { "'" },
                quote_start,
                self.index,
            ));

            // Scan value with interpolation support
            self.scan_attribute_value_text(quote);

            // Closing quote
            if self.peek() == quote {
                let close_quote_start = self.index;
                self.advance();
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::AttrQuote,
                    if quote == '"' { "\"" } else { "'" },
                    close_quote_start,
                    self.index,
                ));
            } else {
                // Missing closing quote - report error
                self.errors.push(HtmlTokenError {
                    msg: "Unexpected character \"EOF\"".to_string(),
                    position: (self.line, self.column),
                });
            }
        } else {
            // Unquoted value - may contain interpolation
            let value_start = self.index;

            // Check if it starts with interpolation
            if self.starts_with(self.interpolation_start) {
                // Emit empty text before interpolation
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::AttrValueText,
                    "",
                    value_start,
                    value_start,
                ));

                // Parse interpolation
                let interp_start = self.index;
                for _ in 0..self.interpolation_start.len() {
                    self.advance();
                }

                let expr_start = self.index;
                while !self.starts_with(self.interpolation_end)
                    && !chars::is_whitespace(self.peek())
                    && self.peek() != '>'
                    && self.peek() != '/'
                    && self.peek() != chars::EOF
                {
                    self.advance();
                }
                let expr = &self.input[expr_start as usize..self.index as usize];

                let has_end = self.starts_with(self.interpolation_end);
                if has_end {
                    for _ in 0..self.interpolation_end.len() {
                        self.advance();
                    }
                }

                let parts = if has_end {
                    vec![
                        self.interpolation_start.to_string(),
                        expr.to_string(),
                        self.interpolation_end.to_string(),
                    ]
                } else {
                    vec![self.interpolation_start.to_string(), expr.to_string()]
                };

                self.tokens.push(HtmlToken::new(
                    HtmlTokenType::AttrValueInterpolation,
                    parts,
                    interp_start,
                    self.index,
                ));

                // Emit empty text after interpolation
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::AttrValueText,
                    "",
                    self.index,
                    self.index,
                ));
            } else {
                // Regular unquoted value
                while !chars::is_whitespace(self.peek())
                    && self.peek() != '>'
                    && self.peek() != '/'
                    && self.peek() != chars::EOF
                {
                    self.advance();
                }
                let value = &self.input[value_start as usize..self.index as usize];
                if !value.is_empty() {
                    self.tokens.push(HtmlToken::with_part(
                        HtmlTokenType::AttrValueText,
                        value,
                        value_start,
                        self.index,
                    ));
                }
            }
        }
    }

    /// Scans attribute value text (with interpolation and entity support).
    fn scan_attribute_value_text(&mut self, quote: char) {
        // Check for empty value (immediately at closing quote)
        if self.peek() == quote {
            // Emit empty AttrValueText token for empty attribute values
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::AttrValueText,
                "",
                self.index,
                self.index,
            ));
            return;
        }

        let mut text_start = self.index;

        while self.peek() != quote && self.peek() != chars::EOF {
            if self.starts_with(self.interpolation_start) {
                // Emit any accumulated text first (or empty text token if at start of interpolation)
                let text = &self.input[text_start as usize..self.index as usize];
                let normalized = normalize_line_endings(text);
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::AttrValueText,
                    &normalized,
                    text_start,
                    self.index,
                ));

                // Handle interpolation inside attribute value
                let interp_start = self.index;
                for _ in 0..self.interpolation_start.len() {
                    self.advance();
                }

                // Track quote state inside interpolation (Angular's _consumeInterpolation logic)
                let mut in_interp_quote: Option<char> = None;
                let expr_start = self.index;

                // Angular's loop structure:
                // - prematureEndPredicate (attribute quote check) is in the while condition
                // - interpolation end `}}` is only checked when NOT in a quote
                while self.peek() != chars::EOF && self.peek() != quote {
                    // Check for interpolation end ONLY when not in a quote
                    if in_interp_quote.is_none() && self.starts_with(self.interpolation_end) {
                        break;
                    }

                    // Read character and advance (matches Angular's flow)
                    let ch = self.advance();

                    // Handle backslash escapes - ALWAYS skip next char after backslash
                    // This matches Angular's behavior at lines 1222-1224 of lexer.ts
                    if ch == '\\' {
                        if self.index < self.length {
                            self.advance(); // skip escaped char
                        }
                        continue;
                    }

                    // Track quote state
                    if let Some(q) = in_interp_quote {
                        if ch == q {
                            in_interp_quote = None;
                        }
                    } else if ch == '"' || ch == '\'' || ch == '`' {
                        // Entering a new quoted string
                        in_interp_quote = Some(ch);
                    }
                }
                let expr = &self.input[expr_start as usize..self.index as usize];

                let has_end = self.starts_with(self.interpolation_end);
                if has_end {
                    for _ in 0..self.interpolation_end.len() {
                        self.advance();
                    }
                }

                // For interpolations that are cut off by the quote but have the closing marker,
                // include the closing marker in parts
                let parts = if has_end {
                    vec![
                        self.interpolation_start.to_string(),
                        expr.to_string(),
                        self.interpolation_end.to_string(),
                    ]
                } else {
                    vec![self.interpolation_start.to_string(), expr.to_string()]
                };

                self.tokens.push(HtmlToken::new(
                    HtmlTokenType::AttrValueInterpolation,
                    parts,
                    interp_start,
                    self.index,
                ));

                // Emit empty text token after interpolation
                text_start = self.index;
                self.tokens.push(HtmlToken::with_part(
                    HtmlTokenType::AttrValueText,
                    "",
                    text_start,
                    text_start,
                ));
            } else if self.peek() == '&' {
                // Try to scan the entity first (without emitting text)
                let entity_start = self.index;
                if self.scan_entity() {
                    // Entity was matched - emit any text before it (may be empty)
                    let text = &self.input[text_start as usize..entity_start as usize];
                    let normalized = normalize_line_endings(text);
                    // Insert before the entity token we just added
                    // Safety: scan_entity() pushes a token when returning true
                    if let Some(entity_token) = self.tokens.pop() {
                        self.tokens.push(HtmlToken::with_part(
                            HtmlTokenType::AttrValueText,
                            &normalized,
                            text_start,
                            entity_start,
                        ));
                        self.tokens.push(entity_token);
                        // Emit empty text token after entity
                        text_start = self.index;
                        self.tokens.push(HtmlToken::with_part(
                            HtmlTokenType::AttrValueText,
                            "",
                            text_start,
                            text_start,
                        ));
                    }
                } else {
                    // scan_entity reverted, so & is just regular text - advance past it
                    self.advance();
                }
            } else {
                self.advance();
            }
        }

        // Emit remaining text (only if there's actual content since we already emit empty tokens after interp/entity)
        if self.index > text_start {
            let text = &self.input[text_start as usize..self.index as usize];
            let normalized = normalize_line_endings(text);
            self.tokens.push(HtmlToken::with_part(
                HtmlTokenType::AttrValueText,
                &normalized,
                text_start,
                self.index,
            ));
        }
    }

    /// Scans a comment.
    fn scan_comment(&mut self, start: u32) {
        // Skip !--
        self.advance();
        self.advance();
        self.advance();

        // Emit COMMENT_START
        self.tokens.push(HtmlToken::empty(HtmlTokenType::CommentStart, start, self.index));

        let content_start = self.index;
        while !self.starts_with("-->") && self.index < self.length {
            self.advance();
        }
        let content = &self.input[content_start as usize..self.index as usize];
        let normalized_content = normalize_line_endings(content);

        // Emit content as RAW_TEXT
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::RawText,
            &normalized_content,
            content_start,
            self.index,
        ));

        // Skip --> and emit COMMENT_END
        if self.starts_with("-->") {
            let end_start = self.index;
            self.advance();
            self.advance();
            self.advance();
            self.tokens.push(HtmlToken::empty(HtmlTokenType::CommentEnd, end_start, self.index));
        } else {
            self.error("Unexpected character \"EOF\"");
        }
    }

    /// Scans a DOCTYPE.
    fn scan_doctype(&mut self, start: u32) {
        // Skip the leading '!' - we're at "!DOCTYPE" or "!doctype"
        self.advance(); // Skip '!'

        // Now scan the content until '>'
        let content_start = self.index;
        while self.peek() != '>' && self.index < self.length {
            self.advance();
        }
        let content = &self.input[content_start as usize..self.index as usize];

        if self.peek() == '>' {
            self.advance();
        } else {
            // Report error for unterminated DOCTYPE
            self.error("Unexpected end of DOCTYPE");
        }

        self.tokens.push(HtmlToken::with_part(HtmlTokenType::DocType, content, start, self.index));
    }

    /// Scans a CDATA section.
    fn scan_cdata(&mut self, start: u32) {
        // Skip ![CDATA[
        for _ in 0..8 {
            self.advance();
        }

        // Emit CDATA_START
        self.tokens.push(HtmlToken::empty(HtmlTokenType::CdataStart, start, self.index));

        let content_start = self.index;
        while !self.starts_with("]]>") && self.index < self.length {
            self.advance();
        }
        let content = &self.input[content_start as usize..self.index as usize];
        let normalized = normalize_line_endings(content);

        // Emit content as RAW_TEXT
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::RawText,
            &normalized,
            content_start,
            self.index,
        ));

        // Skip ]]> and emit CDATA_END
        if self.starts_with("]]>") {
            let end_start = self.index;
            self.advance();
            self.advance();
            self.advance();
            self.tokens.push(HtmlToken::empty(HtmlTokenType::CdataEnd, end_start, self.index));
        } else {
            self.error("Unexpected character \"EOF\"");
        }
    }

    // ========================================================================
    // Expansion Forms (ICU Messages)
    // ========================================================================

    /// Checks if we're currently inside an expansion case (between { and }).
    fn is_in_expansion_case(&self) -> bool {
        !self.expansion_case_stack.is_empty()
            && self
                .expansion_case_stack
                .last()
                .is_some_and(|t| *t == HtmlTokenType::ExpansionCaseExpStart)
    }

    /// Checks if we're currently inside an expansion form.
    fn is_in_expansion_form(&self) -> bool {
        !self.expansion_case_stack.is_empty()
            && self
                .expansion_case_stack
                .last()
                .is_some_and(|t| *t == HtmlTokenType::ExpansionFormStart)
    }

    /// Checks if the current position is the start of an expansion form.
    /// An expansion form starts with `{` but NOT `{{` (interpolation).
    fn is_expansion_form_start(&self) -> bool {
        if self.peek() != '{' {
            return false;
        }
        // Check it's not an interpolation start
        !self.starts_with(self.interpolation_start)
    }

    /// Checks if the current character can start an expansion case value.
    fn is_expansion_case_start(&self) -> bool {
        self.peek() != '}'
    }

    /// Attempts to tokenize an expansion form.
    /// Returns true if tokens were emitted.
    fn scan_expansion_form(&mut self) -> bool {
        if self.is_expansion_form_start() {
            self.scan_expansion_form_start();
            return true;
        }

        if self.is_expansion_case_start() && self.is_in_expansion_form() {
            self.scan_expansion_case_start();
            return true;
        }

        if self.peek() == '}' {
            if self.is_in_expansion_case() {
                self.scan_expansion_case_end();
                return true;
            }

            if self.is_in_expansion_form() {
                self.scan_expansion_form_end();
                return true;
            }
        }

        false
    }

    /// Scans the start of an expansion form: `{value, type,`
    fn scan_expansion_form_start(&mut self) {
        let start = self.index;
        self.advance(); // consume '{'
        self.tokens.push(HtmlToken::empty(HtmlTokenType::ExpansionFormStart, start, self.index));

        self.expansion_case_stack.push(HtmlTokenType::ExpansionFormStart);

        // Read the switch value (until comma)
        let value_start = self.index;
        let condition = self.read_until(',');
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::RawText,
            &condition,
            value_start,
            self.index,
        ));

        if self.peek() == ',' {
            self.advance();
        }
        self.skip_whitespace();

        // Read the type (until comma)
        let type_start = self.index;
        let icu_type = self.read_until(',');
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::RawText,
            &icu_type,
            type_start,
            self.index,
        ));

        if self.peek() == ',' {
            self.advance();
        }
        self.skip_whitespace();
    }

    /// Scans the start of an expansion case: `=value {`
    fn scan_expansion_case_start(&mut self) {
        // Read case value (until '{')
        let value_start = self.index;
        let value = self.read_until('{').trim().to_string();
        self.tokens.push(HtmlToken::with_part(
            HtmlTokenType::ExpansionCaseValue,
            &value,
            value_start,
            self.index,
        ));
        self.skip_whitespace();

        // Consume '{'
        let brace_start = self.index;
        if self.peek() == '{' {
            self.advance();
        }
        self.tokens.push(HtmlToken::empty(
            HtmlTokenType::ExpansionCaseExpStart,
            brace_start,
            self.index,
        ));
        self.skip_whitespace();

        self.expansion_case_stack.push(HtmlTokenType::ExpansionCaseExpStart);
    }

    /// Scans the end of an expansion case: `}`
    fn scan_expansion_case_end(&mut self) {
        let start = self.index;
        self.advance(); // consume '}'
        self.tokens.push(HtmlToken::empty(HtmlTokenType::ExpansionCaseExpEnd, start, self.index));
        self.skip_whitespace();

        self.expansion_case_stack.pop();
    }

    /// Scans the end of an expansion form: `}`
    fn scan_expansion_form_end(&mut self) {
        let start = self.index;
        self.advance(); // consume '}'
        self.tokens.push(HtmlToken::empty(HtmlTokenType::ExpansionFormEnd, start, self.index));

        self.expansion_case_stack.pop();
    }

    /// Reads characters until the given terminator (does not consume the terminator).
    fn read_until(&mut self, terminator: char) -> String {
        let start = self.index;
        while self.peek() != terminator && self.peek() != chars::EOF {
            self.advance();
        }
        self.input[start as usize..self.index as usize].to_string()
    }

    /// Scans text content, handling HTML entities as separate tokens.
    /// Emits empty TEXT tokens around entities for Angular compatibility.
    fn scan_text(&mut self, start: u32) {
        let mut text_start = start;
        let mut had_entity = false;

        loop {
            let ch = self.peek();

            // Stop at EOF
            if ch == chars::EOF {
                break;
            }

            // Stop at `}`:
            // - When blocks are enabled (it becomes BLOCK_CLOSE) - unless we're in expansion or escaped_string
            // - When we're in an expansion case (it ends the case)
            if ch == '}' {
                if self.tokenize_blocks
                    && !self.escaped_string
                    && !self.is_in_expansion_case()
                    && !self.is_in_expansion_form()
                {
                    // `}` becomes BLOCK_CLOSE
                    break;
                }
                if self.tokenize_icu && self.is_in_expansion_case() {
                    // `}` ends the expansion case
                    break;
                }
            }

            // Stop at interpolation start
            if self.starts_with(self.interpolation_start) {
                break;
            }

            // Stop at expansion form start when ICU tokenization is enabled
            if self.tokenize_icu && self.is_expansion_form_start() {
                break;
            }

            // Stop at @let declaration
            if ch == '@' && self.starts_with("@let") {
                let next_char_index = self.index as usize + 4;
                if next_char_index < self.input.len() {
                    let next_char =
                        self.input[next_char_index..].chars().next().unwrap_or(chars::EOF);
                    if chars::is_whitespace(next_char) {
                        break;
                    }
                }
            }

            // Stop at block start (supported block keywords only)
            if self.is_block_start() {
                break;
            }

            // Handle `<` - only stop if it's a valid tag start
            if ch == '<' {
                let next = self.peek_at(1);
                // Valid tag start: `/` (close tag), `!` (comment/doctype/cdata), or letter/underscore
                if next == '/' || next == '!' || next.is_ascii_alphabetic() || next == '_' {
                    break;
                }
                // Otherwise, `<` is just text, continue
            }

            // Handle HTML entities
            if ch == '&' {
                // Try to scan the entity first (without emitting text)
                let entity_start = self.index;
                if self.scan_entity() {
                    // Entity was matched
                    // Safety: scan_entity() pushes a token when returning true
                    if let Some(entity_token) = self.tokens.pop() {
                        if !had_entity {
                            // First entity - emit empty TEXT token at start
                            self.tokens.push(HtmlToken::with_part(
                                HtmlTokenType::Text,
                                "",
                                text_start,
                                text_start,
                            ));
                        }

                        // Emit any text before entity
                        if entity_start > text_start {
                            let text = &self.input[text_start as usize..entity_start as usize];
                            let normalized = normalize_line_endings(text);
                            self.tokens.push(HtmlToken::with_part(
                                HtmlTokenType::Text,
                                &normalized,
                                text_start,
                                entity_start,
                            ));
                        }

                        // Emit entity token
                        self.tokens.push(entity_token);

                        // Emit empty TEXT token after entity
                        self.tokens.push(HtmlToken::with_part(
                            HtmlTokenType::Text,
                            "",
                            self.index,
                            self.index,
                        ));

                        text_start = self.index;
                        had_entity = true;
                        continue;
                    }
                }
                // If entity parsing failed, scan_entity reverted the index
                // so just advance past the & and treat it as regular text
            }

            self.advance();
        }

        // Emit remaining text
        if self.index > text_start {
            let text = &self.input[text_start as usize..self.index as usize];
            let normalized = normalize_line_endings(text);
            // Apply leading trivia calculation for source map accuracy
            let (adjusted_start, full_start) =
                self.calculate_start_with_trivia(text_start, self.index);
            self.tokens.push(HtmlToken::new_with_full_start(
                HtmlTokenType::Text,
                vec![normalized],
                adjusted_start,
                self.index,
                full_start,
            ));
        }
    }

    /// Scans an HTML entity (numeric or named).
    /// Returns true if an entity was successfully parsed and a token was emitted.
    /// Returns false if no valid entity was found (caller should treat `&` as regular text).
    fn scan_entity(&mut self) -> bool {
        let start = self.index;
        let start_line = self.line;
        let start_col = self.column;
        self.advance(); // consume '&'

        if self.peek() == '#' {
            // Numeric entity: &#123; or &#x7B;
            self.advance(); // consume '#'
            let is_hex = self.peek() == 'x' || self.peek() == 'X';
            if is_hex {
                self.advance(); // consume 'x' or 'X'
            }

            let num_start = self.index;
            // Consume digits
            while self.index < self.length {
                let ch = self.peek();
                if is_hex {
                    if !ch.is_ascii_hexdigit() {
                        break;
                    }
                } else if !ch.is_ascii_digit() {
                    break;
                }
                self.advance();
            }

            // Check for semicolon and that we have at least one digit
            if self.peek() == ';' && self.index > num_start {
                let num_str = &self.input[num_start as usize..self.index as usize];
                self.advance(); // consume ';'

                // Parse and decode
                if let Ok(code_point) =
                    if is_hex { u32::from_str_radix(num_str, 16) } else { num_str.parse::<u32>() }
                {
                    if let Some(ch) = char::from_u32(code_point) {
                        let decoded = ch.to_string();
                        let original = self.input[start as usize..self.index as usize].to_string();
                        self.tokens.push(HtmlToken::new(
                            HtmlTokenType::EncodedEntity,
                            vec![decoded, original],
                            start,
                            self.index,
                        ));
                        return true;
                    }
                }
                // Invalid code point - revert
                self.index = start;
                self.line = start_line;
                self.column = start_col;
                return false;
            }

            // Numeric entity doesn't end with semicolon - this is an error
            // Advance cursor to include the peeked character in the error message
            // (unless we're at EOF)
            if self.peek() == chars::EOF {
                // EOF - report "Unexpected character EOF"
                self.errors.push(HtmlTokenError {
                    msg: "Unexpected character \"EOF\"".to_string(),
                    position: (self.line, self.column),
                });
            } else {
                self.advance();
                let entity_str = &self.input[start as usize..self.index as usize];
                let entity_type = if is_hex { "hexadecimal" } else { "decimal" };
                self.errors.push(HtmlTokenError {
                    msg: format!(
                        "Unable to parse entity \"{entity_str}\" - {entity_type} character reference entities must end with \";\""
                    ),
                    position: (self.line, self.column),
                });
            }
            // Revert and treat as text
            self.index = start;
            self.line = start_line;
            self.column = start_col;
            return false;
        }

        // Named entity: &amp;
        let name_start = self.index;
        while self.index < self.length {
            let ch = self.peek();
            if !ch.is_ascii_alphanumeric() {
                break;
            }
            self.advance();
        }

        // Check for semicolon and that we have at least one char
        if self.peek() == ';' && self.index > name_start {
            let name = self.input[name_start as usize..self.index as usize].to_string();
            self.advance(); // consume ';'

            if get_named_entities().contains_key(name.as_str()) {
                // Safety: contains_key just verified the entity exists
                if let Some(decoded) =
                    decode_entity(&self.input[start as usize..self.index as usize])
                {
                    let original = self.input[start as usize..self.index as usize].to_string();
                    self.tokens.push(HtmlToken::new(
                        HtmlTokenType::EncodedEntity,
                        vec![decoded, original],
                        start,
                        self.index,
                    ));
                    return true;
                }
            }

            // Named entity with semicolon but unknown name - this is an error
            self.errors.push(HtmlTokenError {
                msg: format!(
                    "Unknown entity \"{name}\" - use the \"&#<decimal>;\" or  \"&#x<hex>;\" syntax"
                ),
                position: (start_line, start_col),
            });
            // Revert and treat as text
            self.index = start;
            self.line = start_line;
            self.column = start_col;
            return false;
        }

        // Named entity without semicolon - just revert, no error
        self.index = start;
        self.line = start_line;
        self.column = start_col;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<HtmlToken> {
        HtmlLexer::new(input).tokenize().tokens
    }

    #[test]
    fn test_simple_element() {
        let tokens = tokenize("<div></div>");
        assert!(tokens.iter().any(|t| t.token_type == HtmlTokenType::TagOpenStart));
        assert!(tokens.iter().any(|t| t.token_type == HtmlTokenType::TagClose));
    }

    #[test]
    fn test_tag_open_start_parts() {
        let tokens = tokenize("<test>");
        let tag = tokens.iter().find(|t| t.token_type == HtmlTokenType::TagOpenStart).unwrap();
        assert_eq!(tag.prefix(), "");
        assert_eq!(tag.name(), "test");
    }

    #[test]
    fn test_namespace_prefix() {
        let tokens = tokenize("<ns1:test>");
        let tag = tokens.iter().find(|t| t.token_type == HtmlTokenType::TagOpenStart).unwrap();
        assert_eq!(tag.prefix(), "ns1");
        assert_eq!(tag.name(), "test");
    }

    #[test]
    fn test_self_closing() {
        let tokens = tokenize("<input />");
        assert!(tokens.iter().any(|t| t.token_type == HtmlTokenType::TagOpenEndVoid));
    }

    #[test]
    fn test_interpolation() {
        let tokens = tokenize("{{ value }}");
        let interp = tokens.iter().find(|t| t.token_type == HtmlTokenType::Interpolation).unwrap();
        assert_eq!(interp.parts.len(), 3);
        assert_eq!(interp.parts[0], "{{");
        assert_eq!(interp.parts[1], " value ");
        assert_eq!(interp.parts[2], "}}");
    }

    #[test]
    fn test_block() {
        let tokens = tokenize("@if (cond) {}");
        assert!(
            tokens
                .iter()
                .any(|t| t.token_type == HtmlTokenType::BlockOpenStart && t.value() == "if")
        );
    }

    #[test]
    fn test_comment() {
        let tokens = tokenize("<!-- comment -->");
        assert!(tokens.iter().any(|t| t.token_type == HtmlTokenType::CommentStart));
        assert!(tokens.iter().any(|t| t.token_type == HtmlTokenType::RawText));
        assert!(tokens.iter().any(|t| t.token_type == HtmlTokenType::CommentEnd));
    }

    #[test]
    fn test_attributes() {
        let tokens = tokenize(r#"<div class="foo">"#);
        let attr_name = tokens.iter().find(|t| t.token_type == HtmlTokenType::AttrName).unwrap();
        assert_eq!(attr_name.name(), "class");

        let attr_value =
            tokens.iter().find(|t| t.token_type == HtmlTokenType::AttrValueText).unwrap();
        assert_eq!(attr_value.value(), "foo");

        let quotes: Vec<_> =
            tokens.iter().filter(|t| t.token_type == HtmlTokenType::AttrQuote).collect();
        assert_eq!(quotes.len(), 2);
    }
}

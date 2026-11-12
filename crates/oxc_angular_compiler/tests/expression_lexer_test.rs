//! Expression lexer tests.
//!
//! Ported from Angular's `test/expression_parser/lexer_spec.ts`.
//! This is a 1:1 port - tests should match TypeScript exactly.

use oxc_allocator::Allocator;
use oxc_angular_compiler::parser::expression::{Lexer, StringTokenKind, TokenType};

// ============================================================================
// Helper Functions
// ============================================================================

/// Tokenizes a string and returns the tokens.
fn lex(text: &str) -> Vec<TokenInfo> {
    let allocator = Allocator::default();
    let lexer = Lexer::new(&allocator, text);
    let tokens = lexer.tokenize();
    tokens
        .into_iter()
        .map(|t| TokenInfo {
            token_type: t.token_type,
            index: t.index,
            end: t.end,
            num_value: t.num_value,
            str_value: t.str_value.to_string(),
            str_kind: t.str_kind,
        })
        .collect()
}

/// Token info for testing (owned version of Token).
#[derive(Debug, Clone)]
struct TokenInfo {
    token_type: TokenType,
    index: u32,
    end: u32,
    num_value: f64,
    str_value: String,
    str_kind: StringTokenKind,
}

impl TokenInfo {
    fn is_identifier(&self) -> bool {
        self.token_type == TokenType::Identifier
    }

    fn is_private_identifier(&self) -> bool {
        self.token_type == TokenType::PrivateIdentifier
    }

    fn is_keyword(&self) -> bool {
        self.token_type == TokenType::Keyword
    }

    fn is_keyword_undefined(&self) -> bool {
        self.token_type == TokenType::Keyword && self.str_value == "undefined"
    }

    fn is_keyword_typeof(&self) -> bool {
        self.token_type == TokenType::Keyword && self.str_value == "typeof"
    }

    fn is_keyword_void(&self) -> bool {
        self.token_type == TokenType::Keyword && self.str_value == "void"
    }

    fn is_keyword_in(&self) -> bool {
        self.token_type == TokenType::Keyword && self.str_value == "in"
    }

    fn is_string(&self) -> bool {
        self.token_type == TokenType::String
    }

    fn is_number(&self) -> bool {
        self.token_type == TokenType::Number
    }

    fn is_operator(&self, op: &str) -> bool {
        self.token_type == TokenType::Operator && self.str_value == op
    }

    fn is_character(&self, ch: char) -> bool {
        self.token_type == TokenType::Character && self.num_value == f64::from(ch as u32)
    }

    fn is_error(&self) -> bool {
        self.token_type == TokenType::Error
    }

    fn is_regexp_body(&self) -> bool {
        self.token_type == TokenType::RegExpBody
    }

    fn is_regexp_flags(&self) -> bool {
        self.token_type == TokenType::RegExpFlags
    }
}

/// Checks that a token is valid with the given indices.
fn expect_token(token: &TokenInfo, index: u32, end: u32) {
    assert_eq!(token.index, index, "Token index mismatch");
    assert_eq!(token.end, end, "Token end mismatch");
}

/// Checks that a token is a character token.
fn expect_character_token(token: &TokenInfo, index: u32, end: u32, ch: &str) {
    assert_eq!(ch.len(), 1, "Character must be single character");
    let ch = ch.chars().next().unwrap();
    expect_token(token, index, end);
    assert!(token.is_character(ch), "Expected character token '{ch}', got {token:?}");
}

/// Checks that a token is an operator token.
fn expect_operator_token(token: &TokenInfo, index: u32, end: u32, op: &str) {
    expect_token(token, index, end);
    assert!(token.is_operator(op), "Expected operator token '{op}', got {token:?}");
}

/// Checks that a token is a number token.
fn expect_number_token(token: &TokenInfo, index: u32, end: u32, value: f64) {
    expect_token(token, index, end);
    assert!(token.is_number(), "Expected number token, got {token:?}");
    assert!(
        (token.num_value - value).abs() < f64::EPSILON,
        "Expected number {value}, got {}",
        token.num_value
    );
}

/// Checks that a token is a string token with the expected kind.
fn expect_string_token(
    token: &TokenInfo,
    index: u32,
    end: u32,
    value: &str,
    kind: StringTokenKind,
) {
    expect_token(token, index, end);
    assert!(
        token.is_string()
            || token.token_type == TokenType::NoSubstitutionTemplate
            || token.token_type == TokenType::TemplateHead
            || token.token_type == TokenType::TemplateMiddle
            || token.token_type == TokenType::TemplateTail,
        "Expected string/template token, got {token:?}"
    );
    assert_eq!(token.str_kind, kind, "String kind mismatch");
    assert_eq!(token.str_value, value, "String value mismatch");
}

/// Checks that a token is an identifier token.
fn expect_identifier_token(token: &TokenInfo, index: u32, end: u32, value: &str) {
    expect_token(token, index, end);
    assert!(token.is_identifier(), "Expected identifier token, got {token:?}");
    assert_eq!(token.str_value, value, "Identifier value mismatch");
}

/// Checks that a token is a private identifier token.
fn expect_private_identifier_token(token: &TokenInfo, index: u32, end: u32, value: &str) {
    expect_token(token, index, end);
    assert!(token.is_private_identifier(), "Expected private identifier token, got {token:?}");
    assert_eq!(token.str_value, value, "Private identifier value mismatch");
}

/// Checks that a token is a keyword token.
fn expect_keyword_token(token: &TokenInfo, index: u32, end: u32, value: &str) {
    expect_token(token, index, end);
    assert!(token.is_keyword(), "Expected keyword token, got {token:?}");
    assert_eq!(token.str_value, value, "Keyword value mismatch");
}

/// Checks that a token is an error token with exact message.
fn expect_error_token(token: &TokenInfo, index: u32, end: u32, message: &str) {
    expect_token(token, index, end);
    assert!(token.is_error(), "Expected error token, got {token:?}");
    assert_eq!(token.str_value, message, "Error message mismatch");
}

/// Checks that a token is a regexp body token.
fn expect_regexp_body_token(token: &TokenInfo, index: u32, end: u32, value: &str) {
    expect_token(token, index, end);
    assert!(token.is_regexp_body(), "Expected regexp body token, got {token:?}");
    assert_eq!(token.str_value, value, "RegExp body value mismatch");
}

/// Checks that a token is a regexp flags token.
fn expect_regexp_flags_token(token: &TokenInfo, index: u32, end: u32, value: &str) {
    expect_token(token, index, end);
    assert!(token.is_regexp_flags(), "Expected regexp flags token, got {token:?}");
    assert_eq!(token.str_value, value, "RegExp flags value mismatch");
}

// ============================================================================
// Token Tests - describe('token', ...)
// ============================================================================

mod token {
    use super::*;

    #[test]
    fn should_tokenize_a_simple_identifier() {
        let tokens = lex("j");
        assert_eq!(tokens.len(), 1);
        expect_identifier_token(&tokens[0], 0, 1, "j");
    }

    #[test]
    fn should_tokenize_this() {
        let tokens = lex("this");
        assert_eq!(tokens.len(), 1);
        expect_keyword_token(&tokens[0], 0, 4, "this");
    }

    #[test]
    fn should_tokenize_a_dotted_identifier() {
        let tokens = lex("j.k");
        assert_eq!(tokens.len(), 3);
        expect_identifier_token(&tokens[0], 0, 1, "j");
        expect_character_token(&tokens[1], 1, 2, ".");
        expect_identifier_token(&tokens[2], 2, 3, "k");
    }

    #[test]
    fn should_tokenize_a_private_identifier() {
        let tokens = lex("#a");
        assert_eq!(tokens.len(), 1);
        expect_private_identifier_token(&tokens[0], 0, 2, "#a");
    }

    #[test]
    fn should_tokenize_a_property_access_with_private_identifier() {
        let tokens = lex("j.#k");
        assert_eq!(tokens.len(), 3);
        expect_identifier_token(&tokens[0], 0, 1, "j");
        expect_character_token(&tokens[1], 1, 2, ".");
        expect_private_identifier_token(&tokens[2], 2, 4, "#k");
    }

    #[test]
    fn should_throw_an_invalid_character_error_for_hash() {
        expect_error_token(
            &lex("#")[0],
            0,
            1,
            "Lexer Error: Invalid character [#] at column 0 in expression [#]",
        );
        expect_error_token(
            &lex("#0")[0],
            0,
            1,
            "Lexer Error: Invalid character [#] at column 0 in expression [#0]",
        );
    }

    #[test]
    fn should_tokenize_an_operator() {
        let tokens = lex("j-k");
        assert_eq!(tokens.len(), 3);
        expect_operator_token(&tokens[1], 1, 2, "-");
    }

    #[test]
    fn should_tokenize_an_indexed_operator() {
        let tokens = lex("j[k]");
        assert_eq!(tokens.len(), 4);
        expect_character_token(&tokens[1], 1, 2, "[");
        expect_character_token(&tokens[3], 3, 4, "]");
    }

    #[test]
    fn should_tokenize_a_safe_indexed_operator() {
        let tokens = lex("j?.[k]");
        assert_eq!(tokens.len(), 5);
        expect_operator_token(&tokens[1], 1, 3, "?.");
        expect_character_token(&tokens[2], 3, 4, "[");
        expect_character_token(&tokens[4], 5, 6, "]");
    }

    #[test]
    fn should_tokenize_numbers() {
        let tokens = lex("88");
        assert_eq!(tokens.len(), 1);
        expect_number_token(&tokens[0], 0, 2, 88.0);
    }

    #[test]
    fn should_tokenize_numbers_within_index_ops() {
        expect_number_token(&lex("a[22]")[2], 2, 4, 22.0);
    }

    #[test]
    fn should_tokenize_simple_quoted_strings() {
        expect_string_token(&lex(r#""a""#)[0], 0, 3, "a", StringTokenKind::Plain);
    }

    #[test]
    fn should_tokenize_quoted_strings_with_escaped_quotes() {
        expect_string_token(&lex(r#""a\"""#)[0], 0, 5, "a\"", StringTokenKind::Plain);
    }

    #[test]
    fn should_tokenize_a_string() {
        let tokens = lex(r#"j-a.bc[22]+1.3|f:'a\'c':"d\"e""#);
        expect_identifier_token(&tokens[0], 0, 1, "j");
        expect_operator_token(&tokens[1], 1, 2, "-");
        expect_identifier_token(&tokens[2], 2, 3, "a");
        expect_character_token(&tokens[3], 3, 4, ".");
        expect_identifier_token(&tokens[4], 4, 6, "bc");
        expect_character_token(&tokens[5], 6, 7, "[");
        expect_number_token(&tokens[6], 7, 9, 22.0);
        expect_character_token(&tokens[7], 9, 10, "]");
        expect_operator_token(&tokens[8], 10, 11, "+");
        expect_number_token(&tokens[9], 11, 14, 1.3);
        expect_operator_token(&tokens[10], 14, 15, "|");
        expect_identifier_token(&tokens[11], 15, 16, "f");
        expect_character_token(&tokens[12], 16, 17, ":");
        expect_string_token(&tokens[13], 17, 23, "a'c", StringTokenKind::Plain);
        expect_character_token(&tokens[14], 23, 24, ":");
        expect_string_token(&tokens[15], 24, 30, "d\"e", StringTokenKind::Plain);
    }

    #[test]
    fn should_tokenize_undefined() {
        let tokens = lex("undefined");
        expect_keyword_token(&tokens[0], 0, 9, "undefined");
        assert!(tokens[0].is_keyword_undefined());
    }

    #[test]
    fn should_tokenize_typeof() {
        let tokens = lex("typeof");
        expect_keyword_token(&tokens[0], 0, 6, "typeof");
        assert!(tokens[0].is_keyword_typeof());
    }

    #[test]
    fn should_tokenize_void() {
        let tokens = lex("void");
        expect_keyword_token(&tokens[0], 0, 4, "void");
        assert!(tokens[0].is_keyword_void());
    }

    #[test]
    fn should_tokenize_in_keyword() {
        let tokens = lex("in");
        expect_keyword_token(&tokens[0], 0, 2, "in");
        assert!(tokens[0].is_keyword_in());
    }

    #[test]
    fn should_ignore_whitespace() {
        let tokens = lex("a \t \n \r b");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_identifier_token(&tokens[1], 8, 9, "b");
    }

    #[test]
    fn should_skip_control_characters_like_whitespace() {
        // Angular's `scanToken` skips every character up to the space, NUL among them.
        for input in ["a\0b", "a\x01b"] {
            let tokens = lex(input);
            assert_eq!(tokens.len(), 2);
            expect_identifier_token(&tokens[0], 0, 1, "a");
            expect_identifier_token(&tokens[1], 2, 3, "b");
        }
    }

    #[test]
    fn should_tokenize_quoted_string() {
        let tokens = lex(r#"['\'', "\""]"#);
        expect_string_token(&tokens[1], 1, 5, "'", StringTokenKind::Plain);
        expect_string_token(&tokens[3], 7, 11, "\"", StringTokenKind::Plain);
    }

    #[test]
    fn should_tokenize_escaped_quoted_string() {
        let tokens = lex(r#""\"\n\f\r\t\v\u00A0""#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].str_value, "\"\n\x0c\r\t\x0b\u{00A0}");
    }

    #[test]
    fn should_tokenize_unicode() {
        let tokens = lex(r#""\u00A0""#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].str_value, "\u{00A0}");
    }

    #[test]
    fn should_tokenize_relation() {
        let tokens = lex("! == != < > <= >= === !==");
        expect_operator_token(&tokens[0], 0, 1, "!");
        expect_operator_token(&tokens[1], 2, 4, "==");
        expect_operator_token(&tokens[2], 5, 7, "!=");
        expect_operator_token(&tokens[3], 8, 9, "<");
        expect_operator_token(&tokens[4], 10, 11, ">");
        expect_operator_token(&tokens[5], 12, 14, "<=");
        expect_operator_token(&tokens[6], 15, 17, ">=");
        expect_operator_token(&tokens[7], 18, 21, "===");
        expect_operator_token(&tokens[8], 22, 25, "!==");
    }

    #[test]
    fn should_tokenize_statements() {
        let tokens = lex("a;b;");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_character_token(&tokens[1], 1, 2, ";");
        expect_identifier_token(&tokens[2], 2, 3, "b");
        expect_character_token(&tokens[3], 3, 4, ";");
    }

    #[test]
    fn should_tokenize_function_invocation() {
        let tokens = lex("a()");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_character_token(&tokens[1], 1, 2, "(");
        expect_character_token(&tokens[2], 2, 3, ")");
    }

    #[test]
    fn should_tokenize_simple_method_invocations() {
        let tokens = lex("a.method()");
        expect_identifier_token(&tokens[2], 2, 8, "method");
    }

    #[test]
    fn should_tokenize_method_invocation() {
        let tokens = lex("a.b.c (d) - e.f()");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_character_token(&tokens[1], 1, 2, ".");
        expect_identifier_token(&tokens[2], 2, 3, "b");
        expect_character_token(&tokens[3], 3, 4, ".");
        expect_identifier_token(&tokens[4], 4, 5, "c");
        expect_character_token(&tokens[5], 6, 7, "(");
        expect_identifier_token(&tokens[6], 7, 8, "d");
        expect_character_token(&tokens[7], 8, 9, ")");
        expect_operator_token(&tokens[8], 10, 11, "-");
        expect_identifier_token(&tokens[9], 12, 13, "e");
        expect_character_token(&tokens[10], 13, 14, ".");
        expect_identifier_token(&tokens[11], 14, 15, "f");
        expect_character_token(&tokens[12], 15, 16, "(");
        expect_character_token(&tokens[13], 16, 17, ")");
    }

    #[test]
    fn should_tokenize_safe_function_invocation() {
        let tokens = lex("a?.()");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_operator_token(&tokens[1], 1, 3, "?.");
        expect_character_token(&tokens[2], 3, 4, "(");
        expect_character_token(&tokens[3], 4, 5, ")");
    }

    #[test]
    fn should_tokenize_a_safe_method_invocations() {
        let tokens = lex("a.method?.()");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_character_token(&tokens[1], 1, 2, ".");
        expect_identifier_token(&tokens[2], 2, 8, "method");
        expect_operator_token(&tokens[3], 8, 10, "?.");
        expect_character_token(&tokens[4], 10, 11, "(");
        expect_character_token(&tokens[5], 11, 12, ")");
    }

    #[test]
    fn should_tokenize_number() {
        expect_number_token(&lex("0.5")[0], 0, 3, 0.5);
    }

    #[test]
    fn should_tokenize_multiplication_and_exponentiation() {
        let tokens = lex("1 * 2 ** 3");
        expect_number_token(&tokens[0], 0, 1, 1.0);
        expect_operator_token(&tokens[1], 2, 3, "*");
        expect_number_token(&tokens[2], 4, 5, 2.0);
        expect_operator_token(&tokens[3], 6, 8, "**");
        expect_number_token(&tokens[4], 9, 10, 3.0);
    }

    #[test]
    fn should_tokenize_number_with_exponent() {
        let tokens = lex("0.5E-10");
        assert_eq!(tokens.len(), 1);
        expect_number_token(&tokens[0], 0, 7, 0.5e-10);

        let tokens = lex("0.5E+10");
        expect_number_token(&tokens[0], 0, 7, 0.5e10);
    }

    #[test]
    fn should_return_exception_for_invalid_exponent() {
        expect_error_token(
            &lex("0.5E-")[0],
            4,
            5,
            "Lexer Error: Invalid exponent at column 4 in expression [0.5E-]",
        );

        expect_error_token(
            &lex("0.5E-A")[0],
            4,
            5,
            "Lexer Error: Invalid exponent at column 4 in expression [0.5E-A]",
        );
    }

    #[test]
    fn should_tokenize_number_starting_with_a_dot() {
        expect_number_token(&lex(".5")[0], 0, 2, 0.5);
    }

    #[test]
    fn should_throw_error_on_invalid_unicode() {
        expect_error_token(
            &lex(r"'\u1''bla'")[0],
            2,
            2,
            r"Lexer Error: Invalid unicode escape [\u1''b] at column 2 in expression ['\u1''bla']",
        );
    }

    #[test]
    fn should_tokenize_safe_navigation_operator() {
        expect_operator_token(&lex("?.")[0], 0, 2, "?.");
    }

    #[test]
    fn should_tokenize_nullish_coalescing() {
        expect_operator_token(&lex("??")[0], 0, 2, "??");
    }

    #[test]
    fn should_tokenize_number_starting_with_underscore_as_identifier() {
        expect_identifier_token(&lex("_123")[0], 0, 4, "_123");
        expect_identifier_token(&lex("_123_")[0], 0, 5, "_123_");
        expect_identifier_token(&lex("_1_2_3_")[0], 0, 7, "_1_2_3_");
    }

    #[test]
    fn should_tokenize_number_with_separator() {
        expect_number_token(&lex("123_456")[0], 0, 7, 123_456.0);
        expect_number_token(&lex("1_000_000_000")[0], 0, 13, 1_000_000_000.0);
        expect_number_token(&lex("123_456.78")[0], 0, 10, 123_456.78);
        expect_number_token(&lex("123_456_789.123_456_789")[0], 0, 23, 123_456_789.123_456_79);
        expect_number_token(&lex("1_2_3_4")[0], 0, 7, 1234.0);
        expect_number_token(&lex("1_2_3_4.5_6_7_8")[0], 0, 15, 1234.5678);
    }

    #[test]
    fn should_throw_error_for_invalid_number_separators() {
        expect_error_token(
            &lex("123_")[0],
            3,
            3,
            "Lexer Error: Invalid numeric separator at column 3 in expression [123_]",
        );
        expect_error_token(
            &lex("12__3")[0],
            2,
            2,
            "Lexer Error: Invalid numeric separator at column 2 in expression [12__3]",
        );
        expect_error_token(
            &lex("1_2_3_.456")[0],
            5,
            5,
            "Lexer Error: Invalid numeric separator at column 5 in expression [1_2_3_.456]",
        );
        expect_error_token(
            &lex("1_2_3._456")[0],
            6,
            6,
            "Lexer Error: Invalid numeric separator at column 6 in expression [1_2_3._456]",
        );
    }

    #[test]
    fn should_tokenize_assignment_operators() {
        expect_operator_token(&lex("=")[0], 0, 1, "=");
        expect_operator_token(&lex("+=")[0], 0, 2, "+=");
        expect_operator_token(&lex("-=")[0], 0, 2, "-=");
        expect_operator_token(&lex("*=")[0], 0, 2, "*=");
        expect_operator_token(&lex("a /= b")[1], 2, 4, "/=");
        expect_operator_token(&lex("%=")[0], 0, 2, "%=");
        expect_operator_token(&lex("**=")[0], 0, 3, "**=");
        expect_operator_token(&lex("&&=")[0], 0, 3, "&&=");
        expect_operator_token(&lex("||=")[0], 0, 3, "||=");
        expect_operator_token(&lex("??=")[0], 0, 3, "??=");
    }
}

// ============================================================================
// Template Literals Tests - describe('template literals', ...)
// ============================================================================

mod template_literals {
    use super::*;

    #[test]
    fn should_tokenize_template_literal_with_no_interpolations() {
        let tokens = lex("`hello world`");
        assert_eq!(tokens.len(), 1);
        expect_string_token(&tokens[0], 0, 13, "hello world", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_containing_strings() {
        expect_string_token(
            &lex(r#"`a "b" c`"#)[0],
            0,
            9,
            r#"a "b" c"#,
            StringTokenKind::TemplateLiteralEnd,
        );
        expect_string_token(
            &lex("`a 'b' c`")[0],
            0,
            9,
            "a 'b' c",
            StringTokenKind::TemplateLiteralEnd,
        );
        expect_string_token(
            &lex(r"`a \`b\` c`")[0],
            0,
            11,
            "a `b` c",
            StringTokenKind::TemplateLiteralEnd,
        );
        expect_string_token(
            &lex(r#"`a "'\`b\`'" c`"#)[0],
            0,
            15,
            "a \"'`b`'\" c",
            StringTokenKind::TemplateLiteralEnd,
        );
    }

    #[test]
    fn should_tokenize_unicode_inside_a_template_string() {
        let tokens = lex(r"`\u00A0`");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].str_value, "\u{00A0}");
    }

    #[test]
    fn should_skip_a_nul_inside_an_interpolation() {
        let tokens = lex("`${a\0}`");
        assert_eq!(tokens.len(), 5);
        expect_string_token(&tokens[0], 0, 1, "", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 1, 3, "${");
        expect_identifier_token(&tokens[2], 3, 4, "a");
        expect_character_token(&tokens[3], 5, 6, "}");
        expect_string_token(&tokens[4], 6, 7, "", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_with_interpolation_at_end() {
        let tokens = lex("`hello ${name}`");
        assert_eq!(tokens.len(), 5);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_identifier_token(&tokens[2], 9, 13, "name");
        expect_character_token(&tokens[3], 13, 14, "}");
        expect_string_token(&tokens[4], 14, 15, "", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_with_interpolation_at_beginning() {
        let tokens = lex("`${name} Johnson`");
        assert_eq!(tokens.len(), 5);
        expect_string_token(&tokens[0], 0, 1, "", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 1, 3, "${");
        expect_identifier_token(&tokens[2], 3, 7, "name");
        expect_character_token(&tokens[3], 7, 8, "}");
        expect_string_token(&tokens[4], 8, 17, " Johnson", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_with_interpolation_in_middle() {
        let tokens = lex("`foo${bar}baz`");
        assert_eq!(tokens.len(), 5);
        expect_string_token(&tokens[0], 0, 4, "foo", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 4, 6, "${");
        expect_identifier_token(&tokens[2], 6, 9, "bar");
        expect_character_token(&tokens[3], 9, 10, "}");
        expect_string_token(&tokens[4], 10, 14, "baz", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_be_able_to_use_interpolation_characters_inside_template_string() {
        expect_string_token(&lex("`foo $`")[0], 0, 7, "foo $", StringTokenKind::TemplateLiteralEnd);
        expect_string_token(&lex("`foo }`")[0], 0, 7, "foo }", StringTokenKind::TemplateLiteralEnd);
        expect_string_token(
            &lex("`foo $ {}`")[0],
            0,
            10,
            "foo $ {}",
            StringTokenKind::TemplateLiteralEnd,
        );
        expect_string_token(
            &lex(r"`foo \${bar}`")[0],
            0,
            13,
            "foo ${bar}",
            StringTokenKind::TemplateLiteralEnd,
        );
    }

    #[test]
    fn should_tokenize_template_literal_with_several_interpolations() {
        let tokens = lex("`${a} - ${b} - ${c}`");
        assert_eq!(tokens.len(), 13);
        expect_string_token(&tokens[0], 0, 1, "", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 1, 3, "${");
        expect_identifier_token(&tokens[2], 3, 4, "a");
        expect_character_token(&tokens[3], 4, 5, "}");
        expect_string_token(&tokens[4], 5, 8, " - ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[5], 8, 10, "${");
        expect_identifier_token(&tokens[6], 10, 11, "b");
        expect_character_token(&tokens[7], 11, 12, "}");
        expect_string_token(&tokens[8], 12, 15, " - ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[9], 15, 17, "${");
        expect_identifier_token(&tokens[10], 17, 18, "c");
        expect_character_token(&tokens[11], 18, 19, "}");
        // TypeScript test also doesn't assert tokens[12] (TemplateLiteralEnd)
    }

    #[test]
    fn should_tokenize_template_literal_with_object_literal_inside() {
        let tokens = lex("`foo ${{$: true}} baz`");
        assert_eq!(tokens.len(), 9);
        expect_string_token(&tokens[0], 0, 5, "foo ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 5, 7, "${");
        expect_character_token(&tokens[2], 7, 8, "{");
        expect_identifier_token(&tokens[3], 8, 9, "$");
        expect_character_token(&tokens[4], 9, 10, ":");
        expect_keyword_token(&tokens[5], 11, 15, "true");
        expect_character_token(&tokens[6], 15, 16, "}");
        expect_character_token(&tokens[7], 16, 17, "}");
        expect_string_token(&tokens[8], 17, 22, " baz", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_with_template_literals_inside() {
        let tokens = lex("`foo ${`hello ${`${a} - b`}`} baz`");
        assert_eq!(tokens.len(), 13);
        expect_string_token(&tokens[0], 0, 5, "foo ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 5, 7, "${");
        expect_string_token(&tokens[2], 7, 14, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[3], 14, 16, "${");
        expect_string_token(&tokens[4], 16, 17, "", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[5], 17, 19, "${");
        expect_identifier_token(&tokens[6], 19, 20, "a");
        expect_character_token(&tokens[7], 20, 21, "}");
        expect_string_token(&tokens[8], 21, 26, " - b", StringTokenKind::TemplateLiteralEnd);
        expect_character_token(&tokens[9], 26, 27, "}");
        expect_string_token(&tokens[10], 27, 28, "", StringTokenKind::TemplateLiteralEnd);
        expect_character_token(&tokens[11], 28, 29, "}");
        expect_string_token(&tokens[12], 29, 34, " baz", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_two_template_literals_right_after_each_other() {
        let tokens = lex("`hello ${name}``see ${name} later`");
        assert_eq!(tokens.len(), 10);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_identifier_token(&tokens[2], 9, 13, "name");
        expect_character_token(&tokens[3], 13, 14, "}");
        expect_string_token(&tokens[4], 14, 15, "", StringTokenKind::TemplateLiteralEnd);
        expect_string_token(&tokens[5], 15, 20, "see ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[6], 20, 22, "${");
        expect_identifier_token(&tokens[7], 22, 26, "name");
        expect_character_token(&tokens[8], 26, 27, "}");
        expect_string_token(&tokens[9], 27, 34, " later", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_a_concatenated_template_literal() {
        let tokens = lex("`hello ${name}` + 123");
        assert_eq!(tokens.len(), 7);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_identifier_token(&tokens[2], 9, 13, "name");
        expect_character_token(&tokens[3], 13, 14, "}");
        expect_string_token(&tokens[4], 14, 15, "", StringTokenKind::TemplateLiteralEnd);
        expect_operator_token(&tokens[5], 16, 17, "+");
        expect_number_token(&tokens[6], 18, 21, 123.0);
    }

    #[test]
    fn should_tokenize_template_literal_with_pipe_inside_interpolation() {
        let tokens = lex("`hello ${name | capitalize}!!!`");
        assert_eq!(tokens.len(), 7);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_identifier_token(&tokens[2], 9, 13, "name");
        expect_operator_token(&tokens[3], 14, 15, "|");
        expect_identifier_token(&tokens[4], 16, 26, "capitalize");
        expect_character_token(&tokens[5], 26, 27, "}");
        expect_string_token(&tokens[6], 27, 31, "!!!", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_with_pipe_inside_parenthesized_interpolation() {
        let tokens = lex("`hello ${(name | capitalize)}!!!`");
        assert_eq!(tokens.len(), 9);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_character_token(&tokens[2], 9, 10, "(");
        expect_identifier_token(&tokens[3], 10, 14, "name");
        expect_operator_token(&tokens[4], 15, 16, "|");
        expect_identifier_token(&tokens[5], 17, 27, "capitalize");
        expect_character_token(&tokens[6], 27, 28, ")");
        expect_character_token(&tokens[7], 28, 29, "}");
        expect_string_token(&tokens[8], 29, 33, "!!!", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_template_literal_in_literal_object_value() {
        let tokens = lex("{foo: `${name}`}");
        assert_eq!(tokens.len(), 9);
        expect_character_token(&tokens[0], 0, 1, "{");
        expect_identifier_token(&tokens[1], 1, 4, "foo");
        expect_character_token(&tokens[2], 4, 5, ":");
        expect_string_token(&tokens[3], 6, 7, "", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[4], 7, 9, "${");
        expect_identifier_token(&tokens[5], 9, 13, "name");
        expect_character_token(&tokens[6], 13, 14, "}");
        expect_string_token(&tokens[7], 14, 15, "", StringTokenKind::TemplateLiteralEnd);
        expect_character_token(&tokens[8], 15, 16, "}");
    }

    #[test]
    fn should_produce_error_if_template_literal_not_terminated() {
        expect_error_token(
            &lex("`hello")[0],
            6,
            6,
            "Lexer Error: Unterminated template literal at column 6 in expression [`hello]",
        );
    }

    #[test]
    fn should_produce_error_for_unterminated_template_literal_with_interpolation() {
        let tokens = lex("`hello ${name}!");
        assert_eq!(tokens.len(), 5);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_identifier_token(&tokens[2], 9, 13, "name");
        expect_character_token(&tokens[3], 13, 14, "}");
        expect_error_token(
            &tokens[4],
            15,
            15,
            "Lexer Error: Unterminated template literal at column 15 in expression [`hello ${name}!]",
        );
    }

    #[test]
    fn should_produce_error_for_unterminated_template_literal_interpolation() {
        let tokens = lex("`hello ${name!`");
        assert_eq!(tokens.len(), 5);
        expect_string_token(&tokens[0], 0, 7, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[1], 7, 9, "${");
        expect_identifier_token(&tokens[2], 9, 13, "name");
        expect_operator_token(&tokens[3], 13, 14, "!");
        expect_error_token(
            &tokens[4],
            15,
            15,
            "Lexer Error: Unterminated template literal at column 15 in expression [`hello ${name!`]",
        );
    }

    #[test]
    fn should_tokenize_tagged_template_literal_with_no_interpolations() {
        let tokens = lex("tag`hello world`");
        assert_eq!(tokens.len(), 2);
        expect_identifier_token(&tokens[0], 0, 3, "tag");
        expect_string_token(&tokens[1], 3, 16, "hello world", StringTokenKind::TemplateLiteralEnd);
    }

    #[test]
    fn should_tokenize_nested_tagged_template_literals() {
        let tokens = lex("tag`hello ${tag`world`}`");
        assert_eq!(tokens.len(), 7);
        expect_identifier_token(&tokens[0], 0, 3, "tag");
        expect_string_token(&tokens[1], 3, 10, "hello ", StringTokenKind::TemplateLiteralPart);
        expect_operator_token(&tokens[2], 10, 12, "${");
        expect_identifier_token(&tokens[3], 12, 15, "tag");
        expect_string_token(&tokens[4], 15, 22, "world", StringTokenKind::TemplateLiteralEnd);
        expect_character_token(&tokens[5], 22, 23, "}");
        expect_string_token(&tokens[6], 23, 24, "", StringTokenKind::TemplateLiteralEnd);
    }
}

// ============================================================================
// Regular Expression Tests - describe('regular expressions', ...)
// ============================================================================

mod regular_expressions {
    use super::*;

    #[test]
    fn should_tokenize_a_simple_regex() {
        let tokens = lex("/abc/");
        assert_eq!(tokens.len(), 1);
        expect_regexp_body_token(&tokens[0], 0, 5, "abc");
    }

    #[test]
    fn should_tokenize_a_regex_with_flags() {
        let tokens = lex("/abc/gim");
        assert_eq!(tokens.len(), 2);
        expect_regexp_body_token(&tokens[0], 0, 5, "abc");
        expect_regexp_flags_token(&tokens[1], 5, 8, "gim");
    }

    #[test]
    fn should_tokenize_an_identifier_immediately_after_a_regex() {
        let tokens = lex("/abc/ g");
        assert_eq!(tokens.len(), 2);
        expect_regexp_body_token(&tokens[0], 0, 5, "abc");
        expect_identifier_token(&tokens[1], 6, 7, "g");
    }

    #[test]
    fn should_tokenize_a_regex_with_escaped_slashes() {
        let tokens = lex(r"/^http:\/\/foo\.bar/");
        assert_eq!(tokens.len(), 1);
        expect_regexp_body_token(&tokens[0], 0, 20, r"^http:\/\/foo\.bar");
    }

    #[test]
    fn should_tokenize_a_regex_with_unescaped_slashes_in_character_class() {
        let tokens = lex("/[a/]$/");
        assert_eq!(tokens.len(), 1);
        expect_regexp_body_token(&tokens[0], 0, 7, "[a/]$");
    }

    #[test]
    fn should_tokenize_a_regex_with_a_backslash() {
        let tokens = lex(r"/a\w+/");
        assert_eq!(tokens.len(), 1);
        expect_regexp_body_token(&tokens[0], 0, 6, r"a\w+");
    }

    #[test]
    fn should_tokenize_a_regex_after_an_operator() {
        let tokens = lex("a = /b/");
        assert_eq!(tokens.len(), 3);
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_operator_token(&tokens[1], 2, 3, "=");
        expect_regexp_body_token(&tokens[2], 4, 7, "b");
    }

    #[test]
    fn should_tokenize_a_regex_inside_parentheses() {
        let tokens = lex("log(/a/)");
        assert_eq!(tokens.len(), 4);
        expect_identifier_token(&tokens[0], 0, 3, "log");
        expect_character_token(&tokens[1], 3, 4, "(");
        expect_regexp_body_token(&tokens[2], 4, 7, "a");
        expect_character_token(&tokens[3], 7, 8, ")");
    }

    #[test]
    fn should_tokenize_a_regex_at_beginning_of_array() {
        let tokens = lex("[/a/]");
        assert_eq!(tokens.len(), 3);
        expect_character_token(&tokens[0], 0, 1, "[");
        expect_regexp_body_token(&tokens[1], 1, 4, "a");
        expect_character_token(&tokens[2], 4, 5, "]");
    }

    #[test]
    fn should_tokenize_a_regex_in_middle_of_array() {
        let tokens = lex("[1, /a/, 2]");
        assert_eq!(tokens.len(), 7);
        expect_character_token(&tokens[0], 0, 1, "[");
        expect_number_token(&tokens[1], 1, 2, 1.0);
        expect_character_token(&tokens[2], 2, 3, ",");
        expect_regexp_body_token(&tokens[3], 4, 7, "a");
        expect_character_token(&tokens[4], 7, 8, ",");
        expect_number_token(&tokens[5], 9, 10, 2.0);
        expect_character_token(&tokens[6], 10, 11, "]");
    }

    #[test]
    fn should_tokenize_a_regex_inside_object_literal() {
        let tokens = lex("{a: /b/}");
        assert_eq!(tokens.len(), 5);
        expect_character_token(&tokens[0], 0, 1, "{");
        expect_identifier_token(&tokens[1], 1, 2, "a");
        expect_character_token(&tokens[2], 2, 3, ":");
        expect_regexp_body_token(&tokens[3], 4, 7, "b");
        expect_character_token(&tokens[4], 7, 8, "}");
    }

    #[test]
    fn should_tokenize_a_regex_after_negation_operator() {
        let tokens = lex(r#"log(!/a/.test("1"))"#);
        assert_eq!(tokens.len(), 10);
        expect_identifier_token(&tokens[0], 0, 3, "log");
        expect_character_token(&tokens[1], 3, 4, "(");
        expect_operator_token(&tokens[2], 4, 5, "!");
        expect_regexp_body_token(&tokens[3], 5, 8, "a");
        expect_character_token(&tokens[4], 8, 9, ".");
        expect_identifier_token(&tokens[5], 9, 13, "test");
        expect_character_token(&tokens[6], 13, 14, "(");
        expect_string_token(&tokens[7], 14, 17, "1", StringTokenKind::Plain);
        expect_character_token(&tokens[8], 17, 18, ")");
        expect_character_token(&tokens[9], 18, 19, ")");
    }

    #[test]
    fn should_tokenize_a_regex_after_several_negation_operators() {
        let tokens = lex(r#"log(!!!!!!/a/.test("1"))"#);
        assert_eq!(tokens.len(), 15);
        expect_identifier_token(&tokens[0], 0, 3, "log");
        expect_character_token(&tokens[1], 3, 4, "(");
        expect_operator_token(&tokens[2], 4, 5, "!");
        expect_operator_token(&tokens[3], 5, 6, "!");
        expect_operator_token(&tokens[4], 6, 7, "!");
        expect_operator_token(&tokens[5], 7, 8, "!");
        expect_operator_token(&tokens[6], 8, 9, "!");
        expect_operator_token(&tokens[7], 9, 10, "!");
        expect_regexp_body_token(&tokens[8], 10, 13, "a");
        expect_character_token(&tokens[9], 13, 14, ".");
        expect_identifier_token(&tokens[10], 14, 18, "test");
        expect_character_token(&tokens[11], 18, 19, "(");
        expect_string_token(&tokens[12], 19, 22, "1", StringTokenKind::Plain);
        expect_character_token(&tokens[13], 22, 23, ")");
        expect_character_token(&tokens[14], 23, 24, ")");
    }

    #[test]
    fn should_tokenize_a_method_call_on_a_regex() {
        let tokens = lex(r#"/abc/.test("foo")"#);
        assert_eq!(tokens.len(), 6);
        expect_regexp_body_token(&tokens[0], 0, 5, "abc");
        expect_character_token(&tokens[1], 5, 6, ".");
        expect_identifier_token(&tokens[2], 6, 10, "test");
        expect_character_token(&tokens[3], 10, 11, "(");
        expect_string_token(&tokens[4], 11, 16, "foo", StringTokenKind::Plain);
        expect_character_token(&tokens[5], 16, 17, ")");
    }

    #[test]
    fn should_tokenize_a_method_call_with_a_regex_parameter() {
        let tokens = lex(r#""foo".match(/abc/)"#);
        assert_eq!(tokens.len(), 6);
        expect_string_token(&tokens[0], 0, 5, "foo", StringTokenKind::Plain);
        expect_character_token(&tokens[1], 5, 6, ".");
        expect_identifier_token(&tokens[2], 6, 11, "match");
        expect_character_token(&tokens[3], 11, 12, "(");
        expect_regexp_body_token(&tokens[4], 12, 17, "abc");
        expect_character_token(&tokens[5], 17, 18, ")");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_square_bracket() {
        let tokens = lex("a[0] /= b");
        assert_eq!(tokens.len(), 6);
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_character_token(&tokens[1], 1, 2, "[");
        expect_number_token(&tokens[2], 2, 3, 0.0);
        expect_character_token(&tokens[3], 3, 4, "]");
        expect_operator_token(&tokens[4], 5, 7, "/=");
        expect_identifier_token(&tokens[5], 8, 9, "b");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_identifier() {
        let tokens = lex("a / b");
        assert_eq!(tokens.len(), 3);
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_operator_token(&tokens[1], 2, 3, "/");
        expect_identifier_token(&tokens[2], 4, 5, "b");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_number() {
        let tokens = lex("1 / b");
        assert_eq!(tokens.len(), 3);
        expect_number_token(&tokens[0], 0, 1, 1.0);
        expect_operator_token(&tokens[1], 2, 3, "/");
        expect_identifier_token(&tokens[2], 4, 5, "b");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_string() {
        let tokens = lex(r#""a" / b"#);
        assert_eq!(tokens.len(), 3);
        expect_string_token(&tokens[0], 0, 3, "a", StringTokenKind::Plain);
        expect_operator_token(&tokens[1], 4, 5, "/");
        expect_identifier_token(&tokens[2], 6, 7, "b");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_closing_parenthesis() {
        let tokens = lex("(a) / b");
        assert_eq!(tokens.len(), 5);
        expect_character_token(&tokens[0], 0, 1, "(");
        expect_identifier_token(&tokens[1], 1, 2, "a");
        expect_character_token(&tokens[2], 2, 3, ")");
        expect_operator_token(&tokens[3], 4, 5, "/");
        expect_identifier_token(&tokens[4], 6, 7, "b");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_keyword() {
        let tokens = lex("this / b");
        assert_eq!(tokens.len(), 3);
        expect_keyword_token(&tokens[0], 0, 4, "this");
        expect_operator_token(&tokens[1], 5, 6, "/");
        expect_identifier_token(&tokens[2], 7, 8, "b");
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_non_null_assertion_on_identifier() {
        let tokens = lex("foo! / 2");
        assert_eq!(tokens.len(), 4);
        expect_identifier_token(&tokens[0], 0, 3, "foo");
        expect_operator_token(&tokens[1], 3, 4, "!");
        expect_operator_token(&tokens[2], 5, 6, "/");
        expect_number_token(&tokens[3], 7, 8, 2.0);
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_non_null_assertion_on_function_call() {
        let tokens = lex("foo()! / 2");
        assert_eq!(tokens.len(), 6);
        expect_identifier_token(&tokens[0], 0, 3, "foo");
        expect_character_token(&tokens[1], 3, 4, "(");
        expect_character_token(&tokens[2], 4, 5, ")");
        expect_operator_token(&tokens[3], 5, 6, "!");
        expect_operator_token(&tokens[4], 7, 8, "/");
        expect_number_token(&tokens[5], 9, 10, 2.0);
    }

    #[test]
    fn should_not_tokenize_regex_preceded_by_non_null_assertion_on_array() {
        let tokens = lex("[1]! / 2");
        assert_eq!(tokens.len(), 6);
        expect_character_token(&tokens[0], 0, 1, "[");
        expect_number_token(&tokens[1], 1, 2, 1.0);
        expect_character_token(&tokens[2], 2, 3, "]");
        expect_operator_token(&tokens[3], 3, 4, "!");
        expect_operator_token(&tokens[4], 5, 6, "/");
        expect_number_token(&tokens[5], 7, 8, 2.0);
    }

    #[test]
    fn should_not_tokenize_consecutive_regexes() {
        let tokens = lex("/ 1 / 2 / 3 / 4");
        assert_eq!(tokens.len(), 6);
        expect_regexp_body_token(&tokens[0], 0, 5, " 1 ");
        expect_number_token(&tokens[1], 6, 7, 2.0);
        expect_operator_token(&tokens[2], 8, 9, "/");
        expect_number_token(&tokens[3], 10, 11, 3.0);
        expect_operator_token(&tokens[4], 12, 13, "/");
        expect_number_token(&tokens[5], 14, 15, 4.0);
    }

    #[test]
    fn should_not_tokenize_regex_like_characters_inside_pipe() {
        let tokens = lex("foo / 1000 | date: 'M/d/yy'");
        assert_eq!(tokens.len(), 7);
        expect_identifier_token(&tokens[0], 0, 3, "foo");
        expect_operator_token(&tokens[1], 4, 5, "/");
        expect_number_token(&tokens[2], 6, 10, 1000.0);
        expect_operator_token(&tokens[3], 11, 12, "|");
        expect_identifier_token(&tokens[4], 13, 17, "date");
        expect_character_token(&tokens[5], 17, 18, ":");
        expect_string_token(&tokens[6], 19, 27, "M/d/yy", StringTokenKind::Plain);
    }

    #[test]
    fn should_produce_error_for_unterminated_regex() {
        expect_error_token(
            &lex("/a")[0],
            2,
            2,
            "Lexer Error: Unterminated regular expression at column 2 in expression [/a]",
        );
    }
}

// ============================================================================
// Error Tests (RUST-ONLY ADDITIONS - NOT IN TYPESCRIPT)
// These tests are additional coverage not present in the original TypeScript
// lexer_spec.ts. They use lenient assertions to verify error tokens exist.
// ============================================================================

mod errors {
    use super::*;

    #[test]
    fn should_report_a_nul_in_a_string_as_unterminated() {
        // As in Angular: the string ends at the NUL, `b` is read on its own, and the last quote opens another string.
        let tokens = lex("'a\0b'");
        assert_eq!(tokens.len(), 3);
        assert!(tokens[0].is_error());
        expect_identifier_token(&tokens[1], 3, 4, "b");
        assert!(tokens[2].is_error());
    }

    #[test]
    fn should_report_unterminated_string() {
        let tokens = lex(r#""hello"#);
        assert!(tokens.iter().any(super::TokenInfo::is_error));
    }

    #[test]
    fn should_tokenize_increment_decrement_operators() {
        let tokens = lex("++a");
        expect_operator_token(&tokens[0], 0, 2, "++");
        expect_identifier_token(&tokens[1], 2, 3, "a");

        let tokens = lex("a--");
        expect_identifier_token(&tokens[0], 0, 1, "a");
        expect_operator_token(&tokens[1], 1, 3, "--");
    }

    #[test]
    fn should_tokenize_increment_decrement_next_to_plus_minus() {
        // The longest operator wins, so `+++` is `++` then `+`.
        let tokens = lex("a+++b");
        assert_eq!(tokens.len(), 4);
        expect_operator_token(&tokens[1], 1, 3, "++");
        expect_operator_token(&tokens[2], 3, 4, "+");

        let tokens = lex("a+--b");
        expect_operator_token(&tokens[1], 1, 2, "+");
        expect_operator_token(&tokens[2], 2, 4, "--");

        let tokens = lex("a+++=b");
        expect_operator_token(&tokens[1], 1, 3, "++");
        expect_operator_token(&tokens[2], 3, 5, "+=");
    }

    #[test]
    fn should_tokenize_increment_inside_template_literal_interpolation() {
        let tokens = lex("`${a++}`");
        assert!(tokens.iter().all(|t| !t.is_error()));
        assert!(tokens.iter().any(|t| t.is_operator("++")));
    }
}

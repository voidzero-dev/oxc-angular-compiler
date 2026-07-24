//! HTML Lexer tests.
//!
//! Ported from Angular's `test/ml_parser/lexer_spec.ts`.

use oxc_angular_compiler::parser::html::{HtmlLexer, HtmlTokenType, HtmlTokenizeResult};

// ============================================================================
// Helper Functions
// ============================================================================

/// Tokenizes HTML and returns the result.
fn tokenize(input: &str) -> HtmlTokenizeResult {
    HtmlLexer::new(input).tokenize()
}

/// Tokenizes HTML with selectorless mode enabled.
fn tokenize_selectorless(input: &str) -> HtmlTokenizeResult {
    HtmlLexer::new(input).with_selectorless(true).tokenize()
}

/// Tokenizes HTML with expansion forms enabled.
fn tokenize_expansion_forms(input: &str) -> HtmlTokenizeResult {
    HtmlLexer::new(input).with_expansion_forms(true).tokenize()
}

/// Tokenizes and returns humanized parts.
fn tokenize_and_humanize_parts(input: &str) -> Vec<(HtmlTokenType, Vec<String>)> {
    let result = tokenize(input);
    result.tokens.into_iter().map(|t| (t.token_type, t.parts)).collect()
}

/// Tokenizes with expansion forms and returns humanized parts.
fn tokenize_expansion_and_humanize_parts(input: &str) -> Vec<(HtmlTokenType, Vec<String>)> {
    let result = tokenize_expansion_forms(input);
    result.tokens.into_iter().map(|t| (t.token_type, t.parts)).collect()
}

/// Tokenizes with selectorless mode and returns humanized parts.
fn tokenize_selectorless_and_humanize_parts(input: &str) -> Vec<(HtmlTokenType, Vec<String>)> {
    let result = tokenize_selectorless(input);
    result.tokens.into_iter().map(|t| (t.token_type, t.parts)).collect()
}

// ============================================================================
// Basic Tag Tests
// ============================================================================

mod basic_tags {
    use super::*;

    #[test]
    fn should_parse_open_tags_without_prefix() {
        // TS: it("should parse open tags without prefix", ...)
        let result = tokenize_and_humanize_parts("<test>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "test".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_open_tags_with_prefix() {
        // TS: it("should parse namespace prefix", ...)
        let result = tokenize_and_humanize_parts("<ns1:test>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec!["ns1".to_string(), "test".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_close_tags_without_prefix() {
        // TS: it("should parse close tags without prefix", ...)
        let result = tokenize_and_humanize_parts("</test>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagClose, vec![String::new(), "test".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_close_tags_with_prefix() {
        // TS: it("should parse close tags with prefix", ...)
        let result = tokenize_and_humanize_parts("</ns1:test>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagClose, vec!["ns1".to_string(), "test".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_allow_whitespace_in_close_tags() {
        // TS: it("should allow whitespace", ...)
        let result = tokenize_and_humanize_parts("</ test >");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagClose, vec![String::new(), "test".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_report_missing_name_after_close_tag_start() {
        let result = tokenize("</");
        assert!(!result.errors.is_empty(), "Should report error for missing name after </");
    }

    #[test]
    fn should_report_missing_close_tag_end() {
        let result = tokenize("</test");
        assert!(!result.errors.is_empty(), "Should report error for missing >");
    }

    #[test]
    fn should_parse_void_tags() {
        // TS: it("should parse void tags", ...)
        let result = tokenize_and_humanize_parts("<test/>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "test".to_string()]),
                (HtmlTokenType::TagOpenEndVoid, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_tokenize_simple_open_tag() {
        let result = tokenize("<div>");
        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(types.contains(&HtmlTokenType::TagOpenEnd));
        assert!(types.contains(&HtmlTokenType::Eof));
    }

    #[test]
    fn should_tokenize_open_and_close_tags() {
        let result = tokenize("<div></div>");
        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        // Should have tag open start, tag open end, tag close, and EOF
        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(types.contains(&HtmlTokenType::TagOpenEnd));
        assert!(types.contains(&HtmlTokenType::TagClose));
        assert!(types.contains(&HtmlTokenType::Eof));
    }

    #[test]
    fn should_tokenize_self_closing_tag() {
        let result = tokenize("<br/>");
        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(
            types.contains(&HtmlTokenType::TagOpenEndVoid)
                || types.contains(&HtmlTokenType::TagClose)
        );
        assert!(types.contains(&HtmlTokenType::Eof));
    }

    #[test]
    fn should_tokenize_void_element() {
        let result = tokenize("<input>");
        assert!(result.errors.is_empty(), "Should not have errors");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();
        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(types.contains(&HtmlTokenType::Eof));
    }

    #[test]
    fn should_tokenize_element_with_text() {
        let result = tokenize("<div>hello</div>");
        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(types.contains(&HtmlTokenType::Text));
        assert!(types.contains(&HtmlTokenType::Eof));

        // Check text content
        let text_token = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);
        assert!(text_token.is_some());
        assert_eq!(text_token.unwrap().value(), "hello");
    }

    #[test]
    fn should_tokenize_nested_elements() {
        let result = tokenize("<div><span>inner</span></div>");

        let tag_opens: Vec<_> =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::TagOpenStart).collect();

        // Should have both div and span open tags
        assert!(tag_opens.len() >= 2);
    }

    #[test]
    fn should_preserve_tag_name_case() {
        let result = tokenize("<DiV></DiV>");

        // In Angular's lexer, TagOpenStart has parts [prefix, name]
        let tag_open =
            result.tokens.iter().find(|t| t.token_type == HtmlTokenType::TagOpenStart).unwrap();

        assert_eq!(
            tag_open.name(),
            "DiV",
            "Tag name should preserve case, got: {:?}",
            tag_open.parts
        );
    }
}

// ============================================================================
// Attribute Tests
// ============================================================================

mod attributes {
    use super::*;

    #[test]
    fn should_tokenize_attribute_without_value() {
        let result = tokenize("<div disabled></div>");

        let attr_names: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::AttrName)
            .map(oxc_angular_compiler::parser::html::HtmlToken::name)
            .collect();

        assert!(
            attr_names.contains(&"disabled"),
            "Should have 'disabled' attribute, got: {attr_names:?}"
        );
    }

    #[test]
    fn should_tokenize_attribute_with_unquoted_value() {
        let result = tokenize("<div foo=bar></div>");

        let has_attr_name = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrName && t.name() == "foo");
        let has_attr_value = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrValueText && t.value() == "bar");

        assert!(has_attr_name, "Should have attribute name 'foo'");
        assert!(has_attr_value, "Should have attribute value 'bar'");
    }

    #[test]
    fn should_tokenize_attribute_with_single_quoted_value() {
        let result = tokenize("<div foo='bar'></div>");

        let has_attr_value = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrValueText && t.value() == "bar");

        assert!(has_attr_value, "Should have attribute value 'bar'");
    }

    #[test]
    fn should_tokenize_attribute_with_double_quoted_value() {
        let result = tokenize(r#"<div foo="bar"></div>"#);

        let has_attr_value = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrValueText && t.value() == "bar");

        assert!(has_attr_value, "Should have attribute value 'bar'");
    }

    #[test]
    fn should_tokenize_multiple_attributes() {
        let result = tokenize(r#"<div a="1" b="2" c="3"></div>"#);

        let attr_names: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::AttrName)
            .map(oxc_angular_compiler::parser::html::HtmlToken::name)
            .collect();

        // Should have "a", "b", "c" among the attributes
        assert!(attr_names.contains(&"a"), "Missing 'a', got: {attr_names:?}");
        assert!(attr_names.contains(&"b"), "Missing 'b', got: {attr_names:?}");
        assert!(attr_names.contains(&"c"), "Missing 'c', got: {attr_names:?}");
    }

    #[test]
    fn should_tokenize_angular_property_binding() {
        let result = tokenize(r#"<div [prop]="expr"></div>"#);

        let has_attr_name = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrName && t.name() == "[prop]");

        assert!(has_attr_name, "Should have attribute name '[prop]'");
    }

    #[test]
    fn should_tokenize_angular_event_binding() {
        let result = tokenize(r#"<div (click)="handler()"></div>"#);

        let has_attr_name = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrName && t.name() == "(click)");

        assert!(has_attr_name, "Should have attribute name '(click)'");
    }

    #[test]
    fn should_tokenize_angular_two_way_binding() {
        let result = tokenize(r#"<input [(ngModel)]="value">"#);

        let has_attr_name = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrName && t.name() == "[(ngModel)]");

        assert!(has_attr_name, "Should have attribute name '[(ngModel)]'");
    }

    #[test]
    fn should_tokenize_template_reference() {
        let result = tokenize("<div #myRef></div>");

        let has_attr_name = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrName && t.name() == "#myRef");

        assert!(has_attr_name, "Should have attribute name '#myRef'");
    }

    #[test]
    fn should_tokenize_structural_directive() {
        let result = tokenize(r#"<div *ngIf="condition"></div>"#);

        let has_attr_name = result
            .tokens
            .iter()
            .any(|t| t.token_type == HtmlTokenType::AttrName && t.name() == "*ngIf");

        assert!(has_attr_name, "Should have attribute name '*ngIf'");
    }

    // Additional attribute tests ported from Angular's lexer_spec.ts

    #[test]
    fn should_parse_attributes_without_prefix() {
        // TS: it("should parse attributes without prefix", ...)
        let result = tokenize_and_humanize_parts("<t a>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_attributes_with_prefix() {
        // TS: it("should parse attributes with prefix", ...)
        let result = tokenize_and_humanize_parts("<t ns1:a>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec!["ns1".to_string(), "a".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_attributes_whose_prefix_is_not_valid() {
        let result = tokenize_and_humanize_parts("<t (ns1:a)>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "(ns1:a)".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_attributes_with_single_quote_value() {
        // TS: it("should parse attributes with single quote value", ...)
        let result = tokenize_and_humanize_parts("<t a='b'>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["'".to_string()]),
                (HtmlTokenType::AttrValueText, vec!["b".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["'".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_attributes_with_double_quote_value() {
        // TS: it("should parse attributes with double quote value", ...)
        let result = tokenize_and_humanize_parts(r#"<t a="b">"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["\"".to_string()]),
                (HtmlTokenType::AttrValueText, vec!["b".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["\"".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_attributes_with_unquoted_value() {
        // TS: it("should parse attributes with unquoted value", ...)
        let result = tokenize_and_humanize_parts("<t a=b>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrValueText, vec!["b".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_attributes_with_empty_quoted_value() {
        let result = tokenize_and_humanize_parts(r#"<t a="">"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["\"".to_string()]),
                (HtmlTokenType::AttrValueText, vec![String::new()]),
                (HtmlTokenType::AttrQuote, vec!["\"".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_allow_whitespace_in_attributes() {
        // TS: it("should allow whitespace", ...)
        let result = tokenize_and_humanize_parts("<t a = b >");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrValueText, vec!["b".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_values_with_cr_and_lf() {
        // TS: it("should parse values with CR and LF", ...)
        let result = tokenize_and_humanize_parts("<t a='t\ne\rs\r\nt'>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "t".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["'".to_string()]),
                (HtmlTokenType::AttrValueText, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["'".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_report_missing_closing_single_quote() {
        let result = tokenize("<t a='b>");
        assert!(!result.errors.is_empty(), "Should report error for missing closing quote");
    }

    #[test]
    fn should_report_missing_closing_double_quote() {
        let result = tokenize(r#"<t a="b>"#);
        assert!(!result.errors.is_empty(), "Should report error for missing closing quote");
    }
}

// ============================================================================
// Comment Tests
// ============================================================================

mod comments {
    use super::*;

    #[test]
    fn should_parse_comments() {
        // TS: it("should parse comments", ...)
        // Note: Line endings are normalized per HTML5 spec
        let result = tokenize_and_humanize_parts("<!--t\ne\rs\r\nt-->");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::CommentStart, vec![]),
                (HtmlTokenType::RawText, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::CommentEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_tokenize_comment() {
        let result = tokenize("<!-- comment -->");

        // Angular produces CommentStart + RawText + CommentEnd
        let has_comment_start =
            result.tokens.iter().any(|t| t.token_type == HtmlTokenType::CommentStart);
        let raw_text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::RawText);

        assert!(has_comment_start, "Should have CommentStart token");
        assert!(raw_text.is_some(), "Should have RawText token");
        assert!(raw_text.unwrap().value().contains("comment"), "Comment should contain 'comment'");
    }

    #[test]
    fn should_tokenize_empty_comment() {
        let result = tokenize("<!---->");

        let has_comment_start =
            result.tokens.iter().any(|t| t.token_type == HtmlTokenType::CommentStart);

        assert!(has_comment_start, "Should have CommentStart token");
    }

    #[test]
    fn should_tokenize_comment_with_dashes() {
        let result = tokenize("<!-- test -- -->");

        let has_comment_start =
            result.tokens.iter().any(|t| t.token_type == HtmlTokenType::CommentStart);

        assert!(has_comment_start, "Should have CommentStart token");
    }

    #[test]
    fn should_tokenize_multiline_comment() {
        let result = tokenize("<!-- line1\nline2\nline3 -->");

        let raw_text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::RawText);

        assert!(raw_text.is_some(), "Should have RawText token");
        let value = raw_text.unwrap().value();
        assert!(value.contains("line1") && value.contains("line2") && value.contains("line3"));
    }

    #[test]
    fn should_accept_comments_finishing_by_too_many_dashes_even() {
        // TS: it("should accept comments finishing by too many dashes (even number)", ...)
        let result = tokenize_and_humanize_parts("<!-- test ---->");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::CommentStart, vec![]),
                (HtmlTokenType::RawText, vec![" test --".to_string()]),
                (HtmlTokenType::CommentEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_accept_comments_finishing_by_too_many_dashes_odd() {
        // TS: it("should accept comments finishing by too many dashes (odd number)", ...)
        let result = tokenize_and_humanize_parts("<!-- test --->");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::CommentStart, vec![]),
                (HtmlTokenType::RawText, vec![" test -".to_string()]),
                (HtmlTokenType::CommentEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_report_missing_end_comment() {
        // TS: it("should report missing end comment", ...)
        let result = tokenize("<!--");
        assert!(!result.errors.is_empty(), "Should report an error for missing end comment");
    }
}

// ============================================================================
// Text Tests
// ============================================================================

mod text {
    use super::*;

    #[test]
    fn should_tokenize_plain_text() {
        let result = tokenize("hello world");

        let text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);

        assert!(text.is_some());
        assert_eq!(text.unwrap().value(), "hello world");
    }

    #[test]
    fn should_tokenize_text_with_entities() {
        // Entities are now tokenized as EncodedEntity tokens with empty TEXT tokens around them
        let result = tokenize_and_humanize_parts("&amp; &lt; &gt;");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::EncodedEntity, vec!["&".to_string(), "&amp;".to_string()]),
                (HtmlTokenType::Text, vec![" ".to_string()]),
                (HtmlTokenType::EncodedEntity, vec!["<".to_string(), "&lt;".to_string()]),
                (HtmlTokenType::Text, vec![" ".to_string()]),
                (HtmlTokenType::EncodedEntity, vec![">".to_string(), "&gt;".to_string()]),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_tokenize_text_before_element() {
        let result = tokenize("before<div></div>");

        let text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);

        assert!(text.is_some());
        assert_eq!(text.unwrap().value(), "before");
    }

    #[test]
    fn should_tokenize_text_after_element() {
        let result = tokenize("<div></div>after");

        let texts: Vec<_> =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::Text).collect();

        assert!(!texts.is_empty());
        assert!(texts.iter().any(|t| t.value() == "after"));
    }

    #[test]
    fn should_normalize_crlf_in_text() {
        let result = tokenize("line1\r\nline2");

        let text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);

        assert!(text.is_some());
        // CRLF should be normalized to LF
        assert!(!text.unwrap().value().contains('\r'), "Should normalize CRLF to LF");
    }

    // Additional text tests ported from Angular's lexer_spec.ts

    #[test]
    fn should_parse_text() {
        // TS: it("should parse text", ...)
        let result = tokenize_and_humanize_parts("a");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["a".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_handle_cr_and_lf_in_text() {
        // TS: it("should handle CR & LF in text", ...)
        let result = tokenize_and_humanize_parts("t\ne\rs\r\nt");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_text_starting_with_ampersand() {
        // TS: it('should parse text starting with "&"', ...)
        let result = tokenize_and_humanize_parts("a && b &");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["a && b &".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_allow_less_than_in_text_nodes() {
        // TS: it('should allow "<" in text nodes', ...)
        // Angular: "< a>" is text because there's a space after <
        let result = tokenize_and_humanize_parts("< a>");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["< a>".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }
}

// ============================================================================
// Interpolation Tests
// ============================================================================

mod interpolation {
    use super::*;

    #[test]
    fn should_tokenize_interpolation() {
        let result = tokenize("{{expr}}");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        // Angular uses a single Interpolation token with parts [startMarker, expr, endMarker]
        assert!(
            types.contains(&HtmlTokenType::Interpolation) || types.contains(&HtmlTokenType::Text)
        );
    }

    #[test]
    fn should_tokenize_interpolation_in_text() {
        let result = tokenize("before {{expr}} after");

        // Should have text parts and interpolation
        let has_interpolation =
            result.tokens.iter().any(|t| t.token_type == HtmlTokenType::Interpolation);
        assert!(has_interpolation, "Should have interpolation token");
    }

    #[test]
    fn should_tokenize_interpolation_in_attribute() {
        let result = tokenize(r#"<div foo="{{expr}}"></div>"#);

        // Interpolation in attribute value produces AttrValueInterpolation
        let has_interp =
            result.tokens.iter().any(|t| t.token_type == HtmlTokenType::AttrValueInterpolation);

        assert!(has_interp, "Should have AttrValueInterpolation token");
    }

    #[test]
    fn should_tokenize_multiple_interpolations() {
        let result = tokenize("{{a}} and {{b}}");

        let interp_count =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::Interpolation).count();

        assert!(interp_count >= 2, "Should have at least 2 interpolation tokens");
    }

    // Additional interpolation tests ported from Angular's lexer_spec.ts

    #[test]
    fn should_parse_interpolation_with_comment() {
        // TS: it("should parse interpolation", ...) - part with comments
        let result = tokenize_and_humanize_parts("{{ c // comment }}");
        // Should have interpolation with comment inside
        let interp = result.iter().find(|(t, _)| *t == HtmlTokenType::Interpolation);
        assert!(interp.is_some(), "Should have interpolation token");
        if let Some((_, parts)) = interp {
            assert!(parts.len() >= 2, "Should have at least start and expression parts");
        }
    }

    #[test]
    fn should_parse_interpolation_with_quotes() {
        // TS: it("should parse interpolation", ...) - part with quotes
        let result = tokenize("{{ e \"}} ' \" f }}");
        let interp = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Interpolation);
        assert!(interp.is_some(), "Should have interpolation token");
    }

    #[test]
    fn should_handle_cr_and_lf_in_interpolation() {
        // TS: it("should handle CR & LF in interpolation", ...)
        // Angular normalizes CRLF to LF inside interpolations
        let result = tokenize_and_humanize_parts("{{t\ne\rs\r\nt}}");
        let interp = result.iter().find(|(t, _)| *t == HtmlTokenType::Interpolation);
        assert!(interp.is_some(), "Should have interpolation token");
        if let Some((_, parts)) = interp {
            // Expression should have normalized line endings
            assert!(parts.len() >= 2, "Should have parts");
            // The expression part (usually index 1) should have normalized LF
            if parts.len() > 1 {
                assert!(!parts[1].contains("\r\n"), "Should normalize CRLF to LF");
            }
        }
    }
}

// ============================================================================
// Block Tests (@if, @for, etc.)
// ============================================================================

mod blocks {
    use super::*;

    #[test]
    fn should_tokenize_if_block() {
        let result = tokenize("@if (condition) { content }");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::BlockOpenStart), "Should have BlockOpenStart");
        assert!(types.contains(&HtmlTokenType::BlockOpenEnd), "Should have BlockOpenEnd");
        assert!(types.contains(&HtmlTokenType::BlockClose), "Should have BlockClose");
    }

    #[test]
    fn should_tokenize_if_block_name() {
        let result = tokenize("@if (condition) { content }");

        let block_start =
            result.tokens.iter().find(|t| t.token_type == HtmlTokenType::BlockOpenStart);

        assert!(block_start.is_some());
        assert_eq!(block_start.unwrap().value(), "if");
    }

    #[test]
    fn should_tokenize_block_parameter() {
        let result = tokenize("@if (a === 1) { content }");

        let param = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::BlockParameter);

        assert!(param.is_some(), "Should have block parameter");
        assert!(param.unwrap().value().contains('a'));
    }

    #[test]
    fn should_tokenize_for_block() {
        let result = tokenize("@for (item of items; track item.id) { content }");

        let block_start =
            result.tokens.iter().find(|t| t.token_type == HtmlTokenType::BlockOpenStart);

        assert!(block_start.is_some());
        assert_eq!(block_start.unwrap().value(), "for");
    }

    #[test]
    fn should_tokenize_for_block_parameters() {
        let result = tokenize("@for (item of items; track item.id) { content }");

        let params: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::BlockParameter)
            .collect();

        // Should have at least one parameter
        assert!(!params.is_empty(), "Should have block parameters");
    }

    #[test]
    fn should_tokenize_switch_block() {
        let result = tokenize("@switch (expr) { @case (1) { one } }");

        let block_starts: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::BlockOpenStart)
            .map(oxc_angular_compiler::parser::html::HtmlToken::value)
            .collect();

        assert!(block_starts.contains(&"switch"));
        assert!(block_starts.contains(&"case"));
    }

    #[test]
    fn should_tokenize_defer_block() {
        let result = tokenize("@defer { content }");

        let block_start =
            result.tokens.iter().find(|t| t.token_type == HtmlTokenType::BlockOpenStart);

        assert!(block_start.is_some());
        assert_eq!(block_start.unwrap().value(), "defer");
    }

    #[test]
    fn should_tokenize_defer_with_trigger() {
        let result = tokenize("@defer (on viewport) { content }");

        let param = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::BlockParameter);

        assert!(param.is_some());
        assert!(param.unwrap().value().contains("viewport"));
    }

    #[test]
    fn should_tokenize_nested_blocks() {
        let result = tokenize("@if (a) { @if (b) { nested } }");

        let block_starts: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::BlockOpenStart)
            .collect();

        let block_closes: Vec<_> =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::BlockClose).collect();

        assert_eq!(block_starts.len(), 2, "Should have 2 block starts");
        assert_eq!(block_closes.len(), 2, "Should have 2 block closes");
    }

    #[test]
    fn should_tokenize_block_with_html_content() {
        let result = tokenize("@if (cond) { <div>content</div> }");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::BlockOpenStart));
        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(types.contains(&HtmlTokenType::BlockClose));
    }

    #[test]
    fn should_tokenize_else_block() {
        let result = tokenize("@if (cond) { a } @else { b }");

        let block_starts: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::BlockOpenStart)
            .map(oxc_angular_compiler::parser::html::HtmlToken::value)
            .collect();

        assert!(block_starts.contains(&"if"));
        assert!(block_starts.contains(&"else"));
    }

    // Additional block tests ported from Angular's lexer_spec.ts

    #[test]
    fn should_parse_a_block_without_parameters() {
        // TS: it("should parse a block without parameters", ...)
        let result = tokenize_and_humanize_parts("@if {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_block_with_empty_parens() {
        // TS: it("should parse a block without parameters", ...) - variant with ()
        let result = tokenize_and_humanize_parts("@if () {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_block_with_parameters() {
        // TS: it("should parse a block with parameters", ...)
        let result = tokenize_and_humanize_parts("@for (item of items; track item.id) {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["for".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["item of items".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["track item.id".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_block_with_trailing_semicolon() {
        // TS: it("should parse a block with a trailing semicolon after the parameters", ...)
        let result = tokenize_and_humanize_parts("@for (item of items;) {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["for".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["item of items".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_block_with_space_in_name() {
        let result = tokenize_and_humanize_parts("@else if {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["else if".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_else_if_block_with_params() {
        let result = tokenize_and_humanize_parts("@else if (foo !== 2) {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["else if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["foo !== 2".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_defer_block_with_trailing_whitespace() {
        // TS: it("should parse a block with trailing whitespace", ...)
        let result = tokenize_and_humanize_parts("@defer                        {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["defer".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_block_with_arbitrary_spaces_around_parentheses() {
        // TS: it("should parse a block with an arbitrary amount of spaces around the parentheses", ...)
        let expected = vec![
            (HtmlTokenType::BlockOpenStart, vec!["for".to_string()]),
            (HtmlTokenType::BlockParameter, vec!["a".to_string()]),
            (HtmlTokenType::BlockParameter, vec!["b".to_string()]),
            (HtmlTokenType::BlockParameter, vec!["c".to_string()]),
            (HtmlTokenType::BlockOpenEnd, vec![]),
            (HtmlTokenType::Text, vec!["hello".to_string()]),
            (HtmlTokenType::BlockClose, vec![]),
            (HtmlTokenType::Eof, vec![]),
        ];

        assert_eq!(tokenize_and_humanize_parts("@for(a; b; c){hello}"), expected);
        assert_eq!(tokenize_and_humanize_parts("@for      (a; b; c)      {hello}"), expected);
        assert_eq!(tokenize_and_humanize_parts("@for(a; b; c)      {hello}"), expected);
        assert_eq!(tokenize_and_humanize_parts("@for      (a; b; c){hello}"), expected);
    }

    #[test]
    fn should_parse_a_block_with_multiple_trailing_semicolons() {
        // TS: it("should parse a block with multiple trailing semicolons", ...)
        let result = tokenize_and_humanize_parts("@for (item of items;;;;;) {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["for".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["item of items".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_block_with_no_trailing_semicolon() {
        // TS: it("should parse a block with no trailing semicolon", ...)
        let result = tokenize_and_humanize_parts("@for (item of items){hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["for".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["item of items".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_handle_semicolons_braces_and_parentheses_in_block_parameter() {
        // TS: it("should handle semicolons, braces and parentheses used in a block parameter", ...)
        let input = r#"@for (a === ";"; b === ')'; c === "("; d === '}'; e === "{") {hello}"#;
        let result = tokenize_and_humanize_parts(input);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["for".to_string()]),
                (HtmlTokenType::BlockParameter, vec![r#"a === ";""#.to_string()]),
                (HtmlTokenType::BlockParameter, vec![r"b === ')'".to_string()]),
                (HtmlTokenType::BlockParameter, vec![r#"c === "(""#.to_string()]),
                (HtmlTokenType::BlockParameter, vec![r"d === '}'".to_string()]),
                (HtmlTokenType::BlockParameter, vec![r#"e === "{""#.to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_handle_object_literals_and_function_calls_in_block_parameters() {
        // TS: it("should handle object literals and function calls in block parameters", ...)
        let result = tokenize_and_humanize_parts(
            "@defer (on a({a: 1, b: 2}, false, {c: 3}); when b({d: 4})) {hello}",
        );
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["defer".to_string()]),
                (
                    HtmlTokenType::BlockParameter,
                    vec!["on a({a: 1, b: 2}, false, {c: 3})".to_string()]
                ),
                (HtmlTokenType::BlockParameter, vec!["when b({d: 4})".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_block_with_unclosed_parameters() {
        // TS: it("should parse block with unclosed parameters", ...)
        let result = tokenize_and_humanize_parts("@if (a === b {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::IncompleteBlockOpen, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["a === b {hello}".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_block_with_stray_parentheses_in_parameter_position() {
        // TS: it("should parse block with stray parentheses in the parameter position", ...)
        // Note: When blocks are enabled (default), `}` becomes BLOCK_CLOSE even for incomplete blocks
        let result = tokenize_and_humanize_parts("@if a === b) {hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::IncompleteBlockOpen, vec!["if a".to_string()]),
                (HtmlTokenType::Text, vec!["=== b) {hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_report_unclosed_object_literal_inside_parameter() {
        // TS: it("should report unclosed object literal inside a parameter", ...)
        // Note: When blocks are enabled (default), `}` becomes BLOCK_CLOSE even for incomplete blocks
        let result = tokenize_and_humanize_parts("@if ({invalid: true) hello}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::IncompleteBlockOpen, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["{invalid: true".to_string()]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_handle_semicolon_in_nested_string_inside_block_parameter() {
        // TS: it("should handle a semicolon used in a nested string inside a block parameter", ...)
        let result = tokenize_and_humanize_parts(r#"@if (condition === "';'") {hello}"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec![r#"condition === "';'""#.to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_handle_semicolon_next_to_escaped_quote_in_block_parameter() {
        // TS: it("should handle a semicolon next to an escaped quote used in a block parameter", ...)
        // Angular expects: 'condition === "\\";"' which decodes to: condition === "\";"
        // The closing " is included in the parameter (it ends the string literal in the expression)
        let result = tokenize_and_humanize_parts(r#"@if (condition === "\";") {hello}"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                // The parameter includes the full string literal: "\";"
                // Using regular string: " + \ + " + ; + "
                (HtmlTokenType::BlockParameter, vec!["condition === \"\\\";\"".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_mixed_text_and_html_content_in_block() {
        // TS: it("should parse mixed text and html content in a block", ...)
        let result = tokenize_and_humanize_parts("@if (a === 1) {foo <b>bar</b> baz}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["a === 1".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["foo ".to_string()]),
                (HtmlTokenType::TagOpenStart, vec![String::new(), "b".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["bar".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "b".to_string()]),
                (HtmlTokenType::Text, vec![" baz".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_html_tags_with_attributes_containing_curly_braces_inside_blocks() {
        // TS: it("should parse HTML tags with attributes containing curly braces inside blocks", ...)
        let result = tokenize_and_humanize_parts(r#"@if (a === 1) {<div a="}" b="{"></div>}"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["a === 1".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrQuote, vec![r#"""#.to_string()]),
                (HtmlTokenType::AttrValueText, vec!["}".to_string()]),
                (HtmlTokenType::AttrQuote, vec![r#"""#.to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "b".to_string()]),
                (HtmlTokenType::AttrQuote, vec![r#"""#.to_string()]),
                (HtmlTokenType::AttrValueText, vec!["{".to_string()]),
                (HtmlTokenType::AttrQuote, vec![r#"""#.to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_html_tags_with_attribute_containing_block_syntax() {
        // TS: it("should parse HTML tags with attribute containing block syntax", ...)
        let result = tokenize_and_humanize_parts(r#"<div a="@if (foo) {}"></div>"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "a".to_string()]),
                (HtmlTokenType::AttrQuote, vec![r#"""#.to_string()]),
                (HtmlTokenType::AttrValueText, vec!["@if (foo) {}".to_string()]),
                (HtmlTokenType::AttrQuote, vec![r#"""#.to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_deeply_nested_blocks() {
        // TS: it("should parse nested blocks", ...)
        let input = "@if (a) {hello a@if {hello unnamed@if (b) {hello b@if (c) {hello c}}}}";
        let result = tokenize_and_humanize_parts(input);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["a".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello a".to_string()]),
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello unnamed".to_string()]),
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["b".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello b".to_string()]),
                (HtmlTokenType::BlockOpenStart, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec!["c".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello c".to_string()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_block_containing_interpolation() {
        // TS: it("should parse a block containing an interpolation", ...)
        let result = tokenize_and_humanize_parts("@defer {{{message}}}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::BlockOpenStart, vec!["defer".to_string()]),
                (HtmlTokenType::BlockOpenEnd, vec![]),
                (HtmlTokenType::Text, vec![String::new()]),
                (
                    HtmlTokenType::Interpolation,
                    vec!["{{".to_string(), "message".to_string(), "}}".to_string()]
                ),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::BlockClose, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_incomplete_block_start_without_parameters_with_surrounding_text() {
        // TS: it("should parse an incomplete block start without parameters with surrounding text", ...)
        let result = tokenize_and_humanize_parts("My email frodo@for.com");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["My email frodo".to_string()]),
                (HtmlTokenType::IncompleteBlockOpen, vec!["for".to_string()]),
                (HtmlTokenType::Text, vec![".com".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_incomplete_block_start_at_end_of_input() {
        // TS: it("should parse an incomplete block start at the end of the input", ...)
        let result = tokenize_and_humanize_parts("My favorite console is @switch");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["My favorite console is ".to_string()]),
                (HtmlTokenType::IncompleteBlockOpen, vec!["switch".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_incomplete_block_start_with_parens_but_without_params() {
        // TS: it("should parse an incomplete block start with parentheses but without params", ...)
        let result = tokenize_and_humanize_parts("Use the @for() block");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["Use the ".to_string()]),
                (HtmlTokenType::IncompleteBlockOpen, vec!["for".to_string()]),
                (HtmlTokenType::Text, vec!["block".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_incomplete_block_start_with_parens_and_params() {
        // TS: it("should parse an incomplete block start with parentheses and params", ...)
        let result = tokenize_and_humanize_parts(r#"This is the @if({alias: "foo"}) expression"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["This is the ".to_string()]),
                (HtmlTokenType::IncompleteBlockOpen, vec!["if".to_string()]),
                (HtmlTokenType::BlockParameter, vec![r#"{alias: "foo"}"#.to_string()]),
                (HtmlTokenType::Text, vec!["expression".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_at_as_text() {
        // TS: it("should parse @ as text", ...)
        let result = tokenize_and_humanize_parts("@");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["@".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_parse_space_followed_by_at_as_text() {
        // TS: it("should parse space followed by @ as text", ...)
        let result = tokenize_and_humanize_parts(" @");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec![" @".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_parse_at_followed_by_space_as_text() {
        // TS: it("should parse @ followed by space as text", ...)
        let result = tokenize_and_humanize_parts("@ ");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["@ ".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_parse_at_followed_by_newline_and_text_as_text() {
        // TS: it("should parse @ followed by newline and text as text", ...)
        let result = tokenize_and_humanize_parts("@\nfoo");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["@\nfoo".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_parse_at_in_middle_of_text_as_text() {
        // TS: it("should parse @ in the middle of text as text", ...)
        let result = tokenize_and_humanize_parts("foo bar @ baz clink");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["foo bar @ baz clink".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_incomplete_block_with_space_then_name_as_text() {
        // TS: it("should parse incomplete block with space, then name as text", ...)
        let result = tokenize_and_humanize_parts("@ if");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["@ if".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }
}

// ============================================================================
// @let Declaration Tests
// ============================================================================

mod let_declarations {
    use super::*;

    #[test]
    fn should_tokenize_let_declaration() {
        let result = tokenize("@let foo = 123;");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::LetStart), "Should have LetStart");
        // LetStart contains the name in its parts
        assert!(types.contains(&HtmlTokenType::LetValue), "Should have LetValue");
        assert!(types.contains(&HtmlTokenType::LetEnd), "Should have LetEnd");
    }

    #[test]
    fn should_tokenize_let_name() {
        let result = tokenize("@let myVar = 42;");

        let let_start = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::LetStart);

        assert!(let_start.is_some());
        // In our implementation, LetStart contains the variable name
        assert_eq!(let_start.unwrap().value(), "myVar");
    }

    #[test]
    fn should_tokenize_let_value() {
        let result = tokenize("@let x = someExpression;");

        let value = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::LetValue);

        assert!(value.is_some());
        assert!(value.unwrap().value().contains("someExpression"));
    }

    #[test]
    fn should_tokenize_let_with_complex_value() {
        let result = tokenize("@let result = obj.method(arg1, arg2);");

        let value = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::LetValue);

        assert!(value.is_some());
        assert!(value.unwrap().value().contains("obj.method"));
    }

    #[test]
    fn should_tokenize_multiple_let_declarations() {
        let result = tokenize("@let a = 1; @let b = 2;");

        let let_starts: Vec<_> =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::LetStart).collect();

        assert_eq!(let_starts.len(), 2);
    }

    #[test]
    fn should_handle_incomplete_let_declaration() {
        let result = tokenize("@let foo = bar"); // Missing semicolon

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        // For incomplete declarations (missing semicolon), we emit INCOMPLETE_LET instead of LET_START
        assert!(types.contains(&HtmlTokenType::IncompleteLet));
        // Should also have LET_VALUE
        assert!(types.contains(&HtmlTokenType::LetValue));
    }

    #[test]
    fn should_tokenize_let_declaration_with_newline_before_name() {
        // Angular allows any whitespace - including newlines - between `@let` and the
        // declared name (it skips whitespace via `isNotWhitespace`). A newline there must
        // still produce a complete declaration, not an incomplete one.
        let result = tokenize("@let\nfoo = 123;");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::LetStart), "Should have LetStart, got {types:?}");
        assert!(types.contains(&HtmlTokenType::LetValue), "Should have LetValue, got {types:?}");
        assert!(types.contains(&HtmlTokenType::LetEnd), "Should have LetEnd, got {types:?}");
        assert!(
            !types.contains(&HtmlTokenType::IncompleteLet),
            "Should not be treated as incomplete, got {types:?}"
        );

        let let_start = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::LetStart);
        assert_eq!(let_start.unwrap().value(), "foo");
    }
}

// ============================================================================
// DOCTYPE and CDATA Tests
// ============================================================================

mod doctype_and_cdata {
    use super::*;

    #[test]
    fn should_parse_doctypes() {
        // TS: it("should parse doctypes", ...)
        let result = tokenize_and_humanize_parts("<!DOCTYPE html>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::DocType, vec!["DOCTYPE html".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_tokenize_doctype() {
        let result = tokenize("<!DOCTYPE html>");

        let doctype = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::DocType);

        assert!(doctype.is_some(), "Should have DOCTYPE token");
    }

    #[test]
    fn should_report_missing_end_doctype() {
        // TS: it("should report missing end doctype", ...)
        let result = tokenize("<!DOCTYPE html");
        assert!(!result.errors.is_empty(), "Should report an error for missing end doctype");
    }

    #[test]
    fn should_parse_cdata() {
        // TS: it("should parse CDATA", ...)
        // Note: Line endings are normalized per HTML5 spec
        let result = tokenize_and_humanize_parts("<![CDATA[t\ne\rs\r\nt]]>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::CdataStart, vec![]),
                (HtmlTokenType::RawText, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::CdataEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_tokenize_cdata() {
        let result = tokenize("<![CDATA[some data]]>");

        // Angular produces CdataStart + RawText + CdataEnd
        let has_cdata_start =
            result.tokens.iter().any(|t| t.token_type == HtmlTokenType::CdataStart);
        let raw_text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::RawText);

        assert!(has_cdata_start, "Should have CdataStart token");
        assert!(raw_text.is_some(), "Should have RawText token");
        assert!(raw_text.unwrap().value().contains("some data"));
    }

    #[test]
    fn should_tokenize_cdata_with_special_chars() {
        let result = tokenize("<![CDATA[<>&\"']]>");

        let raw_text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::RawText);

        assert!(raw_text.is_some());
        let value = raw_text.unwrap().value();
        assert!(value.contains('<') && value.contains('>') && value.contains('&'));
    }

    #[test]
    fn should_report_missing_end_cdata() {
        // TS: it("should report missing end cdata", ...)
        let result = tokenize("<![CDATA[");
        assert!(!result.errors.is_empty(), "Should report an error for missing end CDATA");
    }
}

// ============================================================================
// Position/Span Tests
// ============================================================================

mod positions {
    use super::*;

    #[test]
    fn should_track_token_start_position() {
        let result = tokenize("<div>");

        let tag = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::TagOpenStart);

        assert!(tag.is_some());
        assert_eq!(tag.unwrap().start, 0);
    }

    #[test]
    fn should_track_token_end_position() {
        let result = tokenize("<div>");

        // EOF should be at end of input
        let eof = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Eof);

        assert!(eof.is_some());
        assert_eq!(eof.unwrap().start, 5);
    }

    #[test]
    fn should_track_text_position() {
        let result = tokenize("<div>text</div>");

        let text = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);

        assert!(text.is_some());
        assert_eq!(text.unwrap().start, 5);
        assert_eq!(text.unwrap().end, 9);
    }

    #[test]
    fn should_track_attribute_position() {
        let result = tokenize(r#"<div id="foo">"#);

        let attr_names: Vec<_> =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::AttrName).collect();

        // Should have the "id" attribute
        assert!(!attr_names.is_empty());
        let id_attr = attr_names.iter().find(|t| t.name() == "id");
        assert!(id_attr.is_some());
    }
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn should_handle_empty_input() {
        let result = tokenize("");

        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert_eq!(types, vec![HtmlTokenType::Eof]);
    }

    #[test]
    fn should_handle_only_whitespace() {
        let result = tokenize("   \n\t  ");

        // Should produce text and EOF
        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::Eof));
    }

    #[test]
    fn should_handle_unclosed_tag() {
        let result = tokenize("<div");

        // Should still produce some tokens and not panic
        assert!(!result.tokens.is_empty());
    }

    #[test]
    fn should_handle_unclosed_comment() {
        let result = tokenize("<!-- unclosed");

        // Should produce tokens and/or errors
        assert!(!result.tokens.is_empty());
    }

    #[test]
    fn should_handle_special_characters_in_text() {
        let result = tokenize("a < b && c > d");

        // Should handle the text with comparison-like chars
        assert!(!result.tokens.is_empty());
    }

    #[test]
    fn should_handle_adjacent_interpolations() {
        let result = tokenize("{{a}}{{b}}");

        // Should not produce errors
        assert!(result.errors.is_empty() || result.tokens.len() > 1);
    }

    #[test]
    fn should_handle_empty_attribute_value() {
        let result = tokenize(r#"<div foo=""></div>"#);

        // Empty attribute value - we have quotes but no text inside
        let quote_count =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::AttrQuote).count();

        // Should have 2 quotes for the empty value
        assert_eq!(quote_count, 2, "Should have 2 quote tokens for empty value");
    }

    #[test]
    fn should_handle_newlines_in_tags() {
        let result = tokenize("<div\n  foo='bar'\n></div>");

        let has_tag = result.tokens.iter().any(|t| t.token_type == HtmlTokenType::TagOpenStart);
        let has_attr = result.tokens.iter().any(|t| t.token_type == HtmlTokenType::AttrName);

        assert!(has_tag);
        assert!(has_attr);
    }

    #[test]
    fn should_handle_curly_braces_in_attributes() {
        let result = tokenize(r#"<div attr="{ foo: 1 }"></div>"#);

        let attr_value =
            result.tokens.iter().find(|t| t.token_type == HtmlTokenType::AttrValueText);

        assert!(attr_value.is_some());
        assert!(attr_value.unwrap().value().contains('{'));
    }
}

// ============================================================================
// Complex Templates
// ============================================================================

mod complex_templates {
    use super::*;

    #[test]
    fn should_tokenize_form_template() {
        let template = r#"<form (submit)="onSubmit()">
            <input type="text" [(ngModel)]="name">
            <button type="submit">Submit</button>
        </form>"#;

        let result = tokenize(template);

        // Should have form, input, button tags
        let tag_opens: Vec<_> =
            result.tokens.iter().filter(|t| t.token_type == HtmlTokenType::TagOpenStart).collect();

        assert!(tag_opens.len() >= 3);
    }

    #[test]
    fn should_tokenize_control_flow_template() {
        let template = r"@if (items.length > 0) {
            @for (item of items; track item.id) {
                <div>{{item.name}}</div>
            } @empty {
                <p>No items</p>
            }
        } @else {
            <p>Nothing to show</p>
        }";

        let result = tokenize(template);

        let block_starts: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::BlockOpenStart)
            .map(oxc_angular_compiler::parser::html::HtmlToken::value)
            .collect();

        assert!(block_starts.contains(&"if"));
        assert!(block_starts.contains(&"for"));
        assert!(block_starts.contains(&"else"));
        assert!(block_starts.contains(&"empty"));
    }

    #[test]
    fn should_tokenize_defer_template() {
        let template = r"@defer (on viewport; when isReady) {
            <heavy-component />
        } @loading (minimum 500ms) {
            <div>Loading...</div>
        } @error {
            <div>Error!</div>
        }";

        let result = tokenize(template);

        let block_starts: Vec<_> = result
            .tokens
            .iter()
            .filter(|t| t.token_type == HtmlTokenType::BlockOpenStart)
            .map(oxc_angular_compiler::parser::html::HtmlToken::value)
            .collect();

        assert!(block_starts.contains(&"defer"));
        assert!(block_starts.contains(&"loading"));
        assert!(block_starts.contains(&"error"));
    }

    #[test]
    fn should_tokenize_mixed_content() {
        let template = r#"<div class="container">
            @let greeting = 'Hello';
            <h1>{{greeting}} World</h1>
            <!-- A comment -->
            @if (showDetails) {
                <p [innerHTML]="details"></p>
            }
        </div>"#;

        let result = tokenize(template);

        // Verify we have all expected token types
        let types: Vec<_> = result.tokens.iter().map(|t| t.token_type).collect();

        assert!(types.contains(&HtmlTokenType::TagOpenStart));
        assert!(types.contains(&HtmlTokenType::LetStart));
        assert!(types.contains(&HtmlTokenType::CommentStart));
        assert!(types.contains(&HtmlTokenType::BlockOpenStart));
    }
}

// ============================================================================
// Line/Column Number Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts describe("line/column numbers")

mod line_column_numbers {
    use super::*;

    /// Helper to tokenize and return line:column for each token
    fn tokenize_and_humanize_line_column(input: &str) -> Vec<(HtmlTokenType, String)> {
        let result = tokenize(input);
        let chars: Vec<char> = input.chars().collect();
        result
            .tokens
            .iter()
            .map(|t| {
                // Get line:column from source span
                // Note: This requires the token to track source position
                // For now, we'll use start offset and compute line:col
                let mut line = 0u32;
                let mut col = 0u32;
                let mut i = 0;
                while i < t.start as usize && i < chars.len() {
                    let ch = chars[i];
                    if ch == '\n' {
                        line += 1;
                        col = 0;
                        i += 1;
                    } else if ch == '\r' {
                        // Check if CRLF
                        if i + 1 < chars.len() && chars[i + 1] == '\n' {
                            // CRLF - treat as single newline, skip both
                            line += 1;
                            col = 0;
                            i += 2;
                        } else {
                            // Standalone CR - ignore for position tracking
                            // (Angular normalizes CR to LF but doesn't count it for column)
                            i += 1;
                        }
                    } else {
                        col += 1;
                        i += 1;
                    }
                }
                (t.token_type, format!("{line}:{col}"))
            })
            .collect()
    }

    #[test]
    fn should_work_without_newlines() {
        // TS: expect(tokenizeAndHumanizeLineColumn("<t>a</t>")).toEqual([...])
        let result = tokenize_and_humanize_line_column("<t>a</t>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, "0:0".to_string()),
                (HtmlTokenType::TagOpenEnd, "0:2".to_string()),
                (HtmlTokenType::Text, "0:3".to_string()),
                (HtmlTokenType::TagClose, "0:4".to_string()),
                (HtmlTokenType::Eof, "0:8".to_string()),
            ]
        );
    }

    #[test]
    fn should_work_with_one_newline() {
        // TS: expect(tokenizeAndHumanizeLineColumn("<t>\na</t>")).toEqual([...])
        let result = tokenize_and_humanize_line_column("<t>\na</t>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, "0:0".to_string()),
                (HtmlTokenType::TagOpenEnd, "0:2".to_string()),
                (HtmlTokenType::Text, "0:3".to_string()),
                (HtmlTokenType::TagClose, "1:1".to_string()),
                (HtmlTokenType::Eof, "1:5".to_string()),
            ]
        );
    }

    #[test]
    fn should_work_with_multiple_newlines() {
        // TS: expect(tokenizeAndHumanizeLineColumn("<t\n>\na</t>")).toEqual([...])
        let result = tokenize_and_humanize_line_column("<t\n>\na</t>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, "0:0".to_string()),
                (HtmlTokenType::TagOpenEnd, "1:0".to_string()),
                (HtmlTokenType::Text, "1:1".to_string()),
                (HtmlTokenType::TagClose, "2:1".to_string()),
                (HtmlTokenType::Eof, "2:5".to_string()),
            ]
        );
    }

    #[test]
    fn should_work_with_cr_and_lf() {
        // TS: expect(tokenizeAndHumanizeLineColumn("<t\n>\r\na\r</t>")).toEqual([...])
        let result = tokenize_and_humanize_line_column("<t\n>\r\na\r</t>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, "0:0".to_string()),
                (HtmlTokenType::TagOpenEnd, "1:0".to_string()),
                (HtmlTokenType::Text, "1:1".to_string()),
                (HtmlTokenType::TagClose, "2:1".to_string()),
                (HtmlTokenType::Eof, "2:5".to_string()),
            ]
        );
    }
}

// ============================================================================
// Component Tags Tests (Selectorless)
// ============================================================================
//
// Ported from Angular's lexer_spec.ts describe("component tags")
// NOTE: These tests require selectorlessEnabled option which may not be implemented yet.

mod component_tags {
    use super::*;

    #[test]
    fn should_parse_a_basic_component_tag() {
        // TS: tokenizeAndHumanizeParts("<MyComp>hello</MyComp>", {selectorlessEnabled: true})
        let result = tokenize_selectorless_and_humanize_parts("<MyComp>hello</MyComp>");
        assert_eq!(
            result,
            vec![
                (
                    HtmlTokenType::ComponentOpenStart,
                    vec!["MyComp".to_string(), String::new(), String::new()]
                ),
                (HtmlTokenType::ComponentOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (
                    HtmlTokenType::ComponentClose,
                    vec!["MyComp".to_string(), String::new(), String::new()]
                ),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_component_tag_with_a_tag_name() {
        // TS: tokenizeAndHumanizeParts("<MyComp:button>hello</MyComp:button>", options)
        let result =
            tokenize_selectorless_and_humanize_parts("<MyComp:button>hello</MyComp:button>");
        assert_eq!(
            result,
            vec![
                (
                    HtmlTokenType::ComponentOpenStart,
                    vec!["MyComp".to_string(), String::new(), "button".to_string()]
                ),
                (HtmlTokenType::ComponentOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (
                    HtmlTokenType::ComponentClose,
                    vec!["MyComp".to_string(), String::new(), "button".to_string()]
                ),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_component_tag_with_a_tag_name_and_namespace() {
        // TS: tokenizeAndHumanizeParts("<MyComp:svg:title>hello</MyComp:svg:title>", options)
        let result =
            tokenize_selectorless_and_humanize_parts("<MyComp:svg:title>hello</MyComp:svg:title>");
        assert_eq!(
            result,
            vec![
                (
                    HtmlTokenType::ComponentOpenStart,
                    vec!["MyComp".to_string(), "svg".to_string(), "title".to_string()]
                ),
                (HtmlTokenType::ComponentOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["hello".to_string()]),
                (
                    HtmlTokenType::ComponentClose,
                    vec!["MyComp".to_string(), "svg".to_string(), "title".to_string()]
                ),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_self_closing_component_tag() {
        // TS: tokenizeAndHumanizeParts("<MyComp/>", options)
        let result = tokenize_selectorless_and_humanize_parts("<MyComp/>");
        assert_eq!(
            result,
            vec![
                (
                    HtmlTokenType::ComponentOpenStart,
                    vec!["MyComp".to_string(), String::new(), String::new()]
                ),
                (HtmlTokenType::ComponentOpenEndVoid, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Selectorless Directives Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts describe("selectorless directives")
// NOTE: These tests require selectorlessEnabled option which may not be implemented yet.

mod selectorless_directives {
    use super::*;

    #[test]
    fn should_parse_a_basic_directive() {
        // TS: tokenizeAndHumanizeParts("<div @MyDir></div>", {selectorlessEnabled: true})
        let result = tokenize_selectorless_and_humanize_parts("<div @MyDir></div>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::DirectiveName, vec!["MyDir".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_directive_with_parentheses_but_no_attributes() {
        // TS: tokenizeAndHumanizeParts("<div @MyDir()></div>", options)
        let result = tokenize_selectorless_and_humanize_parts("<div @MyDir()></div>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::DirectiveName, vec!["MyDir".to_string()]),
                (HtmlTokenType::DirectiveOpen, vec![]),
                (HtmlTokenType::DirectiveClose, vec![]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_a_directive_with_a_single_attribute_without_a_value() {
        // TS: tokenizeAndHumanizeParts("<div @MyDir(foo)></div>", options)
        let result = tokenize_selectorless_and_humanize_parts("<div @MyDir(foo)></div>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::DirectiveName, vec!["MyDir".to_string()]),
                (HtmlTokenType::DirectiveOpen, vec![]),
                (HtmlTokenType::AttrName, vec![String::new(), "foo".to_string()]),
                (HtmlTokenType::DirectiveClose, vec![]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_not_pick_up_selectorless_like_text_inside_a_tag() {
        // TS: tokenizeAndHumanizeParts("<div>@MyDir()</div>", options)
        let result = tokenize_selectorless_and_humanize_parts("<div>@MyDir()</div>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["@MyDir()".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_not_pick_up_selectorless_like_text_inside_an_attribute() {
        // TS: tokenizeAndHumanizeParts('<div hello="@MyDir"></div>', options)
        let result = tokenize_selectorless_and_humanize_parts(r#"<div hello="@MyDir"></div>"#);
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::AttrName, vec![String::new(), "hello".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["\"".to_string()]),
                (HtmlTokenType::AttrValueText, vec!["@MyDir".to_string()]),
                (HtmlTokenType::AttrQuote, vec!["\"".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Expansion Forms Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts describe("expansion forms")
// NOTE: These tests require tokenizeExpansionForms option which may not be implemented yet.

mod expansion_forms {
    use super::*;

    #[test]
    fn should_parse_an_expansion_form() {
        // TS: tokenizeAndHumanizeParts("{one.two, three, =4 {four} =5 {five} foo {bar} }",
        //                              {tokenizeExpansionForms: true})
        let result = tokenize_expansion_and_humanize_parts(
            "{one.two, three, =4 {four} =5 {five} foo {bar} }",
        );
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::ExpansionFormStart, vec![]),
                (HtmlTokenType::RawText, vec!["one.two".to_string()]),
                (HtmlTokenType::RawText, vec!["three".to_string()]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=4".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["four".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=5".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["five".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionCaseValue, vec!["foo".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["bar".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionFormEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_an_expansion_form_with_text_elements_surrounding_it() {
        // TS: tokenizeAndHumanizeParts("before{one.two, three, =4 {four}}after",
        //                              {tokenizeExpansionForms: true})
        let result =
            tokenize_expansion_and_humanize_parts("before{one.two, three, =4 {four}}after");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["before".to_string()]),
                (HtmlTokenType::ExpansionFormStart, vec![]),
                (HtmlTokenType::RawText, vec!["one.two".to_string()]),
                (HtmlTokenType::RawText, vec!["three".to_string()]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=4".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["four".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionFormEnd, vec![]),
                (HtmlTokenType::Text, vec!["after".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_an_expansion_form_as_a_tag_single_child() {
        // TS: tokenizeAndHumanizeParts("<div><span>{a, b, =4 {c}}</span></div>",
        //                              {tokenizeExpansionForms: true})
        let result =
            tokenize_expansion_and_humanize_parts("<div><span>{a, b, =4 {c}}</span></div>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagOpenStart, vec![String::new(), "span".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::ExpansionFormStart, vec![]),
                (HtmlTokenType::RawText, vec!["a".to_string()]),
                (HtmlTokenType::RawText, vec!["b".to_string()]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=4".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["c".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionFormEnd, vec![]),
                (HtmlTokenType::TagClose, vec![String::new(), "span".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "div".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_an_expansion_forms_with_elements_in_it() {
        // TS: tokenizeAndHumanizeParts("{one.two, three, =4 {four <b>a</b>}}",
        //                              {tokenizeExpansionForms: true})
        let result = tokenize_expansion_and_humanize_parts("{one.two, three, =4 {four <b>a</b>}}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::ExpansionFormStart, vec![]),
                (HtmlTokenType::RawText, vec!["one.two".to_string()]),
                (HtmlTokenType::RawText, vec!["three".to_string()]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=4".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["four ".to_string()]),
                (HtmlTokenType::TagOpenStart, vec![String::new(), "b".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["a".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "b".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionFormEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_nested_expansion_forms() {
        // TS: tokenizeAndHumanizeParts("{one.two, three, =4 { {xx, yy, =x {one}} }}",
        //                              {tokenizeExpansionForms: true})
        let result =
            tokenize_expansion_and_humanize_parts("{one.two, three, =4 { {xx, yy, =x {one}} }}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::ExpansionFormStart, vec![]),
                (HtmlTokenType::RawText, vec!["one.two".to_string()]),
                (HtmlTokenType::RawText, vec!["three".to_string()]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=4".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::ExpansionFormStart, vec![]),
                (HtmlTokenType::RawText, vec!["xx".to_string()]),
                (HtmlTokenType::RawText, vec!["yy".to_string()]),
                (HtmlTokenType::ExpansionCaseValue, vec!["=x".to_string()]),
                (HtmlTokenType::ExpansionCaseExpStart, vec![]),
                (HtmlTokenType::Text, vec!["one".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionFormEnd, vec![]),
                (HtmlTokenType::Text, vec![" ".to_string()]),
                (HtmlTokenType::ExpansionCaseExpEnd, vec![]),
                (HtmlTokenType::ExpansionFormEnd, vec![]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Entities Tests (5+ digit codes)
// ============================================================================
//
// Ported from Angular's lexer_spec.ts additional entity tests

mod entities_extended {
    use super::*;

    #[test]
    fn should_parse_entities_with_more_than_4_hex_digits() {
        // TS: it("should parse entities with more than 4 hex digits", ...)
        // &#x1f600; is 😀 emoji (5 hex digits)
        // Note: Empty TEXT tokens are emitted around entities for Angular compatibility
        let result = tokenize_and_humanize_parts("&#x1f600;");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec![String::new()]),
                (
                    HtmlTokenType::EncodedEntity,
                    vec!["\u{1f600}".to_string(), "&#x1f600;".to_string()]
                ),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_entities_with_more_than_4_decimal_digits() {
        // TS: it("should parse entities with more than 4 decimal digits", ...)
        // &#128512; is 😀 emoji (6 decimal digits)
        // Note: Empty TEXT tokens are emitted around entities for Angular compatibility
        let result = tokenize_and_humanize_parts("&#128512;");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec![String::new()]),
                (
                    HtmlTokenType::EncodedEntity,
                    vec!["\u{1f600}".to_string(), "&#128512;".to_string()]
                ),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Unicode Characters Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts "unicode characters" section

mod unicode_characters {
    use super::*;

    #[test]
    fn should_support_unicode_characters() {
        // TS: it("should support unicode characters", ...)
        // İ is a Turkish capital I with dot above (U+0130)
        let result = tokenize("<p>İ</p>");
        let text_token = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);
        assert!(text_token.is_some());
        assert_eq!(text_token.unwrap().value(), "İ");
    }

    #[test]
    fn should_support_emoji_in_text() {
        let result = tokenize("<p>Hello 👋 World</p>");
        let text_token = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);
        assert!(text_token.is_some());
        assert!(text_token.unwrap().value().contains("👋"));
    }

    #[test]
    fn should_support_chinese_characters() {
        let result = tokenize("<p>你好世界</p>");
        let text_token = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);
        assert!(text_token.is_some());
        assert_eq!(text_token.unwrap().value(), "你好世界");
    }

    #[test]
    fn should_support_arabic_characters() {
        let result = tokenize("<p>مرحبا</p>");
        let text_token = result.tokens.iter().find(|t| t.token_type == HtmlTokenType::Text);
        assert!(text_token.is_some());
        assert_eq!(text_token.unwrap().value(), "مرحبا");
    }
}

// ============================================================================
// Raw Text Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts "raw text" section
// Raw text is content inside <script> and <style> tags where HTML entities
// are NOT decoded and other tags are ignored.

mod raw_text {
    use super::*;

    // IGNORED: Our lexer doesn't track tag context for raw text handling.
    // Angular's lexer outputs RawText tokens for <script>/<style> content,
    // but our lexer emits Text tokens. The HTML parser handles tag-specific
    // content type semantics at the parsing stage instead.
    #[test]
    fn should_parse_text_in_script() {
        // TS: it("should parse text", ...)
        // Note: \r\n -> \n normalization happens
        let result = tokenize_and_humanize_parts("<script>t\ne\rs\r\nt</script>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::RawText, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_not_detect_entities_in_raw_text() {
        // TS: it("should not detect entities", ...)
        let result = tokenize_and_humanize_parts("<script>&amp;</script>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::RawText, vec!["&amp;".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_ignore_other_opening_tags_in_raw_text() {
        // TS: it("should ignore other opening tags", ...)
        let result = tokenize_and_humanize_parts("<script>a<div></script>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::RawText, vec!["a<div>".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_ignore_other_closing_tags_in_raw_text() {
        // TS: it("should ignore other closing tags", ...)
        let result = tokenize_and_humanize_parts("<script>a</test></script>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::RawText, vec!["a</test>".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "script".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_text_in_style() {
        // Similar test for style tag
        let result = tokenize_and_humanize_parts("<style>.foo { color: red; }</style>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "style".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::RawText, vec![".foo { color: red; }".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "style".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Escapable Raw Text Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts "escapable raw text" section
// Escapable raw text is content inside <title> and <textarea> tags where
// HTML entities ARE decoded but other tags are ignored.

mod escapable_raw_text {
    use super::*;

    // IGNORED: Our lexer doesn't track tag context for escapable raw text handling.
    // Angular's lexer outputs EscapableRawText tokens for <title>/<textarea> content,
    // but our lexer emits Text tokens. The HTML parser handles tag-specific
    // content type semantics at the parsing stage instead.
    #[test]
    fn should_parse_text_in_title() {
        // TS: it("should parse text", ...)
        let result = tokenize_and_humanize_parts("<title>t\ne\rs\r\nt</title>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::EscapableRawText, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_detect_entities_in_escapable_raw_text() {
        // TS: it("should detect entities", ...)
        let result = tokenize_and_humanize_parts("<title>&amp;</title>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::EscapableRawText, vec![String::new()]),
                (HtmlTokenType::EncodedEntity, vec!["&".to_string(), "&amp;".to_string()]),
                (HtmlTokenType::EscapableRawText, vec![String::new()]),
                (HtmlTokenType::TagClose, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_ignore_other_opening_tags_in_escapable_raw_text() {
        // TS: it("should ignore other opening tags", ...)
        let result = tokenize_and_humanize_parts("<title>a<div></title>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::EscapableRawText, vec!["a<div>".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_ignore_other_closing_tags_in_escapable_raw_text() {
        // TS: it("should ignore other closing tags", ...)
        let result = tokenize_and_humanize_parts("<title>a</test></title>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::EscapableRawText, vec!["a</test>".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "title".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_text_in_textarea() {
        // Similar test for textarea tag
        let result = tokenize_and_humanize_parts("<textarea>Some content</textarea>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec![String::new(), "textarea".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::EscapableRawText, vec!["Some content".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "textarea".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Parsable Data Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts "parsable data" section
// SVG <title> tags are treated differently from HTML <title> - they contain
// parsable data (normal HTML content).

mod parsable_data {
    use super::*;

    #[test]
    fn should_parse_svg_title_as_parsable_data() {
        // TS: it("should parse an SVG <title> tag", ...)
        let result = tokenize_and_humanize_parts("<svg:title>test</svg:title>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec!["svg".to_string(), "title".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["test".to_string()]),
                (HtmlTokenType::TagClose, vec!["svg".to_string(), "title".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_svg_title_with_children() {
        // TS: it("should parse an SVG <title> tag with children", ...)
        let result = tokenize_and_humanize_parts("<svg:title><f>test</f></svg:title>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagOpenStart, vec!["svg".to_string(), "title".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::TagOpenStart, vec![String::new(), "f".to_string()]),
                (HtmlTokenType::TagOpenEnd, vec![]),
                (HtmlTokenType::Text, vec!["test".to_string()]),
                (HtmlTokenType::TagClose, vec![String::new(), "f".to_string()]),
                (HtmlTokenType::TagClose, vec!["svg".to_string(), "title".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

// ============================================================================
// Closing Tags Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts "closing tags" section

mod closing_tags {
    use super::*;

    #[test]
    fn should_parse_closing_tags_without_prefix() {
        // TS: it("should parse closing tags without prefix", ...)
        let result = tokenize_and_humanize_parts("</test>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagClose, vec![String::new(), "test".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_closing_tags_with_prefix() {
        // TS: it("should parse closing tags with prefix", ...)
        let result = tokenize_and_humanize_parts("</ns1:test>");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagClose, vec!["ns1".to_string(), "test".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_allow_whitespace_in_closing_tags() {
        // TS: it("should allow whitespace", ...)
        let result = tokenize_and_humanize_parts("</ test >");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::TagClose, vec![String::new(), "test".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_report_missing_name_after_close_tag_start() {
        // TS: it("should report missing name after </", ...)
        let result = tokenize("</");
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn should_report_missing_close_tag_end() {
        // TS: it("should report missing >", ...)
        let result = tokenize("</test");
        assert!(!result.errors.is_empty());
    }
}

// ============================================================================
// Regular Text Tests
// ============================================================================
//
// Ported from Angular's lexer_spec.ts "regular text" section

mod regular_text {
    use super::*;

    #[test]
    fn should_parse_text() {
        // TS: it("should parse text", ...)
        let result = tokenize_and_humanize_parts("a");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["a".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_handle_cr_lf_in_text() {
        // TS: it("should handle CR & LF in text", ...)
        let result = tokenize_and_humanize_parts("t\ne\rs\r\nt");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec!["t\ne\ns\nt".to_string()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_parse_text_starting_with_ampersand() {
        // TS: it('should parse text starting with "&"', ...)
        let result = tokenize_and_humanize_parts("a && b &");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["a && b &".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    #[test]
    fn should_allow_less_than_in_text_nodes() {
        // TS: it('should allow "<" in text nodes', ...)
        let result = tokenize_and_humanize_parts("< a>");
        assert_eq!(
            result,
            vec![(HtmlTokenType::Text, vec!["< a>".to_string()]), (HtmlTokenType::Eof, vec![]),]
        );
    }

    // IGNORED: Angular's lexer emits empty Text tokens before/after interpolations,
    // but our lexer omits empty text tokens for efficiency. The semantic meaning is the same.
    #[test]
    fn should_be_able_to_escape_opening_brace() {
        // TS: it("should be able to escape {", ...)
        let result = tokenize_and_humanize_parts("{{ \"{\" }}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec![String::new()]),
                (
                    HtmlTokenType::Interpolation,
                    vec!["{{".to_string(), " \"{\" ".to_string(), "}}".to_string()]
                ),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    #[test]
    fn should_be_able_to_escape_double_opening_brace() {
        // TS: it("should be able to escape {{", ...)
        let result = tokenize_and_humanize_parts("{{ \"{{\" }}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec![String::new()]),
                (
                    HtmlTokenType::Interpolation,
                    vec!["{{".to_string(), " \"{{\" ".to_string(), "}}".to_string()]
                ),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }

    // IGNORED: Our lexer handles quote escaping differently. Angular's lexer captures
    // everything up to EOF on mismatched quotes, but our lexer stops at the first `}}`
    // it finds, regardless of quote context.
    #[test]
    fn should_capture_mismatched_quotes_in_interpolation() {
        // TS: it("should capture everything up to the end of file in the interpolation expression part if there are mismatched quotes", ...)
        let result = tokenize_and_humanize_parts("{{ \"{{a}}' }}");
        assert_eq!(
            result,
            vec![
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Interpolation, vec!["{{".to_string(), " \"{{a}}' }}".to_string()]),
                (HtmlTokenType::Text, vec![String::new()]),
                (HtmlTokenType::Eof, vec![]),
            ]
        );
    }
}

use oxc_allocator::Allocator;
use oxc_angular_compiler::ast::html::{
    HtmlAttribute, HtmlBlock, HtmlComment, HtmlElement, HtmlExpansion, HtmlExpansionCase,
    HtmlLetDeclaration, HtmlNode, HtmlText, InterpolatedToken, InterpolatedTokenType,
};
use oxc_angular_compiler::parser::html::{HtmlParser, NGSP_UNICODE};

use super::SubsystemRunner;
use crate::test_case::{HtmlLexerOptions, TestAssertion, TestResult};

/// Tags where whitespace should be preserved
const SKIP_WS_TRIM_TAGS: &[&str] = &["pre", "template", "textarea", "script", "style"];

/// Angular attribute name for preserving whitespace
const PRESERVE_WS_ATTR_NAME: &str = "ngPreserveWhitespaces";

/// Whitespace characters (equivalent to \s with \u00a0 NON-BREAKING SPACE excluded)
/// Based on Angular's WS_CHARS definition
const WS_CHARS: &[char] = &[
    ' ', '\x0C', '\n', '\r', '\t', '\x0B', '\u{1680}', '\u{180e}', '\u{2000}', '\u{2001}',
    '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}',
    '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}',
];

/// Check if a character is considered whitespace (excluding &nbsp;)
fn is_ws_char(c: char) -> bool {
    WS_CHARS.contains(&c)
}

/// Check if a string contains any non-whitespace characters (using Angular's definition)
fn has_non_ws(s: &str) -> bool {
    s.chars().any(|c| !is_ws_char(c))
}

/// Replace &ngsp; (NGSP_UNICODE) with a regular space
fn replace_ngsp(s: &str) -> String {
    s.replace(NGSP_UNICODE, " ")
}

/// Process whitespace in text: replace ngsp and collapse consecutive whitespace (2+)
/// This matches Angular's processWhitespace function
fn process_whitespace(text: &str) -> String {
    let after_ngsp = replace_ngsp(text);

    // Replace 2+ consecutive whitespace chars with a single space
    let mut result = String::with_capacity(after_ngsp.len());
    let mut ws_start: Option<usize> = None;
    let chars: Vec<char> = after_ngsp.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if is_ws_char(chars[i]) {
            if ws_start.is_none() {
                ws_start = Some(i);
            }
        } else {
            if let Some(start) = ws_start {
                let ws_len = i - start;
                if ws_len >= 2 {
                    // Replace 2+ whitespace with single space
                    result.push(' ');
                } else {
                    // Keep single whitespace character as-is
                    result.push(chars[start]);
                }
                ws_start = None;
            }
            result.push(chars[i]);
        }
        i += 1;
    }

    // Handle trailing whitespace
    if let Some(start) = ws_start {
        let ws_len = chars.len() - start;
        if ws_len >= 2 {
            result.push(' ');
        } else {
            result.push(chars[start]);
        }
    }

    result
}

/// Check if an element has the ngPreserveWhitespaces attribute
fn has_preserve_whitespace_attr(attrs: &[HtmlAttribute<'_>]) -> bool {
    attrs.iter().any(|a| a.name.as_str() == PRESERVE_WS_ATTR_NAME)
}

/// Runner for Angular HTML whitespace removal conformance tests
/// Tests parseAndRemoveWS() function
pub struct HtmlWhitespaceRunner;

impl HtmlWhitespaceRunner {
    pub fn new() -> Self {
        Self
    }

    /// Parse HTML, remove whitespace, and return humanized output
    fn parse_and_remove_whitespace(
        &self,
        text: &str,
        options: Option<&HtmlLexerOptions>,
    ) -> Vec<serde_json::Value> {
        let allocator = Allocator::default();

        // Check for tokenize_expansion_forms option
        let expansion_forms = options.is_some_and(|o| o.tokenize_expansion_forms);

        let parser = if expansion_forms {
            HtmlParser::with_expansion_forms(&allocator, text, "test.html")
        } else {
            HtmlParser::new(&allocator, text, "test.html")
        };

        let result = parser.parse();

        // Apply whitespace removal while humanizing
        let mut humanizer = WhitespaceRemovingHumanizer::new();
        humanizer.visit_nodes_with_siblings(&result.nodes, false);
        humanizer.result
    }
}

impl Default for HtmlWhitespaceRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SubsystemRunner for HtmlWhitespaceRunner {
    fn name(&self) -> &'static str {
        "html_whitespace"
    }

    fn description(&self) -> &'static str {
        "Angular HTML whitespace removal (parseAndRemoveWS)"
    }

    fn can_handle(&self, assertion: &TestAssertion) -> bool {
        matches!(assertion, TestAssertion::ParseAndRemoveWhitespace { .. })
    }

    fn run_assertion(&self, assertion: &TestAssertion) -> TestResult {
        match assertion {
            TestAssertion::ParseAndRemoveWhitespace { input, expected, options } => {
                let actual = self.parse_and_remove_whitespace(input, options.as_ref());

                if compare_nodes(expected, &actual) {
                    TestResult::Passed
                } else {
                    TestResult::Failed {
                        expected: format!("{expected:?}"),
                        actual: format!("{actual:?}"),
                        diff: None,
                    }
                }
            }
            _ => {
                TestResult::Skipped { reason: "Not handled by html whitespace runner".to_string() }
            }
        }
    }
}

/// Compare two node arrays for equality
fn compare_nodes(expected: &[serde_json::Value], actual: &[serde_json::Value]) -> bool {
    if expected.len() != actual.len() {
        return false;
    }

    for (exp, act) in expected.iter().zip(actual.iter()) {
        if !compare_node(exp, act) {
            return false;
        }
    }
    true
}

/// Compare a single node
fn compare_node(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Array(exp_arr), serde_json::Value::Array(act_arr)) => {
            if exp_arr.len() != act_arr.len() {
                return false;
            }

            if exp_arr.is_empty() {
                return true;
            }

            // Compare node types (handle html.Text -> Text normalization)
            let exp_type = extract_node_type(&exp_arr[0]);
            let act_type = extract_node_type(&act_arr[0]);
            if exp_type != act_type {
                return false;
            }

            // Compare ALL elements
            for i in 1..exp_arr.len() {
                if !compare_value(&exp_arr[i], &act_arr[i]) {
                    return false;
                }
            }
            true
        }
        _ => compare_value(expected, actual),
    }
}

/// Extract node type name
fn extract_node_type(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(|s| s.strip_prefix("html.").unwrap_or(s).to_string())
}

/// Compare two JSON values with flexible number comparison
/// Also handles NGSP_UNICODE placeholder comparison
fn compare_value(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Number(e), serde_json::Value::Number(a)) => {
            let exp_f = e.as_f64().unwrap_or(f64::NAN);
            let act_f = a.as_f64().unwrap_or(f64::NAN);
            (exp_f - act_f).abs() < 0.0001
        }
        (serde_json::Value::String(e), serde_json::Value::String(a)) => {
            // Handle NGSP_UNICODE placeholder: expected has "NGSP_UNICODE", actual is " " (single space)
            // because we replace NGSP with space before outputting
            if e == "NGSP_UNICODE" && a == " " {
                true
            } else if e.contains(NGSP_UNICODE) || a.contains(NGSP_UNICODE) {
                // Normalize NGSP for comparison
                e.replace(NGSP_UNICODE, " ") == a.replace(NGSP_UNICODE, " ")
            } else {
                e == a
            }
        }
        (serde_json::Value::Array(e), serde_json::Value::Array(a)) => {
            e.len() == a.len() && e.iter().zip(a.iter()).all(|(ev, av)| compare_value(ev, av))
        }
        _ => expected == actual,
    }
}

// ============================================================================
// WhitespaceRemovingHumanizer - Removes whitespace while humanizing
// ============================================================================

/// Context about sibling nodes for whitespace decisions
struct SiblingContext {
    /// Previous sibling (if any)
    prev: Option<NodeKind>,
    /// Next sibling (if any)
    next: Option<NodeKind>,
}

/// Simplified node kind for sibling context
#[derive(Clone, Copy)]
enum NodeKind {
    Text,
    Element,
    Expansion,
    Block,
    Other,
}

impl<'a> From<&HtmlNode<'a>> for NodeKind {
    fn from(node: &HtmlNode<'a>) -> Self {
        match node {
            HtmlNode::Text(_) => NodeKind::Text,
            HtmlNode::Element(_) => NodeKind::Element,
            HtmlNode::Expansion(_) => NodeKind::Expansion,
            HtmlNode::Block(_) => NodeKind::Block,
            _ => NodeKind::Other,
        }
    }
}

/// Humanizer that removes whitespace while generating output
struct WhitespaceRemovingHumanizer {
    result: Vec<serde_json::Value>,
    depth: i32,
    /// Whether we're inside a tag that preserves whitespace
    preserve_whitespace: bool,
}

impl WhitespaceRemovingHumanizer {
    fn new() -> Self {
        Self { result: Vec::new(), depth: 0, preserve_whitespace: false }
    }

    fn push_node(&mut self, items: Vec<serde_json::Value>) {
        self.result.push(serde_json::Value::Array(items));
    }

    /// Visit nodes with sibling context for whitespace decisions
    fn visit_nodes_with_siblings(&mut self, nodes: &[HtmlNode<'_>], preserve_ws: bool) {
        let old_preserve = self.preserve_whitespace;
        self.preserve_whitespace = preserve_ws;

        for (i, node) in nodes.iter().enumerate() {
            let context = SiblingContext {
                prev: if i > 0 { Some(NodeKind::from(&nodes[i - 1])) } else { None },
                next: nodes.get(i + 1).map(NodeKind::from),
            };
            self.visit_node_with_context(node, &context);
        }

        self.preserve_whitespace = old_preserve;
    }

    /// Visit a node with sibling context
    fn visit_node_with_context(&mut self, node: &HtmlNode<'_>, context: &SiblingContext) {
        match node {
            HtmlNode::Text(text) => self.visit_text_with_context(text, context),
            HtmlNode::Element(element) => self.visit_element(element),
            HtmlNode::Comment(comment) => self.visit_comment(comment),
            HtmlNode::Expansion(expansion) => self.visit_expansion(expansion, context),
            HtmlNode::ExpansionCase(case) => self.visit_expansion_case(case),
            HtmlNode::Block(block) => self.visit_block(block),
            HtmlNode::LetDeclaration(decl) => self.visit_let_declaration(decl),
            _ => {}
        }
    }

    /// Visit text node with whitespace removal logic
    fn visit_text_with_context(&mut self, text: &HtmlText<'_>, context: &SiblingContext) {
        let value = text.value.as_str();

        // Check if adjacent to expansion (ICU)
        let has_expansion_sibling = matches!(context.prev, Some(NodeKind::Expansion))
            || matches!(context.next, Some(NodeKind::Expansion));

        // If preserving whitespace (inside <pre>, <textarea>, etc.), emit as-is
        if self.preserve_whitespace {
            let mut items = vec![
                serde_json::json!("html.Text"),
                serde_json::json!(value),
                serde_json::json!(self.depth),
            ];

            for token in &text.tokens {
                let parts: Vec<serde_json::Value> =
                    token.parts.iter().map(|p| serde_json::json!(p.to_string())).collect();
                items.push(serde_json::json!(parts));
            }

            self.push_node(items);
            return;
        }

        // Check if text contains non-whitespace (using Angular's definition, excluding &nbsp;)
        let is_not_blank = has_non_ws(value);

        if !is_not_blank && !has_expansion_sibling {
            // Remove blank text nodes that aren't adjacent to expansions
            return;
        }

        // Process the text value: replace NGSP with space, collapse 2+ whitespace to single space
        // But DO NOT trim leading/trailing (preserveSignificantWhitespace = true)
        let processed = process_whitespace(value);

        // Process tokens to match the new value
        let processed_tokens = self.process_tokens(&text.tokens);

        let mut items = vec![
            serde_json::json!("html.Text"),
            serde_json::json!(processed),
            serde_json::json!(self.depth),
        ];

        for token_parts in processed_tokens {
            items.push(serde_json::json!(token_parts));
        }

        self.push_node(items);
    }

    /// Process tokens for whitespace removal
    fn process_tokens(&self, tokens: &[InterpolatedToken<'_>]) -> Vec<Vec<String>> {
        let mut result: Vec<Vec<String>> = Vec::new();

        for token in tokens {
            match token.token_type {
                InterpolatedTokenType::Text => {
                    let text = token.parts.first().map_or("", oxc_span::Ident::as_str);
                    let processed = process_whitespace(text);
                    result.push(vec![processed]);
                }
                InterpolatedTokenType::Interpolation => {
                    // Interpolation: [startMarker, expression, endMarker]
                    let parts: Vec<String> =
                        token.parts.iter().map(std::string::ToString::to_string).collect();
                    result.push(parts);
                }
                InterpolatedTokenType::EncodedEntity => {
                    // EncodedEntity: [decoded, encoded]
                    // For NGSP, the decoded part is NGSP_UNICODE which should become " "
                    let parts: Vec<String> = token
                        .parts
                        .iter()
                        .map(|p| {
                            let s = p.to_string();
                            // Replace NGSP_UNICODE with space
                            if s.contains(NGSP_UNICODE) { s.replace(NGSP_UNICODE, " ") } else { s }
                        })
                        .collect();
                    result.push(parts);
                }
            }
        }

        result
    }

    fn visit_element(&mut self, element: &HtmlElement<'_>) {
        let name = element.name.to_string();

        // Check if we should preserve whitespace inside this element
        let should_preserve = SKIP_WS_TRIM_TAGS.contains(&name.as_str())
            || has_preserve_whitespace_attr(&element.attrs);

        self.push_node(vec![
            serde_json::json!("html.Element"),
            serde_json::json!(name),
            serde_json::json!(self.depth),
        ]);

        // Visit children with appropriate whitespace preservation
        self.depth += 1;
        self.visit_nodes_with_siblings(&element.children, should_preserve);
        self.depth -= 1;
    }

    fn visit_comment(&mut self, comment: &HtmlComment<'_>) {
        let value = comment.value.to_string();
        let trimmed = value.trim();
        self.push_node(vec![
            serde_json::json!("html.Comment"),
            serde_json::json!(trimmed),
            serde_json::json!(self.depth),
        ]);
    }

    fn visit_expansion(&mut self, expansion: &HtmlExpansion<'_>, _context: &SiblingContext) {
        self.push_node(vec![
            serde_json::json!("html.Expansion"),
            serde_json::json!(expansion.switch_value.to_string()),
            serde_json::json!(expansion.expansion_type.to_string()),
            serde_json::json!(self.depth),
        ]);

        self.depth += 1;
        for case in &expansion.cases {
            self.visit_expansion_case(case);
        }
        self.depth -= 1;
    }

    fn visit_expansion_case(&mut self, case: &HtmlExpansionCase<'_>) {
        self.push_node(vec![
            serde_json::json!("html.ExpansionCase"),
            serde_json::json!(case.value.to_string()),
            serde_json::json!(self.depth),
        ]);
    }

    fn visit_block(&mut self, block: &HtmlBlock<'_>) {
        self.push_node(vec![
            serde_json::json!("html.Block"),
            serde_json::json!(block.name.to_string()),
            serde_json::json!(self.depth),
        ]);

        // Emit BlockParameter nodes
        for param in &block.parameters {
            self.push_node(vec![
                serde_json::json!("html.BlockParameter"),
                serde_json::json!(param.expression.to_string()),
            ]);
        }

        self.depth += 1;
        self.visit_nodes_with_siblings(&block.children, false);
        self.depth -= 1;
    }

    fn visit_let_declaration(&mut self, decl: &HtmlLetDeclaration<'_>) {
        self.push_node(vec![
            serde_json::json!("html.LetDeclaration"),
            serde_json::json!(decl.name.to_string()),
        ]);
    }
}

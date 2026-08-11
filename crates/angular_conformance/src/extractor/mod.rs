//! Spec extractor for Angular conformance testing.
//!
//! This module parses Angular TypeScript spec files and extracts test cases
//! that can be run against the oxc Angular compiler implementation.
//!
//! ## Architecture
//!
//! The extractor walks the AST of Angular spec files looking for:
//! - `describe()` blocks that define test groups
//! - `it()` blocks that define individual tests
//! - Assertion calls like `checkAction()`, `expectFromHtml()`, `humanizeDom()`, etc.
//!
//! ## Module Structure
//!
//! - `mod.rs` - The `SpecExtractor` struct and main `extract()` method
//! - `visitors.rs` - AST visitor implementation and chain detection helpers
//! - `assertion_handlers.rs` - Handlers for extracting various assertion types
//! - `util.rs` - Utility functions for string resolution, JSON extraction, etc.

mod assertion_handlers;
mod util;
mod visitors;

use std::{collections::HashMap, fs, path::Path};

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::test_case::{
    ExpressionTokenAssertion, HtmlLexerOptions, HtmlLexerTestType, TestCase, TestGroup, TestSuite,
};

/// Extracts test cases from Angular TypeScript spec files.
///
/// The extractor parses spec files and builds a hierarchical structure of
/// test groups and test cases with their assertions.
pub struct SpecExtractor {
    source_text: String,
    /// Stack of describe() block names for building paths
    describe_stack: Vec<String>,
    /// Current test group being built
    current_groups: Vec<TestGroup>,
    /// Current test case being built
    current_test: Option<TestCase>,
    /// Current expression lexer test input (set when lex() is called)
    current_lexer_input: Option<String>,
    /// Current expression lexer expected token count
    current_lexer_token_count: Option<usize>,
    /// Current expression lexer token assertions
    current_lexer_assertions: Vec<ExpressionTokenAssertion>,
    /// Current HTML lexer test being built
    current_html_lexer_input: Option<String>,
    /// Current HTML lexer test type
    current_html_lexer_type: Option<HtmlLexerTestType>,
    /// Current HTML lexer expected array
    current_html_lexer_expected: Vec<serde_json::Value>,
    /// Current HTML lexer options
    current_html_lexer_options: Option<HtmlLexerOptions>,
    /// Track variable assignments from parse functions: variable_name -> (parse_fn_name, input_string)
    /// e.g., `const result = parseStyle('input')` -> ("result", ("parseStyle", "input"))
    pending_parse_assignments: HashMap<String, (String, String)>,
    /// Track string variable assignments: variable_name -> string_value
    /// e.g., `const html = '<p></p>'` -> ("html", "<p></p>")
    pending_string_assignments: HashMap<String, String>,
    /// Track parser.parse() result assignments: variable_name -> input_string
    /// e.g., `const ast = parser.parse(html, 'url')` -> ("ast", "<p></p>")
    pending_parse_results: HashMap<String, String>,
}

impl Default for SpecExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SpecExtractor {
    /// Creates a new spec extractor.
    pub fn new() -> Self {
        Self {
            source_text: String::new(),
            describe_stack: vec![],
            current_groups: vec![],
            current_test: None,
            current_lexer_input: None,
            current_lexer_token_count: None,
            current_lexer_assertions: vec![],
            current_html_lexer_input: None,
            current_html_lexer_type: None,
            current_html_lexer_expected: vec![],
            current_html_lexer_options: None,
            pending_parse_assignments: HashMap::new(),
            pending_string_assignments: HashMap::new(),
            pending_parse_results: HashMap::new(),
        }
    }

    /// Parse a TypeScript spec file and extract test cases.
    ///
    /// # Arguments
    ///
    /// * `spec_path` - Path to the Angular spec file (`.spec.ts`)
    ///
    /// # Returns
    ///
    /// A `TestSuite` containing all extracted test groups and test cases.
    pub fn extract(&mut self, spec_path: &Path) -> TestSuite {
        let spec_content = fs::read_to_string(spec_path).unwrap_or_default();
        self.source_text.clone_from(&spec_content);

        let allocator = Allocator::default();
        let source_type = SourceType::ts();

        let ret = Parser::new(&allocator, &spec_content, source_type).parse();
        // Note: We allow parse errors since spec files may have TypeScript features
        // that we don't fully support

        // Initialize with a root group
        self.current_groups.push(TestGroup { name: String::new(), groups: vec![], tests: vec![] });

        self.visit_program(&ret.program);

        // Get the root group
        let root_group = self.current_groups.pop().unwrap_or_default();

        let file_name =
            spec_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();

        // Store a repo-relative path so fixtures are reproducible across machines
        // (absolute paths would leak the generating developer's home directory).
        let file_path = spec_path
            .strip_prefix(project_root::get_project_root().unwrap_or_default())
            .unwrap_or(spec_path)
            .to_string_lossy()
            .to_string();

        TestSuite { name: file_name, file_path, test_groups: root_group.groups }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_spec() {
        let spec_content = r#"
describe('parser', () => {
  describe('parseAction', () => {
    it('should parse numbers', () => {
      checkAction('1');
    });

    it('should parse strings', () => {
      checkAction("'1'", '"1"');
    });
  });
});
"#;

        // Write to temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_spec.ts");
        std::fs::write(&temp_file, spec_content).unwrap();

        let mut extractor = SpecExtractor::new();
        let suite = extractor.extract(&temp_file);

        assert_eq!(suite.test_groups.len(), 1);
        assert_eq!(suite.test_groups[0].name, "parser");
        assert_eq!(suite.test_groups[0].groups.len(), 1);
        assert_eq!(suite.test_groups[0].groups[0].name, "parseAction");
        assert_eq!(suite.test_groups[0].groups[0].tests.len(), 2);

        // Clean up
        std::fs::remove_file(&temp_file).ok();
    }
}

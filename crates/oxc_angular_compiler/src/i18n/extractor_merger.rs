//! I18n message extraction and translation merging.
//!
//! This module provides functionality to:
//! 1. Extract translatable messages from an HTML AST (`extract_messages`)
//! 2. Replace translatable strings with translations (`merge_translations`)
//!
//! Ported from Angular's `i18n/extractor_merger.ts`.

use std::sync::Arc;

use indexmap::IndexMap;
use oxc_span::Span;
use rustc_hash::FxHashMap;

use crate::i18n::ast::{
    Message, MessagePlaceholder, MessageSpan, Node as I18nNode, Text as I18nText,
};
use crate::i18n::parser::{I18nMessageFactory, create_i18n_message_factory};
use crate::i18n::translation_bundle::TranslationBundle;
use crate::util::{ParseSourceFile, ParseSourceSpan};

// ============================================================================
// Translated Text Parser
// ============================================================================

/// Parses a translated text string back into TranslatedNode structures.
///
/// The translated text may contain:
/// - Plain text
/// - HTML elements like `<b>content</b>`
/// - Interpolations like `{{ expr }}`
/// - ICU expressions like `{count, plural, =1 {one} other {many}}`
///
/// This is equivalent to Angular's approach of using HtmlParser to parse the
/// translated string back to HTML nodes.
fn parse_translated_text(text: &str, span: Span) -> Vec<TranslatedNode> {
    let mut parser = TranslatedTextParser::new(text, span);
    parser.parse()
}

/// Parser for translated text strings.
struct TranslatedTextParser<'a> {
    /// The text to parse.
    text: &'a str,
    /// Current position in the text.
    position: usize,
    /// The span to use for generated nodes.
    span: Span,
}

impl<'a> TranslatedTextParser<'a> {
    fn new(text: &'a str, span: Span) -> Self {
        Self { text, position: 0, span }
    }

    fn parse(&mut self) -> Vec<TranslatedNode> {
        self.parse_nodes()
    }

    fn parse_nodes(&mut self) -> Vec<TranslatedNode> {
        let mut nodes = Vec::new();

        while !self.is_eof() {
            if self.at_opening_tag() {
                if let Some(element) = self.parse_element() {
                    nodes.push(element);
                } else {
                    // Failed to parse element, consume as text
                    if let Some(text) = self.consume_text_chunk() {
                        nodes.push(TranslatedNode::Text(text, self.span));
                    }
                }
            } else if self.at_closing_tag() {
                // We hit a closing tag - stop parsing children
                break;
            } else {
                // Parse text until we hit an element or end
                if let Some(text) = self.consume_text_until_tag() {
                    nodes.push(TranslatedNode::Text(text, self.span));
                }
            }
        }

        nodes
    }

    fn is_eof(&self) -> bool {
        self.position >= self.text.len()
    }

    fn remaining(&self) -> &'a str {
        &self.text[self.position..]
    }

    fn at_opening_tag(&self) -> bool {
        let remaining = self.remaining();
        remaining.starts_with('<') && !remaining.starts_with("</")
    }

    fn at_closing_tag(&self) -> bool {
        self.remaining().starts_with("</")
    }

    /// Parses an HTML element like `<tag attr="value">content</tag>` or `<tag/>`.
    fn parse_element(&mut self) -> Option<TranslatedNode> {
        if !self.at_opening_tag() {
            return None;
        }

        // Skip '<'
        self.position += 1;

        // Parse tag name
        let tag_name = self.parse_name()?;
        if tag_name.is_empty() {
            return None;
        }

        // Parse attributes
        let attrs = self.parse_attributes();

        self.skip_whitespace();

        // Check for self-closing or void element
        let is_self_closing = self.remaining().starts_with("/>");
        if is_self_closing {
            self.position += 2; // Skip "/>"
            return Some(TranslatedNode::Element {
                name: tag_name,
                attrs,
                children: vec![],
                span: self.span,
            });
        }

        // Skip '>'
        if !self.remaining().starts_with('>') {
            return None;
        }
        self.position += 1;

        // Parse children
        let children = self.parse_nodes();

        // Parse closing tag
        let closing_tag = format!("</{tag_name}>");
        if self.remaining().starts_with(&closing_tag) {
            self.position += closing_tag.len();
        } else {
            // Try case-insensitive match
            let closing_tag_lower = closing_tag.to_lowercase();
            let remaining_lower = self.remaining().to_lowercase();
            if remaining_lower.starts_with(&closing_tag_lower) {
                self.position += closing_tag.len();
            }
            // If no closing tag found, that's ok - we'll just continue
        }

        Some(TranslatedNode::Element { name: tag_name, attrs, children, span: self.span })
    }

    /// Parses a tag name (alphanumeric + dash).
    fn parse_name(&mut self) -> Option<String> {
        let start = self.position;
        while !self.is_eof() {
            let c = self.text.as_bytes().get(self.position)?;
            if c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_' {
                self.position += 1;
            } else {
                break;
            }
        }
        if self.position > start { Some(self.text[start..self.position].to_string()) } else { None }
    }

    /// Parses attributes from an element.
    fn parse_attributes(&mut self) -> Vec<TranslatedAttribute> {
        let mut attrs = Vec::new();

        loop {
            self.skip_whitespace();

            if self.is_eof()
                || self.remaining().starts_with('>')
                || self.remaining().starts_with("/>")
            {
                break;
            }

            if let Some(attr) = self.parse_attribute() {
                attrs.push(attr);
            } else {
                // Skip unknown character
                self.position += 1;
            }
        }

        attrs
    }

    /// Parses a single attribute like `name="value"` or `name`.
    fn parse_attribute(&mut self) -> Option<TranslatedAttribute> {
        let name = self.parse_name()?;

        self.skip_whitespace();

        // Check for '='
        if self.remaining().starts_with('=') {
            self.position += 1;
            self.skip_whitespace();

            // Parse value (quoted)
            let value = if self.remaining().starts_with('"') {
                self.position += 1;
                let value = self.consume_until('"');
                if self.remaining().starts_with('"') {
                    self.position += 1;
                }
                value
            } else if self.remaining().starts_with('\'') {
                self.position += 1;
                let value = self.consume_until('\'');
                if self.remaining().starts_with('\'') {
                    self.position += 1;
                }
                value
            } else {
                // Unquoted value
                self.consume_unquoted_value()
            };

            Some(TranslatedAttribute { name, value, span: self.span })
        } else {
            // Boolean attribute
            Some(TranslatedAttribute { name, value: String::new(), span: self.span })
        }
    }

    fn skip_whitespace(&mut self) {
        while !self.is_eof() {
            if let Some(c) = self.text.as_bytes().get(self.position) {
                if c.is_ascii_whitespace() {
                    self.position += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    fn consume_until(&mut self, delimiter: char) -> String {
        let start = self.position;
        while !self.is_eof() {
            if self.text[self.position..].starts_with(delimiter) {
                break;
            }
            self.position += 1;
        }
        unescape_html(&self.text[start..self.position])
    }

    fn consume_unquoted_value(&mut self) -> String {
        let start = self.position;
        while !self.is_eof() {
            if let Some(c) = self.text.as_bytes().get(self.position) {
                if c.is_ascii_whitespace() || *c == b'>' || *c == b'/' {
                    break;
                }
                self.position += 1;
            } else {
                break;
            }
        }
        self.text[start..self.position].to_string()
    }

    /// Consumes text until we hit a '<' tag.
    fn consume_text_until_tag(&mut self) -> Option<String> {
        let start = self.position;
        while !self.is_eof() {
            if self.remaining().starts_with('<') {
                break;
            }
            self.position += 1;
        }
        if self.position > start {
            Some(unescape_html(&self.text[start..self.position]))
        } else {
            None
        }
    }

    /// Consumes a single character as text (fallback).
    fn consume_text_chunk(&mut self) -> Option<String> {
        if self.is_eof() {
            return None;
        }
        let start = self.position;
        self.position += 1;
        Some(self.text[start..self.position].to_string())
    }
}

/// Unescapes HTML entities.
fn unescape_html(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

/// Constants for i18n markers.
const I18N_ATTR: &str = "i18n";
const I18N_ATTR_PREFIX: &str = "i18n-";
const MEANING_SEPARATOR: char = '|';
const ID_SEPARATOR: &str = "@@";

/// Result of message extraction.
#[derive(Debug)]
pub struct ExtractionResult {
    /// The extracted messages.
    pub messages: Vec<Message>,
    /// Errors encountered during extraction.
    pub errors: Vec<ParseError>,
}

impl ExtractionResult {
    /// Creates a new extraction result.
    pub fn new(messages: Vec<Message>, errors: Vec<ParseError>) -> Self {
        Self { messages, errors }
    }
}

/// A parse error.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// The source span where the error occurred.
    pub span: ParseSourceSpan,
    /// The error message.
    pub message: String,
}

impl ParseError {
    /// Creates a new parse error.
    pub fn new(span: ParseSourceSpan, message: String) -> Self {
        Self { span, message }
    }
}

/// Visitor mode - Extract or Merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitorMode {
    /// Extracting messages from the template.
    Extract,
    /// Merging translations into the template.
    Merge,
}

/// Extracts translatable messages from an HTML AST.
///
/// # Arguments
/// * `nodes` - The HTML nodes to extract from.
/// * `implicit_tags` - Tag names that are implicitly translatable.
/// * `implicit_attrs` - Attribute names that are implicitly translatable per element.
/// * `preserve_significant_whitespace` - Whether to preserve significant whitespace.
pub fn extract_messages(
    nodes: &[HtmlNodeRef<'_>],
    implicit_tags: &[String],
    implicit_attrs: &FxHashMap<String, Vec<String>>,
    preserve_significant_whitespace: bool,
    source_file: Arc<ParseSourceFile>,
) -> ExtractionResult {
    let mut visitor = I18nVisitor::new(
        implicit_tags,
        implicit_attrs,
        preserve_significant_whitespace,
        source_file,
    );
    visitor.extract(nodes)
}

/// Merges translations into an HTML AST.
///
/// # Arguments
/// * `nodes` - The HTML nodes to merge into.
/// * `translations` - The translation bundle.
/// * `implicit_tags` - Tag names that are implicitly translatable.
/// * `implicit_attrs` - Attribute names that are implicitly translatable per element.
pub fn merge_translations(
    nodes: &[HtmlNodeRef<'_>],
    translations: &TranslationBundle,
    implicit_tags: &[String],
    implicit_attrs: &FxHashMap<String, Vec<String>>,
    source_file: Arc<ParseSourceFile>,
) -> MergeResult {
    let mut visitor = I18nVisitor::new(implicit_tags, implicit_attrs, true, source_file);
    visitor.merge(nodes, translations)
}

/// Result of merging translations.
#[derive(Debug)]
pub struct MergeResult {
    /// The translated nodes.
    pub nodes: Vec<TranslatedNode>,
    /// Errors encountered during merging.
    pub errors: Vec<ParseError>,
}

/// A translated node (placeholder for actual HTML node).
#[derive(Debug, Clone)]
pub enum TranslatedNode {
    /// Plain text.
    Text(String, Span),
    /// An element with children.
    Element {
        /// Element tag name.
        name: String,
        /// Element attributes.
        attrs: Vec<TranslatedAttribute>,
        /// Child nodes.
        children: Vec<TranslatedNode>,
        /// Source span.
        span: Span,
    },
}

/// A translated attribute.
#[derive(Debug, Clone)]
pub struct TranslatedAttribute {
    /// The attribute name.
    pub name: String,
    /// The attribute value.
    pub value: String,
    /// The source span.
    pub span: Span,
}

/// Reference to an HTML node for extraction/merging.
#[derive(Debug, Clone)]
pub enum HtmlNodeRef<'a> {
    /// A text node.
    Text {
        /// Text value.
        value: &'a str,
        /// Source span.
        span: Span,
    },
    /// An element node.
    Element {
        /// Element tag name.
        name: &'a str,
        /// Element attributes.
        attrs: Vec<HtmlAttrRef<'a>>,
        /// Child nodes.
        children: Vec<HtmlNodeRef<'a>>,
        /// Full element span.
        span: Span,
        /// Opening tag span.
        start_span: Span,
        /// Closing tag span (if not self-closing).
        end_span: Option<Span>,
    },
    /// A comment node.
    Comment {
        /// Comment value.
        value: &'a str,
        /// Source span.
        span: Span,
    },
    /// An ICU expansion.
    Expansion {
        /// Switch value expression.
        switch_value: &'a str,
        /// Expansion type (e.g., "plural", "select").
        expansion_type: &'a str,
        /// Expansion cases.
        cases: Vec<HtmlExpansionCaseRef<'a>>,
        /// Source span.
        span: Span,
    },
    /// A control flow block.
    Block {
        /// Block name.
        name: &'a str,
        /// Block parameters.
        parameters: Vec<&'a str>,
        /// Block children.
        children: Vec<HtmlNodeRef<'a>>,
        /// Source span.
        span: Span,
    },
    /// A let declaration.
    LetDeclaration {
        /// Variable name.
        name: &'a str,
        /// Variable value expression.
        value: &'a str,
        /// Source span.
        span: Span,
    },
}

/// Reference to an HTML attribute.
#[derive(Debug, Clone)]
pub struct HtmlAttrRef<'a> {
    /// The attribute name.
    pub name: &'a str,
    /// The attribute value.
    pub value: &'a str,
    /// The source span.
    pub span: Span,
    /// The value tokens (for interpolation detection).
    /// True if the value contains a single interpolation with no other content.
    pub is_interpolation_only: bool,
}

/// Reference to an ICU expansion case.
#[derive(Debug, Clone)]
pub struct HtmlExpansionCaseRef<'a> {
    /// The case value.
    pub value: &'a str,
    /// The case nodes.
    pub nodes: Vec<HtmlNodeRef<'a>>,
    /// The source span.
    pub span: Span,
}

/// The i18n visitor for extraction and merging.
struct I18nVisitor<'a> {
    /// Implicit tags.
    implicit_tags: &'a [String],
    /// Implicit attributes per element.
    implicit_attrs: &'a FxHashMap<String, Vec<String>>,
    /// Whether to preserve significant whitespace (for future use).
    _preserve_significant_whitespace: bool,
    /// Current visitor mode.
    mode: VisitorMode,
    /// Current depth in the tree.
    depth: usize,
    /// Whether we're in an i18n node.
    in_i18n_node: bool,
    /// Whether we're in an implicit node.
    in_implicit_node: bool,
    /// Whether we're in an i18n block.
    in_i18n_block: bool,
    /// Block meaning and description.
    block_meaning_and_desc: String,
    /// Block children.
    block_children: Vec<I18nNode>,
    /// Block start depth.
    block_start_depth: usize,
    /// Whether we're in an ICU.
    in_icu: bool,
    /// Message count at section start.
    msg_count_at_section_start: Option<usize>,
    /// Errors encountered.
    errors: Vec<ParseError>,
    /// Extracted messages.
    messages: Vec<Message>,
    /// The i18n message factory (for future use when full message creation is implemented).
    _create_i18n_message: I18nMessageFactory,
    /// Translation bundle (for merge mode).
    translations: Option<&'a TranslationBundle>,
    /// Source file for span conversion.
    source_file: Arc<ParseSourceFile>,
}

impl<'a> I18nVisitor<'a> {
    fn to_parse_span(&self, span: Span) -> ParseSourceSpan {
        ParseSourceSpan::from_offsets(&self.source_file, span.start, span.end, None, None)
    }

    /// Creates a new i18n visitor.
    fn new(
        implicit_tags: &'a [String],
        implicit_attrs: &'a FxHashMap<String, Vec<String>>,
        preserve_significant_whitespace: bool,
        source_file: Arc<ParseSourceFile>,
    ) -> Self {
        Self {
            implicit_tags,
            implicit_attrs,
            _preserve_significant_whitespace: preserve_significant_whitespace,
            mode: VisitorMode::Extract,
            depth: 0,
            in_i18n_node: false,
            in_implicit_node: false,
            in_i18n_block: false,
            block_meaning_and_desc: String::new(),
            block_children: Vec::new(),
            block_start_depth: 0,
            in_icu: false,
            msg_count_at_section_start: None,
            errors: Vec::new(),
            messages: Vec::new(),
            _create_i18n_message: create_i18n_message_factory(
                !preserve_significant_whitespace,
                preserve_significant_whitespace,
            ),
            translations: None,
            source_file,
        }
    }

    /// Initializes the visitor for a specific mode.
    fn init(&mut self, mode: VisitorMode) {
        self.mode = mode;
        self.in_i18n_block = false;
        self.in_i18n_node = false;
        self.depth = 0;
        self.in_icu = false;
        self.msg_count_at_section_start = None;
        self.errors.clear();
        self.messages.clear();
        self.in_implicit_node = false;
    }

    /// Extracts messages from the tree.
    fn extract(&mut self, nodes: &[HtmlNodeRef<'_>]) -> ExtractionResult {
        self.init(VisitorMode::Extract);

        for node in nodes {
            self.visit_node(node);
        }

        if self.in_i18n_block {
            if let Some(last) = nodes.last() {
                self.report_error(last.span(), "Unclosed block");
            }
        }

        ExtractionResult::new(std::mem::take(&mut self.messages), std::mem::take(&mut self.errors))
    }

    /// Merges translations into the tree.
    fn merge(
        &mut self,
        nodes: &[HtmlNodeRef<'_>],
        translations: &'a TranslationBundle,
    ) -> MergeResult {
        self.init(VisitorMode::Merge);
        self.translations = Some(translations);

        let translated_nodes = self.visit_nodes_for_merge(nodes);

        if self.in_i18n_block {
            if let Some(last) = nodes.last() {
                self.report_error(last.span(), "Unclosed block");
            }
        }

        MergeResult { nodes: translated_nodes, errors: std::mem::take(&mut self.errors) }
    }

    /// Visits a list of nodes for merge, handling i18n blocks properly.
    fn visit_nodes_for_merge(&mut self, nodes: &[HtmlNodeRef<'_>]) -> Vec<TranslatedNode> {
        let mut result = Vec::new();
        let mut i18n_block_nodes: Vec<&HtmlNodeRef<'_>> = Vec::new();
        let mut i18n_block_meta = String::new();

        for node in nodes {
            if let HtmlNodeRef::Comment { value, span } = node {
                if is_opening_comment(value) && !self.in_i18n_block {
                    // Start collecting i18n block nodes
                    self.in_i18n_block = true;
                    i18n_block_meta = extract_i18n_comment_meta(value);
                    i18n_block_nodes.clear();
                    continue;
                } else if is_closing_comment(value) && self.in_i18n_block {
                    // End of i18n block - translate the collected nodes
                    self.in_i18n_block = false;

                    // Convert collected node references to owned HtmlNodeRef for translation
                    let block_children_refs: Vec<HtmlNodeRef<'_>> =
                        i18n_block_nodes.iter().map(|n| (*n).clone()).collect();

                    // Try to translate the block
                    if let Some(translated) =
                        self.translate_message_for_merge(&block_children_refs, &i18n_block_meta)
                    {
                        result.extend(translated);
                    } else {
                        // Fallback: visit each node without translation
                        for block_node in &i18n_block_nodes {
                            if let Some(translated) = self.visit_node_for_merge(block_node) {
                                result.push(translated);
                            }
                        }
                    }

                    i18n_block_nodes.clear();
                    i18n_block_meta.clear();
                    continue;
                } else if is_opening_comment(value) && self.in_i18n_block {
                    self.report_error(
                        *span,
                        "Could not start a block inside a translatable section",
                    );
                    continue;
                } else if is_closing_comment(value) && !self.in_i18n_block {
                    self.report_error(*span, "Trying to close an unopened block");
                    continue;
                }
                // Regular comment inside i18n block - collect it
            }

            if self.in_i18n_block {
                // Collect nodes inside i18n block
                i18n_block_nodes.push(node);
            } else {
                // Process node normally
                if let Some(translated) = self.visit_node_for_merge(node) {
                    result.push(translated);
                }
            }
        }

        result
    }

    /// Visits a node for extraction.
    fn visit_node(&mut self, node: &HtmlNodeRef<'_>) {
        match node {
            HtmlNodeRef::Text { value, span } => self.visit_text(value, *span),
            HtmlNodeRef::Element { name, attrs, children, span, start_span, end_span } => {
                self.visit_element(name, attrs, children, *span, *start_span, *end_span);
            }
            HtmlNodeRef::Comment { value, span } => self.visit_comment(value, *span),
            HtmlNodeRef::Expansion { switch_value, expansion_type, cases, span } => {
                self.visit_expansion(switch_value, expansion_type, cases, *span);
            }
            HtmlNodeRef::Block { name, parameters, children, span } => {
                self.visit_block(name, parameters, children, *span);
            }
            HtmlNodeRef::LetDeclaration { .. } => {
                // Let declarations are not translatable
            }
        }
    }

    /// Visits a node for merge and returns the translated node.
    fn visit_node_for_merge(&mut self, node: &HtmlNodeRef<'_>) -> Option<TranslatedNode> {
        match node {
            HtmlNodeRef::Text { value, span } => {
                Some(TranslatedNode::Text(value.to_string(), *span))
            }
            HtmlNodeRef::Element { name, attrs, children, span, .. } => {
                // Check for i18n attribute
                let i18n_attr = attrs.iter().find(|a| a.name == I18N_ATTR);
                let i18n_meta = i18n_attr.map(|a| a.value).unwrap_or("");

                // Translate the element's children if it has i18n attribute
                let translated_children = if !i18n_meta.is_empty() || i18n_attr.is_some() {
                    // Create a message from children and look up translation
                    if let Some(translated) = self.translate_message_for_merge(children, i18n_meta)
                    {
                        translated
                    } else {
                        // Fallback to original children if no translation found
                        // Use visit_nodes_for_merge to handle nested i18n blocks
                        self.visit_nodes_for_merge(children)
                    }
                } else {
                    // Use visit_nodes_for_merge to handle nested i18n blocks
                    self.visit_nodes_for_merge(children)
                };

                // Translate attributes (with i18n-* handling)
                let translated_attrs = self.translate_attributes_for_merge(name, attrs);

                Some(TranslatedNode::Element {
                    name: name.to_string(),
                    attrs: translated_attrs,
                    children: translated_children,
                    span: *span,
                })
            }
            HtmlNodeRef::Comment { .. } => None,
            HtmlNodeRef::Expansion { switch_value, expansion_type, cases, span } => {
                // ICU expansions - create a simple text representation
                let mut text = String::new();
                text.push('{');
                text.push_str(switch_value);
                text.push_str(", ");
                text.push_str(expansion_type);
                text.push_str(", ");
                for (i, case) in cases.iter().enumerate() {
                    if i > 0 {
                        text.push(' ');
                    }
                    text.push_str(case.value);
                    text.push_str(" {");
                    for node in &case.nodes {
                        if let HtmlNodeRef::Text { value, .. } = node {
                            text.push_str(value);
                        }
                    }
                    text.push('}');
                }
                text.push('}');
                Some(TranslatedNode::Text(text, *span))
            }
            HtmlNodeRef::Block { children, span, .. } => {
                // For blocks, we translate children using visit_nodes_for_merge
                // to handle any nested i18n blocks
                let translated_children = self.visit_nodes_for_merge(children);
                if translated_children.is_empty() {
                    None
                } else {
                    Some(TranslatedNode::Element {
                        name: "ng-container".to_string(),
                        attrs: vec![],
                        children: translated_children,
                        span: *span,
                    })
                }
            }
            HtmlNodeRef::LetDeclaration { .. } => None,
        }
    }

    /// Translates a message for merge mode.
    /// Returns the translated children as TranslatedNode list, or None if no translation found.
    ///
    /// This implements the translation merge logic from Angular's extractor_merger.ts:
    /// 1. Create an i18n Message from the source HTML children
    /// 2. Populate the message's placeholders map with interpolation values
    /// 3. Look up the translation in the TranslationBundle
    /// 4. The TranslationBundle converts i18n nodes to text with placeholder substitution
    /// 5. Parse the translated text back to TranslatedNode structures
    fn translate_message_for_merge(
        &self,
        children: &[HtmlNodeRef<'_>],
        meta: &str,
    ) -> Option<Vec<TranslatedNode>> {
        // Skip empty or placeholder-only messages
        if children.is_empty() || is_placeholder_only_message(children) {
            return None;
        }

        let translations = self.translations?;

        // Create i18n nodes and collect placeholders from the source HTML
        let (i18n_nodes, placeholders) =
            self.convert_children_to_i18n_nodes_with_placeholders(children);
        if i18n_nodes.is_empty() {
            return None;
        }

        let (meaning, description, id) = parse_message_meta(meta);
        let message = Message::new(
            i18n_nodes,
            placeholders,
            FxHashMap::default(), // placeholder_to_message (for nested ICU)
            meaning,
            description,
            id,
        );

        // Look up translation - this returns the translated text with placeholders substituted
        if let Some(translated_text) = translations.get(&message) {
            // Parse the translated text back to TranslatedNode structures
            // This handles HTML elements, interpolations, and plain text
            let span = children.first().map(HtmlNodeRef::span).unwrap_or_default();
            Some(parse_translated_text(&translated_text, span))
        } else {
            None
        }
    }

    /// Converts HTML children to i18n nodes and collects placeholders.
    ///
    /// Returns a tuple of (i18n nodes, placeholders map).
    /// The placeholders map contains interpolation expressions that can be substituted
    /// when the translation is retrieved.
    fn convert_children_to_i18n_nodes_with_placeholders(
        &self,
        children: &[HtmlNodeRef<'_>],
    ) -> (Vec<I18nNode>, FxHashMap<String, MessagePlaceholder>) {
        let mut nodes = Vec::new();
        let mut placeholders = FxHashMap::default();
        let mut interpolation_index = 0u32;

        for child in children {
            self.convert_child_with_placeholders(
                child,
                &mut nodes,
                &mut placeholders,
                &mut interpolation_index,
            );
        }

        (nodes, placeholders)
    }

    /// Converts a single HTML child node to i18n nodes and collects placeholders.
    fn convert_child_with_placeholders(
        &self,
        child: &HtmlNodeRef<'_>,
        nodes: &mut Vec<I18nNode>,
        placeholders: &mut FxHashMap<String, MessagePlaceholder>,
        interpolation_index: &mut u32,
    ) {
        match child {
            HtmlNodeRef::Text { value, span } => {
                // Check for interpolations in text
                let text = *value;
                if text.contains("{{") {
                    // Parse interpolations and create placeholders
                    self.parse_text_with_interpolations(
                        text,
                        *span,
                        nodes,
                        placeholders,
                        interpolation_index,
                    );
                } else if !text.trim().is_empty() {
                    nodes.push(I18nNode::Text(I18nText::new(
                        text.to_string(),
                        self.to_parse_span(*span),
                    )));
                }
            }
            HtmlNodeRef::Element { name, children: el_children, span, .. } => {
                // Recursively convert element children
                let mut child_nodes = Vec::new();
                for el_child in el_children {
                    self.convert_child_with_placeholders(
                        el_child,
                        &mut child_nodes,
                        placeholders,
                        interpolation_index,
                    );
                }
                if !child_nodes.is_empty() {
                    nodes.push(I18nNode::Container(crate::i18n::ast::Container::new(
                        child_nodes,
                        self.to_parse_span(*span),
                    )));
                } else if !name.is_empty() {
                    // Empty element - add as text placeholder
                    nodes.push(I18nNode::Text(I18nText::new(
                        format!("<{name}></{name}>"),
                        self.to_parse_span(*span),
                    )));
                }
            }
            HtmlNodeRef::Expansion { switch_value, expansion_type, cases, span } => {
                // Create ICU node
                let mut icu_cases: IndexMap<String, I18nNode> = IndexMap::default();
                for case in cases {
                    let case_text = case
                        .nodes
                        .iter()
                        .filter_map(|n| {
                            if let HtmlNodeRef::Text { value, .. } = n {
                                Some(value.to_string())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    icu_cases.insert(
                        case.value.to_string(),
                        I18nNode::Text(I18nText::new(case_text, self.to_parse_span(case.span))),
                    );
                }
                let icu = crate::i18n::ast::Icu::new(
                    switch_value.to_string(),
                    expansion_type.to_string(),
                    icu_cases,
                    self.to_parse_span(*span),
                    None,
                );
                nodes.push(I18nNode::Icu(icu));
            }
            HtmlNodeRef::Block { children: block_children, span, .. } => {
                // Recursively convert block children
                let mut child_nodes = Vec::new();
                for block_child in block_children {
                    self.convert_child_with_placeholders(
                        block_child,
                        &mut child_nodes,
                        placeholders,
                        interpolation_index,
                    );
                }
                if !child_nodes.is_empty() {
                    nodes.push(I18nNode::Container(crate::i18n::ast::Container::new(
                        child_nodes,
                        self.to_parse_span(*span),
                    )));
                }
            }
            HtmlNodeRef::Comment { .. } | HtmlNodeRef::LetDeclaration { .. } => {
                // Skip comments and let declarations
            }
        }
    }

    /// Parses text containing interpolations like "Hello {{ name }}!" and creates
    /// i18n nodes with placeholders.
    fn parse_text_with_interpolations(
        &self,
        text: &str,
        span: Span,
        nodes: &mut Vec<I18nNode>,
        placeholders: &mut FxHashMap<String, MessagePlaceholder>,
        interpolation_index: &mut u32,
    ) {
        let mut remaining = text;
        let source_span = self.to_parse_span(span);

        while !remaining.is_empty() {
            if let Some(start_idx) = remaining.find("{{") {
                // Add text before interpolation
                if start_idx > 0 {
                    let text_before = &remaining[..start_idx];
                    if !text_before.trim().is_empty() {
                        nodes.push(I18nNode::Text(I18nText::new(
                            text_before.to_string(),
                            source_span.clone(),
                        )));
                    }
                }

                // Find end of interpolation
                if let Some(end_idx) = remaining[start_idx..].find("}}") {
                    let interp_end = start_idx + end_idx + 2;
                    let interp_text = &remaining[start_idx..interp_end];
                    let expr = remaining[start_idx + 2..start_idx + end_idx].trim();

                    // Create placeholder name
                    let ph_name = if *interpolation_index == 0 {
                        "INTERPOLATION".to_string()
                    } else {
                        format!("INTERPOLATION_{}", interpolation_index)
                    };
                    *interpolation_index += 1;

                    // Add placeholder to map
                    placeholders.insert(
                        ph_name.clone(),
                        MessagePlaceholder {
                            text: interp_text.to_string(),
                            source_span: source_span.clone(),
                        },
                    );

                    // Add placeholder node
                    nodes.push(I18nNode::Placeholder(crate::i18n::ast::Placeholder::new(
                        expr.to_string(),
                        ph_name,
                        source_span.clone(),
                    )));

                    remaining = &remaining[interp_end..];
                } else {
                    // No closing }}, treat rest as text
                    nodes.push(I18nNode::Text(I18nText::new(
                        remaining.to_string(),
                        source_span.clone(),
                    )));
                    break;
                }
            } else {
                // No more interpolations, add remaining text
                if !remaining.trim().is_empty() {
                    nodes.push(I18nNode::Text(I18nText::new(
                        remaining.to_string(),
                        source_span.clone(),
                    )));
                }
                break;
            }
        }
    }

    /// Translates attributes for merge mode, handling i18n-* attributes.
    fn translate_attributes_for_merge(
        &self,
        element_name: &str,
        attrs: &[HtmlAttrRef<'_>],
    ) -> Vec<TranslatedAttribute> {
        let mut explicit_attr_meta: FxHashMap<String, String> = FxHashMap::default();
        let implicit_attr_names =
            self.implicit_attrs.get(element_name).cloned().unwrap_or_default();

        // Collect i18n-* metadata
        for attr in attrs {
            if attr.name.starts_with(I18N_ATTR_PREFIX) {
                let target_name = &attr.name[I18N_ATTR_PREFIX.len()..];
                explicit_attr_meta.insert(target_name.to_string(), attr.value.to_string());
            }
        }

        attrs
            .iter()
            .filter_map(|attr| {
                // Skip i18n-* attributes
                if attr.name == I18N_ATTR || attr.name.starts_with(I18N_ATTR_PREFIX) {
                    return None;
                }

                // Check if this attribute needs translation
                let i18n_meta = explicit_attr_meta.get(attr.name);
                let needs_translation =
                    i18n_meta.is_some() || implicit_attr_names.iter().any(|n| n == attr.name);

                if needs_translation && !attr.is_interpolation_only && !attr.value.trim().is_empty()
                {
                    // Look up translation for this attribute
                    if let Some(translated_value) = self.translate_attribute_value_for_merge(
                        attr.value,
                        i18n_meta.map(String::as_str).unwrap_or(""),
                        attr.span,
                    ) {
                        return Some(TranslatedAttribute {
                            name: attr.name.to_string(),
                            value: translated_value,
                            span: attr.span,
                        });
                    }
                }

                // Return original attribute
                Some(TranslatedAttribute {
                    name: attr.name.to_string(),
                    value: attr.value.to_string(),
                    span: attr.span,
                })
            })
            .collect()
    }

    /// Translates an attribute value for merge mode.
    fn translate_attribute_value_for_merge(
        &self,
        value: &str,
        meta: &str,
        span: Span,
    ) -> Option<String> {
        let translations = self.translations?;

        let (meaning, description, id) = parse_message_meta(meta);
        let nodes =
            vec![I18nNode::Text(I18nText::new(value.to_string(), self.to_parse_span(span)))];

        let message = Message::new(
            nodes,
            FxHashMap::default(),
            FxHashMap::default(),
            meaning,
            description,
            id,
        );

        translations.get(&message)
    }

    /// Visits a text node.
    fn visit_text(&mut self, value: &str, span: Span) {
        if self.is_in_translatable_section() {
            self.maybe_add_block_children_text(value, span);
        }
    }

    /// Visits an element.
    fn visit_element(
        &mut self,
        name: &str,
        attrs: &[HtmlAttrRef<'_>],
        children: &[HtmlNodeRef<'_>],
        _span: Span,
        _start_span: Span,
        _end_span: Option<Span>,
    ) {
        self.depth += 1;
        let was_in_i18n_node = self.in_i18n_node;
        let was_in_implicit_node = self.in_implicit_node;

        // Check for i18n attribute
        let i18n_attr = attrs.iter().find(|a| a.name == I18N_ATTR);
        let i18n_meta = i18n_attr.map(|a| a.value).unwrap_or("");

        let is_implicit = self.implicit_tags.iter().any(|t| t == name)
            && !self.in_icu
            && !self.is_in_translatable_section();
        let is_top_level_implicit = !was_in_implicit_node && is_implicit;
        self.in_implicit_node = was_in_implicit_node || is_implicit;

        if !self.is_in_translatable_section() && !self.in_icu {
            if i18n_attr.is_some() || is_top_level_implicit {
                self.in_i18n_node = true;
                // Add message for the element's content
                self.add_message_from_children(children, i18n_meta);
            }

            if self.mode == VisitorMode::Extract {
                let is_translatable = i18n_attr.is_some() || is_top_level_implicit;
                if is_translatable {
                    self.open_translatable_section();
                }
                for child in children {
                    self.visit_node(child);
                }
                if is_translatable {
                    self.close_translatable_section(children);
                }
            }
        } else {
            if self.mode == VisitorMode::Extract {
                for child in children {
                    self.visit_node(child);
                }
            }
        }

        // Visit attributes for i18n-* markers
        self.visit_attributes_of(name, attrs);

        self.depth -= 1;
        self.in_i18n_node = was_in_i18n_node;
        self.in_implicit_node = was_in_implicit_node;
    }

    /// Visits element attributes for translatable content.
    fn visit_attributes_of(&mut self, element_name: &str, attrs: &[HtmlAttrRef<'_>]) {
        let mut explicit_attr_names: FxHashMap<String, String> = FxHashMap::default();
        let implicit_attr_names =
            self.implicit_attrs.get(element_name).cloned().unwrap_or_default();

        // Collect explicit i18n-* attributes
        for attr in attrs {
            if attr.name.starts_with(I18N_ATTR_PREFIX) {
                let target_name = &attr.name[I18N_ATTR_PREFIX.len()..];
                explicit_attr_names.insert(target_name.to_string(), attr.value.to_string());
            }
        }

        // Process attributes
        for attr in attrs {
            if let Some(i18n_meta) = explicit_attr_names.get(attr.name) {
                self.add_message_from_attr(
                    attr.name,
                    attr.value,
                    i18n_meta,
                    attr.span,
                    attr.is_interpolation_only,
                );
            } else if implicit_attr_names.iter().any(|n| n == attr.name) {
                self.add_message_from_attr(
                    attr.name,
                    attr.value,
                    "",
                    attr.span,
                    attr.is_interpolation_only,
                );
            }
        }
    }

    /// Visits a comment node.
    fn visit_comment(&mut self, value: &str, span: Span) {
        let is_opening = is_opening_comment(value);

        if is_opening && self.is_in_translatable_section() {
            self.report_error(span, "Could not start a block inside a translatable section");
            return;
        }

        let is_closing = is_closing_comment(value);

        if is_closing && !self.in_i18n_block {
            self.report_error(span, "Trying to close an unopened block");
            return;
        }

        if !self.in_i18n_node && !self.in_icu {
            if !self.in_i18n_block {
                if is_opening {
                    // Start i18n block
                    self.in_i18n_block = true;
                    self.block_start_depth = self.depth;
                    self.block_children.clear();
                    self.block_meaning_and_desc = extract_i18n_comment_meta(value);
                    self.open_translatable_section();
                }
            } else if is_closing {
                if self.depth == self.block_start_depth {
                    self.close_translatable_section_from_block();
                    self.in_i18n_block = false;
                    let message =
                        self.add_message_from_block_children(&self.block_meaning_and_desc.clone());
                    // In merge mode, we would translate here
                    if self.mode == VisitorMode::Merge && message.is_some() {
                        // Translation merging would happen here
                    }
                } else {
                    self.report_error(span, "I18N blocks should not cross element boundaries");
                }
            }
        }
    }

    /// Visits an ICU expansion.
    fn visit_expansion(
        &mut self,
        switch_value: &str,
        expansion_type: &str,
        cases: &[HtmlExpansionCaseRef<'_>],
        span: Span,
    ) {
        self.maybe_add_block_children_icu(switch_value, expansion_type, span);

        let was_in_icu = self.in_icu;

        if !self.in_icu {
            // Nested ICUs should not be extracted but translated as a whole
            if self.is_in_translatable_section() {
                // Add ICU message
                self.add_icu_message(switch_value, expansion_type, cases, span);
            }
            self.in_icu = true;
        }

        // Visit cases
        for case in cases {
            for node in &case.nodes {
                self.visit_node(node);
            }
        }

        self.in_icu = was_in_icu;
    }

    /// Visits a block.
    fn visit_block(
        &mut self,
        _name: &str,
        _parameters: &[&str],
        children: &[HtmlNodeRef<'_>],
        _span: Span,
    ) {
        for child in children {
            self.visit_node(child);
        }
    }

    /// Checks if we're in a translatable section.
    fn is_in_translatable_section(&self) -> bool {
        self.msg_count_at_section_start.is_some()
    }

    /// Opens a translatable section.
    fn open_translatable_section(&mut self) {
        if self.is_in_translatable_section() {
            // Already in a section - this is an error we should report elsewhere
            return;
        }
        self.msg_count_at_section_start = Some(self.messages.len());
    }

    /// Closes a translatable section.
    fn close_translatable_section(&mut self, _direct_children: &[HtmlNodeRef<'_>]) {
        if !self.is_in_translatable_section() {
            return;
        }
        self.msg_count_at_section_start = None;
    }

    /// Closes a translatable section from a block.
    fn close_translatable_section_from_block(&mut self) {
        if !self.is_in_translatable_section() {
            return;
        }
        self.msg_count_at_section_start = None;
    }

    /// Maybe adds block children for text.
    fn maybe_add_block_children_text(&mut self, value: &str, span: Span) {
        if self.in_i18n_block && !self.in_icu && self.depth == self.block_start_depth {
            self.block_children
                .push(I18nNode::Text(I18nText::new(value.to_string(), self.to_parse_span(span))));
        }
    }

    /// Maybe adds block children for ICU.
    fn maybe_add_block_children_icu(
        &self,
        _switch_value: &str,
        _expansion_type: &str,
        _span: Span,
    ) {
        // ICU handling would go here
    }

    /// Adds a message from children.
    /// Returns None if children are empty, whitespace-only, or placeholder-only.
    fn add_message_from_children(
        &mut self,
        children: &[HtmlNodeRef<'_>],
        meta: &str,
    ) -> Option<Message> {
        // Skip empty children
        if children.is_empty() {
            return None;
        }

        // Skip placeholder-only messages (e.g., <div i18n>{{ name }}</div>)
        if is_placeholder_only_message(children) {
            return None;
        }

        // Convert children to i18n nodes
        let i18n_nodes = self.convert_children_to_i18n_nodes(children);

        // Skip if all children are whitespace-only
        if i18n_nodes.is_empty() {
            return None;
        }

        let (meaning, description, id) = parse_message_meta(meta);

        let message = Message::new(
            i18n_nodes,
            FxHashMap::default(),
            FxHashMap::default(),
            meaning,
            description,
            id,
        );
        self.messages.push(message.clone());
        Some(message)
    }

    /// Converts HTML children to i18n nodes.
    fn convert_children_to_i18n_nodes(&self, children: &[HtmlNodeRef<'_>]) -> Vec<I18nNode> {
        let mut nodes = Vec::new();

        for child in children {
            match child {
                HtmlNodeRef::Text { value, span } => {
                    // Skip whitespace-only text nodes
                    if !value.trim().is_empty() {
                        nodes.push(I18nNode::Text(I18nText::new(
                            value.to_string(),
                            self.to_parse_span(*span),
                        )));
                    }
                }
                HtmlNodeRef::Element { name, children: el_children, span, .. } => {
                    // Recursively convert element children
                    let child_nodes = self.convert_children_to_i18n_nodes(el_children);
                    if !child_nodes.is_empty() {
                        // Create a container for nested elements
                        nodes.push(I18nNode::Container(crate::i18n::ast::Container::new(
                            child_nodes,
                            self.to_parse_span(*span),
                        )));
                    } else if !name.is_empty() {
                        // Empty element - add as text placeholder
                        nodes.push(I18nNode::Text(I18nText::new(
                            format!("<{name}></{name}>"),
                            self.to_parse_span(*span),
                        )));
                    }
                }
                HtmlNodeRef::Expansion { switch_value, expansion_type, cases, span } => {
                    // Create ICU node
                    let mut icu_cases: IndexMap<String, I18nNode> = IndexMap::default();
                    for case in cases {
                        let case_text = case
                            .nodes
                            .iter()
                            .filter_map(|n| {
                                if let HtmlNodeRef::Text { value, .. } = n {
                                    Some(value.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        icu_cases.insert(
                            case.value.to_string(),
                            I18nNode::Text(I18nText::new(case_text, self.to_parse_span(case.span))),
                        );
                    }
                    let icu = crate::i18n::ast::Icu::new(
                        switch_value.to_string(),
                        expansion_type.to_string(),
                        icu_cases,
                        self.to_parse_span(*span),
                        None,
                    );
                    nodes.push(I18nNode::Icu(icu));
                }
                HtmlNodeRef::Block { children: block_children, span, .. } => {
                    // Recursively convert block children
                    let child_nodes = self.convert_children_to_i18n_nodes(block_children);
                    if !child_nodes.is_empty() {
                        nodes.push(I18nNode::Container(crate::i18n::ast::Container::new(
                            child_nodes,
                            self.to_parse_span(*span),
                        )));
                    }
                }
                HtmlNodeRef::Comment { .. } | HtmlNodeRef::LetDeclaration { .. } => {
                    // Skip comments and let declarations
                }
            }
        }

        nodes
    }

    /// Adds a message from an attribute.
    /// Returns None if the attribute value is empty or interpolation-only.
    fn add_message_from_attr(
        &mut self,
        _name: &str,
        value: &str,
        meta: &str,
        span: Span,
        is_interpolation_only: bool,
    ) {
        // Skip empty attribute values (e.g., <div i18n-title title="">)
        if value.trim().is_empty() {
            return;
        }

        // Skip interpolation-only attribute values (e.g., <div i18n-title title="{{ name }}">)
        if is_interpolation_only {
            return;
        }

        let (meaning, description, id) = parse_message_meta(meta);

        let nodes =
            vec![I18nNode::Text(I18nText::new(value.to_string(), self.to_parse_span(span)))];

        let mut message = Message::new(
            nodes,
            FxHashMap::default(),
            FxHashMap::default(),
            meaning,
            description,
            id,
        );

        let source_span = self.to_parse_span(span);
        message.sources = vec![MessageSpan {
            file_path: source_span.start.file.url.to_string(),
            start_line: source_span.start.line + 1,
            start_col: source_span.start.col + 1,
            end_line: source_span.end.line + 1,
            end_col: source_span.start.col + 1,
        }];

        self.messages.push(message);
    }

    /// Adds a message from block children.
    fn add_message_from_block_children(&mut self, meta: &str) -> Option<Message> {
        if self.block_children.is_empty() {
            return None;
        }

        let (meaning, description, id) = parse_message_meta(meta);

        let message = Message::new(
            std::mem::take(&mut self.block_children),
            FxHashMap::default(),
            FxHashMap::default(),
            meaning,
            description,
            id,
        );
        self.messages.push(message.clone());
        Some(message)
    }

    /// Adds an ICU message.
    fn add_icu_message(
        &mut self,
        switch_value: &str,
        expansion_type: &str,
        cases: &[HtmlExpansionCaseRef<'_>],
        _span: Span,
    ) {
        let mut icu_cases: IndexMap<String, I18nNode> = IndexMap::default();
        for case in cases {
            // Convert case nodes to i18n nodes
            let case_text = case
                .nodes
                .iter()
                .filter_map(|n| {
                    if let HtmlNodeRef::Text { value, .. } = n {
                        Some(value.to_string())
                    } else {
                        None
                    }
                })
                .collect::<String>();
            icu_cases.insert(
                case.value.to_string(),
                I18nNode::Text(I18nText::new(case_text, self.to_parse_span(case.span))),
            );
        }

        let icu = crate::i18n::ast::Icu::new(
            switch_value.to_string(),
            expansion_type.to_string(),
            icu_cases,
            self.to_parse_span(_span),
            None,
        );

        let message = Message::new(
            vec![I18nNode::Icu(icu)],
            FxHashMap::default(),
            FxHashMap::default(),
            String::new(),
            String::new(),
            String::new(),
        );
        self.messages.push(message);
    }

    /// Reports an error.
    fn report_error(&mut self, span: Span, msg: &str) {
        self.errors.push(ParseError::new(self.to_parse_span(span), msg.to_string()));
    }
}

impl HtmlNodeRef<'_> {
    /// Returns the source span for this node.
    fn span(&self) -> Span {
        match self {
            HtmlNodeRef::Text { span, .. }
            | HtmlNodeRef::Element { span, .. }
            | HtmlNodeRef::Comment { span, .. }
            | HtmlNodeRef::Expansion { span, .. }
            | HtmlNodeRef::Block { span, .. }
            | HtmlNodeRef::LetDeclaration { span, .. } => *span,
        }
    }
}

/// Checks if a list of HTML nodes contains only a single interpolation (placeholder-only message).
/// E.g., `<div i18n>{{ name }}</div>` should not be extracted because the message would be
/// meaningless - it's just a placeholder with no translatable content.
fn is_placeholder_only_message(nodes: &[HtmlNodeRef<'_>]) -> bool {
    // Only check if there's a single text node
    if nodes.len() != 1 {
        return false;
    }

    match &nodes[0] {
        HtmlNodeRef::Text { value, .. } => {
            let trimmed = value.trim();
            // Check if the value is only a single interpolation
            // A single interpolation looks like: {{ expr }}
            // Optionally with whitespace around it
            if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                // Count interpolation pairs
                let start_count = trimmed.matches("{{").count();
                let end_count = trimmed.matches("}}").count();
                // If there's exactly one interpolation and no text outside
                if start_count == 1 && end_count == 1 {
                    // Check if there's any text after closing }}
                    let after_close = trimmed.rfind("}}").map(|i| &trimmed[i + 2..]);
                    let before_open = trimmed.find("{{").map(|i| &trimmed[..i]);
                    return after_close.is_none_or(|s| s.trim().is_empty())
                        && before_open.is_none_or(|s| s.trim().is_empty());
                }
            }
            false
        }
        _ => false,
    }
}

/// Checks if a comment is an opening i18n comment.
fn is_opening_comment(value: &str) -> bool {
    value.starts_with("i18n")
}

/// Checks if a comment is a closing i18n comment.
fn is_closing_comment(value: &str) -> bool {
    value == "/i18n"
}

/// Extracts metadata from an i18n comment.
fn extract_i18n_comment_meta(value: &str) -> String {
    // Remove "i18n:" or "i18n" prefix
    let trimmed = value.strip_prefix("i18n:").or_else(|| value.strip_prefix("i18n"));
    trimmed.unwrap_or("").trim().to_string()
}

/// Parses message metadata from an i18n attribute value.
fn parse_message_meta(i18n: &str) -> (String, String, String) {
    if i18n.is_empty() {
        return (String::new(), String::new(), String::new());
    }

    // Format: "meaning|description@@id"
    let (meaning_and_desc, id) = if let Some(id_index) = i18n.find(ID_SEPARATOR) {
        (i18n[..id_index].to_string(), i18n[id_index + 2..].trim().to_string())
    } else {
        (i18n.to_string(), String::new())
    };

    let (meaning, description) = if let Some(sep_index) = meaning_and_desc.find(MEANING_SEPARATOR) {
        (meaning_and_desc[..sep_index].to_string(), meaning_and_desc[sep_index + 1..].to_string())
    } else {
        (String::new(), meaning_and_desc)
    };

    (meaning, description, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::ParseSourceFile;
    use std::sync::Arc;

    #[test]
    fn test_parse_message_meta_empty() {
        let (meaning, description, id) = parse_message_meta("");
        assert_eq!(meaning, "");
        assert_eq!(description, "");
        assert_eq!(id, "");
    }

    #[test]
    fn test_parse_message_meta_description_only() {
        let (meaning, description, id) = parse_message_meta("This is a description");
        assert_eq!(meaning, "");
        assert_eq!(description, "This is a description");
        assert_eq!(id, "");
    }

    #[test]
    fn test_parse_message_meta_meaning_and_description() {
        let (meaning, description, id) = parse_message_meta("greeting|Hello message");
        assert_eq!(meaning, "greeting");
        assert_eq!(description, "Hello message");
        assert_eq!(id, "");
    }

    #[test]
    fn test_parse_message_meta_with_id() {
        let (meaning, description, id) = parse_message_meta("greeting|Hello message@@customId");
        assert_eq!(meaning, "greeting");
        assert_eq!(description, "Hello message");
        assert_eq!(id, "customId");
    }

    #[test]
    fn test_is_opening_comment() {
        assert!(is_opening_comment("i18n"));
        assert!(is_opening_comment("i18n:meaning|description"));
        assert!(!is_opening_comment("/i18n"));
        assert!(!is_opening_comment("not i18n"));
    }

    #[test]
    fn test_is_closing_comment() {
        assert!(is_closing_comment("/i18n"));
        assert!(!is_closing_comment("i18n"));
        assert!(!is_closing_comment("/i18n something"));
    }

    #[test]
    fn test_extract_i18n_comment_meta() {
        assert_eq!(extract_i18n_comment_meta("i18n"), "");
        assert_eq!(extract_i18n_comment_meta("i18n:meaning|desc"), "meaning|desc");
        assert_eq!(extract_i18n_comment_meta("i18n meaning|desc"), "meaning|desc");
    }

    #[test]
    fn test_extract_messages_empty() {
        let source_file = Arc::new(ParseSourceFile::new("", "<test>"));
        let result = extract_messages(&[], &[], &FxHashMap::default(), true, source_file);
        assert!(result.messages.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_parse_translated_text_plain() {
        let nodes = parse_translated_text("Hello World", Span::default());
        assert_eq!(nodes.len(), 1);
        if let TranslatedNode::Text(text, _) = &nodes[0] {
            assert_eq!(text, "Hello World");
        } else {
            panic!("Expected text node");
        }
    }

    #[test]
    fn test_parse_translated_text_with_element() {
        let nodes = parse_translated_text("Hello <b>World</b>!", Span::default());
        assert_eq!(nodes.len(), 3);
        if let TranslatedNode::Text(text, _) = &nodes[0] {
            assert_eq!(text, "Hello ");
        } else {
            panic!("Expected text node for 'Hello '");
        }
        if let TranslatedNode::Element { name, children, .. } = &nodes[1] {
            assert_eq!(name, "b");
            assert_eq!(children.len(), 1);
            if let TranslatedNode::Text(text, _) = &children[0] {
                assert_eq!(text, "World");
            } else {
                panic!("Expected text child");
            }
        } else {
            panic!("Expected element node");
        }
        if let TranslatedNode::Text(text, _) = &nodes[2] {
            assert_eq!(text, "!");
        } else {
            panic!("Expected text node for '!'");
        }
    }

    #[test]
    fn test_parse_translated_text_with_attributes() {
        let nodes = parse_translated_text("<a href=\"test.html\">Link</a>", Span::default());
        assert_eq!(nodes.len(), 1);
        if let TranslatedNode::Element { name, attrs, children, .. } = &nodes[0] {
            assert_eq!(name, "a");
            assert_eq!(attrs.len(), 1);
            assert_eq!(attrs[0].name, "href");
            assert_eq!(attrs[0].value, "test.html");
            assert_eq!(children.len(), 1);
        } else {
            panic!("Expected element node");
        }
    }

    #[test]
    fn test_parse_translated_text_self_closing() {
        let nodes = parse_translated_text("Before<br/>After", Span::default());
        assert_eq!(nodes.len(), 3);
        if let TranslatedNode::Text(text, _) = &nodes[0] {
            assert_eq!(text, "Before");
        }
        if let TranslatedNode::Element { name, children, .. } = &nodes[1] {
            assert_eq!(name, "br");
            assert!(children.is_empty());
        }
        if let TranslatedNode::Text(text, _) = &nodes[2] {
            assert_eq!(text, "After");
        }
    }

    #[test]
    fn test_parse_translated_text_nested_elements() {
        let nodes = parse_translated_text("<div><span>Nested</span></div>", Span::default());
        assert_eq!(nodes.len(), 1);
        if let TranslatedNode::Element { name, children, .. } = &nodes[0] {
            assert_eq!(name, "div");
            assert_eq!(children.len(), 1);
            if let TranslatedNode::Element { name: inner_name, children: inner_children, .. } =
                &children[0]
            {
                assert_eq!(inner_name, "span");
                assert_eq!(inner_children.len(), 1);
            } else {
                panic!("Expected inner element");
            }
        } else {
            panic!("Expected outer element");
        }
    }

    #[test]
    fn test_parse_translated_text_html_entities() {
        let nodes = parse_translated_text("Hello &amp; World", Span::default());
        assert_eq!(nodes.len(), 1);
        if let TranslatedNode::Text(text, _) = &nodes[0] {
            assert_eq!(text, "Hello & World");
        }
    }
}

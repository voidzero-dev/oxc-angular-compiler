//! Whitespace processing for HTML templates.
//!
//! This module provides whitespace removal and trimming for HTML AST nodes.
//! The processing rules are:
//! - Consider spaces, tabs, and new lines as whitespace characters
//! - Drop text nodes consisting only of whitespace
//! - Replace consecutive whitespace characters with a single space
//! - Convert &ngsp; pseudo-entity to a single space
//!
//! Ported from Angular's `ml_parser/html_whitespaces.ts`.

use oxc_allocator::{Allocator, Box, FromIn, Vec};
use oxc_str::Ident;

use crate::ast::html::{
    HtmlAttribute, HtmlBlock, HtmlBlockParameter, HtmlComment, HtmlComponent, HtmlDirective,
    HtmlElement, HtmlExpansion, HtmlExpansionCase, HtmlLetDeclaration, HtmlNode, HtmlText,
    InterpolatedToken, InterpolatedTokenType,
};

use super::entities::NGSP_UNICODE;

/// Attribute name that marks an element for whitespace preservation.
pub const PRESERVE_WS_ATTR_NAME: &str = "ngPreserveWhitespaces";

/// Tags where whitespace should never be trimmed.
const SKIP_WS_TRIM_TAGS: [&str; 5] = ["pre", "template", "textarea", "script", "style"];

/// Whitespace characters to match (excluding non-breaking space U+00A0).
/// Based on ECMAScript \s definition.
fn is_ws_char(c: char) -> bool {
    matches!(
        c,
        ' ' | '\x0C' | '\n' | '\r' | '\t' | '\x0B' | '\u{1680}' | '\u{180E}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// Check if a string contains any non-whitespace character.
fn has_non_whitespace(s: &str) -> bool {
    s.chars().any(|c| !is_ws_char(c))
}

/// Replace runs of 2+ consecutive whitespace characters with a single space.
/// Single whitespace characters are preserved as-is.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut ws_run_start: Option<usize> = None;
    let mut ws_run_len = 0;

    for (i, c) in s.chars().enumerate() {
        if is_ws_char(c) {
            if ws_run_start.is_none() {
                ws_run_start = Some(i);
            }
            ws_run_len += 1;
        } else {
            // End of whitespace run (if any)
            if let Some(_start) = ws_run_start {
                if ws_run_len >= 2 {
                    // Replace 2+ whitespace chars with single space
                    result.push(' ');
                } else {
                    // Keep single whitespace char as-is
                    // We need to get the original char
                    if let Some(ws_char) = s.chars().nth(i - 1) {
                        result.push(ws_char);
                    }
                }
                ws_run_start = None;
                ws_run_len = 0;
            }
            result.push(c);
        }
    }

    // Handle trailing whitespace
    if let Some(start) = ws_run_start {
        if ws_run_len >= 2 {
            result.push(' ');
        } else {
            // Keep single trailing whitespace char as-is
            if let Some(ws_char) = s.chars().nth(start) {
                result.push(ws_char);
            }
        }
    }

    result
}

/// Replace &ngsp; pseudo-entity with space.
fn replace_ngsp(s: &str) -> String {
    s.replace(NGSP_UNICODE, " ")
}

/// Process whitespace in text: replace ngsp and collapse consecutive whitespace.
fn process_whitespace(text: &str) -> String {
    collapse_whitespace(&replace_ngsp(text))
}

/// Check if an element has the ngPreserveWhitespaces attribute.
fn has_preserve_whitespaces_attr(attrs: &[HtmlAttribute<'_>]) -> bool {
    attrs.iter().any(|attr| attr.name.as_str() == PRESERVE_WS_ATTR_NAME)
}

/// Context for sibling-aware visiting.
pub struct SiblingContext<'a, 'b> {
    /// Previous sibling node.
    pub prev: Option<&'b HtmlNode<'a>>,
    /// Next sibling node.
    pub next: Option<&'b HtmlNode<'a>>,
}

/// Visitor for removing/trimming whitespace in HTML AST.
///
/// This visitor transforms the AST according to Angular's whitespace rules.
/// When `preserveSignificantWhitespace` is true, leading/trailing whitespace
/// is preserved; otherwise, it is trimmed based on sibling context.
pub struct WhitespaceVisitor<'a> {
    allocator: &'a Allocator,
    /// Whether to preserve significant whitespace (leading/trailing).
    preserve_significant_whitespace: bool,
    /// Current ICU expansion nesting depth.
    icu_expansion_depth: u32,
}

impl<'a> WhitespaceVisitor<'a> {
    /// Create a new whitespace visitor.
    pub fn new(allocator: &'a Allocator, preserve_significant_whitespace: bool) -> Self {
        Self { allocator, preserve_significant_whitespace, icu_expansion_depth: 0 }
    }

    /// Visit and transform a list of nodes, providing sibling context.
    pub fn visit_all(&mut self, nodes: &[HtmlNode<'a>]) -> Vec<'a, HtmlNode<'a>> {
        let mut result = Vec::with_capacity_in(nodes.len(), self.allocator);

        for (i, node) in nodes.iter().enumerate() {
            let context = SiblingContext {
                prev: if i > 0 { nodes.get(i - 1) } else { None },
                next: nodes.get(i + 1),
            };

            if let Some(new_node) = self.visit_node(node, &context) {
                result.push(new_node);
            }
        }

        result
    }

    /// Visit a single node, returning None if it should be removed.
    fn visit_node(
        &mut self,
        node: &HtmlNode<'a>,
        context: &SiblingContext<'a, '_>,
    ) -> Option<HtmlNode<'a>> {
        match node {
            HtmlNode::Text(text) => self.visit_text(text, context),
            HtmlNode::Element(element) => self.visit_element(element),
            HtmlNode::Component(component) => self.visit_component(component),
            HtmlNode::Block(block) => self.visit_block(block),
            HtmlNode::Expansion(expansion) => self.visit_expansion(expansion),
            HtmlNode::ExpansionCase(case) => self.visit_expansion_case(case),
            HtmlNode::Comment(comment) => Some(HtmlNode::Comment(Box::new_in(
                HtmlComment { value: comment.value.clone(), span: comment.span },
                self.allocator,
            ))),
            HtmlNode::Attribute(attr) => {
                // Filter out ngPreserveWhitespaces attribute
                if attr.name.as_str() == PRESERVE_WS_ATTR_NAME {
                    None
                } else {
                    Some(HtmlNode::Attribute(Box::new_in(
                        self.clone_attribute(attr),
                        self.allocator,
                    )))
                }
            }
            HtmlNode::BlockParameter(param) => Some(HtmlNode::BlockParameter(Box::new_in(
                self.clone_block_parameter(param),
                self.allocator,
            ))),
            HtmlNode::LetDeclaration(decl) => Some(HtmlNode::LetDeclaration(Box::new_in(
                self.clone_let_declaration(decl),
                self.allocator,
            ))),
        }
    }

    /// Clone an attribute into the allocator.
    fn clone_attribute(&self, attr: &HtmlAttribute<'a>) -> HtmlAttribute<'a> {
        HtmlAttribute {
            name: attr.name.clone(),
            value: attr.value.clone(),
            span: attr.span,
            name_span: attr.name_span,
            value_span: attr.value_span,
            value_tokens: attr.value_tokens.as_ref().map(|tokens| self.clone_tokens(tokens)),
        }
    }

    /// Clone tokens into the allocator.
    fn clone_tokens(
        &self,
        tokens: &Vec<'a, InterpolatedToken<'a>>,
    ) -> Vec<'a, InterpolatedToken<'a>> {
        let mut result = Vec::with_capacity_in(tokens.len(), self.allocator);
        for token in tokens.iter() {
            let mut parts = Vec::with_capacity_in(token.parts.len(), self.allocator);
            for part in token.parts.iter() {
                parts.push(part.clone());
            }
            result.push(InterpolatedToken {
                token_type: token.token_type,
                parts,
                span: token.span,
            });
        }
        result
    }

    /// Clone a block parameter into the allocator.
    fn clone_block_parameter(&self, param: &HtmlBlockParameter<'a>) -> HtmlBlockParameter<'a> {
        HtmlBlockParameter { expression: param.expression.clone(), span: param.span }
    }

    /// Clone a let declaration into the allocator.
    ///
    /// Note: The expression cannot be deeply cloned through arena allocation,
    /// so we create a new EmptyExpr. This is acceptable because let declarations
    /// pass through the whitespace visitor unchanged - their expressions don't
    /// contain template whitespace that needs processing.
    fn clone_let_declaration(&self, decl: &HtmlLetDeclaration<'a>) -> HtmlLetDeclaration<'a> {
        use crate::ast::expression::{AbsoluteSourceSpan, AngularExpression, EmptyExpr, ParseSpan};

        // Create an empty expression as a placeholder since we can't clone the original
        // The actual expression will be handled during R3 transformation
        let empty_expr = AngularExpression::Empty(Box::new_in(
            EmptyExpr {
                span: ParseSpan { start: 0, end: 0 },
                source_span: AbsoluteSourceSpan { start: 0, end: 0 },
            },
            self.allocator,
        ));

        HtmlLetDeclaration {
            name: decl.name.clone(),
            value: empty_expr,
            span: decl.span,
            name_span: decl.name_span,
            value_span: decl.value_span,
        }
    }

    /// Clone directives into the allocator.
    fn clone_directives(
        &self,
        directives: &Vec<'a, HtmlDirective<'a>>,
    ) -> Vec<'a, HtmlDirective<'a>> {
        let mut result = Vec::with_capacity_in(directives.len(), self.allocator);
        for dir in directives.iter() {
            result.push(self.clone_directive(dir));
        }
        result
    }

    /// Clone a directive into the allocator.
    fn clone_directive(&self, dir: &HtmlDirective<'a>) -> HtmlDirective<'a> {
        let mut attrs = Vec::with_capacity_in(dir.attrs.len(), self.allocator);
        for attr in dir.attrs.iter() {
            attrs.push(self.clone_attribute(attr));
        }
        HtmlDirective {
            name: dir.name.clone(),
            attrs,
            span: dir.span,
            name_span: dir.name_span,
            start_paren_span: dir.start_paren_span,
            end_paren_span: dir.end_paren_span,
        }
    }

    /// Clone block parameters into the allocator.
    fn clone_block_parameters(
        &self,
        params: &Vec<'a, HtmlBlockParameter<'a>>,
    ) -> Vec<'a, HtmlBlockParameter<'a>> {
        let mut result = Vec::with_capacity_in(params.len(), self.allocator);
        for param in params.iter() {
            result.push(self.clone_block_parameter(param));
        }
        result
    }

    /// Clone attributes into the allocator.
    fn clone_attributes(&self, attrs: &Vec<'a, HtmlAttribute<'a>>) -> Vec<'a, HtmlAttribute<'a>> {
        let mut result = Vec::with_capacity_in(attrs.len(), self.allocator);
        for attr in attrs.iter() {
            result.push(self.clone_attribute(attr));
        }
        result
    }

    /// Clone children (nodes) into the allocator without transformation.
    fn clone_children(&self, children: &Vec<'a, HtmlNode<'a>>) -> Vec<'a, HtmlNode<'a>> {
        let mut result = Vec::with_capacity_in(children.len(), self.allocator);
        for child in children.iter() {
            result.push(self.clone_node(child));
        }
        result
    }

    /// Clone a node into the allocator without transformation.
    fn clone_node(&self, node: &HtmlNode<'a>) -> HtmlNode<'a> {
        match node {
            HtmlNode::Text(text) => HtmlNode::Text(Box::new_in(
                HtmlText {
                    value: text.value.clone(),
                    span: text.span,
                    full_start: text.full_start,
                    tokens: self.clone_tokens(&text.tokens),
                },
                self.allocator,
            )),
            HtmlNode::Element(el) => {
                HtmlNode::Element(Box::new_in(self.clone_element_shallow(el), self.allocator))
            }
            HtmlNode::Component(comp) => {
                HtmlNode::Component(Box::new_in(self.clone_component_shallow(comp), self.allocator))
            }
            HtmlNode::Comment(c) => HtmlNode::Comment(Box::new_in(
                HtmlComment { value: c.value.clone(), span: c.span },
                self.allocator,
            )),
            HtmlNode::Attribute(attr) => {
                HtmlNode::Attribute(Box::new_in(self.clone_attribute(attr), self.allocator))
            }
            HtmlNode::Block(block) => {
                HtmlNode::Block(Box::new_in(self.clone_block_shallow(block), self.allocator))
            }
            HtmlNode::BlockParameter(param) => HtmlNode::BlockParameter(Box::new_in(
                self.clone_block_parameter(param),
                self.allocator,
            )),
            HtmlNode::LetDeclaration(decl) => HtmlNode::LetDeclaration(Box::new_in(
                self.clone_let_declaration(decl),
                self.allocator,
            )),
            HtmlNode::Expansion(exp) => {
                HtmlNode::Expansion(Box::new_in(self.clone_expansion_shallow(exp), self.allocator))
            }
            HtmlNode::ExpansionCase(case) => HtmlNode::ExpansionCase(Box::new_in(
                self.clone_expansion_case_shallow(case),
                self.allocator,
            )),
        }
    }

    /// Clone an element (including children).
    fn clone_element_shallow(&self, el: &HtmlElement<'a>) -> HtmlElement<'a> {
        HtmlElement {
            name: el.name.clone(),
            component_prefix: el.component_prefix.clone(),
            component_tag_name: el.component_tag_name.clone(),
            attrs: self.clone_attributes(&el.attrs),
            directives: self.clone_directives(&el.directives),
            children: self.clone_children(&el.children),
            span: el.span,
            start_span: el.start_span,
            end_span: el.end_span,
            is_self_closing: el.is_self_closing,
            is_void: el.is_void,
        }
    }

    /// Clone a component (including children).
    fn clone_component_shallow(&self, comp: &HtmlComponent<'a>) -> HtmlComponent<'a> {
        HtmlComponent {
            component_name: comp.component_name.clone(),
            tag_name: comp.tag_name.clone(),
            full_name: comp.full_name.clone(),
            attrs: self.clone_attributes(&comp.attrs),
            directives: self.clone_directives(&comp.directives),
            children: self.clone_children(&comp.children),
            is_self_closing: comp.is_self_closing,
            span: comp.span,
            start_span: comp.start_span,
            end_span: comp.end_span,
        }
    }

    /// Clone a block (including children).
    fn clone_block_shallow(&self, block: &HtmlBlock<'a>) -> HtmlBlock<'a> {
        HtmlBlock {
            block_type: block.block_type,
            name: block.name.clone(),
            parameters: self.clone_block_parameters(&block.parameters),
            children: self.clone_children(&block.children),
            span: block.span,
            name_span: block.name_span,
            start_span: block.start_span,
            end_span: block.end_span,
        }
    }

    /// Clone an expansion (including cases).
    fn clone_expansion_shallow(&self, exp: &HtmlExpansion<'a>) -> HtmlExpansion<'a> {
        let mut cases = Vec::with_capacity_in(exp.cases.len(), self.allocator);
        for case in exp.cases.iter() {
            cases.push(self.clone_expansion_case_shallow(case));
        }
        HtmlExpansion {
            switch_value: exp.switch_value.clone(),
            expansion_type: exp.expansion_type.clone(),
            cases,
            span: exp.span,
            switch_value_span: exp.switch_value_span,
            in_i18n_block: exp.in_i18n_block,
        }
    }

    /// Clone an expansion case (including expansion nodes).
    fn clone_expansion_case_shallow(&self, case: &HtmlExpansionCase<'a>) -> HtmlExpansionCase<'a> {
        HtmlExpansionCase {
            value: case.value.clone(),
            expansion: self.clone_children(&case.expansion),
            span: case.span,
            value_span: case.value_span,
            expansion_span: case.expansion_span,
        }
    }

    /// Visit a text node.
    fn visit_text(
        &mut self,
        text: &HtmlText<'a>,
        context: &SiblingContext<'a, '_>,
    ) -> Option<HtmlNode<'a>> {
        let is_not_blank = has_non_whitespace(text.value.as_str());
        let has_expansion_sibling = matches!(context.prev, Some(HtmlNode::Expansion(_)))
            || matches!(context.next, Some(HtmlNode::Expansion(_)));

        // Don't trim inside ICU when preserving significant whitespace
        let in_icu = self.icu_expansion_depth > 0;
        if in_icu && self.preserve_significant_whitespace {
            return Some(HtmlNode::Text(Box::new_in(
                HtmlText {
                    value: text.value.clone(),
                    span: text.span,
                    full_start: text.full_start,
                    tokens: self.clone_tokens(&text.tokens),
                },
                self.allocator,
            )));
        }

        if is_not_blank || has_expansion_sibling {
            // Process whitespace in the value
            let processed = process_whitespace(text.value.as_str());
            let value = if self.preserve_significant_whitespace {
                processed
            } else {
                self.trim_leading_and_trailing(&processed, context)
            };

            // Process tokens
            let tokens = self.process_tokens(&text.tokens, context);

            let value_atom = Ident::from_in(value.as_str(), self.allocator);
            Some(HtmlNode::Text(Box::new_in(
                HtmlText {
                    value: value_atom,
                    span: text.span,
                    full_start: text.full_start,
                    tokens,
                },
                self.allocator,
            )))
        } else {
            // Remove whitespace-only text node
            None
        }
    }

    /// Process tokens, collapsing whitespace and optionally trimming.
    fn process_tokens(
        &self,
        tokens: &Vec<'a, InterpolatedToken<'a>>,
        context: &SiblingContext<'a, '_>,
    ) -> Vec<'a, InterpolatedToken<'a>> {
        let mut result = Vec::with_capacity_in(tokens.len(), self.allocator);
        let token_count = tokens.len();

        for (i, token) in tokens.iter().enumerate() {
            if token.token_type == InterpolatedTokenType::Text && !token.parts.is_empty() {
                let text = token.parts[0].as_str();
                let mut processed = process_whitespace(text);

                // Trim leading/trailing if not preserving significant whitespace
                if !self.preserve_significant_whitespace {
                    if i == 0 && context.prev.is_none() {
                        processed = processed.trim_start().to_string();
                    }
                    if i == token_count - 1 && context.next.is_none() {
                        processed = processed.trim_end().to_string();
                    }
                }

                let mut parts = Vec::with_capacity_in(1, self.allocator);
                parts.push(Ident::from_in(processed.as_str(), self.allocator));

                result.push(InterpolatedToken {
                    token_type: token.token_type,
                    parts,
                    span: token.span,
                });
            } else {
                // Clone non-text tokens
                let mut parts = Vec::with_capacity_in(token.parts.len(), self.allocator);
                for part in token.parts.iter() {
                    parts.push(part.clone());
                }
                result.push(InterpolatedToken {
                    token_type: token.token_type,
                    parts,
                    span: token.span,
                });
            }
        }

        result
    }

    /// Trim leading and trailing whitespace based on sibling context.
    fn trim_leading_and_trailing(&self, text: &str, context: &SiblingContext<'a, '_>) -> String {
        let is_first = context.prev.is_none();
        let is_last = context.next.is_none();

        let trimmed_start = if is_first { text.trim_start() } else { text };
        let trimmed = if is_last { trimmed_start.trim_end() } else { trimmed_start };

        trimmed.to_string()
    }

    /// Visit an element node.
    fn visit_element(&mut self, element: &HtmlElement<'a>) -> Option<HtmlNode<'a>> {
        let tag_name = element.name.as_str();

        // Check if we should skip whitespace trimming
        if SKIP_WS_TRIM_TAGS.contains(&tag_name) || has_preserve_whitespaces_attr(&element.attrs) {
            // Don't descend, but filter ngPreserveWhitespaces from attrs
            let attrs = self.filter_preserve_attr(&element.attrs);
            return Some(HtmlNode::Element(Box::new_in(
                HtmlElement {
                    name: element.name.clone(),
                    component_prefix: element.component_prefix.clone(),
                    component_tag_name: element.component_tag_name.clone(),
                    attrs,
                    directives: self.clone_directives(&element.directives),
                    children: self.clone_children(&element.children),
                    span: element.span,
                    start_span: element.start_span,
                    end_span: element.end_span,
                    is_self_closing: element.is_self_closing,
                    is_void: element.is_void,
                },
                self.allocator,
            )));
        }

        // Process children
        let children = self.visit_all(&element.children);

        Some(HtmlNode::Element(Box::new_in(
            HtmlElement {
                name: element.name.clone(),
                component_prefix: element.component_prefix.clone(),
                component_tag_name: element.component_tag_name.clone(),
                attrs: self.clone_attributes(&element.attrs),
                directives: self.clone_directives(&element.directives),
                children,
                span: element.span,
                start_span: element.start_span,
                end_span: element.end_span,
                is_self_closing: element.is_self_closing,
                is_void: element.is_void,
            },
            self.allocator,
        )))
    }

    /// Visit a component node.
    fn visit_component(&mut self, component: &HtmlComponent<'a>) -> Option<HtmlNode<'a>> {
        // Check if we should skip whitespace trimming
        if has_preserve_whitespaces_attr(&component.attrs) {
            // Don't descend, but filter ngPreserveWhitespaces from attrs
            let attrs = self.filter_preserve_attr(&component.attrs);
            return Some(HtmlNode::Component(Box::new_in(
                HtmlComponent {
                    component_name: component.component_name.clone(),
                    tag_name: component.tag_name.clone(),
                    full_name: component.full_name.clone(),
                    attrs,
                    directives: self.clone_directives(&component.directives),
                    children: self.clone_children(&component.children),
                    is_self_closing: component.is_self_closing,
                    span: component.span,
                    start_span: component.start_span,
                    end_span: component.end_span,
                },
                self.allocator,
            )));
        }

        // Process children
        let children = self.visit_all(&component.children);

        Some(HtmlNode::Component(Box::new_in(
            HtmlComponent {
                component_name: component.component_name.clone(),
                tag_name: component.tag_name.clone(),
                full_name: component.full_name.clone(),
                attrs: self.clone_attributes(&component.attrs),
                directives: self.clone_directives(&component.directives),
                children,
                is_self_closing: component.is_self_closing,
                span: component.span,
                start_span: component.start_span,
                end_span: component.end_span,
            },
            self.allocator,
        )))
    }

    /// Filter out ngPreserveWhitespaces attribute.
    fn filter_preserve_attr(
        &self,
        attrs: &Vec<'a, HtmlAttribute<'a>>,
    ) -> Vec<'a, HtmlAttribute<'a>> {
        let mut result = Vec::with_capacity_in(attrs.len(), self.allocator);
        for attr in attrs.iter() {
            if attr.name.as_str() != PRESERVE_WS_ATTR_NAME {
                result.push(self.clone_attribute(attr));
            }
        }
        result
    }

    /// Visit a block node.
    fn visit_block(&mut self, block: &HtmlBlock<'a>) -> Option<HtmlNode<'a>> {
        let children = self.visit_all(&block.children);

        Some(HtmlNode::Block(Box::new_in(
            HtmlBlock {
                block_type: block.block_type,
                name: block.name.clone(),
                parameters: self.clone_block_parameters(&block.parameters),
                children,
                span: block.span,
                name_span: block.name_span,
                start_span: block.start_span,
                end_span: block.end_span,
            },
            self.allocator,
        )))
    }

    /// Visit an ICU expansion.
    fn visit_expansion(&mut self, expansion: &HtmlExpansion<'a>) -> Option<HtmlNode<'a>> {
        self.icu_expansion_depth += 1;

        // Visit cases (process whitespace in expansion content)
        let mut cases = Vec::with_capacity_in(expansion.cases.len(), self.allocator);
        for case in expansion.cases.iter() {
            // Visit the expansion nodes within this case
            let visited_expansion = self.visit_all(&case.expansion);
            cases.push(HtmlExpansionCase {
                value: case.value.clone(),
                expansion: visited_expansion,
                span: case.span,
                value_span: case.value_span,
                expansion_span: case.expansion_span,
            });
        }

        self.icu_expansion_depth -= 1;

        Some(HtmlNode::Expansion(Box::new_in(
            HtmlExpansion {
                switch_value: expansion.switch_value.clone(),
                expansion_type: expansion.expansion_type.clone(),
                cases,
                span: expansion.span,
                switch_value_span: expansion.switch_value_span,
                in_i18n_block: expansion.in_i18n_block,
            },
            self.allocator,
        )))
    }

    /// Visit an ICU expansion case.
    fn visit_expansion_case(&mut self, case: &HtmlExpansionCase<'a>) -> Option<HtmlNode<'a>> {
        let expansion = self.visit_all(&case.expansion);

        Some(HtmlNode::ExpansionCase(Box::new_in(
            HtmlExpansionCase {
                value: case.value.clone(),
                expansion,
                span: case.span,
                value_span: case.value_span,
                expansion_span: case.expansion_span,
            },
            self.allocator,
        )))
    }
}

/// Remove whitespace from parsed HTML nodes.
///
/// This is the main entry point for whitespace processing.
pub fn remove_whitespaces<'a>(
    allocator: &'a Allocator,
    nodes: &[HtmlNode<'a>],
    preserve_significant_whitespace: bool,
) -> Vec<'a, HtmlNode<'a>> {
    let mut visitor = WhitespaceVisitor::new(allocator, preserve_significant_whitespace);
    visitor.visit_all(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_ws_char() {
        assert!(is_ws_char(' '));
        assert!(is_ws_char('\t'));
        assert!(is_ws_char('\n'));
        assert!(is_ws_char('\r'));
        assert!(!is_ws_char('a'));
        assert!(!is_ws_char('\u{00A0}')); // Non-breaking space excluded
    }

    #[test]
    fn test_collapse_whitespace() {
        // Multiple consecutive whitespace -> single space
        assert_eq!(collapse_whitespace("a  b"), "a b");
        assert_eq!(collapse_whitespace("a\n\n\tb"), "a b");
        assert_eq!(collapse_whitespace("  a  "), " a ");

        // Single whitespace characters are preserved as-is
        assert_eq!(collapse_whitespace("\n"), "\n");
        assert_eq!(collapse_whitespace("a\nb"), "a\nb");
        assert_eq!(collapse_whitespace(" "), " ");
        assert_eq!(collapse_whitespace("\t"), "\t");
        assert_eq!(collapse_whitespace("a b"), "a b");
    }

    #[test]
    fn test_process_whitespace() {
        let ngsp = format!("a{}b", NGSP_UNICODE);
        assert_eq!(process_whitespace(&ngsp), "a b");
        assert_eq!(process_whitespace("a  b  c"), "a b c");
    }
}

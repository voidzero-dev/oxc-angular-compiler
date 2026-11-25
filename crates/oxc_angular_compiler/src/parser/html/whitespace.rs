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

use std::borrow::Cow;

use oxc_allocator::{Allocator, Box, FromIn, Vec};
use oxc_str::Ident;

use crate::ast::html::{
    HtmlAttribute, HtmlBlock, HtmlBlockParameter, HtmlComment, HtmlComponent, HtmlDirective,
    HtmlElement, HtmlExpansion, HtmlExpansionCase, HtmlLetDeclaration, HtmlNode, HtmlText,
    InterpolatedToken, InterpolatedTokenType,
};

use super::entities::NGSP_UNICODE;
use crate::util::chars::is_js_whitespace;

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

/// Bytes that can start a whitespace char (or NGSP) in UTF-8.
/// 1 = an ASCII whitespace byte. 2 = the lead byte of a 3-byte char that may be
/// whitespace (U+1680, U+180E, U+2000-U+200A, U+2028, U+2029, U+202F, U+205F,
/// U+3000, U+FEFF) or NGSP (U+E500). Everything else is 0.
static WS_START: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b' ' as usize] = 1;
    t[b'\t' as usize] = 1;
    t[b'\n' as usize] = 1;
    t[0x0B] = 1; // \v
    t[0x0C] = 1; // \f
    t[b'\r' as usize] = 1;
    t[0xE1] = 2; // U+1680, U+180E
    t[0xE2] = 2; // U+2000-U+200A, U+2028, U+2029, U+202F, U+205F
    t[0xE3] = 2; // U+3000
    t[0xEE] = 2; // U+E500 (NGSP)
    t[0xEF] = 2; // U+FEFF
    t
};

// `ws_len_at` matches NGSP by its UTF-8 bytes EE 94 80.
const _: () = assert!(NGSP_UNICODE as u32 == 0xE500);

/// The byte length of the whitespace char (or NGSP) at byte `i`, or 0 when there
/// is none. The flag is true for NGSP.
#[inline]
fn ws_len_at(b: &[u8], i: usize) -> (usize, bool) {
    match WS_START[b[i] as usize] {
        0 => (0, false),
        1 => (1, false),
        _ => {
            // `b` is valid UTF-8, so a 0xE_ lead byte is always followed by 2 bytes.
            let (b1, b2) = (b[i + 1], b[i + 2]);
            let is_ws = match b[i] {
                0xE1 => (b1 == 0x9A && b2 == 0x80) || (b1 == 0xA0 && b2 == 0x8E),
                0xE2 => {
                    (b1 == 0x80 && (b2 <= 0x8A || b2 == 0xA8 || b2 == 0xA9 || b2 == 0xAF))
                        || (b1 == 0x81 && b2 == 0x9F)
                }
                0xE3 => b1 == 0x80 && b2 == 0x80,
                0xEF => b1 == 0xBB && b2 == 0xBF,
                // 0xEE: only NGSP.
                _ => return if b1 == 0x94 && b2 == 0x80 { (3, true) } else { (0, false) },
            };
            if is_ws { (3, false) } else { (0, false) }
        }
    }
}

/// Process whitespace in text like Angular's `processWhitespace`: replace NGSP
/// with a space, then replace each run of 2+ whitespace chars with one space.
/// A single whitespace char is kept as is.
///
/// One pass over the bytes. Unchanged parts are copied in slices, and when
/// nothing changes the input is returned as is, with no allocation. Every change
/// makes the text shorter, so an output of the same length is the input.
fn process_whitespace(text: &str) -> Cow<'_, str> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Option<String> = None;
    // Bytes from `copy_from` up to the current run are not copied yet.
    let mut copy_from = 0;
    let mut i = 0;
    while i < n {
        let (len, is_ngsp) = ws_len_at(b, i);
        if len == 0 {
            i += 1;
            continue;
        }
        let run_start = i;
        i += len;
        let mut count = 1;
        while i < n {
            let (len, _) = ws_len_at(b, i);
            if len == 0 {
                break;
            }
            i += len;
            count += 1;
        }
        // A run of one char is kept, unless it is NGSP.
        if count >= 2 || is_ngsp {
            let o = out.get_or_insert_with(|| String::with_capacity(n));
            o.push_str(&text[copy_from..run_start]);
            o.push(' ');
            copy_from = i;
        }
    }
    match out {
        Some(mut o) => {
            o.push_str(&text[copy_from..]);
            Cow::Owned(o)
        }
        None => Cow::Borrowed(text),
    }
}

/// `String.prototype.trimStart`.
fn js_trim_start(s: &str) -> &str {
    s.trim_start_matches(is_js_whitespace)
}

/// `String.prototype.trimEnd`.
fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_whitespace)
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
        let mut result = Vec::with_capacity_in(nodes.len(), &self.allocator);

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
                &self.allocator,
            ))),
            HtmlNode::Attribute(attr) => {
                // Filter out ngPreserveWhitespaces attribute
                if attr.name.as_str() == PRESERVE_WS_ATTR_NAME {
                    None
                } else {
                    Some(HtmlNode::Attribute(Box::new_in(
                        self.clone_attribute(attr),
                        &self.allocator,
                    )))
                }
            }
            HtmlNode::BlockParameter(param) => Some(HtmlNode::BlockParameter(Box::new_in(
                self.clone_block_parameter(param),
                &self.allocator,
            ))),
            HtmlNode::LetDeclaration(decl) => Some(HtmlNode::LetDeclaration(Box::new_in(
                self.clone_let_declaration(decl),
                &self.allocator,
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
        let mut result = Vec::with_capacity_in(tokens.len(), &self.allocator);
        for token in tokens.iter() {
            let mut parts = Vec::with_capacity_in(token.parts.len(), &self.allocator);
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
            &self.allocator,
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
        let mut result = Vec::with_capacity_in(directives.len(), &self.allocator);
        for dir in directives.iter() {
            result.push(self.clone_directive(dir));
        }
        result
    }

    /// Clone a directive into the allocator.
    fn clone_directive(&self, dir: &HtmlDirective<'a>) -> HtmlDirective<'a> {
        let mut attrs = Vec::with_capacity_in(dir.attrs.len(), &self.allocator);
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
        let mut result = Vec::with_capacity_in(params.len(), &self.allocator);
        for param in params.iter() {
            result.push(self.clone_block_parameter(param));
        }
        result
    }

    /// Clone attributes into the allocator.
    fn clone_attributes(&self, attrs: &Vec<'a, HtmlAttribute<'a>>) -> Vec<'a, HtmlAttribute<'a>> {
        let mut result = Vec::with_capacity_in(attrs.len(), &self.allocator);
        for attr in attrs.iter() {
            result.push(self.clone_attribute(attr));
        }
        result
    }

    /// Clone children (nodes) into the allocator without transformation.
    fn clone_children(&self, children: &Vec<'a, HtmlNode<'a>>) -> Vec<'a, HtmlNode<'a>> {
        let mut result = Vec::with_capacity_in(children.len(), &self.allocator);
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
                &self.allocator,
            )),
            HtmlNode::Element(el) => {
                HtmlNode::Element(Box::new_in(self.clone_element_shallow(el), &self.allocator))
            }
            HtmlNode::Component(comp) => HtmlNode::Component(Box::new_in(
                self.clone_component_shallow(comp),
                &self.allocator,
            )),
            HtmlNode::Comment(c) => HtmlNode::Comment(Box::new_in(
                HtmlComment { value: c.value.clone(), span: c.span },
                &self.allocator,
            )),
            HtmlNode::Attribute(attr) => {
                HtmlNode::Attribute(Box::new_in(self.clone_attribute(attr), &self.allocator))
            }
            HtmlNode::Block(block) => {
                HtmlNode::Block(Box::new_in(self.clone_block_shallow(block), &self.allocator))
            }
            HtmlNode::BlockParameter(param) => HtmlNode::BlockParameter(Box::new_in(
                self.clone_block_parameter(param),
                &self.allocator,
            )),
            HtmlNode::LetDeclaration(decl) => HtmlNode::LetDeclaration(Box::new_in(
                self.clone_let_declaration(decl),
                &self.allocator,
            )),
            HtmlNode::Expansion(exp) => {
                HtmlNode::Expansion(Box::new_in(self.clone_expansion_shallow(exp), &self.allocator))
            }
            HtmlNode::ExpansionCase(case) => HtmlNode::ExpansionCase(Box::new_in(
                self.clone_expansion_case_shallow(case),
                &self.allocator,
            )),
        }
    }

    /// Clone an element (including children).
    fn clone_element_shallow(&self, el: &HtmlElement<'a>) -> HtmlElement<'a> {
        HtmlElement {
            name: el.name.clone(),
            component_prefix: el.component_prefix.clone(),
            component_tag_name: el.component_tag_name.clone(),
            is_component: el.is_component,
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
        let mut cases = Vec::with_capacity_in(exp.cases.len(), &self.allocator);
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
                &self.allocator,
            )));
        }

        if is_not_blank || has_expansion_sibling {
            // Process whitespace in the value
            let processed = process_whitespace(text.value.as_str());
            let value = if self.preserve_significant_whitespace {
                &processed
            } else {
                Self::trim_leading_and_trailing(&processed, context)
            };

            // Process tokens
            let tokens = self.process_tokens(&text.tokens, context);

            Some(HtmlNode::Text(Box::new_in(
                HtmlText {
                    value: self.ident_if_changed(text.value, value),
                    span: text.span,
                    full_start: text.full_start,
                    tokens,
                },
                &self.allocator,
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
        let mut result = Vec::with_capacity_in(tokens.len(), &self.allocator);
        let token_count = tokens.len();

        for (i, token) in tokens.iter().enumerate() {
            if token.token_type == InterpolatedTokenType::Text && !token.parts.is_empty() {
                let original = token.parts[0];
                let processed = process_whitespace(original.as_str());
                let mut text: &str = &processed;

                // Trim leading/trailing if not preserving significant whitespace
                if !self.preserve_significant_whitespace {
                    if i == 0 && context.prev.is_none() {
                        text = js_trim_start(text);
                    }
                    if i == token_count - 1 && context.next.is_none() {
                        text = js_trim_end(text);
                    }
                }

                let mut parts = Vec::with_capacity_in(1, &self.allocator);
                parts.push(self.ident_if_changed(original, text));

                result.push(InterpolatedToken {
                    token_type: token.token_type,
                    parts,
                    span: token.span,
                });
            } else {
                // Clone non-text tokens
                let mut parts = Vec::with_capacity_in(token.parts.len(), &self.allocator);
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

    /// Trim leading and trailing whitespace based on sibling context, with JS
    /// `trimStart` / `trimEnd` rules like Angular.
    fn trim_leading_and_trailing<'s>(text: &'s str, context: &SiblingContext<'a, '_>) -> &'s str {
        let is_first = context.prev.is_none();
        let is_last = context.next.is_none();

        let trimmed_start = if is_first { js_trim_start(text) } else { text };
        if is_last { js_trim_end(trimmed_start) } else { trimmed_start }
    }

    /// `new_text` is `original` after whitespace processing and trimming. Every
    /// change makes the text shorter, so the same length means no change: reuse
    /// `original` then, and only allocate a new `Ident` when the text changed.
    fn ident_if_changed(&self, original: Ident<'a>, new_text: &str) -> Ident<'a> {
        if new_text.len() == original.as_str().len() {
            original
        } else {
            Ident::from_in(new_text, self.allocator)
        }
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
                    is_component: element.is_component,
                    attrs,
                    directives: self.clone_directives(&element.directives),
                    children: self.clone_children(&element.children),
                    span: element.span,
                    start_span: element.start_span,
                    end_span: element.end_span,
                    is_self_closing: element.is_self_closing,
                    is_void: element.is_void,
                },
                &self.allocator,
            )));
        }

        // Process children
        let children = self.visit_all(&element.children);

        Some(HtmlNode::Element(Box::new_in(
            HtmlElement {
                name: element.name.clone(),
                component_prefix: element.component_prefix.clone(),
                component_tag_name: element.component_tag_name.clone(),
                is_component: element.is_component,
                attrs: self.clone_attributes(&element.attrs),
                directives: self.clone_directives(&element.directives),
                children,
                span: element.span,
                start_span: element.start_span,
                end_span: element.end_span,
                is_self_closing: element.is_self_closing,
                is_void: element.is_void,
            },
            &self.allocator,
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
                &self.allocator,
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
            &self.allocator,
        )))
    }

    /// Filter out ngPreserveWhitespaces attribute.
    fn filter_preserve_attr(
        &self,
        attrs: &Vec<'a, HtmlAttribute<'a>>,
    ) -> Vec<'a, HtmlAttribute<'a>> {
        let mut result = Vec::with_capacity_in(attrs.len(), &self.allocator);
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
            &self.allocator,
        )))
    }

    /// Visit an ICU expansion.
    fn visit_expansion(&mut self, expansion: &HtmlExpansion<'a>) -> Option<HtmlNode<'a>> {
        self.icu_expansion_depth += 1;

        // Visit cases (process whitespace in expansion content)
        let mut cases = Vec::with_capacity_in(expansion.cases.len(), &self.allocator);
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
            &self.allocator,
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
            &self.allocator,
        )))
    }
}

/// Remove whitespace from parsed template nodes, as Angular's `parseTemplate` does when
/// `preserveWhitespaces` is off.
pub fn remove_whitespaces<'a>(
    allocator: &'a Allocator,
    nodes: &[HtmlNode<'a>],
    preserve_significant_whitespace: bool,
) -> Vec<'a, HtmlNode<'a>> {
    let mut visitor = WhitespaceVisitor::new(allocator, preserve_significant_whitespace);
    // Angular's parseTemplate visits the root nodes with `visitAll`, not
    // `visitAllWithSiblings`, so root-level text has no sibling context: whitespace next to a
    // root-level ICU is removed, while inside elements it is kept.
    let mut result = Vec::with_capacity_in(nodes.len(), &allocator);
    let no_siblings = SiblingContext { prev: None, next: None };
    for node in nodes {
        if let Some(new_node) = visitor.visit_node(node, &no_siblings) {
            result.push(new_node);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::html::HtmlParser;

    /// Angular's `processWhitespace`, char by char: replace NGSP with a space,
    /// then `/[WS]{2,}/g` -> `' '`.
    fn reference_process_whitespace(text: &str) -> String {
        let replaced = text.replace(NGSP_UNICODE, " ");
        let mut result = String::new();
        let mut run = String::new();
        let mut run_len = 0;
        for c in replaced.chars().chain(std::iter::once('x')) {
            if is_ws_char(c) {
                run.push(c);
                run_len += 1;
                continue;
            }
            if run_len >= 2 {
                result.push(' ');
            } else {
                result.push_str(&run);
            }
            run.clear();
            run_len = 0;
            result.push(c);
        }
        result.pop(); // the 'x' sentinel
        result
    }

    fn check(s: &str) {
        let got = process_whitespace(s);
        assert_eq!(got, reference_process_whitespace(s), "on {s:?}");
        // No allocation when nothing changes, and a change always makes it shorter.
        assert_eq!(matches!(got, Cow::Borrowed(_)), got.len() == s.len(), "on {s:?}");
    }

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
    fn test_process_whitespace() {
        // Multiple consecutive whitespace -> single space
        assert_eq!(process_whitespace("a  b"), "a b");
        assert_eq!(process_whitespace("a\n\n\tb"), "a b");
        assert_eq!(process_whitespace("  a  "), " a ");
        assert_eq!(process_whitespace("a  b  c"), "a b c");
        assert_eq!(process_whitespace("a\u{3000}\u{FEFF}b"), "a b");

        // Single whitespace characters are preserved as-is
        assert_eq!(process_whitespace("\n"), "\n");
        assert_eq!(process_whitespace("a\nb"), "a\nb");
        assert_eq!(process_whitespace(" "), " ");
        assert_eq!(process_whitespace("\t"), "\t");
        assert_eq!(process_whitespace("a b"), "a b");
        assert_eq!(process_whitespace("a\t"), "a\t");
        assert_eq!(process_whitespace("a\u{3000}b"), "a\u{3000}b");

        // NGSP becomes a space, also alone.
        assert_eq!(process_whitespace(&format!("a{NGSP_UNICODE}b")), "a b");
        assert_eq!(process_whitespace(&format!("a{NGSP_UNICODE}\nb")), "a b");

        // Nothing to change: the input is borrowed.
        assert!(matches!(process_whitespace("a b\nc"), Cow::Borrowed("a b\nc")));
        assert!(matches!(process_whitespace(""), Cow::Borrowed("")));
        assert!(matches!(process_whitespace("a  b"), Cow::Owned(_)));
    }

    #[test]
    fn process_whitespace_matches_reference() {
        const ALPHABET: &[char] = &[
            ' ', ' ', ' ', '\n', '\n', '\t', '\r', '\x0B', '\x0C', 'a', 'b', '{', '}', '\u{1680}',
            '\u{180E}', '\u{2000}', '\u{2005}', '\u{200A}', '\u{200B}', '\u{2027}', '\u{2028}',
            '\u{2029}', '\u{202F}', '\u{205F}', '\u{2060}', '\u{3000}', '\u{3001}', '\u{FEFF}',
            '\u{FF01}', '\u{E500}', '\u{E500}', '\u{E501}', '\u{E000}', '\u{00A0}', '\u{0085}',
            'é', '日', '😀', '\u{1681}', '\u{180D}', '\u{200C}',
        ];
        // Small xorshift, so the test is the same on every run.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % n as u64).unwrap()
        };
        for _ in 0..50_000 {
            let len = next(24);
            let s: String =
                std::iter::repeat_with(|| ALPHABET[next(ALPHABET.len())]).take(len).collect();
            check(&s);
        }
        // Every char in the BMP (this covers all 3-byte UTF-8 chars), alone,
        // doubled and next to a space.
        for c in (0u32..=0xFFFF).filter_map(char::from_u32) {
            check(&c.to_string());
            check(&format!("x{c}{c}y"));
            check(&format!("{c} "));
            check(&format!(" {c}x"));
        }
    }

    #[test]
    fn js_trim_rules() {
        // JS trims U+FEFF, Rust's `trim` does not.
        assert_eq!(js_trim_start("\u{FEFF} a\u{FEFF}"), "a\u{FEFF}");
        assert_eq!(js_trim_end("\u{FEFF}a \u{FEFF}"), "\u{FEFF}a");
        // Rust's `trim` strips U+0085, JS does not.
        assert_eq!(js_trim_start("\u{85}a"), "\u{85}a");
        assert_eq!(js_trim_end("a\u{85}"), "a\u{85}");
        // NBSP is trimmed by both.
        assert_eq!(js_trim_start("\u{A0}a"), "a");
    }

    fn text_values(nodes: &[HtmlNode<'_>]) -> std::vec::Vec<(String, std::vec::Vec<String>)> {
        nodes
            .iter()
            .filter_map(|node| match node {
                HtmlNode::Text(text) => Some((
                    text.value.as_str().to_string(),
                    text.tokens
                        .iter()
                        .filter(|t| t.token_type == InterpolatedTokenType::Text)
                        .map(|t| t.parts[0].as_str().to_string())
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn trims_with_js_rules() {
        let allocator = Allocator::default();
        // Root nodes have no siblings, so each text is trimmed at both ends.
        let source = "\u{FEFF}a\u{FEFF}";
        let parsed = HtmlParser::new(&allocator, source, "x.html").parse();
        let nodes = remove_whitespaces(&allocator, &parsed.nodes, false);
        assert_eq!(text_values(&nodes), [("a".to_string(), vec!["a".to_string()])]);

        let source = "\u{85}a\u{85}";
        let parsed = HtmlParser::new(&allocator, source, "x.html").parse();
        let nodes = remove_whitespaces(&allocator, &parsed.nodes, false);
        assert_eq!(
            text_values(&nodes),
            [("\u{85}a\u{85}".to_string(), vec!["\u{85}a\u{85}".to_string()])]
        );
    }

    #[test]
    fn many_siblings_keep_order_and_reuse_unchanged_text() {
        let allocator = Allocator::default();
        let mut source = String::from("<div>");
        for i in 0..300 {
            // Even texts need no change, odd ones collapse a run.
            if i % 2 == 0 {
                source.push_str("<b></b> t");
                source.push_str(&i.to_string());
                source.push(' ');
            } else {
                source.push_str("<b></b>  t");
                source.push_str(&i.to_string());
                source.push_str("\n\n");
            }
        }
        source.push_str("</div>");
        let parsed = HtmlParser::new(&allocator, allocator.alloc_str(&source), "x.html").parse();
        let HtmlNode::Element(div) = &parsed.nodes[0] else { panic!() };

        for preserve in [true, false] {
            let nodes = remove_whitespaces(&allocator, &parsed.nodes, preserve);
            let HtmlNode::Element(out) = &nodes[0] else { panic!() };
            assert_eq!(out.children.len(), div.children.len());

            let mut texts = 0;
            for (before, after) in div.children.iter().zip(out.children.iter()) {
                let (HtmlNode::Text(before), HtmlNode::Text(after)) = (before, after) else {
                    assert!(matches!(after, HtmlNode::Element(_)));
                    continue;
                };
                let i = texts;
                texts += 1;
                // The last text is trimmed at its end when not preserving.
                let expected =
                    if i == 299 && !preserve { format!(" t{i}") } else { format!(" t{i} ") };
                assert_eq!(after.value.as_str(), expected);
                assert_eq!(after.tokens.len(), 1);
                assert_eq!(after.tokens[0].parts[0].as_str(), expected);
                // An unchanged (even) text reuses the original string.
                if i % 2 == 0 {
                    assert!(std::ptr::eq(after.value.as_str(), before.value.as_str()));
                    assert!(std::ptr::eq(
                        after.tokens[0].parts[0].as_str(),
                        before.tokens[0].parts[0].as_str()
                    ));
                }
            }
            assert_eq!(texts, 300);
        }
    }
}

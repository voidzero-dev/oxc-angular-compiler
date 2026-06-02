//! HTML template parser.
//!
//! Parses Angular HTML templates from tokens.
//!
//! Ported from Angular's `ml_parser/parser.ts`.

use std::sync::Arc;

use oxc_allocator::{Allocator, Box, FromIn, Vec};
use oxc_span::Span;
use oxc_str::Ident;

use crate::ast::html::{
    BlockType, HtmlAttribute, HtmlBlock, HtmlBlockParameter, HtmlComment, HtmlDirective,
    HtmlElement, HtmlExpansion, HtmlExpansionCase, HtmlLetDeclaration, HtmlNode, HtmlText,
    InterpolatedToken, InterpolatedTokenType,
};
use crate::parser::expression::BindingParser;
use crate::transform::control_flow::is_else_if_pattern;
use crate::util::{ParseError, ParseLocation, ParseSourceFile, ParseSourceSpan};

use super::entities::decode_entities_in_string;
use super::lexer::{HtmlLexer, HtmlToken, HtmlTokenType};
use super::tags::{
    get_html_tag_definition, get_ns_prefix, is_void_element, merge_ns_and_name, split_ns_name,
};

/// Result of parsing an HTML template.
pub struct HtmlParseResult<'a> {
    /// The root nodes.
    pub nodes: Vec<'a, HtmlNode<'a>>,
    /// Parsing errors.
    pub errors: std::vec::Vec<ParseError>,
}

/// Index into the container stack, pointing to a Block or Element.
#[derive(Debug, Clone, Copy)]
enum ContainerIndex {
    /// Index of a Block node.
    Block(usize),
    /// Index of an Element node.
    Element(usize),
}

/// HTML template parser.
pub struct HtmlParser<'a> {
    /// The allocator.
    allocator: &'a Allocator,
    /// The source file for error reporting.
    source_file: Arc<ParseSourceFile>,
    /// The tokens.
    tokens: std::vec::Vec<HtmlToken>,
    /// Current token index.
    index: usize,
    /// Parsing errors.
    errors: std::vec::Vec<ParseError>,
    /// Root nodes (nodes not inside any container).
    root_nodes: std::vec::Vec<HtmlNode<'a>>,
    /// All Block nodes created during parsing (for container stack).
    blocks: std::vec::Vec<HtmlBlock<'a>>,
    /// All Element nodes created during parsing (for container stack).
    elements: std::vec::Vec<HtmlElement<'a>>,
    /// Stack of containers (Blocks and Elements) for parent-child tracking.
    container_stack: std::vec::Vec<ContainerIndex>,
}

impl<'a> HtmlParser<'a> {
    /// Creates a new parser.
    pub fn new(allocator: &'a Allocator, source: &'a str, url: &str) -> Self {
        Self::new_internal(allocator, source, url, false, false, None, true, true, None)
    }

    /// Creates a new parser with selectorless mode enabled.
    pub fn with_selectorless(allocator: &'a Allocator, source: &'a str, url: &str) -> Self {
        Self::new_internal(allocator, source, url, true, false, None, true, true, None)
    }

    /// Creates a new parser with expansion forms (ICU messages) enabled.
    pub fn with_expansion_forms(allocator: &'a Allocator, source: &'a str, url: &str) -> Self {
        Self::new_internal(allocator, source, url, false, true, None, true, true, None)
    }

    /// Creates a new parser with expansion forms and leading trivia chars enabled.
    pub fn with_expansion_forms_and_trivia(
        allocator: &'a Allocator,
        source: &'a str,
        url: &str,
        leading_trivia_chars: std::vec::Vec<char>,
    ) -> Self {
        Self::new_internal(
            allocator,
            source,
            url,
            false,
            true,
            Some(leading_trivia_chars),
            true,
            true,
            None,
        )
    }

    /// Creates a new parser with the given template options.
    ///
    /// This is the recommended way to create a parser as it allows
    /// full control over parsing behavior.
    pub fn with_options(
        allocator: &'a Allocator,
        source: &'a str,
        url: &str,
        options: &super::super::ParseTemplateOptions,
    ) -> Self {
        Self::new_internal(
            allocator,
            source,
            url,
            options.enable_selectorless,
            options.tokenize_expansion_forms,
            options.leading_trivia_chars.clone(),
            options.enable_block_syntax,
            options.enable_let_syntax,
            options.interpolation.as_ref().map(|(s, e)| (s.as_str(), e.as_str())),
        )
    }

    /// Internal constructor.
    fn new_internal(
        allocator: &'a Allocator,
        source: &'a str,
        url: &str,
        selectorless: bool,
        expansion_forms: bool,
        leading_trivia_chars: Option<std::vec::Vec<char>>,
        tokenize_blocks: bool,
        tokenize_let: bool,
        interpolation: Option<(&str, &str)>,
    ) -> Self {
        let mut lexer = HtmlLexer::new(source)
            .with_selectorless(selectorless)
            .with_expansion_forms(expansion_forms)
            .with_blocks(tokenize_blocks)
            .with_let(tokenize_let);

        if let Some(chars) = leading_trivia_chars {
            lexer = lexer.with_leading_trivia_chars(chars);
        }

        if let Some((start, end)) = interpolation {
            lexer = lexer.with_interpolation(start, end);
        }

        let result = lexer.tokenize();
        let source_file = Arc::new(ParseSourceFile::new(source.to_string(), url.to_string()));

        // Convert lexer errors to ParseErrors
        let mut errors = std::vec::Vec::new();
        for lex_err in result.errors {
            let loc = ParseLocation::new(source_file.clone(), lex_err.position.0, 0, 0);
            let span = ParseSourceSpan::new(loc.clone(), loc);
            errors.push(ParseError::new(span, lex_err.msg));
        }

        Self {
            allocator,
            source_file,
            tokens: result.tokens,
            index: 0,
            errors,
            root_nodes: std::vec::Vec::new(),
            blocks: std::vec::Vec::new(),
            elements: std::vec::Vec::new(),
            container_stack: std::vec::Vec::new(),
        }
    }

    /// Creates a ParseError at the given offset with the given message.
    fn make_error(&self, offset: u32, msg: impl Into<String>) -> ParseError {
        let loc = ParseLocation::new(self.source_file.clone(), offset, 0, 0);
        let span = ParseSourceSpan::new(loc.clone(), loc);
        ParseError::new(span, msg)
    }

    /// Build a parse error whose span covers `[start, end)`. Used for diagnostics that
    /// upstream attaches to the full source span of a token (e.g. the unexpected
    /// closing-tag errors, which use `endToken.sourceSpan`), rather than a zero-length
    /// caret at the token start. Uses `from_offsets` so the start/end `ParseLocation`s
    /// carry correct line/column (computed via `location_at`), matching Angular's
    /// `endToken.sourceSpan` on multiline templates — not fabricated `(0, 0)` line/col.
    fn make_error_span(&self, start: u32, end: u32, msg: impl Into<String>) -> ParseError {
        let span = ParseSourceSpan::from_offsets(&self.source_file, start, end, None, None);
        ParseError::new(span, msg)
    }

    /// Parses the template.
    pub fn parse(mut self) -> HtmlParseResult<'a> {
        // Parse all nodes using container stack
        while !self.at_end() {
            self.parse_and_add_node();
        }

        // Close all remaining containers at EOF (error recovery)
        // This ensures we still produce AST nodes even for unclosed elements
        // Process from top of stack (innermost) to bottom (outermost)
        while let Some(container) = self.container_stack.pop() {
            match container {
                ContainerIndex::Block(idx) => {
                    let block = &self.blocks[idx];
                    let err = self.make_error(
                        block.span.start,
                        format!("Unclosed block \"@{}\"", block.name),
                    );
                    self.errors.push(err);

                    // Convert block to node and add to parent
                    let block = std::mem::replace(
                        &mut self.blocks[idx],
                        HtmlBlock {
                            block_type: BlockType::If,
                            name: Ident::from(""),
                            parameters: Vec::new_in(self.allocator),
                            children: Vec::new_in(self.allocator),
                            span: Span::new(0, 0),
                            name_span: Span::new(0, 0),
                            start_span: Span::new(0, 0),
                            end_span: None,
                        },
                    );
                    let node = HtmlNode::Block(Box::new_in(block, self.allocator));
                    self.add_to_parent(node);
                }
                ContainerIndex::Element(idx) => {
                    // Angular's parser is lenient and doesn't report errors for unclosed elements.
                    // The browser's HTML parser auto-closes elements, and Angular follows this behavior.
                    // We still recover by creating AST nodes for unclosed elements (below).

                    // Convert element to node and add to parent
                    let element = std::mem::replace(
                        &mut self.elements[idx],
                        HtmlElement {
                            name: Ident::from(""),
                            component_prefix: None,
                            component_tag_name: None,
                            attrs: Vec::new_in(self.allocator),
                            directives: Vec::new_in(self.allocator),
                            children: Vec::new_in(self.allocator),
                            span: Span::new(0, 0),
                            start_span: Span::new(0, 0),
                            end_span: None,
                            is_self_closing: false,
                            is_void: false,
                            is_component: false,
                        },
                    );
                    let node = HtmlNode::Element(Box::new_in(element, self.allocator));
                    self.add_to_parent(node);
                }
            }
        }

        // Convert root_nodes to arena-allocated Vec
        let mut nodes = Vec::new_in(self.allocator);
        for node in self.root_nodes {
            nodes.push(node);
        }

        HtmlParseResult { nodes, errors: self.errors }
    }

    // ========================================================================
    // Container stack management (ported from Angular's _TreeBuilder)
    // ========================================================================

    /// Adds a node to the current container (or root if no container).
    fn add_to_parent(&mut self, node: HtmlNode<'a>) {
        if let Some(container) = self.container_stack.last().copied() {
            match container {
                ContainerIndex::Block(idx) => {
                    self.blocks[idx].children.push(node);
                }
                ContainerIndex::Element(idx) => {
                    self.elements[idx].children.push(node);
                }
            }
        } else {
            self.root_nodes.push(node);
        }
    }

    /// Pushes a block onto the container stack.
    fn push_block_container(&mut self, block: HtmlBlock<'a>) -> usize {
        let idx = self.blocks.len();
        self.blocks.push(block);
        self.container_stack.push(ContainerIndex::Block(idx));
        idx
    }

    /// Pushes an element onto the container stack.
    fn push_element_container(&mut self, element: HtmlElement<'a>) -> usize {
        let idx = self.elements.len();
        self.elements.push(element);
        self.container_stack.push(ContainerIndex::Element(idx));
        idx
    }

    /// Returns the namespace-relevant *name* of the closest element-like parent on
    /// the container stack, skipping non-element containers (e.g. blocks).
    ///
    /// Mirrors upstream `Parser._getClosestElementLikeParent` (ml_parser/parser.ts:903-912)
    /// combined with the per-node-kind name selection in `_getPrefix`
    /// (ml_parser/parser.ts:991):
    ///   `const parentName = parent instanceof html.Element ? parent.name : parent.tagName;`
    /// i.e. for an `html.Element` parent it is `parent.name`, but for an
    /// `html.Component` parent it is the RESOLVED component `tagName` (e.g.
    /// `:svg:button`), NOT the component class name (`MyComp`). `getNsPrefix` is then
    /// applied to this string, so children of a namespaced selectorless component
    /// inherit `:svg:`/`:math:` just like children of a real namespaced element.
    /// Block containers do not affect namespace inheritance, so SVG children inside
    /// `@if {...}` still inherit `:svg:`.
    ///
    /// In OXC a selectorless component is stored as an `HtmlElement` whose `name` is
    /// the component class name (always starting uppercase or `_`, since only such
    /// tags produce a `ComponentOpenStart` token) and whose `component_prefix` /
    /// `component_tag_name` hold the resolved prefix and raw tag-name part. For those
    /// we compose the resolved `tagName` exactly as `_getComponentTagName`
    /// (ml_parser/parser.ts:946-961) / the R3 transform (`html_to_r3.rs`) do.
    fn closest_element_like_parent_name(&self) -> Option<String> {
        for &container in self.container_stack.iter().rev() {
            if let ContainerIndex::Element(idx) = container {
                let element = &self.elements[idx];
                return Some(Self::namespace_parent_name(element));
            }
        }
        None
    }

    /// Computes the namespace-inheritance parent name for an element-like parent,
    /// mirroring upstream `_getPrefix`'s `parent instanceof html.Element ? parent.name
    /// : parent.tagName` (ml_parser/parser.ts:991).
    ///
    /// For a normal element this is just `element.name`. For a selectorless component
    /// (detected by an uppercase/`_` leading char, matching the lexer's
    /// `ComponentOpenStart` rule and the R3 transform's component check) it is the
    /// RESOLVED component tag name composed from `component_prefix` /
    /// `component_tag_name`, exactly as `_getComponentTagName`
    /// (ml_parser/parser.ts:946-961):
    ///   - prefix=Some, tag=Some -> `:{prefix}:{tag}`
    ///   - prefix=Some, tag=None -> `:{prefix}:ng-component`
    ///   - prefix=None, tag=Some -> `{tag}`
    ///   - prefix=None, tag=None -> the bare component name (upstream returns `null`,
    ///     i.e. no namespace; `getNsPrefix` on a plain name yields `None`, so children
    ///     stay HTML — the bare component name has the same effect).
    fn namespace_parent_name(element: &HtmlElement<'a>) -> String {
        let name = element.name.as_str();
        let is_component = name.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c == '_');
        if !is_component {
            return name.to_string();
        }
        match (element.component_prefix.as_ref(), element.component_tag_name.as_ref()) {
            (Some(prefix), Some(tag)) => format!(":{prefix}:{tag}"),
            (Some(prefix), None) => format!(":{prefix}:ng-component"),
            (None, Some(tag)) => tag.to_string(),
            (None, None) => name.to_string(),
        }
    }

    /// Returns the name used to MATCH a close tag against this open element on the
    /// container stack, mirroring upstream `_popContainer`'s
    /// `node instanceof html.Component ? node.fullName : node.name`
    /// (ml_parser/parser.ts:600).
    ///
    /// For a normal element this is `element.name`. For a selectorless component the
    /// stored `name` is only the class name, so we compose the RESOLVED full name
    /// exactly as `_getComponentFullName` (ml_parser/parser.ts:932-944) /
    /// the R3 transform do, from `component_prefix` / `component_tag_name`:
    ///   - tagName null (no prefix, no tag part) -> the bare component name
    ///   - tagName `:prefix:tag` (a prefix is present) -> `componentName:prefix:tag`
    ///   - tagName `tag` (no prefix) -> `componentName:tag`
    fn element_match_name(element: &HtmlElement<'a>) -> String {
        let name = element.name.as_str();
        let is_component = name.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c == '_');
        if !is_component {
            return name.to_string();
        }
        match (element.component_prefix.as_ref(), element.component_tag_name.as_ref()) {
            // Prefix present -> tagName is `:prefix:tag` (or `:prefix:ng-component`);
            // full name is `componentName` + `:prefix:tag` (tagName already starts ':').
            (Some(prefix), Some(tag)) => format!("{name}:{prefix}:{tag}"),
            (Some(prefix), None) => format!("{name}:{prefix}:ng-component"),
            // No prefix, a tag part -> full name is `componentName:tag`.
            (None, Some(tag)) => format!("{name}:{tag}"),
            // No prefix, no tag part -> tagName null -> full name is the bare class name.
            (None, None) => name.to_string(),
        }
    }

    /// Resolves the namespace prefix for an element, mirroring upstream
    /// `Parser._getPrefix` (ml_parser/parser.ts ~973-1001).
    ///
    /// Order of precedence:
    ///   1. An explicit prefix from the source (`<svg:rect>`).
    ///   2. The tag's own `implicit_namespace_prefix` (e.g. `<svg>` -> `svg`).
    ///   3. The parent element's namespace prefix, inherited unless the parent's
    ///      tag definition sets `prevent_namespace_inheritance` (e.g. `foreignObject`).
    ///
    /// `explicit_prefix` is the prefix lexed from the source (empty when absent).
    /// `local_name` is the prefix-stripped tag name.
    fn resolve_element_prefix(&self, explicit_prefix: &str, local_name: &str) -> Option<String> {
        // 1 & 2: explicit prefix, falling back to the tag's implicit namespace prefix.
        let mut prefix: Option<String> = if explicit_prefix.is_empty() {
            get_html_tag_definition(local_name).implicit_namespace_prefix.map(str::to_string)
        } else {
            Some(explicit_prefix.to_string())
        };

        // 3: inherit the parent element's prefix when the child has none and the
        // parent does not prevent namespace inheritance.
        if prefix.is_none()
            && let Some(parent_name) = self.closest_element_like_parent_name()
        {
            let (_, parent_local) = split_ns_name(&parent_name);
            let parent_def = get_html_tag_definition(parent_local);
            if !parent_def.prevent_namespace_inheritance {
                prefix = get_ns_prefix(&parent_name).map(str::to_string);
            }
        }

        prefix
    }

    /// Pops a block from the container stack and returns it as an HtmlNode.
    fn pop_block_container(&mut self, end_span: Option<Span>) -> Option<HtmlNode<'a>> {
        // Find the topmost block in the stack
        for i in (0..self.container_stack.len()).rev() {
            if let ContainerIndex::Block(idx) = self.container_stack[i] {
                // Remove from stack
                self.container_stack.remove(i);
                // Get the block and update its end span
                let mut block = std::mem::replace(
                    &mut self.blocks[idx],
                    HtmlBlock {
                        block_type: BlockType::If,
                        name: Ident::from(""),
                        parameters: Vec::new_in(self.allocator),
                        children: Vec::new_in(self.allocator),
                        span: Span::new(0, 0),
                        name_span: Span::new(0, 0),
                        start_span: Span::new(0, 0),
                        end_span: None,
                    },
                );
                if let Some(es) = end_span {
                    block.end_span = Some(es);
                    block.span = Span::new(block.span.start, es.end);
                }
                return Some(HtmlNode::Block(Box::new_in(block, self.allocator)));
            }
        }
        None
    }

    /// Pops an element from the container stack matching the given tag name.
    /// If there are unclosed elements above the matching one, they are implicitly closed first.
    ///
    /// Returns the closed element node along with a boolean flag indicating whether any
    /// implicitly-closed container above the matching one required an explicit end tag
    /// (i.e. its tag definition does not have `closed_by_parent`, or it was a block).
    /// Mirrors Angular's `_popContainer`, which surfaces this as an "Unexpected closing
    /// tag" diagnostic on the closing tag itself.
    fn pop_element_container(
        &mut self,
        tag_name: &str,
        end_span: Option<Span>,
    ) -> (Option<HtmlNode<'a>>, bool) {
        // Find the matching element in the stack and its element index
        let mut match_stack_idx = None;
        let mut match_elem_idx = None;
        for i in (0..self.container_stack.len()).rev() {
            if let ContainerIndex::Element(idx) = self.container_stack[i] {
                // Match by the element's full name. For a selectorless component this
                // is the composed `componentName(:prefix):tag` (upstream `_popContainer`
                // compares `node.fullName` for components), NOT the bare class name.
                if Self::element_match_name(&self.elements[idx]) == tag_name {
                    match_stack_idx = Some(i);
                    match_elem_idx = Some(idx);
                    break;
                }
            }
        }

        let Some(match_stack_idx) = match_stack_idx else {
            return (None, false);
        };
        let Some(match_elem_idx) = match_elem_idx else {
            return (None, false);
        };

        let mut unexpected_close_detected = false;

        // Implicitly close all containers above the matching one (in reverse order)
        while self.container_stack.len() > match_stack_idx + 1 {
            let Some(container) = self.container_stack.pop() else {
                break; // Should not happen, but handle gracefully
            };
            match container {
                ContainerIndex::Element(idx) => {
                    // Only elements whose end tag is truly optional (closed_by_parent) may
                    // be implicitly closed silently. Anything else means this end tag is
                    // jumping past a still-open element, which is what Angular's reference
                    // parser flags via `unexpectedCloseTagDetected`.
                    let elem_name = self.elements[idx].name.as_str().to_lowercase();
                    if !get_html_tag_definition(&elem_name).closed_by_parent {
                        unexpected_close_detected = true;
                    }
                    // Get the element without end_span (implicitly closed)
                    let element = std::mem::replace(
                        &mut self.elements[idx],
                        HtmlElement {
                            name: Ident::from(""),
                            component_prefix: None,
                            component_tag_name: None,
                            attrs: Vec::new_in(self.allocator),
                            directives: Vec::new_in(self.allocator),
                            children: Vec::new_in(self.allocator),
                            span: Span::new(0, 0),
                            start_span: Span::new(0, 0),
                            end_span: None,
                            is_self_closing: false,
                            is_void: false,
                            is_component: false,
                        },
                    );
                    // Add to the next parent on the stack (which is now the matching element)
                    self.add_to_parent(HtmlNode::Element(Box::new_in(element, self.allocator)));
                }
                ContainerIndex::Block(idx) => {
                    // Blocks are never implicitly closed by a parent end tag.
                    unexpected_close_detected = true;
                    // Close blocks too (implicitly)
                    let block = std::mem::replace(
                        &mut self.blocks[idx],
                        HtmlBlock {
                            block_type: BlockType::If,
                            name: Ident::from(""),
                            parameters: Vec::new_in(self.allocator),
                            children: Vec::new_in(self.allocator),
                            span: Span::new(0, 0),
                            name_span: Span::new(0, 0),
                            start_span: Span::new(0, 0),
                            end_span: None,
                        },
                    );
                    self.add_to_parent(HtmlNode::Block(Box::new_in(block, self.allocator)));
                }
            }
        }

        // Now pop the matching element from the stack
        self.container_stack.pop();

        // Get the element and update its end span
        let mut element = std::mem::replace(
            &mut self.elements[match_elem_idx],
            HtmlElement {
                name: Ident::from(""),
                component_prefix: None,
                component_tag_name: None,
                attrs: Vec::new_in(self.allocator),
                directives: Vec::new_in(self.allocator),
                children: Vec::new_in(self.allocator),
                span: Span::new(0, 0),
                start_span: Span::new(0, 0),
                end_span: None,
                is_self_closing: false,
                is_void: false,
                is_component: false,
            },
        );
        if let Some(es) = end_span {
            element.end_span = Some(es);
            element.span = Span::new(element.span.start, es.end);
        }
        (Some(HtmlNode::Element(Box::new_in(element, self.allocator))), unexpected_close_detected)
    }

    /// Auto-closes elements that have optional end tags based on HTML5 rules.
    /// Called when a new element is about to be opened.
    fn auto_close_element_if_needed(&mut self, new_tag: &str) {
        // Keep closing elements while the current one should be auto-closed
        loop {
            let current_tag = if let Some(&container_idx) = self.container_stack.last() {
                if let ContainerIndex::Element(elem_idx) = container_idx {
                    self.elements[elem_idx].name.as_str().to_lowercase()
                } else {
                    break;
                }
            } else {
                break;
            };

            if should_auto_close(&current_tag, new_tag) {
                // Pop the current element and add it to its parent. Implicit auto-close
                // from a new opening tag never produces an "unexpected close" diagnostic
                // because the loop pops the top element on each iteration — there are no
                // skipped containers above the match.
                let (node, _) = self.pop_element_container(&current_tag, None);
                if let Some(node) = node {
                    self.add_to_parent(node);
                }
            } else {
                break;
            }
        }
    }

    // ========================================================================
    // Token utilities
    // ========================================================================

    /// Returns the current token.
    fn peek(&self) -> Option<&HtmlToken> {
        self.tokens.get(self.index)
    }

    /// Advances to the next token and returns the current one.
    fn advance(&mut self) -> Option<&HtmlToken> {
        let token = self.tokens.get(self.index);
        if self.index < self.tokens.len() {
            self.index += 1;
        }
        token
    }

    /// Returns true if at end.
    fn at_end(&self) -> bool {
        self.peek().map(|t| t.token_type == HtmlTokenType::Eof).unwrap_or(true)
    }

    /// Creates a span for the given offsets.
    fn make_span(&self, start: u32, end: u32) -> Span {
        Span::new(start, end)
    }

    // ========================================================================
    // Parsing
    // ========================================================================

    /// Parses a single node and adds it to the current container.
    fn parse_and_add_node(&mut self) {
        let Some(token) = self.peek() else { return };

        match token.token_type {
            HtmlTokenType::TagOpenStart | HtmlTokenType::ComponentOpenStart => {
                self.consume_element_start();
            }
            HtmlTokenType::TagClose | HtmlTokenType::ComponentClose => {
                // Closing tag (</div> or </Comp>) - try to match with open element
                self.consume_element_end();
            }
            HtmlTokenType::Text
            | HtmlTokenType::EncodedEntity
            | HtmlTokenType::Interpolation
            | HtmlTokenType::EscapableRawText
            | HtmlTokenType::RawText => {
                // All text-like tokens are merged into a single text node
                if let Some(node) = self.consume_text() {
                    self.add_to_parent(node);
                }
            }
            HtmlTokenType::CommentStart => {
                if let Some(node) = self.parse_comment() {
                    self.add_to_parent(node);
                }
            }
            HtmlTokenType::CdataStart => {
                // CDATA sections become text nodes
                if let Some(node) = self.parse_cdata() {
                    self.add_to_parent(node);
                }
            }
            HtmlTokenType::BlockOpenStart => {
                self.consume_block_open();
            }
            HtmlTokenType::BlockClose => {
                self.consume_block_close();
            }
            HtmlTokenType::IncompleteBlockOpen => {
                // Upstream v21.2.7 routes INCOMPLETE_BLOCK_OPEN to `_consumeIncompleteBlock`
                // (ml_parser/parser.ts:132-134); without this arm the token would hit the
                // default `_ => advance()` below and be silently dropped.
                self.consume_incomplete_block();
            }
            HtmlTokenType::LetStart => {
                if let Some(node) = self.parse_let_declaration() {
                    self.add_to_parent(node);
                }
            }
            HtmlTokenType::ExpansionFormStart => {
                if let Some(node) = self.parse_expansion() {
                    self.add_to_parent(node);
                }
            }
            HtmlTokenType::IncompleteTagOpen => {
                // Handle incomplete tag (error recovery)
                if let Some(node) = self.parse_incomplete_tag() {
                    self.add_to_parent(node);
                }
            }
            _ => {
                // Skip unknown tokens
                self.advance();
            }
        }
    }

    /// Consumes an element start tag and pushes it onto the container stack.
    fn consume_element_start(&mut self) {
        let Some(start_token) = self.advance() else {
            return; // No token to consume
        };
        let start = start_token.start;
        // A `ComponentOpenStart` is only ever emitted by the lexer when
        // selectorless mode is enabled (`lexer.rs` gates `scan_component_open` on
        // `self.selectorless_enabled`). This is the AUTHORITATIVE marker that the
        // element is a selectorless component, carried on the AST so the R3
        // transform never has to fall back to a leading-character casing heuristic
        // (which wrongly classified uppercase normal elements like `<IFRAME>`).
        let is_component = start_token.token_type == HtmlTokenType::ComponentOpenStart;
        // TagOpenStart has parts [prefix, name]
        // ComponentOpenStart has parts [component_name, prefix, tag_name]
        let (tag_name, local_name, _has_ns_prefix, component_prefix, component_tag_name) =
            if start_token.token_type == HtmlTokenType::ComponentOpenStart {
                // For components, extract all three parts:
                // parts[0] = component_name, parts[1] = prefix, parts[2] = tag_name
                let component_name = start_token.parts.first().cloned().unwrap_or_default();
                let explicit_prefix = start_token.parts.get(1).cloned().unwrap_or_default();
                let raw_tag_name = start_token.parts.get(2).cloned().unwrap_or_default();

                // Resolve the namespace prefix EXACTLY like upstream `_getPrefix`
                // (ml_parser/parser.ts ~963-1002), which is shared between normal
                // elements and components: an explicit prefix (`<MyComp:svg:title>`),
                // else the tag's own `implicit_namespace_prefix`, else the closest
                // element-like parent's prefix (so `<svg><MyComp:button>` resolves to
                // prefix `svg`, yielding tagName `:svg:button` /
                // fullName `MyComp:svg:button`), unless the parent prevents inheritance
                // (e.g. `foreignObject`). This mirrors `_getComponentTagName` /
                // `_getComponentFullName`, which route the component tag name through the
                // same prefix/namespace logic as normal elements. We pass the component's
                // *tag-name* part (`parts[2]`) as the local name so an empty tag name still
                // inherits the parent namespace (upstream then falls back to `ng-component`,
                // done in the R3 transform). Resolve BEFORE `auto_close_element_if_needed`
                // pops the parent, matching upstream which computes the name from the
                // closest element-like parent before `_pushContainer`. The local-name copy
                // releases the `start_token` borrow (which aliases `&self`) before calling
                // `resolve_element_prefix(&self, ..)`.
                let resolved_prefix = self.resolve_element_prefix(&explicit_prefix, &raw_tag_name);

                // Store the RESOLVED prefix and the raw tag_name for HtmlElement; the R3
                // transform composes them into `:prefix:tag` and `ComponentName:prefix:tag`.
                let tag_opt =
                    if raw_tag_name.is_empty() { None } else { Some(raw_tag_name.clone()) };

                (component_name.clone(), component_name, false, resolved_prefix, tag_opt)
            } else {
                // For regular tags, resolve the namespace prefix the way upstream
                // `Parser._getElementFullName` / `_getPrefix` do: an explicit prefix
                // (`<svg:rect>`), else the tag's own `implicit_namespace_prefix`
                // (`<svg>` -> `svg`), else the closest element-like parent's prefix
                // (so an implicit `<style>`/`<rect>` inside `<svg>` becomes
                // `:svg:style` / `:svg:rect`), unless the parent prevents inheritance
                // (e.g. `foreignObject`). This must run before `auto_close_element_if_needed`
                // pops the parent, matching upstream which computes `fullName` from the
                // closest element-like parent before `_pushContainer` performs auto-close.
                // Copy the prefix/name to owned strings to release the borrow of
                // `start_token` (which aliases `&self`) before calling
                // `resolve_element_prefix(&self, ..)`.
                let explicit_prefix = start_token.prefix().to_string();
                let name = start_token.name().to_string();
                let has_prefix = !explicit_prefix.is_empty();
                let resolved_prefix = self.resolve_element_prefix(&explicit_prefix, &name);
                let full_name = merge_ns_and_name(resolved_prefix.as_deref(), &name);
                (full_name, name.clone(), has_prefix, None, None)
            };

        // Check if we need to auto-close the current element (HTML5 optional end tags)
        self.auto_close_element_if_needed(&tag_name);

        // Parse attributes and directives
        let (attrs, directives) = self.parse_attributes_and_directives();

        // Check for self-closing (/>) or regular close (>)
        // Track the end position of the consumed closing token
        let (is_self_closing, tag_end) = if let Some(token) = self.peek() {
            if token.token_type == HtmlTokenType::TagOpenEndVoid
                || token.token_type == HtmlTokenType::ComponentOpenEndVoid
            {
                let end_pos = token.end;
                self.advance();

                // Validate self-closing: only void, custom, and foreign elements can be self-closed.
                // Upstream `Parser._consumeElementStartTag` (ml_parser/parser.ts:405) uses
                //   `!(tagDef?.canSelfClose || getNsPrefix(fullName) !== null || tagDef?.isVoid)`
                // — the "foreign" test is `getNsPrefix(fullName) !== null` on the *resolved* full
                // name, NOT on whether the SOURCE tag had an explicit prefix. This matters for
                // namespace inheritance (F5): `<svg><p/></svg>` resolves the implicit child to
                // `:svg:p`, whose `getNsPrefix` is `svg` (non-null), so it is foreign and may
                // self-close even though `<p>` carried no explicit prefix (`_has_ns_prefix == false`).
                // We therefore key the foreign decision off the resolved `tag_name`'s namespace.
                let tag_def = get_html_tag_definition(&local_name);
                let is_foreign = get_ns_prefix(&tag_name).is_some();
                if !(tag_def.can_self_close || is_foreign || tag_def.is_void) {
                    let err = self.make_error(
                        start,
                        format!(
                            "Only void, custom and foreign elements can be self closed \"{}\"",
                            local_name
                        ),
                    );
                    self.errors.push(err);
                }

                (true, Some(end_pos))
            } else if token.token_type == HtmlTokenType::TagOpenEnd
                || token.token_type == HtmlTokenType::ComponentOpenEnd
            {
                let end_pos = token.end;
                self.advance();
                (false, Some(end_pos))
            } else {
                (false, None)
            }
        } else {
            (false, None)
        };

        // Use the consumed token's end position, not the next token's start
        // This is important when leadingTriviaChars is enabled, as the next token's
        // start may be after stripped trivia
        let end = tag_end.unwrap_or_else(|| self.peek().map(|t| t.start).unwrap_or(start));
        let start_span = self.make_span(start, end);
        let span = self.make_span(start, end);

        // For self-closing elements, end_span equals start_span
        // For void elements or regular elements, end_span is set later or stays None
        let end_span = if is_self_closing { Some(start_span) } else { None };
        let is_void = is_void_element(&tag_name);

        let element = HtmlElement {
            name: Ident::from_in(tag_name.clone(), self.allocator),
            component_prefix: component_prefix.map(|p| Ident::from_in(p, self.allocator)),
            component_tag_name: component_tag_name.map(|t| Ident::from_in(t, self.allocator)),
            attrs,
            directives,
            children: Vec::new_in(self.allocator),
            span,
            start_span,
            end_span,
            is_self_closing,
            is_void,
            is_component,
        };

        if is_self_closing || is_void {
            // Self-closing elements are complete immediately
            self.add_to_parent(HtmlNode::Element(Box::new_in(element, self.allocator)));
        } else {
            // Push onto container stack for child parsing
            self.push_element_container(element);
        }
    }

    /// Consumes an element end tag and pops the matching element from the stack.
    fn consume_element_end(&mut self) {
        let Some(token) = self.advance() else {
            return; // No token to consume
        };
        let start = token.start;
        let end = token.end;
        // TagClose has parts [prefix, name]
        // ComponentClose has parts [component_name, prefix, tag_name]
        let is_component_close = token.token_type == HtmlTokenType::ComponentClose;
        // For a component close, the bare class name (parts[0]); used to decide the
        // "did you mean" vs generic suffix, matching upstream
        // `container.componentName === endToken.parts[0]` (ml_parser/parser.ts:537).
        let close_component_name: String = if is_component_close {
            token.parts.first().cloned().unwrap_or_default()
        } else {
            String::new()
        };
        let (tag_name, local_name) = if is_component_close {
            // Resolve the close's FULL name with the SAME logic as the open
            // (`_getComponentFullName` -> `_getComponentTagName` -> `_getPrefix`):
            // an explicit prefix (parts[1]), else the tag's implicit namespace, else
            // inherited from the closest element-like parent — which at close time is
            // the still-open component itself (carrying its resolved tagName, e.g.
            // `:svg:button`), so `</MyComp:button>` under `<svg>` inherits `:svg:` and
            // matches the open `MyComp:svg:button`. Copy parts to owned strings to
            // release the `token` borrow (which aliases `&self`) before calling
            // `resolve_element_prefix(&self, ..)`.
            let component_name = close_component_name.clone();
            let explicit_prefix = token.parts.get(1).cloned().unwrap_or_default();
            let raw_tag_name = token.parts.get(2).cloned().unwrap_or_default();
            let resolved_prefix = self.resolve_element_prefix(&explicit_prefix, &raw_tag_name);
            // Compose tagName (None when no prefix and no tag part) then full name,
            // mirroring `_getComponentTagName` / `_getComponentFullName`.
            let tag_name_opt: Option<String> = match (&resolved_prefix, raw_tag_name.is_empty()) {
                (Some(prefix), false) => Some(format!(":{prefix}:{raw_tag_name}")),
                (Some(prefix), true) => Some(format!(":{prefix}:ng-component")),
                (None, false) => Some(raw_tag_name.clone()),
                (None, true) => None,
            };
            let full_name = match &tag_name_opt {
                Some(tag) if tag.starts_with(':') => format!("{component_name}{tag}"),
                Some(tag) => format!("{component_name}:{tag}"),
                None => component_name.clone(),
            };
            // `local_name` (void-element/tag-definition lookups) uses the local tag
            // part; for a bare component close there is none, so fall back to the
            // component name (never a void element).
            let local = if raw_tag_name.is_empty() { component_name } else { raw_tag_name };
            (full_name, local)
        } else {
            // For regular tags, resolve the namespace prefix exactly like the start
            // tag (upstream `_consumeEndTag` -> `_getElementFullName`). When closing a
            // tag, the closest element-like parent is the matching open element itself
            // (still on the container stack), so an implicit `</style>` inside `<svg>`
            // inherits `:svg:` from its own `:svg:style` open tag and matches it.
            // Copy the prefix/name to owned strings to release the borrow of `token`
            // (which aliases `&self`) before calling `resolve_element_prefix(&self, ..)`.
            let explicit_prefix = token.prefix().to_string();
            let name = token.name().to_string();
            let resolved_prefix = self.resolve_element_prefix(&explicit_prefix, &name);
            let full_name = merge_ns_and_name(resolved_prefix.as_deref(), &name);
            (full_name, name)
        };
        let end_span = self.make_span(start, end);

        // Void-element end tags are only diagnosed for NORMAL elements (upstream's
        // void check lives in `_consumeElementEndTag`, ml_parser/parser.ts:572-579,
        // NOT in `_consumeComponentEndTag`). A component close whose tag part happens
        // to be a void HTML name (e.g. `</MyComp:input>`) must NOT trigger it.
        if !is_component_close {
            // Voidness is checked on the RESOLVED FULL name (`fullName`), exactly
            // like upstream `_consumeElementEndTag` -> `_getTagDefinition(fullName)`
            // (ml_parser/parser.ts:572). A namespaced close such as `</svg:input>`
            // or an implicit-namespace `<svg><input></input></svg>` resolves to
            // `:svg:input`, and `getHtmlTagDefinition(':svg:input')` falls back to
            // the DEFAULT (non-void) definition (html_tags.ts:192 -> no lowercase
            // match -> DEFAULT_TAG_DEFINITION), so upstream ACCEPTS the close and
            // pops the element with NO error. Using the NS-stripped `local_name`
            // here ("input") would wrongly diagnose the void error and bail before
            // popping. Plain `<input></input>` (no namespace) has `tag_name ==
            // local_name == "input"` -> void -> error retained, matching upstream.
            let tag_def = get_html_tag_definition(&tag_name);
            if tag_def.is_void {
                // Span is the FULL close-token source span (`endTagToken.sourceSpan`,
                // ml_parser/parser.ts:573-579), not a zero-length caret at the start.
                // Matches the I17 unexpected-close path below. The error message
                // quotes the LOCAL name (`endTagToken.parts[1]`, ml_parser/parser.ts:
                // 577), not the full name — for void elements `tag_name` is never
                // namespaced anyway, so `local_name == tag_name` in this branch.
                let err = self.make_error_span(
                    start,
                    end,
                    format!("Void elements do not have end tags \"{}\"", local_name),
                );
                self.errors.push(err);
                return;
            }
        }

        // Pop the matching element from the stack
        let (node, unexpected_close) = self.pop_element_container(&tag_name, Some(end_span));

        if is_component_close {
            // Component close: upstream `_consumeComponentEndTag` (ml_parser/parser.ts:
            // 530-545) reports an error whenever `_popContainer` returns false, i.e. on
            // NO match OR a match that required implicitly closing a still-open container
            // above it. The suffix is `, did you mean "<fullName>"?` when the CURRENT
            // topmost container is a component with the same `componentName` as the close
            // (parts[0]), else the generic ". It may happen ..." (NO trailing URL).
            if node.is_none() || unexpected_close {
                let suffix = match self.container_stack.last() {
                    Some(&ContainerIndex::Element(idx))
                        if self.elements[idx].name.as_str() == close_component_name
                            && self.elements[idx]
                                .name
                                .as_str()
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_uppercase() || c == '_') =>
                    {
                        let open_full = Self::element_match_name(&self.elements[idx]);
                        format!(", did you mean \"{open_full}\"?")
                    }
                    _ => ". It may happen when the tag has already been closed by another tag."
                        .to_string(),
                };
                // Span is the FULL close-token source span (`endToken.sourceSpan`,
                // ml_parser/parser.ts:544), not a zero-length caret at the start.
                let err = self.make_error_span(
                    start,
                    end,
                    format!("Unexpected closing tag \"{tag_name}\"{suffix}"),
                );
                self.errors.push(err);
            }
            if let Some(node) = node {
                self.add_to_parent(node);
            }
            return;
        }

        // Element close: upstream `_consumeElementEndTag` (ml_parser/parser.ts:580-583)
        // reports a SINGLE error message whenever `_popContainer` returns false — i.e. on
        // NO match (stray close) OR a match that required implicitly closing a still-open
        // container above it (`unexpected_close`). Both use the IDENTICAL message INCLUDING
        // the trailing W3C help URL, and the span is the FULL close-token source span
        // (`endTagToken.sourceSpan`), not a zero-length caret at the start. (The component
        // close path above intentionally omits the URL, mirroring `_consumeComponentEndTag`.)
        if node.is_none() || unexpected_close {
            let err = self.make_error_span(
                start,
                end,
                format!("Unexpected closing tag \"{}\". It may happen when the tag has already been closed by another tag. For more info see https://www.w3.org/TR/html5/syntax.html#closing-elements-that-have-implied-end-tags", tag_name),
            );
            self.errors.push(err);
        }
        if let Some(node) = node {
            self.add_to_parent(node);
        }
    }

    /// Parses attributes and directives.
    /// Returns (attributes, directives).
    /// New token sequence: AttrName → AttrQuote? → AttrValueText*/AttrValueInterpolation* → AttrQuote?
    /// Also handles directive tokens (DirectiveName, DirectiveOpen, DirectiveClose) when selectorless is enabled.
    fn parse_attributes_and_directives(
        &mut self,
    ) -> (Vec<'a, HtmlAttribute<'a>>, Vec<'a, HtmlDirective<'a>>) {
        let mut attrs = Vec::new_in(self.allocator);
        let mut directives = Vec::new_in(self.allocator);

        while let Some(token) = self.peek() {
            // Handle directive tokens (selectorless mode)
            if token.token_type == HtmlTokenType::DirectiveName {
                if let Some(directive) = self.parse_directive() {
                    directives.push(directive);
                }
                continue;
            }

            if token.token_type != HtmlTokenType::AttrName {
                break;
            }

            let Some(name_token) = self.advance() else {
                break; // Should not happen after peek, but handle gracefully
            };
            // Include namespace prefix if present (e.g., xlink:href -> :xlink:href)
            // Angular uses :prefix:name format for namespaced attributes
            let prefix = name_token.prefix();
            let base_name = name_token.name();
            let name = if prefix.is_empty() {
                base_name.to_string()
            } else {
                format!(":{prefix}:{base_name}")
            };
            let name_start = name_token.start;
            let name_end = name_token.end;

            // Track the end of the entire attribute (including closing quote)
            let mut attr_end = name_end;

            // Check for value (AttrQuote + AttrValueText/AttrValueInterpolation + AttrQuote)
            // Also collect tokens for proper humanization
            let (value, value_span, value_tokens) = if let Some(tok) = self.peek() {
                if tok.token_type == HtmlTokenType::AttrQuote {
                    self.advance(); // consume opening quote

                    // Collect value parts and tokens
                    let mut value_text = String::new();
                    let mut tokens: std::vec::Vec<(
                        InterpolatedTokenType,
                        std::vec::Vec<String>,
                        Span,
                    )> = std::vec::Vec::new();
                    let value_start = self.peek().map(|t| t.start).unwrap_or(name_end);
                    let mut value_end = value_start;

                    while let Some(val_tok) = self.peek() {
                        match val_tok.token_type {
                            HtmlTokenType::AttrValueText => {
                                let tok_val = val_tok.value().to_string();
                                let tok_start = val_tok.start;
                                let tok_end = val_tok.end;
                                value_text.push_str(&tok_val);
                                value_end = tok_end;
                                self.advance();
                                let tok_span = self.make_span(tok_start, tok_end);
                                tokens.push((InterpolatedTokenType::Text, vec![tok_val], tok_span));
                            }
                            HtmlTokenType::AttrValueInterpolation => {
                                // Interpolation parts: [startMarker, expression, endMarker]
                                let tok_parts: std::vec::Vec<String> =
                                    val_tok.parts.iter().map(|s| s.to_string()).collect();
                                let tok_start = val_tok.start;
                                let tok_end = val_tok.end;
                                // For backward compatibility, decode HTML entities in interpolation
                                let joined = tok_parts.join("");
                                value_text.push_str(&decode_entities_in_string(&joined));
                                value_end = tok_end;
                                self.advance();
                                let tok_span = self.make_span(tok_start, tok_end);
                                tokens.push((
                                    InterpolatedTokenType::Interpolation,
                                    tok_parts,
                                    tok_span,
                                ));
                            }
                            HtmlTokenType::EncodedEntity => {
                                // Encoded entity parts: [decodedValue, originalEntity]
                                let decoded = val_tok
                                    .parts
                                    .first()
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                let encoded =
                                    val_tok.parts.get(1).map(|s| s.to_string()).unwrap_or_default();
                                let tok_start = val_tok.start;
                                let tok_end = val_tok.end;
                                value_text.push_str(&decoded);
                                value_end = tok_end;
                                self.advance();
                                let tok_span = self.make_span(tok_start, tok_end);
                                tokens.push((
                                    InterpolatedTokenType::EncodedEntity,
                                    vec![decoded, encoded],
                                    tok_span,
                                ));
                            }
                            HtmlTokenType::AttrQuote => {
                                // Include closing quote in attribute span
                                attr_end = val_tok.end;
                                self.advance(); // consume closing quote
                                break;
                            }
                            _ => break,
                        }
                    }

                    let vs = self.make_span(value_start, value_end);
                    (value_text, Some(vs), Some(tokens))
                } else if tok.token_type == HtmlTokenType::AttrValueText
                    || tok.token_type == HtmlTokenType::AttrValueInterpolation
                {
                    // Unquoted attribute value - may have multiple tokens (text + interpolation)
                    let mut value_text = String::new();
                    let mut tokens: std::vec::Vec<(
                        InterpolatedTokenType,
                        std::vec::Vec<String>,
                        Span,
                    )> = std::vec::Vec::new();
                    let value_start = tok.start;
                    let mut value_end = tok.start;

                    while let Some(val_tok) = self.peek() {
                        match val_tok.token_type {
                            HtmlTokenType::AttrValueText => {
                                let tok_val = val_tok.value().to_string();
                                let tok_start = val_tok.start;
                                let tok_end = val_tok.end;
                                value_text.push_str(&tok_val);
                                value_end = tok_end;
                                self.advance();
                                let tok_span = self.make_span(tok_start, tok_end);
                                tokens.push((InterpolatedTokenType::Text, vec![tok_val], tok_span));
                            }
                            HtmlTokenType::AttrValueInterpolation => {
                                // Interpolation parts: [startMarker, expression, endMarker]
                                let tok_parts: std::vec::Vec<String> =
                                    val_tok.parts.iter().map(|s| s.to_string()).collect();
                                let tok_start = val_tok.start;
                                let tok_end = val_tok.end;
                                // For backward compatibility, decode HTML entities in interpolation
                                let joined = tok_parts.join("");
                                value_text.push_str(&decode_entities_in_string(&joined));
                                value_end = tok_end;
                                self.advance();
                                let tok_span = self.make_span(tok_start, tok_end);
                                tokens.push((
                                    InterpolatedTokenType::Interpolation,
                                    tok_parts,
                                    tok_span,
                                ));
                            }
                            _ => break,
                        }
                    }

                    attr_end = value_end;
                    let vs = self.make_span(value_start, value_end);
                    (value_text, Some(vs), Some(tokens))
                } else {
                    (String::new(), None, None)
                }
            } else {
                (String::new(), None, None)
            };

            let span = self.make_span(name_start, attr_end);
            let name_span = self.make_span(name_start, name_end);

            // Convert tokens to arena-allocated format
            let arena_value_tokens = value_tokens.map(|tokens| {
                let mut arena_tokens = Vec::new_in(self.allocator);
                for (token_type, parts, tok_span) in tokens {
                    let mut arena_parts = Vec::new_in(self.allocator);
                    for part in parts {
                        arena_parts.push(Ident::from_in(part, self.allocator));
                    }
                    arena_tokens.push(InterpolatedToken {
                        token_type,
                        parts: arena_parts,
                        span: tok_span,
                    });
                }
                arena_tokens
            });

            let attr = HtmlAttribute {
                name: Ident::from_in(name, self.allocator),
                value: Ident::from_in(value, self.allocator),
                span,
                name_span,
                value_span,
                value_tokens: arena_value_tokens,
            };
            attrs.push(attr);
        }

        (attrs, directives)
    }

    /// Checks if the current parent element should have its leading LF stripped.
    /// According to HTML spec, the first LF after textarea, pre, and listing start tags
    /// should be ignored.
    fn should_strip_leading_lf(&self) -> bool {
        if let Some(&container_idx) = self.container_stack.last() {
            if let ContainerIndex::Element(elem_idx) = container_idx {
                let element = &self.elements[elem_idx];
                // Check if this is a textarea, pre, or listing element with no children yet
                let tag_name = element.name.to_lowercase();
                if matches!(tag_name.as_str(), "textarea" | "pre" | "listing")
                    && element.children.is_empty()
                {
                    return true;
                }
            }
        }
        false
    }

    /// Consumes text content (Text, Interpolation, EncodedEntity tokens).
    /// Ported from Angular's `_consumeText` method.
    /// This is the unified text parser that collects tokens for whitespace transforms.
    fn consume_text(&mut self) -> Option<HtmlNode<'a>> {
        let first_token = self.advance()?;

        // Extract values from first token before any further borrowing
        let start = first_token.start;
        let mut end = first_token.end;
        // Capture full_start from the first token for source map accuracy
        let full_start = first_token.full_start;
        let first_token_type = first_token.token_type;
        let first_token_parts: std::vec::Vec<String> =
            first_token.parts.iter().map(|s| s.to_string()).collect();

        // Collect all tokens for the Text node
        let mut tokens: std::vec::Vec<(InterpolatedTokenType, std::vec::Vec<String>, Span)> =
            std::vec::Vec::new();

        // Build the decoded text value while collecting tokens
        let mut text = String::new();

        // Process first token
        let first_span = self.make_span(start, end);
        match first_token_type {
            HtmlTokenType::Text | HtmlTokenType::EscapableRawText | HtmlTokenType::RawText => {
                let value = first_token_parts.first().cloned().unwrap_or_default();
                text.push_str(&value);
                tokens.push((InterpolatedTokenType::Text, vec![value], first_span));
            }
            HtmlTokenType::Interpolation => {
                // For backward compatibility, decode HTML entities in interpolation
                // (same as Angular's _consumeText in parser.ts)
                let joined = first_token_parts.join("");
                let decoded_expr = decode_entities_in_string(&joined);
                text.push_str(&decoded_expr);
                tokens.push((InterpolatedTokenType::Interpolation, first_token_parts, first_span));
            }
            HtmlTokenType::EncodedEntity => {
                let decoded = first_token_parts.first().cloned().unwrap_or_default();
                let encoded = first_token_parts.get(1).cloned().unwrap_or_default();
                text.push_str(&decoded);
                tokens.push((
                    InterpolatedTokenType::EncodedEntity,
                    vec![decoded, encoded],
                    first_span,
                ));
            }
            _ => {}
        }

        // Merge adjacent Text, Interpolation, EncodedEntity tokens.
        // IMPORTANT: Do NOT include EscapableRawText/RawText here.
        // Angular's parser (parser.ts _consumeText) only continues for TEXT, INTERPOLATION,
        // and ENCODED_ENTITY tokens. Inside raw text elements like <textarea>, <script>, <style>,
        // <pre>, text tokens after entities are of type ESCAPABLE_RAW_TEXT, not TEXT.
        // By not including them here, we match Angular's behavior of creating separate text nodes
        // when there's an encoded entity (like &#10;) inside a raw text element.
        // See: https://github.com/angular/angular/blob/main/packages/compiler/src/ml_parser/parser.ts
        while let Some(next_tok) = self.peek() {
            match next_tok.token_type {
                HtmlTokenType::Text
                | HtmlTokenType::Interpolation
                | HtmlTokenType::EncodedEntity => {
                    // Safety: peek() just returned Some, so advance() should too
                    let Some(tok) = self.advance() else { break };
                    let tok_type = tok.token_type;
                    let tok_parts: std::vec::Vec<String> =
                        tok.parts.iter().map(|s| s.to_string()).collect();
                    let tok_start = tok.start;
                    let tok_end = tok.end;
                    end = tok_end;

                    let tok_span = self.make_span(tok_start, tok_end);

                    match tok_type {
                        HtmlTokenType::Text => {
                            let value = tok_parts.first().cloned().unwrap_or_default();
                            text.push_str(&value);
                            tokens.push((InterpolatedTokenType::Text, vec![value], tok_span));
                        }
                        HtmlTokenType::Interpolation => {
                            // For backward compatibility, decode HTML entities in interpolation
                            // (same as Angular's _consumeText in parser.ts)
                            let joined = tok_parts.join("");
                            let decoded_expr = decode_entities_in_string(&joined);
                            text.push_str(&decoded_expr);
                            tokens.push((
                                InterpolatedTokenType::Interpolation,
                                tok_parts,
                                tok_span,
                            ));
                        }
                        HtmlTokenType::EncodedEntity => {
                            let decoded = tok_parts.first().cloned().unwrap_or_default();
                            let encoded = tok_parts.get(1).cloned().unwrap_or_default();
                            text.push_str(&decoded);
                            tokens.push((
                                InterpolatedTokenType::EncodedEntity,
                                vec![decoded, encoded],
                                tok_span,
                            ));
                        }
                        _ => {}
                    }
                }
                _ => break,
            }
        }

        // Strip leading LF for textarea, pre, and listing elements (HTML spec)
        if self.should_strip_leading_lf() && text.starts_with('\n') {
            text = text[1..].to_string();
            // Also update the first token if it was a text token
            if let Some((InterpolatedTokenType::Text, parts, _)) = tokens.first_mut() {
                if let Some(first_part) = parts.first_mut() {
                    if first_part.starts_with('\n') {
                        *first_part = first_part[1..].to_string();
                    }
                }
            }
            // If stripping LF results in empty text, don't emit a text node
            if text.is_empty() {
                return None;
            }
        }

        // Angular only creates text nodes when the text has content.
        // See Angular's parser.ts `_consumeText`: `if (text.length > 0) { ... }`
        // Skip if text value is empty or there are no tokens at all.
        if text.is_empty() || tokens.is_empty() {
            return None;
        }

        // Convert to arena-allocated tokens
        let mut arena_tokens = Vec::new_in(self.allocator);
        for (token_type, parts, span) in tokens {
            let mut arena_parts = Vec::new_in(self.allocator);
            for part in parts {
                arena_parts.push(Ident::from_in(part, self.allocator));
            }
            arena_tokens.push(InterpolatedToken { token_type, parts: arena_parts, span });
        }

        let span = self.make_span(start, end);
        let text_node = HtmlText {
            value: Ident::from_in(text, self.allocator),
            span,
            full_start,
            tokens: arena_tokens,
        };
        Some(HtmlNode::Text(Box::new_in(text_node, self.allocator)))
    }

    /// Parses an incomplete tag (error recovery).
    /// Creates an element with no children and no end tag.
    fn parse_incomplete_tag(&mut self) -> Option<HtmlNode<'a>> {
        let token = self.advance()?;
        let start = token.start;
        let end = token.end;

        // IncompleteTagOpen has parts [prefix, name]
        let tag_name = token.name().to_string();

        // Skip if name is empty
        if tag_name.is_empty() {
            // Report error but don't create element
            let err = self.make_error(start, "Unexpected '<' in text");
            self.errors.push(err);
            return None;
        }

        let span = self.make_span(start, end);
        let start_span = span.clone();

        // Report parse error
        let err = self.make_error(start, format!("Unexpected end of tag '{}'", tag_name));
        self.errors.push(err);

        // Create element with no end span (incomplete)
        // Note: is_self_closing is false because this is an incomplete tag, not explicitly self-closing
        let element = HtmlElement {
            name: Ident::from_in(tag_name.clone(), self.allocator),
            component_prefix: None,
            component_tag_name: None,
            attrs: Vec::new_in(self.allocator),
            directives: Vec::new_in(self.allocator),
            children: Vec::new_in(self.allocator),
            span,
            start_span,
            end_span: None,
            is_self_closing: false,
            is_void: is_void_element(&tag_name),
            // An incomplete tag (`IncompleteTagOpen`) is never a selectorless
            // component: the lexer only emits `ComponentOpenStart` for components.
            is_component: false,
        };
        Some(HtmlNode::Element(Box::new_in(element, self.allocator)))
    }

    /// Parses a comment.
    /// In the new lexer: CommentStart → RawText (content) → CommentEnd
    fn parse_comment(&mut self) -> Option<HtmlNode<'a>> {
        let start_token = self.advance()?; // consume CommentStart
        let start = start_token.start;

        // Get comment content from RawText token
        let value = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::RawText {
                let v = tok.value().to_string();
                self.advance();
                v
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Skip CommentEnd token
        let end = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::CommentEnd {
                let e = tok.end;
                self.advance();
                e
            } else {
                self.peek().map(|t| t.start).unwrap_or(start)
            }
        } else {
            start
        };

        let span = self.make_span(start, end);
        let comment = HtmlComment { value: Ident::from_in(value, self.allocator), span };
        Some(HtmlNode::Comment(Box::new_in(comment, self.allocator)))
    }

    /// Parses a CDATA section and converts it to a text node.
    /// In the lexer: CdataStart → RawText (content) → CdataEnd
    fn parse_cdata(&mut self) -> Option<HtmlNode<'a>> {
        let start_token = self.advance()?; // consume CdataStart
        let start = start_token.start;

        // Get CDATA content from RawText token
        let value = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::RawText {
                let v = tok.value().to_string();
                self.advance();
                v
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Skip CdataEnd token
        let end = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::CdataEnd {
                let e = tok.end;
                self.advance();
                e
            } else {
                self.peek().map(|t| t.start).unwrap_or(start)
            }
        } else {
            start
        };

        let span = self.make_span(start, end);
        // CDATA content becomes a text node with a single text token
        let mut tokens = Vec::new_in(self.allocator);
        let mut parts = Vec::new_in(self.allocator);
        parts.push(Ident::from_in(value.clone(), self.allocator));
        tokens.push(InterpolatedToken { token_type: InterpolatedTokenType::Text, parts, span });
        // CDATA tokens don't have leading trivia stripped
        let text = HtmlText {
            value: Ident::from_in(value, self.allocator),
            span,
            full_start: None,
            tokens,
        };
        Some(HtmlNode::Text(Box::new_in(text, self.allocator)))
    }

    /// Parses a @let declaration.
    /// In the new lexer: LetStart (with name in value()) → LetValue → LetEnd/IncompleteLet
    fn parse_let_declaration(&mut self) -> Option<HtmlNode<'a>> {
        let start_token = self.advance()?; // consume LetStart
        let start = start_token.start;
        let start_end = start_token.end;

        // In the new lexer, LetStart contains the variable name in its parts
        let name = start_token.value().to_string();
        // Name span should cover just the variable name, which ends at start_end
        // and starts at start_end - name.len()
        let name_start = start_end - name.len() as u32;
        let name_span = self.make_span(name_start, start_end);

        // Get value expression
        let (value_text, value_span) = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::LetValue {
                let v = tok.value().to_string();
                let vs = self.make_span(tok.start, tok.end);
                self.advance();
                (v, vs)
            } else {
                (String::new(), self.make_span(start_end, start_end))
            }
        } else {
            (String::new(), self.make_span(start_end, start_end))
        };

        // Skip LetEnd or IncompleteLet token
        // For sourceSpan, we want to end BEFORE the semicolon (at tok.start), not after it
        let end = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::LetEnd {
                // LetEnd is the semicolon - span should end before it
                let e = tok.start;
                self.advance();
                e
            } else if tok.token_type == HtmlTokenType::IncompleteLet {
                // Incomplete let - include the whole thing
                let e = tok.end;
                self.advance();
                e
            } else {
                value_span.end
            }
        } else {
            value_span.end
        };

        let span = self.make_span(start, end);

        // Parse the value expression using the expression parser
        let expr_parser = BindingParser::new(self.allocator);
        let value_str = self.allocator.alloc_str(&value_text);
        let parse_result = expr_parser.parse_binding(value_str, value_span);

        let let_decl = HtmlLetDeclaration {
            name: Ident::from_in(name, self.allocator),
            value: parse_result.ast,
            span,
            name_span,
            value_span,
        };

        Some(HtmlNode::LetDeclaration(Box::new_in(let_decl, self.allocator)))
    }

    /// Parses an ICU expansion form.
    fn parse_expansion(&mut self) -> Option<HtmlNode<'a>> {
        let start_token = self.advance()?; // consume ExpansionFormStart
        let start = start_token.start;

        // Get switch value (RawText)
        let (switch_value, switch_value_span) = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::RawText {
                let v = tok.value().to_string();
                let vs = self.make_span(tok.start, tok.end);
                self.advance();
                (v, vs)
            } else {
                (String::new(), self.make_span(start, start))
            }
        } else {
            (String::new(), self.make_span(start, start))
        };

        // Get expansion type (RawText)
        let expansion_type = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::RawText {
                let v = tok.value().to_string();
                self.advance();
                v
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Parse cases
        let mut cases = Vec::new_in(self.allocator);
        while let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::ExpansionFormEnd {
                break;
            }

            if tok.token_type == HtmlTokenType::ExpansionCaseValue {
                if let Some(case) = self.parse_expansion_case() {
                    cases.push(case);
                }
            } else {
                // Skip unexpected tokens
                self.advance();
            }
        }

        // Consume ExpansionFormEnd
        let end = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::ExpansionFormEnd {
                let e = tok.end;
                self.advance();
                e
            } else {
                self.peek().map(|t| t.start).unwrap_or(start)
            }
        } else {
            start
        };

        let span = self.make_span(start, end);

        let expansion = HtmlExpansion {
            switch_value: Ident::from_in(switch_value, self.allocator),
            expansion_type: Ident::from_in(expansion_type, self.allocator),
            cases,
            span,
            switch_value_span,
            in_i18n_block: false, // Set by i18n processing when inside i18n blocks
        };

        Some(HtmlNode::Expansion(Box::new_in(expansion, self.allocator)))
    }

    /// Parses a single expansion case.
    ///
    /// This follows Angular's approach: we parse the expansion case content
    /// in isolation, collecting nodes separately from the main container stack.
    fn parse_expansion_case(&mut self) -> Option<HtmlExpansionCase<'a>> {
        let value_token = self.advance()?; // consume ExpansionCaseValue
        let case_start = value_token.start;
        let value = value_token.value().to_string();
        let value_start = value_token.start;
        let value_end = value_token.end;
        let value_span = self.make_span(value_start, value_end);

        // Consume ExpansionCaseExpStart
        let exp_start = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::ExpansionCaseExpStart {
                let s = tok.end;
                self.advance();
                s
            } else {
                value_end
            }
        } else {
            value_end
        };

        // Parse expansion content in isolation
        // Following Angular's approach: save current state, parse content, restore state
        // This ensures elements inside expansion cases go to the expansion, not the parent
        let saved_root_nodes = std::mem::take(&mut self.root_nodes);
        let saved_container_stack = std::mem::take(&mut self.container_stack);
        let saved_elements = std::mem::take(&mut self.elements);
        let saved_blocks = std::mem::take(&mut self.blocks);

        // Parse content until ExpansionCaseExpEnd
        while let Some(tok) = self.peek() {
            match tok.token_type {
                HtmlTokenType::ExpansionCaseExpEnd => break,
                HtmlTokenType::ExpansionFormEnd => break,
                HtmlTokenType::Text
                | HtmlTokenType::EncodedEntity
                | HtmlTokenType::Interpolation => {
                    if let Some(node) = self.consume_text() {
                        self.add_to_parent(node);
                    }
                }
                HtmlTokenType::TagOpenStart | HtmlTokenType::ComponentOpenStart => {
                    self.consume_element_start();
                }
                HtmlTokenType::TagClose | HtmlTokenType::ComponentClose => {
                    self.consume_element_end();
                }
                HtmlTokenType::ExpansionFormStart => {
                    // Nested expansion
                    if let Some(node) = self.parse_expansion() {
                        self.add_to_parent(node);
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }

        // Close any unclosed elements within this expansion case
        while let Some(container) = self.container_stack.pop() {
            match container {
                ContainerIndex::Element(idx) => {
                    let element = std::mem::replace(
                        &mut self.elements[idx],
                        HtmlElement {
                            name: Ident::from(""),
                            component_prefix: None,
                            component_tag_name: None,
                            attrs: Vec::new_in(self.allocator),
                            directives: Vec::new_in(self.allocator),
                            children: Vec::new_in(self.allocator),
                            span: Span::new(0, 0),
                            start_span: Span::new(0, 0),
                            end_span: None,
                            is_self_closing: false,
                            is_void: false,
                            is_component: false,
                        },
                    );
                    self.add_to_parent(HtmlNode::Element(Box::new_in(element, self.allocator)));
                }
                ContainerIndex::Block(idx) => {
                    let block = std::mem::replace(
                        &mut self.blocks[idx],
                        HtmlBlock {
                            block_type: BlockType::If,
                            name: Ident::from(""),
                            parameters: Vec::new_in(self.allocator),
                            children: Vec::new_in(self.allocator),
                            span: Span::new(0, 0),
                            name_span: Span::new(0, 0),
                            start_span: Span::new(0, 0),
                            end_span: None,
                        },
                    );
                    self.add_to_parent(HtmlNode::Block(Box::new_in(block, self.allocator)));
                }
            }
        }

        // Collect parsed nodes and restore state
        let expansion_nodes = std::mem::replace(&mut self.root_nodes, saved_root_nodes);
        self.container_stack = saved_container_stack;
        self.elements = saved_elements;
        self.blocks = saved_blocks;

        // Convert to allocator vec
        let mut expansion = Vec::new_in(self.allocator);
        for node in expansion_nodes {
            expansion.push(node);
        }

        // Consume ExpansionCaseExpEnd
        let exp_end = if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::ExpansionCaseExpEnd {
                let e = tok.end;
                self.advance();
                e
            } else {
                self.peek().map(|t| t.start).unwrap_or(exp_start)
            }
        } else {
            exp_start
        };

        let span = self.make_span(case_start, exp_end);
        let expansion_span = self.make_span(exp_start, exp_end);

        Some(HtmlExpansionCase {
            value: Ident::from_in(value, self.allocator),
            expansion,
            span,
            value_span,
            expansion_span,
        })
    }

    /// Consumes a block open (@if, @for, etc.) and pushes it onto the container stack.
    fn consume_block_open(&mut self) {
        let Some(token) = self.advance() else {
            return; // No token to consume
        };
        let name = token.value().to_string();
        let start = token.start;
        let name_end = token.end;

        let block_type = classify_block_type(&name);

        // Collect block parameters
        let mut parameters = Vec::new_in(self.allocator);
        while let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::BlockParameter {
                let Some(param_token) = self.advance() else {
                    break; // Should not happen after peek, but handle gracefully
                };
                // Extract values before making immutable borrow
                let param_text = param_token.value().to_string();
                let param_start = param_token.start;
                let param_end = param_token.end;
                let param_span = self.make_span(param_start, param_end);
                parameters.push(HtmlBlockParameter {
                    expression: Ident::from_in(&param_text, self.allocator),
                    span: param_span,
                });
            } else {
                break;
            }
        }

        // Skip BlockOpenEnd token if present (the `{`)
        if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::BlockOpenEnd {
                self.advance();
            }
        }

        let end = self.peek().map(|t| t.start).unwrap_or(start);
        let span = self.make_span(start, end);
        let name_span = self.make_span(start, name_end);
        let start_span = self.make_span(start, end);

        let block = HtmlBlock {
            block_type,
            name: Ident::from_in(name, self.allocator),
            parameters,
            children: Vec::new_in(self.allocator),
            span,
            name_span,
            start_span,
            end_span: None,
        };

        // Push block onto container stack - children will be added as we parse
        self.push_block_container(block);
    }

    /// Consumes an `IncompleteBlockOpen` token (e.g. `@default never` WITHOUT a trailing
    /// `;`, or any `@name` that never opened a `{ }` body). Mirrors upstream v21.2.7
    /// `Parser._consumeIncompleteBlock` (ml_parser/parser.ts:786-812): it collects any
    /// trailing `BlockParameter` tokens, builds the Block node (using the token's part as
    /// the name, so the block is NOT silently dropped), pushes it and immediately closes it
    /// (incomplete blocks have no children), then reports the exact "Incomplete block ..."
    /// diagnostic. Without this, `parse_and_add_node`'s default arm would just `advance()`
    /// past the token and the `@default never` would vanish with zero errors.
    fn consume_incomplete_block(&mut self) {
        let Some(token) = self.advance() else {
            return; // No token to consume
        };
        // `token.parts[0]` is the block name, e.g. `default never` (upstream `token.parts[0]`).
        let name = token.value().to_string();
        let start = token.start;
        let name_end = token.end;
        let block_type = classify_block_type(&name);

        // Collect any trailing block parameters (matches upstream's `while BLOCK_PARAMETER`).
        let mut parameters = Vec::new_in(self.allocator);
        while let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::BlockParameter {
                let Some(param_token) = self.advance() else {
                    break; // Should not happen after peek, but handle gracefully
                };
                let param_text = param_token.value().to_string();
                let param_start = param_token.start;
                let param_end = param_token.end;
                let param_span = self.make_span(param_start, param_end);
                parameters.push(HtmlBlockParameter {
                    expression: Ident::from_in(&param_text, self.allocator),
                    span: param_span,
                });
            } else {
                break;
            }
        }

        let end = self.peek().map(|t| t.start).unwrap_or(start);
        let span = self.make_span(start, end);
        let name_span = self.make_span(start, name_end);
        let start_span = self.make_span(start, end);

        let block = HtmlBlock {
            block_type,
            name: Ident::from_in(name.clone(), self.allocator),
            parameters,
            children: Vec::new_in(self.allocator),
            span,
            name_span,
            start_span,
            end_span: None,
        };

        // Incomplete blocks have no children: push then immediately close (pop) so the node
        // is preserved in the tree, mirroring upstream `_pushContainer` + `_popContainer`.
        self.push_block_container(block);
        if let Some(node) = self.pop_block_container(None) {
            self.add_to_parent(node);
        }

        // Report the upstream-exact diagnostic (ml_parser/parser.ts:804-811).
        let err = self.make_error(
            start,
            format!(
                "Incomplete block \"{name}\". If you meant to write the @ character, \
                 you should use the \"&#64;\" HTML entity instead."
            ),
        );
        self.errors.push(err);
    }

    /// Parses a directive token sequence: DirectiveName → DirectiveOpen? → attrs → DirectiveClose?
    /// Returns an HtmlDirective node representing the selectorless directive.
    fn parse_directive(&mut self) -> Option<HtmlDirective<'a>> {
        // Consume DirectiveName token
        let name_token = self.advance()?;
        let directive_start = name_token.start;
        let name = name_token.value().to_string();
        let name_end = name_token.end;
        let name_span = self.make_span(directive_start, name_end);

        let mut attrs = Vec::new_in(self.allocator);
        let mut start_paren_span = None;
        let mut end_paren_span = None;
        let mut directive_end = name_end;

        // Check for DirectiveOpen (opening paren)
        if let Some(tok) = self.peek() {
            if tok.token_type == HtmlTokenType::DirectiveOpen {
                start_paren_span = Some(self.make_span(tok.start, tok.end));
                self.advance(); // consume (

                // Parse attributes within directive
                while let Some(inner_tok) = self.peek() {
                    match inner_tok.token_type {
                        HtmlTokenType::AttrName => {
                            // Parse attribute
                            // Safety: peek() just returned Some, so advance() should too
                            let Some(attr_token) = self.advance() else { break };
                            let attr_name = attr_token.name().to_string();
                            let attr_name_start = attr_token.start;
                            let attr_name_end = attr_token.end;
                            let mut attr_end = attr_name_end;

                            // Check for value
                            let (attr_value, value_span) = if let Some(val_tok) = self.peek() {
                                if val_tok.token_type == HtmlTokenType::AttrQuote {
                                    self.advance(); // consume opening quote

                                    // Collect value parts
                                    let mut value_text = String::new();
                                    let value_start =
                                        self.peek().map(|t| t.start).unwrap_or(attr_name_end);
                                    let mut value_end = value_start;

                                    while let Some(part_tok) = self.peek() {
                                        match part_tok.token_type {
                                            HtmlTokenType::AttrValueText => {
                                                value_text.push_str(part_tok.value());
                                                value_end = part_tok.end;
                                                self.advance();
                                            }
                                            HtmlTokenType::AttrValueInterpolation => {
                                                if part_tok.parts.len() >= 3 {
                                                    value_text.push_str(&part_tok.parts[0]);
                                                    value_text.push_str(&part_tok.parts[1]);
                                                    value_text.push_str(&part_tok.parts[2]);
                                                } else {
                                                    value_text.push_str(part_tok.value());
                                                }
                                                value_end = part_tok.end;
                                                self.advance();
                                            }
                                            HtmlTokenType::EncodedEntity => {
                                                value_text.push_str(part_tok.value());
                                                value_end = part_tok.end;
                                                self.advance();
                                            }
                                            HtmlTokenType::AttrQuote => {
                                                attr_end = part_tok.end;
                                                self.advance(); // consume closing quote
                                                break;
                                            }
                                            _ => break,
                                        }
                                    }

                                    (value_text, Some(self.make_span(value_start, value_end)))
                                } else if val_tok.token_type == HtmlTokenType::AttrValueText {
                                    let v = val_tok.value().to_string();
                                    let vs = self.make_span(val_tok.start, val_tok.end);
                                    attr_end = val_tok.end;
                                    self.advance();
                                    (v, Some(vs))
                                } else {
                                    (String::new(), None)
                                }
                            } else {
                                (String::new(), None)
                            };

                            let attr_span = self.make_span(attr_name_start, attr_end);
                            let attr_name_span = self.make_span(attr_name_start, attr_name_end);

                            attrs.push(HtmlAttribute {
                                name: Ident::from_in(attr_name, self.allocator),
                                value: Ident::from_in(attr_value, self.allocator),
                                span: attr_span,
                                name_span: attr_name_span,
                                value_span,
                                value_tokens: None,
                            });
                        }
                        HtmlTokenType::DirectiveClose => {
                            // Safety: peek() just returned Some, so advance() should too
                            let Some(close_tok) = self.advance() else { break };
                            let close_start = close_tok.start;
                            let close_end = close_tok.end;
                            end_paren_span = Some(self.make_span(close_start, close_end));
                            directive_end = close_end;
                            break;
                        }
                        _ => break,
                    }
                }
            }
        }

        let span = self.make_span(directive_start, directive_end);

        Some(HtmlDirective {
            name: Ident::from_in(name, self.allocator),
            attrs,
            span,
            name_span,
            start_paren_span,
            end_paren_span,
        })
    }

    /// Consumes a block close (`}`) and pops the block from the stack.
    fn consume_block_close(&mut self) {
        let Some(token) = self.advance() else {
            return; // No token to consume
        };
        let start = token.start;
        let end = token.end;
        let end_span = self.make_span(start, end);

        // Pop the block from the stack and add it to the parent
        if let Some(node) = self.pop_block_container(Some(end_span)) {
            self.add_to_parent(node);
        } else {
            // No matching block - report error
            let err = self.make_error(
                start,
                "Unexpected closing block. The block may have been closed earlier. \
                 If you meant to write the } character, you should use the \"&#125;\" \
                 HTML entity instead.",
            );
            self.errors.push(err);
        }
    }
}

/// Classifies a block name (the lexer's `BlockOpenStart`/`IncompleteBlockOpen` part) into
/// a `BlockType`. Used by both `consume_block_open` and `consume_incomplete_block` so an
/// incomplete `@default never` carries the same `BlockType::DefaultNever` it would when valid.
fn classify_block_type(name: &str) -> BlockType {
    match name {
        "if" => BlockType::If,
        "else" => BlockType::Else,
        // Match Angular's ELSE_IF_PATTERN: /^else[^\S\r\n]+if/
        // Any block name starting with "else " followed by "if" (e.g. "else if",
        // "else ifx") is classified as ElseIf, matching Angular's regex-based
        // connected-block detection.
        _ if is_else_if_pattern(name) => BlockType::ElseIf,
        "for" => BlockType::For,
        "empty" => BlockType::Empty,
        "switch" => BlockType::Switch,
        "case" => BlockType::Case,
        "default" => BlockType::Default,
        // Angular v21.2.7 exhaustive-switch feature: `@default never;`.
        "default never" => BlockType::DefaultNever,
        "defer" => BlockType::Defer,
        "placeholder" => BlockType::Placeholder,
        "loading" => BlockType::Loading,
        "error" => BlockType::Error,
        _ => BlockType::If, // Default
    }
}

/// Checks if the current element should be auto-closed when a new element is opened.
/// Uses the tag definitions from tags.rs to match Angular's behavior exactly.
fn should_auto_close(current_tag: &str, new_tag: &str) -> bool {
    let tag_def = get_html_tag_definition(current_tag);
    tag_def.is_closed_by_child(new_tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_element() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<div></div>", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1);
        assert!(matches!(&result.nodes[0], HtmlNode::Element(_)));
    }

    #[test]
    fn test_parse_self_closing() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<input />", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1);
    }

    #[test]
    fn test_parse_text() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "Hello world", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1);
        assert!(matches!(&result.nodes[0], HtmlNode::Text(_)));
    }

    // ---- Finding 2: default-mode `<_foo>` is text, not an element ----
    //
    // Oracle (`@angular/compiler@21.2.7` `HtmlParser.parse(..)`):
    //   default `<_foo></_foo>`      -> ONE root Text node "<_foo>" + a single
    //                                   "Unexpected closing tag \"_foo\"" parse error.
    //   selectorless `<_foo></_foo>` -> a Component node (`is_component == true`).
    //   default `<MyCmp></MyCmp>`    -> a normal Element named "MyCmp" (control).
    #[test]
    fn test_unexpected_close_error_span_has_correct_line_col_multiline() {
        // FINDING 3 (span fidelity): the unexpected-closing-tag error span must carry the
        // correct LINE/COLUMN (not fabricated 0/0) on multiline templates, matching
        // Angular's `endToken.sourceSpan`. For "<div>\n  text\n</span>" the stray
        // `</span>` close spans offset 13..20, line 2 col 0..7 (0-based line/col).
        // Verified vs @angular/compiler@21.2.7.
        let allocator = Allocator::default();
        let result = HtmlParser::new(&allocator, "<div>\n  text\n</span>", "test.html").parse();
        assert_eq!(result.errors.len(), 1, "errors: {:?}", result.errors);
        let span = &result.errors[0].span;
        assert_eq!((span.start.offset, span.start.line, span.start.col), (13, 2, 0), "start loc");
        assert_eq!((span.end.offset, span.end.line, span.end.col), (20, 2, 7), "end loc");
        assert!(
            result.errors[0].msg.ends_with("implied-end-tags"),
            "msg: {:?}",
            result.errors[0].msg
        );
    }

    #[test]
    fn test_parse_default_mode_underscore_tag_is_text() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<_foo></_foo>", "test.html");
        let result = parser.parse();
        // Open tag becomes a Text node "<_foo>".
        assert_eq!(result.nodes.len(), 1, "expected a single Text node, got {:?}", result.nodes);
        match &result.nodes[0] {
            HtmlNode::Text(t) => assert_eq!(t.value.as_str(), "<_foo>"),
            other => panic!("expected Text node, got {other:?}"),
        }
        // The only error is the unexpected-closing-tag error for `</_foo>`; the lexer's
        // unexpected-character error for the open tag is swallowed (upstream parity).
        assert_eq!(result.errors.len(), 1, "errors: {:?}", result.errors);
        // FINDING 3: in DEFAULT mode `</_foo>` is an ELEMENT close, so the message
        // includes the W3C help URL and the span is the FULL close-token span [6-13].
        // Verified vs @angular/compiler@21.2.7 (`_consumeElementEndTag`, parser.ts:581).
        assert_eq!(
            result.errors[0].msg,
            "Unexpected closing tag \"_foo\". It may happen when the tag has already been closed by another tag. For more info see https://www.w3.org/TR/html5/syntax.html#closing-elements-that-have-implied-end-tags",
            "unexpected error message: {:?}",
            result.errors[0].msg
        );
        assert_eq!(
            (result.errors[0].span.start.offset, result.errors[0].span.end.offset),
            (6, 13),
            "error span must be the full close-token span"
        );
    }

    // ---- Finding 2 (parser): mid-text selectorless `<_foo>` is TEXT + dangling close ----
    //
    // Oracle (`@angular/compiler@21.2.7`, `{selectorlessEnabled:true}`):
    //   `x<_foo></_foo>`            -> Text "x<_foo>" [0-7] + a COMPONENT-close error
    //                                  (NO URL) at span [7-14].
    //   `<div>x<_Foo></_Foo></div>`-> div with child Text "x<_Foo>" [5-12] + a
    //                                  component-close error at [12-19].
    #[test]
    fn test_parse_selectorless_midtext_underscore_is_text() {
        let allocator = Allocator::default();
        let result =
            HtmlParser::with_selectorless(&allocator, "x<_foo></_foo>", "test.html").parse();
        // Single root Text node "x<_foo>" — the `<_foo>` open is NOT a component mid-text.
        assert_eq!(result.nodes.len(), 1, "nodes: {:?}", result.nodes);
        match &result.nodes[0] {
            HtmlNode::Text(t) => assert_eq!(t.value.as_str(), "x<_foo>"),
            other => panic!("expected Text node, got {other:?}"),
        }
        // The dangling `</_foo>` is a COMPONENT close: error has NO W3C URL, span [7-14].
        assert_eq!(result.errors.len(), 1, "errors: {:?}", result.errors);
        assert_eq!(
            result.errors[0].msg,
            "Unexpected closing tag \"_foo\". It may happen when the tag has already been closed by another tag.",
            "msg: {:?}",
            result.errors[0].msg
        );
        assert_eq!((result.errors[0].span.start.offset, result.errors[0].span.end.offset), (7, 14));
    }

    #[test]
    fn test_parse_selectorless_midtext_underscore_inside_element() {
        let allocator = Allocator::default();
        let result =
            HtmlParser::with_selectorless(&allocator, "<div>x<_Foo></_Foo></div>", "test.html")
                .parse();
        // One root `div` containing a single Text child "x<_Foo>".
        assert_eq!(result.nodes.len(), 1, "nodes: {:?}", result.nodes);
        match &result.nodes[0] {
            HtmlNode::Element(el) => {
                assert_eq!(el.name.as_str(), "div");
                assert_eq!(el.children.len(), 1, "children: {:?}", el.children);
                match &el.children[0] {
                    HtmlNode::Text(t) => assert_eq!(t.value.as_str(), "x<_Foo>"),
                    other => panic!("expected Text child, got {other:?}"),
                }
            }
            other => panic!("expected div Element, got {other:?}"),
        }
        // The dangling `</_Foo>` component close error at [12-19].
        assert_eq!(result.errors.len(), 1, "errors: {:?}", result.errors);
        assert_eq!(
            (result.errors[0].span.start.offset, result.errors[0].span.end.offset),
            (12, 19)
        );
    }

    #[test]
    fn test_parse_selectorless_midtext_uppercase_is_component() {
        // Control: mid-text uppercase `<Foo>` IS a tag start and opens a component.
        let allocator = Allocator::default();
        let result = HtmlParser::with_selectorless(&allocator, "x<Foo></Foo>", "test.html").parse();
        assert_eq!(result.errors.len(), 0, "errors: {:?}", result.errors);
        assert_eq!(result.nodes.len(), 2, "nodes: {:?}", result.nodes);
        match &result.nodes[0] {
            HtmlNode::Text(t) => assert_eq!(t.value.as_str(), "x"),
            other => panic!("expected Text node, got {other:?}"),
        }
        match &result.nodes[1] {
            HtmlNode::Element(el) => {
                assert!(el.is_component, "mid-text `<Foo>` must be a component");
                assert_eq!(el.name.as_str(), "Foo");
            }
            other => panic!("expected component Element, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_selectorless_mode_underscore_tag_is_component() {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, "<_foo></_foo>", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1, "nodes: {:?}", result.nodes);
        match &result.nodes[0] {
            HtmlNode::Element(el) => {
                assert!(el.is_component, "selectorless `<_foo>` must be a component");
                assert_eq!(el.name.as_str(), "_foo");
            }
            other => panic!("expected component Element, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_default_mode_uppercase_tag_is_normal_element() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<MyCmp></MyCmp>", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1);
        match &result.nodes[0] {
            HtmlNode::Element(el) => {
                assert!(!el.is_component, "default-mode `<MyCmp>` must be a normal element");
                assert_eq!(el.name.as_str(), "MyCmp");
            }
            other => panic!("expected Element, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_nested() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<div><span>text</span></div>", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1);
        if let HtmlNode::Element(el) = &result.nodes[0] {
            assert_eq!(el.children.len(), 1);
        }
    }

    #[test]
    fn test_parse_attributes() {
        let allocator = Allocator::default();
        let parser =
            HtmlParser::new(&allocator, "<div id=\"test\" class=\"foo\"></div>", "test.html");
        let result = parser.parse();
        assert_eq!(result.nodes.len(), 1);
        if let HtmlNode::Element(el) = &result.nodes[0] {
            assert_eq!(el.attrs.len(), 2);
        }
    }

    #[test]
    fn test_parse_block_with_children() {
        let allocator = Allocator::default();
        // Template: @if (condition) {<div>content</div>} (no whitespace)
        let parser =
            HtmlParser::new(&allocator, "@if (condition) {<div>content</div>}", "test.html");
        let result = parser.parse();
        assert_eq!(result.errors.len(), 0, "Expected no errors: {:?}", result.errors);
        assert_eq!(result.nodes.len(), 1, "Expected 1 root node");

        if let HtmlNode::Block(block) = &result.nodes[0] {
            assert_eq!(block.name.as_str(), "if");
            assert_eq!(block.block_type, BlockType::If);
            // Find the element child (skip whitespace text nodes)
            let element_count =
                block.children.iter().filter(|c| matches!(c, HtmlNode::Element(_))).count();
            assert_eq!(element_count, 1, "Block should have 1 element child");
            for child in block.children.iter() {
                if let HtmlNode::Element(el) = child {
                    assert_eq!(el.name.as_str(), "div");
                    assert!(el.children.len() >= 1, "Div should have text child");
                }
            }
        } else {
            panic!("Expected Block node");
        }
    }

    #[test]
    fn test_parse_nested_blocks() {
        let allocator = Allocator::default();
        // Template: @if (a) {@if (b) {<span>nested</span>}} (no extra whitespace)
        let parser =
            HtmlParser::new(&allocator, "@if (a) {@if (b) {<span>nested</span>}}", "test.html");
        let result = parser.parse();
        assert_eq!(result.errors.len(), 0, "Expected no errors: {:?}", result.errors);
        assert_eq!(result.nodes.len(), 1);

        if let HtmlNode::Block(outer) = &result.nodes[0] {
            assert_eq!(outer.name.as_str(), "if");
            // Find the inner block (skip whitespace text nodes)
            let inner_blocks: std::vec::Vec<_> =
                outer.children.iter().filter(|c| matches!(c, HtmlNode::Block(_))).collect();
            assert_eq!(inner_blocks.len(), 1, "Outer block should have 1 block child");

            if let HtmlNode::Block(inner) = inner_blocks[0] {
                assert_eq!(inner.name.as_str(), "if");
                // Find element children in inner block
                let element_count =
                    inner.children.iter().filter(|c| matches!(c, HtmlNode::Element(_))).count();
                assert_eq!(element_count, 1, "Inner block should have 1 element child");
            } else {
                panic!("Expected inner Block node");
            }
        } else {
            panic!("Expected outer Block node");
        }
    }

    #[test]
    fn test_parse_for_block() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(
            &allocator,
            "@for (item of items; track item.id) { <li>{{item.name}}</li> }",
            "test.html",
        );
        let result = parser.parse();
        assert_eq!(result.errors.len(), 0, "Expected no errors: {:?}", result.errors);
        assert_eq!(result.nodes.len(), 1);

        if let HtmlNode::Block(block) = &result.nodes[0] {
            assert_eq!(block.name.as_str(), "for");
            assert_eq!(block.block_type, BlockType::For);
            assert!(block.children.len() >= 1, "For block should have children");
        } else {
            panic!("Expected Block node");
        }
    }

    #[test]
    fn test_parse_unclosed_block_error() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "@if (x) { <div>no close", "test.html");
        let result = parser.parse();
        // Should report unclosed block error
        assert!(!result.errors.is_empty(), "Expected unclosed block error");
        assert!(
            result.errors.iter().any(|e| e.msg.contains("Unclosed block")),
            "At least one error message should mention unclosed block"
        );
    }

    #[test]
    fn test_parse_let_declaration() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "@let count = items.length;", "test.html");
        let result = parser.parse();
        assert_eq!(result.errors.len(), 0, "Expected no errors: {:?}", result.errors);
        assert_eq!(result.nodes.len(), 1, "Expected 1 node");

        if let HtmlNode::LetDeclaration(decl) = &result.nodes[0] {
            assert_eq!(decl.name.as_str(), "count");
        } else {
            panic!("Expected LetDeclaration node, got {:?}", result.nodes[0]);
        }
    }

    #[test]
    fn test_parse_let_inside_block() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(
            &allocator,
            "@if (condition) {@let total = sum(items);<div>{{total}}</div>}",
            "test.html",
        );
        let result = parser.parse();
        assert_eq!(result.errors.len(), 0, "Expected no errors: {:?}", result.errors);
        assert_eq!(result.nodes.len(), 1, "Expected 1 root node");

        if let HtmlNode::Block(block) = &result.nodes[0] {
            // Should have at least the let declaration and div element
            let let_count =
                block.children.iter().filter(|c| matches!(c, HtmlNode::LetDeclaration(_))).count();
            assert_eq!(let_count, 1, "Block should have 1 let declaration");
        } else {
            panic!("Expected Block node");
        }
    }

    // Regression: https://github.com/voidzero-dev/oxc-angular-compiler/issues/290
    // Closing a parent while a non-optional-end-tag child is still open used to be
    // silently accepted, leaving `result.errors` empty.
    #[test]
    fn test_parse_unclosed_inner_tag_reports_error() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<div><span></div>", "test.html");
        let result = parser.parse();
        assert!(
            !result.errors.is_empty(),
            "Expected a diagnostic when </div> closes an unclosed <span>"
        );
        assert!(
            result.errors.iter().any(|e| e.msg.contains("Unexpected closing tag \"div\"")),
            "Diagnostic should mention the closing tag that triggered the implicit close, got: {:?}",
            result.errors
        );
        // Recovery must still produce an AST for the outer <div>.
        assert_eq!(result.nodes.len(), 1, "Recovery should still yield the outer element");
    }

    #[test]
    fn test_parse_mismatched_close_does_not_error_for_optional_end_tag() {
        // `<li>` has closed_by_parent = true, so closing the surrounding <ul> after an
        // unclosed <li> should remain silent (mirrors Angular's reference parser).
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<ul><li>item</ul>", "test.html");
        let result = parser.parse();
        assert!(
            result.errors.is_empty(),
            "Implicitly closing optional-end-tag elements should not emit errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_parse_multiple_unclosed_inner_tags_reports_error() {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, "<section><div><span></section>", "test.html");
        let result = parser.parse();
        assert!(
            result.errors.iter().any(|e| e.msg.contains("Unexpected closing tag \"section\"")),
            "Should report unexpected closing tag when </section> skips past open <div>/<span>, got: {:?}",
            result.errors
        );
    }
}

//! I18n message parser.
//!
//! Converts HTML AST nodes to i18n Message AST.
//!
//! Ported from Angular's `i18n/i18n_parser.ts`.

use std::sync::Arc;

use indexmap::IndexMap;
use oxc_span::Span;
use rustc_hash::FxHashMap;

use crate::ast::html::{
    HtmlAttribute, HtmlBlock, HtmlComponent, HtmlElement, HtmlExpansion, HtmlNode, HtmlText,
    InterpolatedToken, InterpolatedTokenType,
};
use crate::i18n::ast::{
    BlockPlaceholder, Container, Icu, IcuPlaceholder, Message, MessagePlaceholder, Node,
    Placeholder, TagPlaceholder, Text,
};
use crate::i18n::placeholder::PlaceholderRegistry;
use crate::parser::html::is_void_element;
use crate::util::{ParseSourceFile, ParseSourceSpan};

/// Context for the i18n visitor.
#[derive(Debug)]
pub struct I18nVisitorContext {
    /// Whether the current context is inside an ICU expression.
    pub is_icu: bool,
    /// Current ICU nesting depth.
    pub icu_depth: u32,
    /// Placeholder name registry for generating unique names.
    pub placeholder_registry: PlaceholderRegistry,
    /// Map from placeholder name to content.
    pub placeholders: FxHashMap<String, MessagePlaceholder>,
    /// Map from placeholder name to nested message.
    pub placeholder_to_message: FxHashMap<String, Message>,
    /// Source file for span conversion.
    pub source_file: Arc<ParseSourceFile>,
}

impl I18nVisitorContext {
    fn new(source_file: Arc<ParseSourceFile>) -> Self {
        Self {
            is_icu: false,
            icu_depth: 0,
            placeholder_registry: PlaceholderRegistry::default(),
            placeholders: FxHashMap::default(),
            placeholder_to_message: FxHashMap::default(),
            source_file,
        }
    }
}

/// Callback for visiting nodes (for custom transformations).
pub type VisitNodeFn = fn(&HtmlNode<'_>, Node) -> Node;

/// Default no-op visitor function.
fn noop_visit_node(_: &HtmlNode<'_>, i18n: Node) -> Node {
    i18n
}

/// Factory for creating i18n messages from HTML nodes.
#[derive(Debug)]
pub struct I18nMessageFactory {
    /// Whether to retain empty tokens.
    retain_empty_tokens: bool,
    /// Whether to preserve expression whitespace.
    preserve_expression_whitespace: bool,
}

impl I18nMessageFactory {
    /// Creates a new message factory.
    pub fn new(retain_empty_tokens: bool, preserve_expression_whitespace: bool) -> Self {
        Self { retain_empty_tokens, preserve_expression_whitespace }
    }

    /// Converts HTML nodes to an i18n Message.
    pub fn create_message(
        &self,
        nodes: &[HtmlNode<'_>],
        meaning: Option<&str>,
        description: Option<&str>,
        custom_id: Option<&str>,
        visit_node_fn: Option<VisitNodeFn>,
        source_file: Arc<ParseSourceFile>,
    ) -> Message {
        let mut context = I18nVisitorContext::new(source_file);
        let visit_fn = visit_node_fn.unwrap_or(noop_visit_node);

        // Check if this is a single ICU expression
        context.is_icu = nodes.len() == 1 && matches!(nodes.first(), Some(HtmlNode::Expansion(_)));

        let i18n_nodes = self.visit_all(nodes, &mut context, visit_fn);

        Message::new(
            i18n_nodes,
            context.placeholders,
            context.placeholder_to_message,
            meaning.unwrap_or("").to_string(),
            description.unwrap_or("").to_string(),
            custom_id.unwrap_or("").to_string(),
        )
    }

    /// Visits all HTML nodes and converts them to i18n nodes.
    fn visit_all(
        &self,
        nodes: &[HtmlNode<'_>],
        context: &mut I18nVisitorContext,
        visit_fn: VisitNodeFn,
    ) -> Vec<Node> {
        let mut result = Vec::new();
        for node in nodes {
            if let Some(i18n_node) = self.visit_node(node, context, visit_fn) {
                result.push(i18n_node);
            }
        }
        result
    }

    /// Visits a single HTML node and converts it to an i18n node.
    fn visit_node(
        &self,
        node: &HtmlNode<'_>,
        context: &mut I18nVisitorContext,
        visit_fn: VisitNodeFn,
    ) -> Option<Node> {
        let i18n_node = match node {
            HtmlNode::Text(text) => self.visit_text(text, context),
            HtmlNode::Element(element) => self.visit_element(element, context, visit_fn),
            HtmlNode::Component(component) => self.visit_component(component, context, visit_fn),
            HtmlNode::Comment(_) => None, // Skip comments in i18n
            HtmlNode::Block(block) => self.visit_block(block, context, visit_fn),
            HtmlNode::Expansion(expansion) => self.visit_expansion(expansion, context, visit_fn),
            HtmlNode::Attribute(attr) => self.visit_attribute(attr, context),
            // Skip these in i18n processing
            HtmlNode::ExpansionCase(_)
            | HtmlNode::BlockParameter(_)
            | HtmlNode::LetDeclaration(_) => None,
        };

        i18n_node.map(|n| visit_fn(node, n))
    }

    /// Visits a text node and extracts interpolations.
    fn visit_text(&self, text: &HtmlText<'_>, context: &mut I18nVisitorContext) -> Option<Node> {
        let value = text.value.as_str();

        // Skip empty text unless configured to retain
        if value.trim().is_empty() && !self.retain_empty_tokens {
            return None;
        }

        // Check if text contains interpolations
        if value.contains("{{") && value.contains("}}") {
            return Some(self.visit_text_with_interpolation(value, text.span, context));
        }

        let source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            text.span.start,
            text.span.end,
            text.full_start,
            None,
        );
        Some(Node::Text(Text::new(value.to_string(), source_span)))
    }

    /// Visits an attribute node and extracts interpolations from its value.
    /// Angular's i18n_parser.ts: visitAttribute()
    fn visit_attribute(
        &self,
        attribute: &HtmlAttribute<'_>,
        context: &mut I18nVisitorContext,
    ) -> Option<Node> {
        // Use value_span if available, otherwise fall back to the whole attribute span
        let value_span = attribute.value_span.unwrap_or(attribute.span);
        let source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            value_span.start,
            value_span.end,
            None,
            None,
        );

        // Check if attribute has value tokens (for interpolation processing)
        match &attribute.value_tokens {
            None => {
                // No tokens - return simple text node
                Some(Node::Text(Text::new(attribute.value.to_string(), source_span)))
            }
            Some(tokens) if tokens.is_empty() || tokens.len() == 1 => {
                // Empty or single token - return simple text node
                Some(Node::Text(Text::new(attribute.value.to_string(), source_span)))
            }
            Some(tokens) => {
                // Multiple tokens - process with interpolation
                Some(self.visit_text_with_interpolation_tokens(tokens, value_span, context))
            }
        }
    }

    /// Visits text with interpolations and creates Text and Placeholder nodes.
    fn visit_text_with_interpolation(
        &self,
        text: &str,
        span: Span,
        context: &mut I18nVisitorContext,
    ) -> Node {
        let mut nodes: Vec<Node> = Vec::new();
        let mut current_pos = 0;
        let mut has_interpolation = false;

        let text_span =
            ParseSourceSpan::from_offsets(&context.source_file, span.start, span.end, None, None);

        // Find all interpolations {{ expr }}
        while let Some(start) = text[current_pos..].find("{{") {
            let abs_start = current_pos + start;

            // Add text before interpolation
            if start > 0 {
                let text_before = &text[current_pos..abs_start];
                if !text_before.is_empty() || self.retain_empty_tokens {
                    // Try to merge with previous text node
                    if let Some(Node::Text(prev)) = nodes.last_mut() {
                        prev.value.push_str(text_before);
                    } else {
                        nodes.push(Node::Text(Text::new(
                            text_before.to_string(),
                            text_span.clone(),
                        )));
                    }
                }
            }

            // Find the closing }}
            if let Some(end) = text[abs_start + 2..].find("}}") {
                let abs_end = abs_start + 2 + end;
                let expr = text[abs_start + 2..abs_end].trim();

                if !expr.is_empty() {
                    has_interpolation = true;

                    // Extract placeholder name from expression or use default
                    let custom_name = extract_placeholder_name(expr);
                    let base_name = custom_name.as_deref().unwrap_or("INTERPOLATION");
                    let ph_name =
                        context.placeholder_registry.get_placeholder_name(base_name, expr);

                    // Record placeholder content
                    let placeholder_text = if self.preserve_expression_whitespace {
                        text[abs_start..abs_end + 2].to_string()
                    } else {
                        format!("{{{{{}}}}}", expr)
                    };

                    context.placeholders.insert(
                        ph_name.clone(),
                        MessagePlaceholder {
                            text: placeholder_text,
                            source_span: text_span.clone(),
                        },
                    );

                    nodes.push(Node::Placeholder(Placeholder::new(
                        expr.to_string(),
                        ph_name,
                        text_span.clone(),
                    )));
                }

                current_pos = abs_end + 2;
            } else {
                // No closing }}, treat as text
                break;
            }
        }

        // Add remaining text after last interpolation
        if current_pos < text.len() {
            let remaining = &text[current_pos..];
            if !remaining.is_empty() || self.retain_empty_tokens {
                // Try to merge with previous text node
                if let Some(Node::Text(prev)) = nodes.last_mut() {
                    prev.value.push_str(remaining);
                } else {
                    nodes.push(Node::Text(Text::new(remaining.to_string(), text_span.clone())));
                }
            }
        }

        // If we found interpolations, return a Container; otherwise return single node
        if has_interpolation && nodes.len() > 1 {
            Node::Container(Container::new(nodes, text_span))
        } else if let Some(node) = nodes.pop() {
            node
        } else {
            Node::Text(Text::new(text.to_string(), text_span))
        }
    }

    /// Visits text with interpolations using pre-parsed tokens (from HtmlAttribute.value_tokens).
    /// This method processes the token sequence directly rather than scanning for {{ }}.
    /// Angular's i18n_parser.ts: _visitTextWithInterpolation()
    fn visit_text_with_interpolation_tokens(
        &self,
        tokens: &[InterpolatedToken<'_>],
        source_span: Span,
        context: &mut I18nVisitorContext,
    ) -> Node {
        let mut nodes: Vec<Node> = Vec::new();
        let mut has_interpolation = false;

        let overall_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            source_span.start,
            source_span.end,
            None,
            None,
        );

        for token in tokens {
            let token_span = ParseSourceSpan::from_offsets(
                &context.source_file,
                token.span.start,
                token.span.end,
                None,
                None,
            );

            match token.token_type {
                InterpolatedTokenType::Interpolation => {
                    has_interpolation = true;

                    // Parts: [startMarker, expression, endMarker]
                    let start_marker = token.parts.first().map(|a| a.as_str()).unwrap_or("{{");
                    let expression = token.parts.get(1).map(|a| a.as_str()).unwrap_or("");
                    let end_marker = token.parts.get(2).map(|a| a.as_str()).unwrap_or("}}");

                    // Normalize expression if not preserving whitespace
                    let normalized = if self.preserve_expression_whitespace {
                        expression.to_string()
                    } else {
                        expression.trim().to_string()
                    };

                    // Extract placeholder name from expression or use default
                    let custom_name = extract_placeholder_name(&normalized);
                    let base_name = custom_name.as_deref().unwrap_or("INTERPOLATION");
                    let ph_name =
                        context.placeholder_registry.get_placeholder_name(base_name, &normalized);

                    // Record placeholder content
                    let placeholder_text = if self.preserve_expression_whitespace {
                        token.parts.iter().map(|p| p.as_str()).collect::<String>()
                    } else {
                        format!("{}{}{}", start_marker, normalized, end_marker)
                    };

                    context.placeholders.insert(
                        ph_name.clone(),
                        MessagePlaceholder {
                            text: placeholder_text,
                            source_span: token_span.clone(),
                        },
                    );

                    nodes
                        .push(Node::Placeholder(Placeholder::new(normalized, ph_name, token_span)));
                }
                InterpolatedTokenType::Text | InterpolatedTokenType::EncodedEntity => {
                    // Get the text content (first part of the token)
                    let text_value = token.parts.first().map(|a| a.as_str()).unwrap_or("");

                    // Check if we should include this token
                    if !text_value.is_empty() || self.retain_empty_tokens {
                        // Try to merge with previous text node
                        if let Some(Node::Text(prev)) = nodes.last_mut() {
                            prev.value.push_str(text_value);
                            // Extend source span to include this token
                            prev.source_span = ParseSourceSpan::from_offsets(
                                &context.source_file,
                                prev.source_span.start.offset,
                                token.span.end,
                                None,
                                None,
                            );
                        } else {
                            nodes.push(Node::Text(Text::new(text_value.to_string(), token_span)));
                        }
                    } else if self.retain_empty_tokens {
                        // Retain empty tokens for consistent node counts between passes
                        nodes.push(Node::Text(Text::new(text_value.to_string(), token_span)));
                    }
                }
            }
        }

        // Return result based on what we found
        if has_interpolation && nodes.len() > 1 {
            Node::Container(Container::new(nodes, overall_span))
        } else if nodes.len() == 1 {
            nodes
                .into_iter()
                .next()
                .unwrap_or_else(|| Node::Text(Text::new(String::new(), overall_span)))
        } else if nodes.is_empty() {
            Node::Text(Text::new(String::new(), overall_span))
        } else {
            Node::Container(Container::new(nodes, overall_span))
        }
    }

    /// Visits an ICU expansion node.
    fn visit_expansion(
        &self,
        expansion: &HtmlExpansion<'_>,
        context: &mut I18nVisitorContext,
        visit_fn: VisitNodeFn,
    ) -> Option<Node> {
        context.icu_depth += 1;

        // Build ICU cases (ordered for consistent serialization)
        let mut cases: IndexMap<String, Node> = IndexMap::default();
        for case in &expansion.cases {
            let mut case_nodes = self.visit_all(&case.expansion, context, visit_fn);
            let case_node = if case_nodes.len() == 1 {
                case_nodes.pop().unwrap_or_else(|| {
                    let case_span = ParseSourceSpan::from_offsets(
                        &context.source_file,
                        case.expansion_span.start,
                        case.expansion_span.end,
                        None,
                        None,
                    );
                    Node::Container(Container::new(Vec::new(), case_span))
                })
            } else {
                let case_span = ParseSourceSpan::from_offsets(
                    &context.source_file,
                    case.expansion_span.start,
                    case.expansion_span.end,
                    None,
                    None,
                );
                Node::Container(Container::new(case_nodes, case_span))
            };
            cases.insert(case.value.to_string(), case_node);
        }

        // Store the ICU type string as-is (for placeholder naming and serialization)
        let icu_type = expansion.expansion_type.to_string();

        // Create the ICU node
        let icu_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            expansion.span.start,
            expansion.span.end,
            None,
            None,
        );
        let mut icu = Icu::new(
            expansion.switch_value.to_string(),
            icu_type.clone(),
            cases,
            icu_span.clone(),
            None,
        );

        context.icu_depth -= 1;

        // Determine if this should be an ICU or IcuPlaceholder
        if context.is_icu || context.icu_depth > 0 {
            // Returns an ICU node when:
            // - the message (vs a part of the message) is an ICU message, or
            // - the ICU message is nested.
            let exp_ph = context
                .placeholder_registry
                .get_unique_placeholder(&format!("VAR_{}", icu_type.to_uppercase()));
            icu.expression_placeholder = Some(exp_ph.clone());
            context.placeholders.insert(
                exp_ph,
                MessagePlaceholder {
                    text: expansion.switch_value.to_string(),
                    source_span: ParseSourceSpan::from_offsets(
                        &context.source_file,
                        expansion.switch_value_span.start,
                        expansion.switch_value_span.end,
                        None,
                        None,
                    ),
                },
            );
            Some(Node::Icu(icu))
        } else {
            // Returns a placeholder for top-level ICU messages
            // ICU placeholders should not be replaced with their original content
            // but with their translations.
            let ph_name =
                context.placeholder_registry.get_placeholder_name("ICU", &icu_span.to_string());

            // Create a nested message containing the ICU node
            // We clone the ICU and create a message with it set as isIcu=true
            let mut nested_icu = icu.clone();
            let exp_ph = context
                .placeholder_registry
                .get_unique_placeholder(&format!("VAR_{}", icu_type.to_uppercase()));
            nested_icu.expression_placeholder = Some(exp_ph.clone());

            let mut nested_placeholders = FxHashMap::default();
            nested_placeholders.insert(
                exp_ph,
                MessagePlaceholder {
                    text: expansion.switch_value.to_string(),
                    source_span: ParseSourceSpan::from_offsets(
                        &context.source_file,
                        expansion.switch_value_span.start,
                        expansion.switch_value_span.end,
                        None,
                        None,
                    ),
                },
            );

            let nested_message = Message::new(
                vec![Node::Icu(nested_icu)],
                nested_placeholders,
                FxHashMap::default(),
                String::new(),
                String::new(),
                String::new(),
            );
            context.placeholder_to_message.insert(ph_name.clone(), nested_message);

            Some(Node::IcuPlaceholder(IcuPlaceholder::new(icu, ph_name, icu_span)))
        }
    }

    /// Visits an element node.
    fn visit_element(
        &self,
        element: &HtmlElement<'_>,
        context: &mut I18nVisitorContext,
        visit_fn: VisitNodeFn,
    ) -> Option<Node> {
        let tag_name = element.name.as_str();
        let is_void = is_void_element(tag_name);

        // Convert element attributes to an IndexMap for placeholder registry (ordered for consistent serialization)
        let mut attrs: IndexMap<String, String> = element
            .attrs
            .iter()
            .map(|attr| (attr.name.to_string(), attr.value.to_string()))
            .collect();
        // Include selectorless directive attrs in the signature (Angular behavior).
        for directive in &element.directives {
            for attr in &directive.attrs {
                attrs.insert(attr.name.to_string(), attr.value.to_string());
            }
        }

        // Generate placeholder names for the tag
        let start_name =
            context.placeholder_registry.get_start_tag_placeholder_name(tag_name, &attrs, is_void);
        let close_name = if is_void {
            String::new()
        } else {
            context.placeholder_registry.get_close_tag_placeholder_name(tag_name)
        };

        // Visit children
        let children = self.visit_all(&element.children, context, visit_fn);

        let source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            element.span.start,
            element.span.end,
            None,
            None,
        );
        let start_source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            element.start_span.start,
            element.start_span.end,
            None,
            None,
        );
        let end_source_span = element.end_span.map(|span| {
            ParseSourceSpan::from_offsets(&context.source_file, span.start, span.end, None, None)
        });
        let end_source_span_for_placeholder = end_source_span.clone();

        // Create the tag placeholder
        let placeholder = TagPlaceholder::new(
            tag_name.to_string(),
            attrs.clone(),
            start_name.clone(),
            close_name.clone(),
            children,
            is_void,
            source_span.clone(),
            Some(start_source_span.clone()),
            end_source_span_for_placeholder,
        );

        // Record the placeholder content
        context.placeholders.insert(
            start_name.clone(),
            MessagePlaceholder {
                text: start_source_span.to_string(),
                source_span: start_source_span.clone(),
            },
        );
        if !is_void {
            let close_span = end_source_span.clone().unwrap_or_else(|| source_span.clone());
            context.placeholders.insert(
                close_name,
                MessagePlaceholder { text: format!("</{tag_name}>"), source_span: close_span },
            );
        }

        Some(Node::TagPlaceholder(placeholder))
    }

    /// Visits a component node.
    /// Components are handled similarly to elements but use the component name for placeholders.
    fn visit_component(
        &self,
        component: &HtmlComponent<'_>,
        context: &mut I18nVisitorContext,
        visit_fn: VisitNodeFn,
    ) -> Option<Node> {
        // Use full_name (e.g., "MyComp" or "MyComp:button") as the tag name
        let tag_name = component.full_name.as_str();

        // Convert component attributes to an IndexMap for placeholder registry
        let mut attrs: IndexMap<String, String> = component
            .attrs
            .iter()
            .map(|attr| (attr.name.to_string(), attr.value.to_string()))
            .collect();
        // Include selectorless directive attrs in the signature
        for directive in &component.directives {
            for attr in &directive.attrs {
                attrs.insert(attr.name.to_string(), attr.value.to_string());
            }
        }

        // Components are never void
        let is_void = false;

        // Generate placeholder names for the tag
        let start_name =
            context.placeholder_registry.get_start_tag_placeholder_name(tag_name, &attrs, is_void);
        let close_name = context.placeholder_registry.get_close_tag_placeholder_name(tag_name);

        // Visit children
        let children = self.visit_all(&component.children, context, visit_fn);

        let source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            component.span.start,
            component.span.end,
            None,
            None,
        );
        let start_source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            component.start_span.start,
            component.start_span.end,
            None,
            None,
        );
        let end_source_span = component.end_span.map(|span| {
            ParseSourceSpan::from_offsets(&context.source_file, span.start, span.end, None, None)
        });
        let end_source_span_for_placeholder = end_source_span.clone();

        // Create the tag placeholder
        let placeholder = TagPlaceholder::new(
            tag_name.to_string(),
            attrs.clone(),
            start_name.clone(),
            close_name.clone(),
            children,
            is_void,
            source_span.clone(),
            Some(start_source_span.clone()),
            end_source_span_for_placeholder,
        );

        // Record the placeholder content
        context.placeholders.insert(
            start_name.clone(),
            MessagePlaceholder {
                text: start_source_span.to_string(),
                source_span: start_source_span.clone(),
            },
        );
        let close_span = end_source_span.clone().unwrap_or_else(|| source_span.clone());
        context.placeholders.insert(
            close_name,
            MessagePlaceholder { text: format!("</{tag_name}>"), source_span: close_span },
        );

        Some(Node::TagPlaceholder(placeholder))
    }

    /// Visits a block node.
    fn visit_block(
        &self,
        block: &HtmlBlock<'_>,
        context: &mut I18nVisitorContext,
        visit_fn: VisitNodeFn,
    ) -> Option<Node> {
        let block_name = block.name.as_str();

        // Visit children
        let children = self.visit_all(&block.children, context, visit_fn);

        let source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            block.span.start,
            block.span.end,
            None,
            None,
        );
        let start_source_span = ParseSourceSpan::from_offsets(
            &context.source_file,
            block.start_span.start,
            block.start_span.end,
            None,
            None,
        );
        let end_source_span = block.end_span.map(|span| {
            ParseSourceSpan::from_offsets(&context.source_file, span.start, span.end, None, None)
        });
        let end_source_span_for_placeholder = end_source_span.clone();

        // @switch is not represented as a placeholder in Angular.
        if block_name == "switch" {
            return Some(Node::Container(Container::new(children, source_span)));
        }

        // Collect parameters as strings
        let parameters: Vec<String> =
            block.parameters.iter().map(|p| p.expression.to_string()).collect();

        // Generate placeholder names for the block
        let start_name =
            context.placeholder_registry.get_start_block_placeholder_name(block_name, &parameters);
        let close_name = context.placeholder_registry.get_close_block_placeholder_name(block_name);

        // Create the block placeholder
        let placeholder = BlockPlaceholder::new(
            block_name.to_string(),
            parameters,
            start_name.clone(),
            close_name.clone(),
            children,
            source_span.clone(),
            Some(start_source_span.clone()),
            end_source_span_for_placeholder,
        );

        // Record the placeholder content
        context.placeholders.insert(
            start_name.clone(),
            MessagePlaceholder {
                text: start_source_span.to_string(),
                source_span: start_source_span.clone(),
            },
        );
        let close_span = end_source_span.clone().unwrap_or_else(|| source_span.clone());
        context.placeholders.insert(
            close_name,
            MessagePlaceholder {
                text: end_source_span.map(|s| s.to_string()).unwrap_or_else(|| "}".to_string()),
                source_span: close_span,
            },
        );

        Some(Node::BlockPlaceholder(placeholder))
    }
}

/// Creates a new i18n message factory.
pub fn create_i18n_message_factory(
    retain_empty_tokens: bool,
    preserve_expression_whitespace: bool,
) -> I18nMessageFactory {
    I18nMessageFactory::new(retain_empty_tokens, preserve_expression_whitespace)
}

/// Extracts a custom placeholder name from an expression if present.
/// Looks for comments like `// i18n(ph="CUSTOM_NAME")` in the expression.
///
/// Supported formats:
/// - `/* i18n(ph="NAME") */` - block comment format
/// - `// i18n(ph="NAME")` - line comment format (at the end)
///
/// Returns `Some(name)` if a custom placeholder name is found, `None` otherwise.
fn extract_placeholder_name(expression: &str) -> Option<String> {
    // Look for block comment format: /* i18n(ph="NAME") */
    if let Some(start) = expression.find("i18n(ph=") {
        let rest = &expression[start + 8..]; // Skip "i18n(ph="

        // Determine quote type (single or double)
        let (quote, rest) = if rest.starts_with('"') {
            ('"', &rest[1..])
        } else if rest.starts_with('\'') {
            ('\'', &rest[1..])
        } else {
            return None;
        };

        // Find the closing quote
        if let Some(end) = rest.find(quote) {
            let name = &rest[..end];
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::html::HtmlText;
    use crate::util::ParseSourceFile;
    use oxc_allocator::{Allocator, Box, Vec as AllocVec};
    use oxc_str::Ident;
    use std::sync::Arc;

    #[test]
    fn test_create_simple_message() {
        let factory = create_i18n_message_factory(false, false);
        let source_file = Arc::new(ParseSourceFile::new("", "<test>"));
        let message = factory.create_message(&[], None, None, None, None, source_file);
        assert!(message.nodes.is_empty());
    }

    #[test]
    fn test_factory_options() {
        let factory = I18nMessageFactory::new(true, true);
        assert!(factory.retain_empty_tokens);
        assert!(factory.preserve_expression_whitespace);
    }

    #[test]
    fn test_text_with_interpolation() {
        let allocator = Allocator::default();
        let factory = create_i18n_message_factory(false, false);

        let text = HtmlText {
            value: Ident::from("Hello {{name}}!"),
            span: Span::default(),
            full_start: None,
            tokens: AllocVec::new_in(&allocator),
        };

        let nodes = vec![HtmlNode::Text(Box::new_in(text, &allocator))];
        let source_file = Arc::new(ParseSourceFile::new("Hello {{name}}!", "<test>"));
        let message = factory.create_message(&nodes, None, None, None, None, source_file);

        // Should have one Container with Text, Placeholder, Text inside
        assert_eq!(message.nodes.len(), 1);
        if let Node::Container(container) = &message.nodes[0] {
            assert_eq!(container.children.len(), 3);
            assert!(matches!(&container.children[0], Node::Text(t) if t.value == "Hello "));
            assert!(matches!(&container.children[1], Node::Placeholder(p) if p.value == "name"));
            assert!(matches!(&container.children[2], Node::Text(t) if t.value == "!"));
        } else {
            panic!("Expected Container node");
        }

        // Should have placeholder registered
        assert!(message.placeholders.contains_key("INTERPOLATION"));
    }

    #[test]
    fn test_text_without_interpolation() {
        let allocator = Allocator::default();
        let factory = create_i18n_message_factory(false, false);

        let text = HtmlText {
            value: Ident::from("Hello World"),
            span: Span::default(),
            full_start: None,
            tokens: AllocVec::new_in(&allocator),
        };

        let nodes = vec![HtmlNode::Text(Box::new_in(text, &allocator))];
        let source_file = Arc::new(ParseSourceFile::new("Hello World", "<test>"));
        let message = factory.create_message(&nodes, None, None, None, None, source_file);

        // Should have one Text node
        assert_eq!(message.nodes.len(), 1);
        assert!(matches!(&message.nodes[0], Node::Text(t) if t.value == "Hello World"));
    }

    #[test]
    fn test_multiple_interpolations() {
        let allocator = Allocator::default();
        let factory = create_i18n_message_factory(false, false);

        let text = HtmlText {
            value: Ident::from("{{greeting}} {{name}}!"),
            span: Span::default(),
            full_start: None,
            tokens: AllocVec::new_in(&allocator),
        };

        let nodes = vec![HtmlNode::Text(Box::new_in(text, &allocator))];
        let source_file = Arc::new(ParseSourceFile::new("{{greeting}} {{name}}!", "<test>"));
        let message = factory.create_message(&nodes, None, None, None, None, source_file);

        // Should have Container with multiple placeholders
        assert_eq!(message.nodes.len(), 1);
        if let Node::Container(container) = &message.nodes[0] {
            // " " between placeholders might be merged
            assert!(container.children.len() >= 2);
            // Check we have at least 2 placeholders
            let placeholder_count =
                container.children.iter().filter(|n| matches!(n, Node::Placeholder(_))).count();
            assert_eq!(placeholder_count, 2);
        } else {
            panic!("Expected Container node");
        }
    }
}

//! HTML AST to R3 AST transformation.
//!
//! This module transforms parsed HTML template AST nodes into R3 AST nodes,
//! which are the intermediate representation used by the IR pipeline.
//!
//! Ported from Angular's `render3/r3_template_transform.ts`.

use oxc_allocator::{Allocator, Box, FromIn, HashMap, Vec};
use oxc_span::{Atom, Span};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast::expression::{
    ASTWithSource, AbsoluteSourceSpan, AngularExpression, BindingType, ParseSpan, ParsedEventType,
};
use crate::ast::html::{
    BlockType, HtmlAttribute, HtmlBlock, HtmlComponent, HtmlDirective, HtmlElement, HtmlExpansion,
    HtmlNode, HtmlText, InterpolatedTokenType,
};
use crate::ast::r3::{
    I18nBlockPlaceholder, I18nContainer, I18nIcu, I18nIcuPlaceholder, I18nMessage, I18nMeta,
    I18nNode, I18nPlaceholder, I18nTagPlaceholder, I18nText, R3BoundAttribute, R3BoundEvent,
    R3BoundText, R3Comment, R3Component, R3Content, R3DeferredBlock, R3Directive, R3Element,
    R3ForLoopBlock, R3Icu, R3IcuPlaceholder, R3IfBlock, R3IfBlockBranch, R3LetDeclaration, R3Node,
    R3ParseResult, R3Reference, R3SwitchBlock, R3Template, R3TemplateAttr, R3Text, R3TextAttribute,
    R3Variable, SecurityContext, serialize_i18n_nodes,
};
use crate::i18n::parser::I18nMessageFactory;
use crate::i18n::placeholder::PlaceholderRegistry;
use crate::parser::expression::{BindingParser, find_comment_start};
use crate::parser::html::decode_entities_in_string;
use crate::schema::get_security_context;
use crate::transform::control_flow::{parse_conditional_params, parse_defer_triggers};
use crate::util::{ParseError, parse_loading_parameters, parse_placeholder_parameters};

/// Regex pattern for binding prefixes.
/// Matches: bind-, let-, ref-/#, on-, bindon-, @
const BIND_NAME_PREFIXES: &[(&str, BindingPrefix)] = &[
    ("bind-", BindingPrefix::Bind),
    ("let-", BindingPrefix::Let),
    ("ref-", BindingPrefix::Ref),
    ("#", BindingPrefix::Ref),
    ("on-", BindingPrefix::On),
    ("bindon-", BindingPrefix::BindOn),
    ("@", BindingPrefix::At),
];

/// Binding delimiters for Angular syntax.
const BANANA_BOX_START: &str = "[(";
const BANANA_BOX_END: &str = ")]";
const PROPERTY_START: char = '[';
const PROPERTY_END: char = ']';
const EVENT_START: char = '(';
const EVENT_END: char = ')';

/// Template attribute prefix for structural directives.
const TEMPLATE_ATTR_PREFIX: char = '*';

/// Tags that cannot be used as selectorless component tags.
/// These are special HTML or Angular elements that have reserved semantics.
const UNSUPPORTED_SELECTORLESS_TAGS: &[&str] =
    &["link", "style", "script", "ng-template", "ng-container", "ng-content"];

/// Binding prefix types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingPrefix {
    /// Property binding (bind-)
    Bind,
    /// Let variable (let-)
    Let,
    /// Template reference (ref- or #)
    Ref,
    /// Event binding (on-)
    On,
    /// Two-way binding (bindon-)
    BindOn,
    /// Animation (@)
    At,
}

/// Info extracted from a template attribute (structural directive).
#[derive(Debug, Clone)]
struct TemplateAttrInfo<'a> {
    /// The attribute name (e.g., `*ngFor`).
    name: Atom<'a>,
    /// The attribute value (e.g., `let item of items`).
    value: Atom<'a>,
    /// The full span of the attribute.
    span: Span,
    /// The span of the attribute name.
    name_span: Span,
    /// The span of the attribute value (if present).
    value_span: Option<Span>,
}

/// Options for the HTML to R3 transformation.
#[derive(Debug, Clone, Default)]
pub struct TransformOptions {
    /// Whether to collect comment nodes.
    pub collect_comment_nodes: bool,
}

/// Inserts or updates a var entry in an ordered Vec, preserving first-insertion order.
/// This matches JS object semantics where reassigning an existing key keeps its position.
fn ordered_insert_var<'a>(
    vec: &mut Vec<'a, (Atom<'a>, R3BoundText<'a>)>,
    key: Atom<'a>,
    value: R3BoundText<'a>,
) {
    if let Some(existing) = vec.iter_mut().find(|(k, _)| *k == key) {
        existing.1 = value;
    } else {
        vec.push((key, value));
    }
}

/// Inserts or updates a placeholder entry in an ordered Vec, preserving first-insertion order.
/// This matches JS object semantics where reassigning an existing key keeps its position.
fn ordered_insert_placeholder<'a>(
    vec: &mut Vec<'a, (Atom<'a>, R3IcuPlaceholder<'a>)>,
    key: Atom<'a>,
    value: R3IcuPlaceholder<'a>,
) {
    if let Some(existing) = vec.iter_mut().find(|(k, _)| *k == key) {
        existing.1 = value;
    } else {
        vec.push((key, value));
    }
}

/// Transforms HTML AST to R3 AST.
pub struct HtmlToR3Transform<'a> {
    allocator: &'a Allocator,
    /// The source text of the template (for extracting attribute values).
    source_text: &'a str,
    /// Binding parser for parsing expressions.
    binding_parser: BindingParser<'a>,
    /// Parse errors.
    /// Uses std::vec::Vec since ParseError contains Drop types (Arc, String).
    errors: std::vec::Vec<ParseError>,
    styles: Vec<'a, Atom<'a>>,
    style_urls: Vec<'a, Atom<'a>>,
    ng_content_selectors: Vec<'a, Atom<'a>>,
    comment_nodes: Option<Vec<'a, R3Comment<'a>>>,
    processed_nodes: FxHashSet<usize>,
    namespace_stack: std::vec::Vec<ElementNamespace>,
    /// Depth counter for ngNonBindable. When > 0, bindings are suppressed.
    non_bindable_depth: u32,
    /// Depth counter for i18n context. When > 0, ICU expansions are emitted.
    i18n_depth: u32,
    /// Counter for generating unique block placeholder names (for i18n).
    block_placeholder_counter: u32,
    /// Counters for generating unique ICU placeholder names (e.g., VAR_PLURAL, VAR_PLURAL_1, etc.).
    /// Maps base name (e.g., "VAR_PLURAL") to count.
    icu_placeholder_counts: FxHashMap<String, u32>,
    /// Placeholder registry for generating unique tag placeholder names within i18n blocks.
    /// Reset when entering a new i18n block.
    i18n_placeholder_registry: PlaceholderRegistry,
    /// Counter for generating unique i18n message instance IDs.
    ///
    /// Each i18n message gets a unique instance ID that's used to track message identity
    /// across moves/copies. This ensures that when the same attribute is processed twice
    /// (e.g., by `ingestControlFlowInsertionPoint` and `ingestStaticAttributes`), both
    /// can share the same i18n context.
    i18n_message_instance_counter: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementNamespace {
    Html,
    Svg,
    Math,
}

impl<'a> HtmlToR3Transform<'a> {
    /// Creates a new HTML to R3 transformer.
    pub fn new(allocator: &'a Allocator, source_text: &'a str, options: TransformOptions) -> Self {
        let comment_nodes =
            if options.collect_comment_nodes { Some(Vec::new_in(allocator)) } else { None };

        Self {
            allocator,
            source_text,
            binding_parser: BindingParser::new(allocator),
            errors: std::vec::Vec::new(),
            styles: Vec::new_in(allocator),
            style_urls: Vec::new_in(allocator),
            ng_content_selectors: Vec::new_in(allocator),
            comment_nodes,
            processed_nodes: FxHashSet::default(),
            namespace_stack: std::vec::Vec::new(),
            non_bindable_depth: 0,
            i18n_depth: 0,
            block_placeholder_counter: 0,
            icu_placeholder_counts: FxHashMap::default(),
            i18n_placeholder_registry: PlaceholderRegistry::new(),
            i18n_message_instance_counter: 0,
        }
    }

    /// Allocates a new unique instance ID for an i18n message.
    fn allocate_i18n_message_instance_id(&mut self) -> u32 {
        let id = self.i18n_message_instance_counter;
        self.i18n_message_instance_counter += 1;
        id
    }

    /// Transforms HTML nodes to R3 nodes.
    pub fn transform(mut self, html_nodes: &[HtmlNode<'a>]) -> R3ParseResult<'a> {
        let nodes = self.visit_siblings(html_nodes);

        R3ParseResult {
            nodes,
            errors: self.errors,
            styles: self.styles,
            style_urls: self.style_urls,
            ng_content_selectors: self.ng_content_selectors,
            comment_nodes: self.comment_nodes,
        }
    }

    /// Visits a list of sibling nodes, handling connected blocks.
    fn visit_siblings(&mut self, siblings: &[HtmlNode<'a>]) -> Vec<'a, R3Node<'a>> {
        let mut result = Vec::new_in(self.allocator);

        for (index, node) in siblings.iter().enumerate() {
            // Skip nodes that were already processed as connected blocks
            let node_id = node as *const _ as usize;
            if self.processed_nodes.contains(&node_id) {
                continue;
            }

            // In ngNonBindable context, blocks should be flattened into text nodes + children
            // (not wrapped in a Template). TypeScript returns [startText, ...children, endText].flat()
            if self.non_bindable_depth > 0 {
                if let HtmlNode::Block(block) = node {
                    let block_nodes = self.visit_block_as_text_flat(block);
                    for child in block_nodes {
                        result.push(child);
                    }
                    continue;
                }
            }

            if let Some(r3_node) = self.visit_node_with_siblings(node, index, siblings) {
                result.push(r3_node);
            }
        }

        result
    }

    /// Visits a node with sibling context for connected block handling.
    fn visit_node_with_siblings(
        &mut self,
        node: &HtmlNode<'a>,
        index: usize,
        siblings: &[HtmlNode<'a>],
    ) -> Option<R3Node<'a>> {
        match node {
            HtmlNode::Block(block) => self.visit_block_with_siblings(block, index, siblings),
            _ => self.visit_node(node),
        }
    }

    /// Visits an HTML node and returns the corresponding R3 node.
    fn visit_node(&mut self, node: &HtmlNode<'a>) -> Option<R3Node<'a>> {
        match node {
            HtmlNode::Element(element) => self.visit_element(element),
            HtmlNode::Component(component) => self.visit_html_component(component),
            HtmlNode::Text(text) => self.visit_text(text),
            HtmlNode::Comment(comment) => self.visit_comment(comment),
            HtmlNode::Block(block) => self.visit_block(block),
            HtmlNode::LetDeclaration(decl) => {
                // Inside ngNonBindable, @let becomes plain text
                // Angular reconstructs: `@let ${name} = ${value};`
                if self.non_bindable_depth > 0 {
                    // Get the raw value text from source
                    let value_text = if decl.value_span.start < decl.value_span.end
                        && (decl.value_span.end as usize) <= self.source_text.len()
                    {
                        &self.source_text
                            [decl.value_span.start as usize..decl.value_span.end as usize]
                    } else {
                        ""
                    };
                    // Reconstruct the @let text with semicolon
                    let reconstructed = format!("@let {} = {};", decl.name.as_str(), value_text);
                    let text_value = Atom::from_in(reconstructed.as_str(), self.allocator);
                    let r3_text = R3Text { value: text_value, source_span: decl.span };
                    return Some(R3Node::Text(Box::new_in(r3_text, self.allocator)));
                }
                self.visit_let_declaration(decl)
            }
            HtmlNode::Expansion(expansion) => self.visit_expansion(expansion),
            HtmlNode::ExpansionCase(_) => {
                // Expansion cases are handled within visit_expansion
                None
            }
            HtmlNode::Attribute(_) | HtmlNode::BlockParameter(_) => {
                // These are not standalone nodes in R3 AST
                None
            }
        }
    }

    /// Visits an HTML element.
    fn visit_element(&mut self, element: &HtmlElement<'a>) -> Option<R3Node<'a>> {
        let raw_name = element.name.as_str();

        // Check for special elements
        if raw_name == "script" {
            return None;
        }
        if raw_name == "style" {
            // Extract style content
            if let Some(content) = self.get_text_content(element) {
                self.styles.push(content);
            }
            return None;
        }
        if raw_name == "link" {
            // Collect stylesheet URLs
            if let Some(href) = self.get_stylesheet_href(element) {
                self.style_urls.push(href);
            }
            // Filter out <link rel="stylesheet"> inside ngNonBindable elements
            if self.non_bindable_depth > 0 && self.is_stylesheet_link(element) {
                return None;
            }
        }

        // Parse attributes
        let (attributes, inputs, outputs, references, variables, template_attr) =
            self.parse_attributes(&element.attrs, raw_name, raw_name == "ng-template");

        // Resolve namespace for this element and its children.
        // Note: foreignObject is an SVG element but its children use HTML namespace.
        // We need to distinguish between the element's own namespace (for naming) and
        // the namespace for its children (pushed to stack).
        let parent_namespace = self.current_namespace();
        let child_namespace = self.resolve_namespace(raw_name, parent_namespace);

        // For foreignObject in SVG context: the element itself is SVG, only children are HTML.
        // For all other elements: element namespace equals child namespace.
        let element_namespace = if parent_namespace == ElementNamespace::Svg
            && raw_name.eq_ignore_ascii_case("foreignObject")
        {
            ElementNamespace::Svg
        } else {
            child_namespace
        };
        self.namespace_stack.push(child_namespace);

        // Check if element has ngNonBindable attribute
        let has_non_bindable =
            element.attrs.iter().any(|attr| attr.name.as_str() == "ngNonBindable");

        // Check if element has i18n attribute (marks ICU expansions for translation)
        // and extract its metadata for the i18n block generation
        let i18n_attr = element.attrs.iter().find(|attr| attr.name.as_str() == "i18n");
        let has_i18n = i18n_attr.is_some();

        // Generate i18n message string from HTML children BEFORE transforming to R3.
        // This is needed because the message text comes from the original HTML content.
        // Ported from Angular's I18nMetaVisitor._generateI18nMessage in i18n/meta.ts.
        let i18n_message_string = if let Some(attr) = i18n_attr {
            if !element.children.is_empty() {
                // Use I18nMessageFactory to convert children to i18n AST and serialize
                let factory = I18nMessageFactory::new(false, true);
                let source_file = std::sync::Arc::new(crate::util::ParseSourceFile::new(
                    self.source_text,
                    "<template>",
                ));
                let meaning = if attr.value.contains('|') {
                    Some(attr.value.split('|').next().unwrap_or("").trim())
                } else {
                    None
                };
                let description = if attr.value.contains('|') {
                    attr.value.split('|').nth(1).map(|s| {
                        if let Some(pos) = s.find("@@") { s[..pos].trim() } else { s.trim() }
                    })
                } else if let Some(pos) = attr.value.find("@@") {
                    Some(attr.value[..pos].trim())
                } else if !attr.value.is_empty() {
                    Some(attr.value.as_str())
                } else {
                    None
                };
                let custom_id = attr.value.find("@@").map(|pos| &attr.value.as_str()[pos + 2..]);

                let message = factory.create_message(
                    &element.children,
                    meaning,
                    description,
                    custom_id,
                    None,
                    source_file,
                );
                message.serialize()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Now create the i18n metadata with the generated message string
        let i18n_meta = if let Some(attr) = i18n_attr {
            // Element has its own i18n attribute - parse it as a Message with message string
            let instance_id = self.allocate_i18n_message_instance_id();
            Some(parse_i18n_meta_with_message(
                self.allocator,
                attr.value.as_str(),
                instance_id,
                &i18n_message_string,
            ))
        } else if self.i18n_depth > 0 {
            // Element is inside an i18n block - create a TagPlaceholder for it.
            // This matches TypeScript's behavior where child elements inside i18n blocks
            // get TagPlaceholder metadata assigned during i18n message parsing.
            // Reference: i18n_parser.ts line 266-277
            Some(self.create_i18n_tag_placeholder_for_element(element))
        } else {
            None
        };

        // Increment non_bindable depth if this element has ngNonBindable
        if has_non_bindable {
            self.non_bindable_depth += 1;
        }

        // Increment i18n depth if this element has i18n attribute.
        // Also reset the placeholder registry when entering a new i18n root block.
        if has_i18n {
            if self.i18n_depth == 0 {
                // Starting a new i18n root - reset the placeholder registry
                self.i18n_placeholder_registry = PlaceholderRegistry::new();
            }
            self.i18n_depth += 1;
        }

        // Visit children
        let children = self.visit_children(&element.children);

        // Decrement non_bindable depth if we incremented it
        if has_non_bindable {
            self.non_bindable_depth -= 1;
        }

        // Decrement i18n depth if we incremented it
        if has_i18n {
            self.i18n_depth -= 1;
        }

        self.namespace_stack.pop();

        // Transform selectorless directives from HTML AST
        let directives = self.transform_directives(&element.directives, raw_name);

        // Determine if element is self-closing (explicitly closed with />)
        let is_self_closing = element.is_self_closing;

        // Check for ng-content
        // Reference: r3_template_transform.ts lines 191-204
        // Children are passed directly without extra whitespace filtering
        if raw_name == "ng-content" {
            let selector = self.get_ng_content_selector(element);
            self.ng_content_selectors.push(selector.clone());

            // For ng-content, include the structural directive attribute (*ngIf, etc.)
            // as a text attribute. This is needed because the projection instruction
            // includes these attributes in its output (e.g., ["*ngIf", "!subtitle()"]).
            // Reference: r3_template_transform.ts line 193 - all attrs are included
            let mut content_attributes = attributes;
            if let Some(ref tpl_attr) = template_attr {
                content_attributes.push(R3TextAttribute {
                    name: tpl_attr.name.clone(),
                    value: tpl_attr.value.clone(),
                    source_span: tpl_attr.span,
                    key_span: Some(tpl_attr.name_span),
                    value_span: tpl_attr.value_span,
                    i18n: None,
                });
            }

            let content = R3Content {
                selector,
                attributes: content_attributes,
                children, // Pass children directly like TypeScript does
                is_self_closing,
                source_span: element.span,
                start_source_span: element.start_span,
                end_source_span: element.end_span,
                i18n: None,
            };
            let mut result = R3Node::Content(Box::new_in(content, self.allocator));

            // Wrap in template if has structural directive (*ngIf, etc.)
            // Reference: r3_template_transform.ts lines 266-277
            if let Some(template_attr_info) = template_attr {
                result = self.wrap_in_template(result, element, template_attr_info);
            }

            return Some(result);
        }

        // Check for ng-template
        if raw_name == "ng-template" {
            let name = self.qualify_element_name(element.name.clone(), element_namespace);
            let template = R3Template {
                tag_name: Some(name),
                attributes,
                inputs,
                outputs,
                directives,
                template_attrs: Vec::new_in(self.allocator),
                children,
                references,
                variables,
                is_self_closing,
                source_span: element.span,
                start_source_span: element.start_span,
                end_source_span: element.end_span,
                i18n: i18n_meta,
            };
            let mut result = R3Node::Template(Box::new_in(template, self.allocator));

            // Wrap in another template if has structural directive (*ngIf, etc.)
            if let Some(template_attr_info) = template_attr {
                result = self.wrap_in_template(result, element, template_attr_info);
            }

            return Some(result);
        }

        // Validate ng-container attribute bindings
        // Reference: r3_template_transform.ts lines 238-247
        if raw_name == "ng-container" {
            use crate::ast::expression::BindingType;
            for input in inputs.iter() {
                if input.binding_type == BindingType::Attribute {
                    self.report_error(
                        "Attribute bindings are not supported on ng-container. Use property bindings instead.",
                        input.source_span,
                    );
                }
            }
        }

        let name = self.qualify_element_name(element.name.clone(), element_namespace);

        // Check if this is a component (uppercase first letter or underscore)
        let first_char = raw_name.chars().next().unwrap_or('a');
        let is_component = first_char.is_ascii_uppercase() || first_char == '_';

        let mut result = if is_component {
            // Validate selectorless component - check for unsupported tags
            let tag_name_lower = raw_name.to_ascii_lowercase();
            if UNSUPPORTED_SELECTORLESS_TAGS.contains(&tag_name_lower.as_str()) {
                self.report_error(
                    &format!("Tag name \"{}\" cannot be used as a component tag", raw_name),
                    element.start_span,
                );
                return None;
            }

            // Validate selectorless references
            self.validate_selectorless_references(&references);

            // Compute tag_name from component_prefix and component_tag_name
            // Format: ":prefix:tag_name" (e.g., ":svg:rect") or just "tag_name"
            let tag_name = match (&element.component_prefix, &element.component_tag_name) {
                (None, None) => None,
                (None, Some(tag)) => Some(tag.clone()),
                (Some(prefix), None) => {
                    // Has prefix but no tag name - use "ng-component" as default
                    Some(Atom::from_in(&format!(":{}:ng-component", prefix), self.allocator))
                }
                (Some(prefix), Some(tag)) => {
                    // Both prefix and tag name: ":prefix:tag_name"
                    Some(Atom::from_in(&format!(":{}:{}", prefix, tag), self.allocator))
                }
            };

            // Compute full_name: "ComponentName:prefix:tag_name" or "ComponentName:tag_name"
            let full_name = match &tag_name {
                Some(tag) if tag.starts_with(':') => {
                    // Namespace format: "MyComp:svg:rect" (tag_name already has :prefix:)
                    Atom::from_in(&format!("{}{}", element.name, tag), self.allocator)
                }
                Some(tag) => {
                    // Simple format: "MyComp:div"
                    Atom::from_in(&format!("{}:{}", element.name, tag), self.allocator)
                }
                None => element.name.clone(),
            };

            // Create R3Component for uppercase element names
            let r3_component = R3Component {
                component_name: element.name.clone(),
                tag_name,
                full_name,
                attributes,
                inputs,
                outputs,
                directives,
                children,
                references,
                is_self_closing,
                source_span: element.span,
                start_source_span: element.start_span,
                end_source_span: element.end_span,
                i18n: i18n_meta,
            };
            R3Node::Component(Box::new_in(r3_component, self.allocator))
        } else {
            // Regular element
            let r3_element = R3Element {
                name,
                attributes,
                inputs,
                outputs,
                directives,
                children,
                references,
                is_self_closing,
                source_span: element.span,
                start_source_span: element.start_span,
                end_source_span: element.end_span,
                is_void: self.is_void_element(element.name.as_str()),
                i18n: i18n_meta,
            };
            R3Node::Element(Box::new_in(r3_element, self.allocator))
        };

        // Wrap in template if has structural directive
        if let Some(template_attr_info) = template_attr {
            result = self.wrap_in_template(result, element, template_attr_info);
        }

        Some(result)
    }

    /// Visits an HTML component (selectorless component AST node).
    fn visit_html_component(&mut self, component: &HtmlComponent<'a>) -> Option<R3Node<'a>> {
        // Parse attributes
        let (attributes, inputs, outputs, references, _variables, template_attr) =
            self.parse_attributes(&component.attrs, component.full_name.as_str(), false);

        // Resolve namespace for this component and its children.
        let parent_namespace = self.current_namespace();
        let element_namespace =
            self.resolve_namespace(component.full_name.as_str(), parent_namespace);
        self.namespace_stack.push(element_namespace);

        // Check if component has ngNonBindable attribute
        let has_non_bindable =
            component.attrs.iter().any(|attr| attr.name.as_str() == "ngNonBindable");

        // Check if component has i18n attribute
        let has_i18n = component.attrs.iter().any(|attr| attr.name.as_str() == "i18n");

        // Increment non_bindable depth if this component has ngNonBindable
        if has_non_bindable {
            self.non_bindable_depth += 1;
        }

        // Increment i18n depth if this component has i18n attribute.
        // Also reset the placeholder registry when entering a new i18n root block.
        if has_i18n {
            if self.i18n_depth == 0 {
                // Starting a new i18n root - reset the placeholder registry
                self.i18n_placeholder_registry = PlaceholderRegistry::new();
            }
            self.i18n_depth += 1;
        }

        // Visit children
        let children = self.visit_children(&component.children);

        // Decrement non_bindable depth if we incremented it
        if has_non_bindable {
            self.non_bindable_depth -= 1;
        }

        // Decrement i18n depth if we incremented it
        if has_i18n {
            self.i18n_depth -= 1;
        }

        self.namespace_stack.pop();

        // Transform selectorless directives from HTML AST
        // For components, tag_name may be None (e.g., `<MyComp>`), in which case we use empty string
        // which matches TypeScript's behavior where elementName can be null.
        let element_name = component.tag_name.as_ref().map(|a| a.as_str()).unwrap_or("");
        let directives = self.transform_directives(&component.directives, element_name);

        // Validate selectorless references
        self.validate_selectorless_references(&references);

        // Create R3Component
        let r3_component = R3Component {
            component_name: component.component_name.clone(),
            tag_name: component.tag_name.clone(),
            full_name: component.full_name.clone(),
            attributes,
            inputs,
            outputs,
            directives,
            children,
            references,
            is_self_closing: component.is_self_closing,
            source_span: component.span,
            start_source_span: component.start_span,
            end_source_span: component.end_span,
            i18n: None,
        };
        let mut result = R3Node::Component(Box::new_in(r3_component, self.allocator));

        // Wrap in template if has structural directive
        if let Some(template_attr_info) = template_attr {
            result = self.wrap_component_in_template(result, component, template_attr_info);
        }

        Some(result)
    }

    fn current_namespace(&self) -> ElementNamespace {
        self.namespace_stack.last().copied().unwrap_or(ElementNamespace::Html)
    }

    fn resolve_namespace(&self, raw_name: &str, parent: ElementNamespace) -> ElementNamespace {
        if let Some(explicit) = Self::namespace_from_prefixed_name(raw_name) {
            return explicit;
        }

        if raw_name.eq_ignore_ascii_case("svg") {
            return ElementNamespace::Svg;
        }
        if raw_name.eq_ignore_ascii_case("math") {
            return ElementNamespace::Math;
        }

        match parent {
            ElementNamespace::Svg => {
                if raw_name.eq_ignore_ascii_case("foreignObject") {
                    ElementNamespace::Html
                } else {
                    ElementNamespace::Svg
                }
            }
            ElementNamespace::Math => ElementNamespace::Math,
            ElementNamespace::Html => ElementNamespace::Html,
        }
    }

    fn namespace_from_prefixed_name(raw_name: &str) -> Option<ElementNamespace> {
        if raw_name.starts_with(':') {
            if let Some((prefix, _)) = raw_name[1..].split_once(':') {
                return Self::namespace_from_prefix(prefix);
            }
        }

        if let Some((prefix, _)) = raw_name.split_once(':') {
            return Self::namespace_from_prefix(prefix);
        }

        None
    }

    fn namespace_from_prefix(prefix: &str) -> Option<ElementNamespace> {
        if prefix.eq_ignore_ascii_case("svg") {
            Some(ElementNamespace::Svg)
        } else if prefix.eq_ignore_ascii_case("math") {
            Some(ElementNamespace::Math)
        } else {
            None
        }
    }

    fn qualify_element_name(&self, name: Atom<'a>, namespace: ElementNamespace) -> Atom<'a> {
        if namespace == ElementNamespace::Html {
            return name;
        }

        let name_str = name.as_str();
        if name_str.starts_with(':') {
            return name;
        }

        if let Some((prefix, local)) = name_str.split_once(':') {
            if Self::namespace_from_prefix(prefix).is_some() {
                let qualified = format!(":{}:{}", prefix, local);
                return Atom::from_in(&qualified, self.allocator);
            }
        }

        let ns = match namespace {
            ElementNamespace::Svg => "svg",
            ElementNamespace::Math => "math",
            ElementNamespace::Html => return name,
        };
        let qualified = format!(":{}:{}", ns, name_str);
        Atom::from_in(&qualified, self.allocator)
    }

    /// Transforms HTML directives to R3 directives.
    /// Implements validation from Angular's `extractDirectives()` and `validateSelectorlessReferences()`.
    fn transform_directives(
        &mut self,
        html_directives: &[HtmlDirective<'a>],
        element_name: &str,
    ) -> Vec<'a, R3Directive<'a>> {
        let mut directives = Vec::new_in(self.allocator);
        let mut seen_directives: FxHashSet<&str> = FxHashSet::default();

        for html_dir in html_directives {
            let directive_name = html_dir.name.as_str();

            // Check for duplicate directives
            // Reference: r3_template_transform.ts lines 924-930
            if seen_directives.contains(directive_name) {
                self.report_error(
                    &format!(
                        "Cannot apply directive \"{}\" multiple times on the same element",
                        directive_name
                    ),
                    html_dir.span,
                );
                continue;
            }
            seen_directives.insert(directive_name);

            // Parse directive attributes similar to element attributes
            let mut attributes = Vec::new_in(self.allocator);
            let mut inputs = Vec::new_in(self.allocator);
            let mut outputs = Vec::new_in(self.allocator);
            let mut references = Vec::new_in(self.allocator);
            let mut seen_reference_names: FxHashSet<&str> = FxHashSet::default();
            let mut invalid = false;

            for attr in html_dir.attrs.iter() {
                let attr_name = attr.name.as_str();
                let attr_value = attr.value.as_str();

                // Check for unsupported attributes
                // Reference: r3_template_transform.ts lines 908-922
                if attr_name.starts_with(TEMPLATE_ATTR_PREFIX) {
                    self.report_error(
                        &format!(
                            "Shorthand template syntax \"{}\" is not supported inside a directive context",
                            attr_name
                        ),
                        attr.span,
                    );
                    invalid = true;
                    continue;
                }

                if attr_name == "ngProjectAs" || attr_name == "ngNonBindable" {
                    self.report_error(
                        &format!(
                            "Attribute \"{}\" is not supported in a directive context",
                            attr_name
                        ),
                        attr.span,
                    );
                    invalid = true;
                    continue;
                }

                // Check for reference syntax (#ref or ref-)
                if attr_name.starts_with('#') || attr_name.starts_with("ref-") {
                    let ref_name =
                        if attr_name.starts_with('#') { &attr_name[1..] } else { &attr_name[4..] };

                    // Validate reference - no value allowed in directive context
                    // Reference: r3_template_transform.ts lines 1121-1140
                    if !attr_value.is_empty() {
                        self.report_error(
                            "Cannot specify a value for a local reference in this context",
                            attr.value_span.unwrap_or(attr.span),
                        );
                        invalid = true;
                    }

                    // Check for duplicate reference names
                    if seen_reference_names.contains(ref_name) {
                        self.report_error("Duplicate reference names are not allowed", attr.span);
                        invalid = true;
                    } else {
                        seen_reference_names.insert(ref_name);
                        references.push(R3Reference {
                            name: Atom::from_in(ref_name, self.allocator),
                            value: Atom::from_in("", self.allocator),
                            source_span: attr.span,
                            key_span: attr.name_span,
                            value_span: None,
                        });
                    }
                    continue;
                }

                // Check for binding prefix syntax (bind-, on-, bindon-)
                if let Some((prefix, rest)) = self.parse_binding_prefix(attr_name) {
                    match prefix {
                        BindingPrefix::Bind => {
                            // bind-prop="value" - property binding
                            // Check for animation bindings: bind-@prop or bind-animate-prop
                            let (prop_name, binding_type) =
                                if let Some(anim_name) = rest.strip_prefix('@') {
                                    (anim_name, BindingType::Animation)
                                } else if let Some(anim_name) = rest.strip_prefix("animate-") {
                                    (anim_name, BindingType::Animation)
                                } else {
                                    (rest, BindingType::Property)
                                };

                            // Check for non-property bindings which are not allowed
                            if prop_name.starts_with("attr.")
                                || prop_name.starts_with("class.")
                                || prop_name.starts_with("style.")
                            {
                                self.report_error(
                                    "Binding is not supported in a directive context",
                                    attr.span,
                                );
                                invalid = true;
                                continue;
                            }

                            let value_span = attr.value_span.unwrap_or(attr.span);
                            let parse_result =
                                self.binding_parser.parse_binding(attr_value, value_span);

                            inputs.push(R3BoundAttribute {
                                name: Atom::from_in(prop_name, self.allocator),
                                binding_type,
                                value: parse_result.ast,
                                unit: None,
                                source_span: attr.span,
                                key_span: attr.name_span,
                                value_span: Some(value_span),
                                i18n: None,
                                security_context: get_security_context(element_name, prop_name),
                            });
                        }
                        BindingPrefix::On => {
                            // on-event="handler" - event binding
                            let value_span = attr.value_span.unwrap_or(attr.span);
                            let parse_result =
                                self.binding_parser.parse_event(attr_value, value_span);

                            outputs.push(R3BoundEvent {
                                name: Atom::from_in(rest, self.allocator),
                                handler: parse_result.ast,
                                target: None,
                                event_type: ParsedEventType::Regular,
                                phase: None,
                                source_span: attr.span,
                                key_span: attr.name_span,
                                handler_span: value_span,
                            });
                        }
                        BindingPrefix::BindOn => {
                            // bindon-prop="value" - two-way binding
                            let value_span = attr.value_span.unwrap_or(attr.span);
                            let parse_result =
                                self.binding_parser.parse_binding(attr_value, value_span);

                            inputs.push(R3BoundAttribute {
                                name: Atom::from_in(rest, self.allocator),
                                binding_type: BindingType::TwoWay,
                                value: parse_result.ast,
                                unit: None,
                                source_span: attr.span,
                                key_span: attr.name_span,
                                value_span: Some(value_span),
                                i18n: None,
                                security_context: get_security_context(element_name, rest),
                            });

                            // Two-way binding also creates an output event
                            let event_name =
                                Atom::from_in(&format!("{}Change", rest), self.allocator);
                            let event_parse_result =
                                self.binding_parser.parse_event(attr_value, value_span);

                            outputs.push(R3BoundEvent {
                                name: event_name,
                                handler: event_parse_result.ast,
                                target: None,
                                event_type: ParsedEventType::TwoWay,
                                phase: None,
                                source_span: attr.span,
                                key_span: attr.name_span,
                                handler_span: value_span,
                            });
                        }
                        BindingPrefix::At => {
                            // @trigger="value" - legacy animation trigger
                            let value_span = attr.value_span.unwrap_or(attr.span);
                            // For legacy animation triggers, use "undefined" when there's no expression.
                            // This matches TypeScript's behavior: `expression || 'undefined'`
                            let value_str =
                                if attr_value.is_empty() { "undefined" } else { attr_value };
                            let parse_result =
                                self.binding_parser.parse_binding(value_str, value_span);

                            inputs.push(R3BoundAttribute {
                                name: Atom::from_in(rest, self.allocator),
                                binding_type: BindingType::LegacyAnimation,
                                value: parse_result.ast,
                                unit: None,
                                source_span: attr.span,
                                key_span: attr.name_span,
                                value_span: Some(value_span),
                                i18n: None,
                                security_context: crate::ast::r3::SecurityContext::None,
                            });
                        }
                        BindingPrefix::Ref | BindingPrefix::Let => {
                            // Already handled above
                        }
                    }
                    continue;
                }

                // Check for binding syntax
                if attr_name.starts_with("[(") && attr_name.ends_with(")]") {
                    // Two-way binding: [(prop)]="value" - allowed
                    let prop_name = &attr_name[2..attr_name.len() - 2];
                    let value_str = self.allocator.alloc_str(attr_value);
                    let value_span = attr.value_span.unwrap_or(attr.span);
                    let parse_result = self.binding_parser.parse_binding(value_str, value_span);

                    inputs.push(R3BoundAttribute {
                        name: Atom::from_in(prop_name, self.allocator),
                        binding_type: BindingType::TwoWay,
                        value: parse_result.ast,
                        unit: None,
                        source_span: attr.span,
                        key_span: attr.name_span,
                        value_span: Some(value_span),
                        i18n: None,
                        security_context: get_security_context(element_name, prop_name),
                    });

                    // Two-way binding also creates an output event
                    let event_name = Atom::from_in(&format!("{}Change", prop_name), self.allocator);
                    let event_value_str = self.allocator.alloc_str(attr_value);
                    let event_parse_result =
                        self.binding_parser.parse_event(event_value_str, value_span);

                    outputs.push(R3BoundEvent {
                        name: event_name,
                        handler: event_parse_result.ast,
                        target: None,
                        event_type: ParsedEventType::TwoWay,
                        phase: None,
                        source_span: attr.span,
                        key_span: attr.name_span,
                        handler_span: value_span,
                    });
                } else if attr_name.starts_with('[') && attr_name.ends_with(']') {
                    // Property binding: [prop]="value"
                    let prop_name = &attr_name[1..attr_name.len() - 1];

                    // Check for non-property bindings which are not allowed
                    // Reference: r3_template_transform.ts lines 946-951
                    if prop_name.starts_with("attr.") {
                        self.report_error(
                            "Binding is not supported in a directive context",
                            attr.span,
                        );
                        invalid = true;
                        continue;
                    }
                    if prop_name.starts_with("class.") {
                        self.report_error(
                            "Binding is not supported in a directive context",
                            attr.span,
                        );
                        invalid = true;
                        continue;
                    }
                    if prop_name.starts_with("style.") {
                        self.report_error(
                            "Binding is not supported in a directive context",
                            attr.span,
                        );
                        invalid = true;
                        continue;
                    }
                    if prop_name.starts_with('@') {
                        self.report_error(
                            "Binding is not supported in a directive context",
                            attr.span,
                        );
                        invalid = true;
                        continue;
                    }

                    let value_str = self.allocator.alloc_str(attr_value);
                    let value_span = attr.value_span.unwrap_or(attr.span);
                    let parse_result = self.binding_parser.parse_binding(value_str, value_span);

                    inputs.push(R3BoundAttribute {
                        name: Atom::from_in(prop_name, self.allocator),
                        binding_type: BindingType::Property,
                        value: parse_result.ast,
                        unit: None,
                        source_span: attr.span,
                        key_span: attr.name_span,
                        value_span: Some(value_span),
                        i18n: None,
                        security_context: get_security_context(element_name, prop_name),
                    });
                } else if attr_name.starts_with('(') && attr_name.ends_with(')') {
                    // Event binding: (event)="handler"
                    let event_name = &attr_name[1..attr_name.len() - 1];

                    // Check for legacy animation syntax (@) or (@name.phase)
                    if event_name.starts_with('@') {
                        let anim_name = event_name.strip_prefix('@').unwrap_or("");
                        outputs.push(self.create_legacy_animation_event(anim_name, attr));
                    } else {
                        // Regular event handling
                        let value_str = self.allocator.alloc_str(attr_value);
                        let value_span = attr.value_span.unwrap_or(attr.span);
                        let parse_result = self.binding_parser.parse_event(value_str, value_span);

                        outputs.push(R3BoundEvent {
                            name: Atom::from_in(event_name, self.allocator),
                            handler: parse_result.ast,
                            target: None,
                            event_type: ParsedEventType::Regular,
                            phase: None,
                            source_span: attr.span,
                            key_span: attr.name_span,
                            handler_span: value_span,
                        });
                    }
                } else {
                    // Plain text attribute
                    attributes.push(R3TextAttribute {
                        name: attr.name.clone(),
                        value: attr.value.clone(),
                        source_span: attr.span,
                        key_span: Some(attr.name_span),
                        value_span: attr.value_span,
                        i18n: None,
                    });
                }
            }

            // Skip this directive if validation failed
            if invalid {
                continue;
            }

            // Calculate source spans for the directive
            // start_source_span should include the opening paren if present
            let source_span = html_dir.span;
            let start_source_span = if let Some(ref paren_span) = html_dir.start_paren_span {
                // Include the opening paren in the start span
                Span::new(html_dir.name_span.start, paren_span.end)
            } else {
                html_dir.name_span
            };
            let end_source_span = html_dir.end_paren_span;

            directives.push(R3Directive {
                name: html_dir.name.clone(),
                attributes,
                inputs,
                outputs,
                references,
                source_span,
                start_source_span,
                end_source_span,
                i18n: None,
            });
        }

        directives
    }

    /// Generates a unique placeholder name for ICU variables.
    ///
    /// Similar to Angular's PlaceholderRegistry.getUniquePlaceholder:
    /// - First call with "VAR_PLURAL" returns "VAR_PLURAL"
    /// - Second call with "VAR_PLURAL" returns "VAR_PLURAL_1"
    /// - Third call with "VAR_PLURAL" returns "VAR_PLURAL_2"
    ///
    /// Ported from Angular's `placeholder.ts:96-98` and `_generateUniqueName:151-161`.
    fn generate_unique_icu_placeholder(&mut self, base_name: &str) -> String {
        let count = self.icu_placeholder_counts.entry(base_name.to_string()).or_insert(0);
        let result =
            if *count == 0 { base_name.to_string() } else { format!("{}_{}", base_name, count) };
        *count += 1;
        result
    }

    /// Resets the ICU placeholder counters for a new ICU context.
    ///
    /// This should be called when entering a new top-level ICU to ensure
    /// placeholder names are unique within each ICU but can restart for different ICUs.
    fn reset_icu_placeholder_counts(&mut self) {
        self.icu_placeholder_counts.clear();
    }

    /// Visits an ICU expansion (e.g., {var, plural, other { ... }}).
    /// Only emits ICU nodes when inside an i18n context.
    /// An i18n context is present when:
    /// - The expansion is inside an element with i18n attribute (tracked via i18n_depth)
    /// - Or the expansion.in_i18n_block flag is set (for i18n block comments)
    ///
    /// Ported from Angular's r3_template_transform.ts:301-337
    fn visit_expansion(&mut self, expansion: &HtmlExpansion<'a>) -> Option<R3Node<'a>> {
        // Do not generate Icu if it was created outside of i18n block/element in a template
        // Reference: r3_template_transform.ts:301-306
        let in_i18n_context = expansion.in_i18n_block || self.i18n_depth > 0;
        if !in_i18n_context {
            return None;
        }

        // Reset ICU placeholder counters for this new top-level ICU.
        // This ensures unique placeholder names within each ICU context.
        // Ported from Angular's PlaceholderRegistry usage in i18n_parser.ts.
        self.reset_icu_placeholder_counts();

        // Create i18n metadata with proper ICU placeholder
        // This matches Angular's behavior where expansion.i18n is a Message
        // containing a single IcuPlaceholder node
        let icu_type_upper = expansion.expansion_type.as_str().to_uppercase();
        let base_name = format!("VAR_{}", icu_type_upper);
        let expression_placeholder =
            Atom::from_in(&self.generate_unique_icu_placeholder(&base_name), self.allocator);
        let icu_placeholder_name = Atom::from_in("ICU", self.allocator);

        // Create the I18nIcu for the i18n metadata
        // The cases are empty here since they're parsed separately into R3Icu.placeholders
        let i18n_icu = I18nIcu {
            expression: expansion.switch_value.clone(),
            icu_type: expansion.expansion_type.clone(),
            cases: HashMap::new_in(self.allocator),
            source_span: expansion.span,
            expression_placeholder: Some(expression_placeholder.clone()),
        };

        // Create the IcuPlaceholder wrapping the ICU
        let i18n_icu_placeholder = I18nIcuPlaceholder {
            value: Box::new_in(i18n_icu, self.allocator),
            name: icu_placeholder_name,
            source_span: expansion.span,
        };

        // Create the Message containing the single IcuPlaceholder
        let mut nodes = Vec::new_in(self.allocator);
        nodes.push(I18nNode::IcuPlaceholder(i18n_icu_placeholder));

        // Serialize the message string for goog.getMsg and $localize
        let message_string_str = serialize_i18n_nodes(&nodes);
        let message_string = Atom::from_in(&*message_string_str, self.allocator);

        let i18n_message = I18nMessage {
            instance_id: self.allocate_i18n_message_instance_id(),
            nodes,
            meaning: Atom::from(""),
            description: Atom::from(""),
            custom_id: Atom::from(""),
            id: Atom::from(""),
            legacy_ids: Vec::new_in(self.allocator),
            message_string,
        };

        // Create variable for the switch value (using VAR_* placeholder name)
        let mut vars = Vec::new_in(self.allocator);
        let switch_value_str = expansion.switch_value.as_str();
        let switch_value_span = expansion.switch_value_span;

        // Parse the switch value as a binding expression
        let value_str = self.allocator.alloc_str(switch_value_str);
        let parse_result = self.binding_parser.parse_binding(value_str, switch_value_span);

        // Parse placeholders from expansion cases FIRST (before adding the outer VAR).
        // This matches Angular's visitExpansion behavior where nested ICUs are visited first,
        // and their VAR_* placeholders are added before the outer ICU's VAR_*.
        // Ported from Angular's i18n_parser.ts:137-159
        let mut placeholders = Vec::new_in(self.allocator);
        for case in expansion.cases.iter() {
            self.extract_placeholders_from_nodes(&case.expansion, &mut placeholders, &mut vars);
        }

        // Add the outer ICU's var AFTER processing all children.
        // This ensures the correct order: nested ICU vars first, then outer ICU var.
        // Use the unique VAR_* placeholder name as the key, matching Angular's behavior.
        // The expression_placeholder was already generated above with getUniquePlaceholder.
        ordered_insert_var(
            &mut vars,
            expression_placeholder.clone(),
            R3BoundText { value: parse_result.ast, source_span: switch_value_span, i18n: None },
        );

        let icu = R3Icu {
            vars,
            placeholders,
            source_span: expansion.span,
            i18n: Some(I18nMeta::Message(i18n_message)),
        };

        Some(R3Node::Icu(Box::new_in(icu, self.allocator)))
    }

    /// Extracts placeholders and nested ICU vars from expansion case nodes.
    fn extract_placeholders_from_nodes(
        &mut self,
        nodes: &[HtmlNode<'a>],
        placeholders: &mut Vec<'a, (Atom<'a>, R3IcuPlaceholder<'a>)>,
        vars: &mut Vec<'a, (Atom<'a>, R3BoundText<'a>)>,
    ) {
        for node in nodes {
            match node {
                HtmlNode::Text(text) => {
                    // Extract individual interpolations from text
                    self.extract_interpolations_as_placeholders(
                        text.value.as_str(),
                        text.span,
                        placeholders,
                    );
                }
                HtmlNode::Expansion(nested_expansion) => {
                    // Process nested ICU: first visit children, then add the VAR.
                    // This matches Angular's visitExpansion behavior in i18n_parser.ts:137-159
                    // where nested ICUs are visited first (depth-first), and their VAR_*
                    // placeholders are added AFTER visiting all children.

                    // Parse the switch value expression
                    let switch_value_str = nested_expansion.switch_value.as_str();
                    let switch_value_span = nested_expansion.switch_value_span;
                    let value_str = self.allocator.alloc_str(switch_value_str);
                    let parse_result =
                        self.binding_parser.parse_binding(value_str, switch_value_span);

                    // Generate unique VAR_* placeholder name for the nested ICU BEFORE recursion.
                    // The counter is incremented immediately, so nested ICUs get sequential names.
                    // This matches Angular's getUniquePlaceholder in i18n_parser.ts:153
                    let icu_type_upper = nested_expansion.expansion_type.as_str().to_uppercase();
                    let base_name = format!("VAR_{}", icu_type_upper);
                    let unique_name = self.generate_unique_icu_placeholder(&base_name);
                    let var_placeholder_name = Atom::from_in(&unique_name, self.allocator);

                    // Recursively extract from nested expansion cases FIRST.
                    // This ensures placeholders and further nested ICU vars are processed
                    // before adding this ICU's VAR_*.
                    for case in nested_expansion.cases.iter() {
                        self.extract_placeholders_from_nodes(&case.expansion, placeholders, vars);
                    }

                    // Add the nested ICU's var AFTER processing all its children.
                    // Use the unique VAR_* placeholder name as the key, not the raw switch value.
                    // This is critical: when multiple nested ICUs have the same switch value
                    // (e.g., same pipe expression), they MUST have separate entries in the vars
                    // collection. Angular uses unique placeholder names (VAR_SELECT, VAR_SELECT_1,
                    // VAR_SELECT_2) to ensure each nested ICU creates its own TextOp with its
                    // own pipe slot allocation.
                    ordered_insert_var(
                        vars,
                        var_placeholder_name,
                        R3BoundText {
                            value: parse_result.ast,
                            source_span: switch_value_span,
                            i18n: None,
                        },
                    );
                }
                _ => {}
            }
        }
    }

    /// Extracts individual interpolations from text and adds them as placeholders.
    fn extract_interpolations_as_placeholders(
        &mut self,
        text: &str,
        base_span: Span,
        placeholders: &mut Vec<'a, (Atom<'a>, R3IcuPlaceholder<'a>)>,
    ) {
        // Use default Angular interpolation markers
        let start_marker = "{{";
        let end_marker = "}}";

        let mut pos = 0;
        while pos < text.len() {
            // Find start of interpolation
            if let Some(start_idx) = text[pos..].find(start_marker) {
                let abs_start = pos + start_idx;
                let content_start = abs_start + start_marker.len();

                // Find end of interpolation
                if let Some(end_idx) = text[content_start..].find(end_marker) {
                    let content_end = content_start + end_idx;
                    let abs_end = content_end + end_marker.len();

                    // Extract the full interpolation including markers
                    let interpolation = &text[abs_start..abs_end];
                    let expr_content = &text[content_start..content_end];

                    // Calculate the span for this interpolation
                    let interp_span = Span::new(
                        base_span.start + abs_start as u32,
                        base_span.start + abs_end as u32,
                    );

                    // Parse the expression
                    let value_str = self.allocator.alloc_str(expr_content);
                    let parse_result = self.binding_parser.parse_binding(value_str, interp_span);

                    let placeholder_key = Atom::from_in(interpolation, self.allocator);
                    let bound_text = R3BoundText {
                        value: parse_result.ast,
                        source_span: interp_span,
                        i18n: None,
                    };
                    ordered_insert_placeholder(
                        placeholders,
                        placeholder_key,
                        R3IcuPlaceholder::BoundText(bound_text),
                    );

                    pos = abs_end;
                } else {
                    break; // No closing marker found
                }
            } else {
                break; // No more interpolations
            }
        }
    }

    /// Visits a text node.
    fn visit_text(&mut self, text: &HtmlText<'a>) -> Option<R3Node<'a>> {
        use crate::parser::html::NGSP_UNICODE;

        // Replace &ngsp; pseudo-entity with space before processing
        // Reference: r3_template_transform.ts line 1056
        let value_str = text.value.as_str();
        let has_ngsp = value_str.contains(NGSP_UNICODE);

        // Check if text contains interpolation (only when not in ngNonBindable context)
        if self.non_bindable_depth == 0 && self.has_interpolation(value_str) {
            // Check if there are HTML entities that could affect span calculations.
            // If so, use the tokens which have correct source spans.
            let has_entities = text
                .tokens
                .iter()
                .any(|t| matches!(t.token_type, InterpolatedTokenType::EncodedEntity));

            // Create i18n metadata for bound text when inside an i18n block.
            // This creates a Container with Text and Placeholder nodes that match
            // the structure expected by the ingest phase.
            // Ported from Angular's r3_template_transform.ts I18nMetaVisitor.visitText
            let i18n_meta = if self.i18n_depth > 0 {
                let processed_text = if has_ngsp {
                    value_str.replace(NGSP_UNICODE, " ")
                } else {
                    value_str.to_string()
                };
                Some(self.create_i18n_meta_for_text_with_interpolation(&processed_text, text.span))
            } else {
                None
            };

            if has_entities {
                // Use tokens to parse interpolations with correct source spans
                if let Some(expr) = self.parse_interpolation_from_tokens(text) {
                    let bound_text =
                        R3BoundText { value: expr, source_span: text.span, i18n: i18n_meta };
                    return Some(R3Node::BoundText(Box::new_in(bound_text, self.allocator)));
                }
            } else {
                // No entities - use the simple path with decoded text
                let interpolation_text = if has_ngsp {
                    let replaced = value_str.replace(NGSP_UNICODE, " ");
                    self.allocator.alloc_str(&replaced)
                } else {
                    self.allocator.alloc_str(value_str)
                };

                // Create bound text with interpolation expression
                if let Some(expr) = self.parse_interpolation(interpolation_text, text.span) {
                    let bound_text =
                        R3BoundText { value: expr, source_span: text.span, i18n: i18n_meta };
                    return Some(R3Node::BoundText(Box::new_in(bound_text, self.allocator)));
                }
            }
        }

        // Static text - use value with ngsp replaced
        let value_atom = if has_ngsp {
            let value_no_ngsp = value_str.replace(NGSP_UNICODE, " ");
            Atom::from_in(&value_no_ngsp, self.allocator)
        } else {
            text.value.clone()
        };
        let r3_text = R3Text { value: value_atom, source_span: text.span };
        Some(R3Node::Text(Box::new_in(r3_text, self.allocator)))
    }

    /// Visits a comment node.
    /// Angular's visitComment returns null - comments are collected separately, not in AST.
    fn visit_comment(&mut self, comment: &crate::ast::html::HtmlComment<'a>) -> Option<R3Node<'a>> {
        // Only collect comment nodes when the flag is set
        // Angular's visitComment (r3_template_transform.ts:344-349) always returns null
        // Comments are NOT included in the AST output - they're collected separately
        if let Some(ref mut comments) = self.comment_nodes {
            let r3_comment = R3Comment { value: comment.value.clone(), source_span: comment.span };
            comments.push(r3_comment);
        }

        // Never include comments in AST output - matches Angular's behavior
        None
    }

    /// Visits a block inside ngNonBindable context, converting it to text nodes.
    /// This is equivalent to TypeScript's NonBindableVisitor.visitBlock().
    ///
    /// In ngNonBindable context, we emit the block's source as plain text.
    /// TypeScript returns an array [startText, ...children, endText].flat(Infinity),
    /// but since our visitor returns a single node, we create a wrapper template
    /// containing all the text nodes and children.
    fn visit_block_as_text(&mut self, block: &HtmlBlock<'a>) -> Option<R3Node<'a>> {
        let mut nodes = Vec::new_in(self.allocator);

        // Get the text for the block's start source span (e.g., "@if (condition) {")
        let start_text = if block.start_span.start < block.start_span.end
            && (block.start_span.end as usize) <= self.source_text.len()
        {
            &self.source_text[block.start_span.start as usize..block.start_span.end as usize]
        } else {
            ""
        };

        // Add start text node
        nodes.push(R3Node::Text(Box::new_in(
            R3Text {
                value: Atom::from(self.allocator.alloc_str(start_text)),
                source_span: block.start_span,
            },
            self.allocator,
        )));

        // Visit children recursively (they're also in ngNonBindable context)
        let children = self.visit_children(&block.children);
        for child in children {
            nodes.push(child);
        }

        // Add end text node if present (e.g., "}")
        if let Some(end_span) = block.end_span {
            let end_text = if end_span.start < end_span.end
                && (end_span.end as usize) <= self.source_text.len()
            {
                &self.source_text[end_span.start as usize..end_span.end as usize]
            } else {
                ""
            };

            nodes.push(R3Node::Text(Box::new_in(
                R3Text {
                    value: Atom::from(self.allocator.alloc_str(end_text)),
                    source_span: end_span,
                },
                self.allocator,
            )));
        }

        // Since we can only return a single node but TypeScript returns an array,
        // we wrap everything in an implicit template (with no tag name).
        // This preserves all the content while matching the single-node return type.
        Some(R3Node::Template(Box::new_in(
            R3Template {
                tag_name: None,
                attributes: Vec::new_in(self.allocator),
                inputs: Vec::new_in(self.allocator),
                outputs: Vec::new_in(self.allocator),
                directives: Vec::new_in(self.allocator),
                template_attrs: Vec::new_in(self.allocator),
                children: nodes,
                references: Vec::new_in(self.allocator),
                variables: Vec::new_in(self.allocator),
                source_span: block.span,
                start_source_span: block.start_span,
                end_source_span: block.end_span,
                i18n: None,
                is_self_closing: false,
            },
            self.allocator,
        )))
    }

    /// Visits a block inside ngNonBindable context, returning a flat Vec of nodes.
    /// This is the correct implementation matching TypeScript's NonBindableVisitor.visitBlock().
    /// TypeScript returns [startText, ...children, endText].flat(Infinity).
    fn visit_block_as_text_flat(&mut self, block: &HtmlBlock<'a>) -> Vec<'a, R3Node<'a>> {
        let mut nodes = Vec::new_in(self.allocator);

        // Get the text for the block's start source span (e.g., "@defer (when condition) {")
        let start_text = if block.start_span.start < block.start_span.end
            && (block.start_span.end as usize) <= self.source_text.len()
        {
            &self.source_text[block.start_span.start as usize..block.start_span.end as usize]
        } else {
            ""
        };

        // Add start text node
        nodes.push(R3Node::Text(Box::new_in(
            R3Text {
                value: Atom::from(self.allocator.alloc_str(start_text)),
                source_span: block.start_span,
            },
            self.allocator,
        )));

        // Visit children recursively - they are also in ngNonBindable context
        // visit_children will handle nested blocks properly via visit_siblings
        let children = self.visit_children(&block.children);
        for child in children {
            nodes.push(child);
        }

        // Add end text node if present (e.g., "}")
        if let Some(end_span) = block.end_span {
            let end_text = if end_span.start < end_span.end
                && (end_span.end as usize) <= self.source_text.len()
            {
                &self.source_text[end_span.start as usize..end_span.end as usize]
            } else {
                ""
            };

            nodes.push(R3Node::Text(Box::new_in(
                R3Text {
                    value: Atom::from(self.allocator.alloc_str(end_text)),
                    source_span: end_span,
                },
                self.allocator,
            )));
        }

        nodes
    }

    /// Visits a block (control flow) - simple version without sibling context.
    fn visit_block(&mut self, block: &HtmlBlock<'a>) -> Option<R3Node<'a>> {
        // For blocks without sibling context, use empty connected blocks
        self.visit_block_with_siblings(block, 0, &[])
    }

    /// Visits a block with sibling context for connected block handling.
    fn visit_block_with_siblings(
        &mut self,
        block: &HtmlBlock<'a>,
        index: usize,
        siblings: &[HtmlNode<'a>],
    ) -> Option<R3Node<'a>> {
        // Inside ngNonBindable, convert blocks to plain text (like TypeScript's NonBindableVisitor)
        if self.non_bindable_depth > 0 {
            return self.visit_block_as_text(block);
        }

        match block.block_type {
            BlockType::If => {
                // First, find and collect connected block indices and whitespace
                let (connected_indices, whitespace_indices) =
                    Self::find_connected_block_indices(index, siblings, |bt| {
                        matches!(bt, BlockType::Else | BlockType::ElseIf)
                    });
                // Mark connected blocks as processed
                for &idx in &connected_indices {
                    let node_id = &siblings[idx] as *const _ as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Mark whitespace between blocks as processed
                for &idx in &whitespace_indices {
                    let node_id = &siblings[idx] as *const _ as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Collect references to the blocks
                let connected: std::vec::Vec<_> = connected_indices
                    .iter()
                    .filter_map(|&idx| {
                        if let HtmlNode::Block(b) = &siblings[idx] {
                            Some(b.as_ref())
                        } else {
                            None
                        }
                    })
                    .collect();
                self.visit_if_block(block, &connected)
            }
            BlockType::Else | BlockType::ElseIf => {
                // These are handled as part of @if
                None
            }
            BlockType::For => {
                // First, find and collect connected block indices and whitespace
                let (connected_indices, whitespace_indices) =
                    Self::find_connected_block_indices(index, siblings, |bt| {
                        matches!(bt, BlockType::Empty)
                    });
                // Mark connected blocks as processed
                for &idx in &connected_indices {
                    let node_id = &siblings[idx] as *const _ as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Mark whitespace between blocks as processed
                for &idx in &whitespace_indices {
                    let node_id = &siblings[idx] as *const _ as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Collect references to the blocks
                let connected: std::vec::Vec<_> = connected_indices
                    .iter()
                    .filter_map(|&idx| {
                        if let HtmlNode::Block(b) = &siblings[idx] {
                            Some(b.as_ref())
                        } else {
                            None
                        }
                    })
                    .collect();
                self.visit_for_block(block, &connected)
            }
            BlockType::Empty => {
                // Handled as part of @for
                None
            }
            BlockType::Switch => self.visit_switch_block(block),
            BlockType::Case | BlockType::Default => {
                // Handled as part of @switch
                None
            }
            BlockType::Defer => {
                // First, find and collect connected block indices and whitespace
                let (connected_indices, whitespace_indices) =
                    Self::find_connected_block_indices(index, siblings, |bt| {
                        matches!(bt, BlockType::Placeholder | BlockType::Loading | BlockType::Error)
                    });
                // Mark connected blocks as processed
                for &idx in &connected_indices {
                    let node_id = &siblings[idx] as *const _ as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Mark whitespace between blocks as processed
                for &idx in &whitespace_indices {
                    let node_id = &siblings[idx] as *const _ as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Collect references to the blocks
                let connected: std::vec::Vec<_> = connected_indices
                    .iter()
                    .filter_map(|&idx| {
                        if let HtmlNode::Block(b) = &siblings[idx] {
                            Some(b.as_ref())
                        } else {
                            None
                        }
                    })
                    .collect();
                self.visit_defer_block(block, &connected)
            }
            BlockType::Placeholder | BlockType::Loading | BlockType::Error => {
                // Handled as part of @defer
                None
            }
        }
    }

    /// Finds indices of connected blocks and whitespace to skip.
    /// Returns (connected_block_indices, whitespace_indices_to_skip).
    fn find_connected_block_indices<F>(
        primary_index: usize,
        siblings: &[HtmlNode<'a>],
        is_connected: F,
    ) -> (std::vec::Vec<usize>, std::vec::Vec<usize>)
    where
        F: Fn(BlockType) -> bool,
    {
        let mut connected_indices = std::vec::Vec::new();
        let mut whitespace_indices = std::vec::Vec::new();

        for i in (primary_index + 1)..siblings.len() {
            let node = &siblings[i];

            // Skip over comments and mark them as consumed
            if matches!(node, HtmlNode::Comment(_)) {
                whitespace_indices.push(i);
                continue;
            }

            // Skip empty text nodes between blocks and mark them as consumed
            if let HtmlNode::Text(text) = node {
                if text.value.trim().is_empty() {
                    whitespace_indices.push(i);
                    continue;
                }
            }

            // Check if this is a connected block
            if let HtmlNode::Block(block) = node {
                if is_connected(block.block_type) {
                    connected_indices.push(i);
                    continue;
                }
            }

            // Stop at any non-connected node
            break;
        }

        (connected_indices, whitespace_indices)
    }

    /// Visits a @let declaration.
    fn visit_let_declaration(
        &mut self,
        decl: &crate::ast::html::HtmlLetDeclaration<'a>,
    ) -> Option<R3Node<'a>> {
        // Extract the value expression text from source and parse it
        let value_span = decl.value_span;
        let mut had_parse_errors = false;
        let value = if value_span.start < value_span.end
            && (value_span.end as usize) <= self.source_text.len()
        {
            let value_str = &self.source_text[value_span.start as usize..value_span.end as usize];
            let result = self.binding_parser.parse_binding(value_str, value_span);
            // Collect any parse errors
            had_parse_errors = !result.errors.is_empty();
            for error in result.errors {
                self.errors.push(error);
            }
            result.ast
        } else {
            self.create_empty_expression(value_span)
        };

        // Validate that @let value is not empty
        // Angular reports: "@let declaration value cannot be empty" (r3_template_transform.ts:351-364)
        if !had_parse_errors && matches!(value, crate::ast::expression::AngularExpression::Empty(_))
        {
            self.report_error("@let declaration value cannot be empty", decl.value_span);
        }

        let r3_decl = R3LetDeclaration {
            name: decl.name.clone(),
            value,
            source_span: decl.span,
            name_span: decl.name_span,
            value_span: decl.value_span,
        };
        Some(R3Node::LetDeclaration(Box::new_in(r3_decl, self.allocator)))
    }

    /// Creates an i18n BlockPlaceholder for a control flow block when inside an i18n context.
    /// Returns Some if we're inside an i18n region (i18n_depth > 0), None otherwise.
    /// Ported from Angular's i18n_parser.ts:visitBlock which creates BlockPlaceholder nodes.
    fn create_block_placeholder(
        &mut self,
        block_name: &str,
        parameters: &[Atom<'a>],
        source_span: Span,
        start_source_span: Span,
        end_source_span: Option<Span>,
    ) -> Option<I18nMeta<'a>> {
        // Only create placeholders when inside an i18n context
        if self.i18n_depth == 0 {
            return None;
        }

        // Generate unique placeholder names (following Angular's placeholder naming convention)
        // Angular uses START_BLOCK_IF, START_BLOCK_IF_1, etc.
        let block_upper = block_name.to_uppercase().replace(' ', "_");
        let count = self.block_placeholder_counter;
        self.block_placeholder_counter += 1;

        let start_name = if count == 0 {
            Atom::from_in(format!("START_BLOCK_{}", block_upper).as_str(), self.allocator)
        } else {
            Atom::from_in(format!("START_BLOCK_{}_{}", block_upper, count).as_str(), self.allocator)
        };

        let close_name = if count == 0 {
            Atom::from_in(format!("CLOSE_BLOCK_{}", block_upper).as_str(), self.allocator)
        } else {
            Atom::from_in(format!("CLOSE_BLOCK_{}_{}", block_upper, count).as_str(), self.allocator)
        };

        // Convert parameters to Atom vec
        let mut params = Vec::new_in(self.allocator);
        for p in parameters {
            params.push(p.clone());
        }

        let placeholder = I18nBlockPlaceholder {
            name: Atom::from_in(block_name, self.allocator),
            parameters: params,
            start_name,
            close_name,
            children: Vec::new_in(self.allocator),
            source_span,
            start_source_span: Some(start_source_span),
            end_source_span,
        };

        Some(I18nMeta::BlockPlaceholder(placeholder))
    }

    /// Visits an @if block with connected @else/@else if blocks.
    fn visit_if_block(
        &mut self,
        block: &HtmlBlock<'a>,
        connected_blocks: &[&HtmlBlock<'a>],
    ) -> Option<R3Node<'a>> {
        let mut branches = Vec::new_in(self.allocator);

        // Parse the main @if branch parameters (condition and optional "as" alias)
        let main_params = parse_conditional_params(
            self.allocator,
            &block.parameters,
            &self.binding_parser,
            block.start_span,
            "if",
        );

        // Report any parse errors
        for error in main_params.errors {
            self.errors.push(crate::util::ParseError {
                span: crate::util::ParseSourceSpan::new(
                    crate::util::ParseLocation::new(
                        std::sync::Arc::new(crate::util::ParseSourceFile::new(
                            self.source_text,
                            "template",
                        )),
                        block.span.start,
                        0,
                        0,
                    ),
                    crate::util::ParseLocation::new(
                        std::sync::Arc::new(crate::util::ParseSourceFile::new(
                            self.source_text,
                            "template",
                        )),
                        block.span.end,
                        0,
                        0,
                    ),
                ),
                msg: error,
                level: crate::util::ParseErrorLevel::Error,
            });
        }

        let children = self.visit_children(&block.children);

        // Create i18n placeholder if inside an i18n context
        let i18n =
            self.create_block_placeholder("if", &[], block.span, block.start_span, block.end_span);

        let main_branch = R3IfBlockBranch {
            expression: main_params.expression.map(|e| e.ast),
            children,
            expression_alias: main_params.expression_alias,
            source_span: block.span,
            start_source_span: block.start_span,
            end_source_span: block.end_span,
            name_span: block.name_span,
            i18n,
        };
        branches.push(main_branch);

        // Validate connected blocks and process @else if and @else blocks
        let mut has_else = false;

        for (i, connected) in connected_blocks.iter().enumerate() {
            let children = self.visit_children(&connected.children);

            match connected.block_type {
                BlockType::ElseIf => {
                    // Parse @else if parameters (condition and optional "as" alias)
                    let params = parse_conditional_params(
                        self.allocator,
                        &connected.parameters,
                        &self.binding_parser,
                        connected.start_span,
                        "else if",
                    );

                    // Report any parse errors
                    for error in params.errors {
                        self.errors.push(crate::util::ParseError {
                            span: crate::util::ParseSourceSpan::new(
                                crate::util::ParseLocation::new(
                                    std::sync::Arc::new(crate::util::ParseSourceFile::new(
                                        self.source_text,
                                        "template",
                                    )),
                                    connected.span.start,
                                    0,
                                    0,
                                ),
                                crate::util::ParseLocation::new(
                                    std::sync::Arc::new(crate::util::ParseSourceFile::new(
                                        self.source_text,
                                        "template",
                                    )),
                                    connected.span.end,
                                    0,
                                    0,
                                ),
                            ),
                            msg: error,
                            level: crate::util::ParseErrorLevel::Error,
                        });
                    }

                    // Create i18n placeholder if inside an i18n context
                    let i18n = self.create_block_placeholder(
                        "else if",
                        &[],
                        connected.span,
                        connected.start_span,
                        connected.end_span,
                    );

                    let branch = R3IfBlockBranch {
                        expression: params.expression.map(|e| e.ast),
                        children,
                        expression_alias: params.expression_alias,
                        source_span: connected.span,
                        start_source_span: connected.start_span,
                        end_source_span: connected.end_span,
                        name_span: connected.name_span,
                        i18n,
                    };
                    branches.push(branch);
                }
                BlockType::Else => {
                    // Validation: check for duplicate @else
                    if has_else {
                        self.report_error(
                            "Conditional can only have one @else block",
                            connected.start_span,
                        );
                    }
                    // Validation: @else must be the last block
                    else if connected_blocks.len() > 1 && i < connected_blocks.len() - 1 {
                        self.report_error(
                            "@else block must be last inside the conditional",
                            connected.start_span,
                        );
                    }
                    // Validation: @else cannot have parameters
                    else if !connected.parameters.is_empty() {
                        self.report_error(
                            "@else block cannot have parameters",
                            connected.start_span,
                        );
                    }
                    has_else = true;

                    // Create i18n placeholder if inside an i18n context
                    let i18n = self.create_block_placeholder(
                        "else",
                        &[],
                        connected.span,
                        connected.start_span,
                        connected.end_span,
                    );

                    // @else has no condition (null expression) and no alias
                    let branch = R3IfBlockBranch {
                        expression: None,
                        children,
                        expression_alias: None,
                        source_span: connected.span,
                        start_source_span: connected.start_span,
                        end_source_span: connected.end_span,
                        name_span: connected.name_span,
                        i18n,
                    };
                    branches.push(branch);
                }
                _ => {
                    // Unrecognized block
                    self.report_error(
                        &format!("Unrecognized conditional block @{}", connected.name.as_str()),
                        connected.start_span,
                    );
                }
            }
        }

        // Calculate the outer span to encompass all branches
        let end_source_span =
            if let Some(last) = connected_blocks.last() { last.end_span } else { block.end_span };

        let source_span = if let Some(last) = connected_blocks.last() {
            Span::new(block.span.start, last.span.end)
        } else {
            block.span
        };

        let if_block = R3IfBlock {
            branches,
            source_span,
            start_source_span: block.start_span,
            end_source_span,
            name_span: block.name_span,
        };
        Some(R3Node::IfBlock(Box::new_in(if_block, self.allocator)))
    }

    /// Visits a @for block with connected @empty block.
    fn visit_for_block(
        &mut self,
        block: &HtmlBlock<'a>,
        connected_blocks: &[&HtmlBlock<'a>],
    ) -> Option<R3Node<'a>> {
        use crate::ast::r3::R3ForLoopBlockEmpty;
        use crate::transform::control_flow::parse_for_loop_parameters;

        // Parse loop parameters using the control flow parser
        let params = parse_for_loop_parameters(
            self.allocator,
            &block.parameters,
            &self.binding_parser,
            block.start_span,
        );

        // Add any parse errors
        for error in params.errors {
            self.errors.push(crate::util::ParseError {
                span: crate::util::ParseSourceSpan::new(
                    crate::util::ParseLocation::new(
                        std::sync::Arc::new(crate::util::ParseSourceFile::new(
                            self.source_text,
                            "template",
                        )),
                        block.span.start,
                        0,
                        0,
                    ),
                    crate::util::ParseLocation::new(
                        std::sync::Arc::new(crate::util::ParseSourceFile::new(
                            self.source_text,
                            "template",
                        )),
                        block.span.end,
                        0,
                        0,
                    ),
                ),
                msg: error,
                level: crate::util::ParseErrorLevel::Error,
            });
        }

        let item = params.item;
        let expression = params.expression;
        let context_variables = params.context_variables;

        // Get track expression or create empty one for error recovery
        let (track_by, track_keyword_span) = if let Some(track_info) = params.track_by {
            (track_info.expression, track_info.keyword_span)
        } else {
            // Track is required but missing - report error and create empty for error recovery
            self.report_error("@for loop must have a \"track\" expression", block.start_span);
            let empty_ast = ASTWithSource {
                ast: self.create_empty_expression(block.span),
                source: None,
                location: Atom::from(""),
                absolute_offset: block.span.start,
            };
            (empty_ast, block.name_span)
        };

        let children = self.visit_children(&block.children);

        // Process and validate connected @empty block
        let mut empty: Option<R3ForLoopBlockEmpty<'a>> = None;

        for connected in connected_blocks {
            match connected.block_type {
                BlockType::Empty => {
                    // Validation: check for duplicate @empty
                    if empty.is_some() {
                        self.report_error(
                            "@for loop can only have one @empty block",
                            connected.span,
                        );
                    }
                    // Validation: @empty cannot have parameters
                    else if !connected.parameters.is_empty() {
                        self.report_error("@empty block cannot have parameters", connected.span);
                    } else {
                        let empty_children = self.visit_children(&connected.children);
                        // Create i18n placeholder for @empty block if inside i18n context
                        let empty_i18n = self.create_block_placeholder(
                            "empty",
                            &[],
                            connected.span,
                            connected.start_span,
                            connected.end_span,
                        );
                        empty = Some(R3ForLoopBlockEmpty {
                            children: empty_children,
                            source_span: connected.span,
                            start_source_span: connected.start_span,
                            end_source_span: connected.end_span,
                            name_span: connected.name_span,
                            i18n: empty_i18n,
                        });
                    }
                }
                _ => {
                    // Unrecognized block
                    self.report_error(
                        &format!("Unrecognized @for loop block \"{}\"", connected.name.as_str()),
                        connected.span,
                    );
                }
            }
        }

        // Calculate the outer span to encompass @empty block if present
        let end_source_span =
            if let Some(last) = connected_blocks.last() { last.end_span } else { block.end_span };

        let source_span = if let Some(last) = connected_blocks.last() {
            Span::new(block.span.start, last.span.end)
        } else {
            block.span
        };

        // Create i18n placeholder for @for block if inside i18n context
        let i18n = self.create_block_placeholder(
            "for",
            &[],
            source_span,
            block.start_span,
            end_source_span,
        );

        let for_block = R3ForLoopBlock {
            item,
            expression,
            track_by,
            track_keyword_span,
            context_variables,
            children,
            empty,
            source_span,
            main_block_span: block.span,
            start_source_span: block.start_span,
            end_source_span,
            name_span: block.name_span,
            i18n,
        };
        Some(R3Node::ForLoopBlock(Box::new_in(for_block, self.allocator)))
    }

    /// Visits a @switch block.
    ///
    /// This implements case grouping logic from Angular's `createSwitchBlock` in r3_control_flow.ts.
    /// Consecutive cases without bodies are grouped together into a single `SwitchBlockCaseGroup`.
    fn visit_switch_block(&mut self, block: &HtmlBlock<'a>) -> Option<R3Node<'a>> {
        use crate::ast::html::{BlockType, HtmlNode};
        use crate::ast::r3::{R3SwitchBlockCase, R3SwitchBlockCaseGroup};

        // Validation: @switch must have exactly one parameter
        let expression = if block.parameters.len() != 1 {
            self.report_error("@switch block must have exactly one parameter", block.start_span);
            self.create_empty_expression(block.span)
        } else {
            let expr_str = block.parameters[0].expression.as_str();
            let parsed = self.binding_parser.parse_binding(expr_str, block.parameters[0].span);
            parsed.ast
        };

        let mut groups = Vec::new_in(self.allocator);
        let mut unknown_blocks = Vec::new_in(self.allocator);
        let mut collected_cases: std::vec::Vec<R3SwitchBlockCase<'a>> = std::vec::Vec::new();
        let mut first_case_start: Option<Span> = None;

        for child in &block.children {
            // Skip non-block nodes (only process block children)
            let child_block = match child {
                HtmlNode::Block(b) => b,
                _ => continue,
            };

            // Check if this is a valid case/default block
            // Note: @case with no parameters is treated as unknown (same as Angular)
            let is_case =
                child_block.block_type == BlockType::Case && !child_block.parameters.is_empty();
            let is_default = child_block.block_type == BlockType::Default;

            if !is_case && !is_default {
                unknown_blocks.push(crate::ast::r3::R3UnknownBlock {
                    name: child_block.name.clone(),
                    source_span: child_block.span,
                    name_span: child_block.name_span,
                });
                continue;
            }

            // Parse expression for @case blocks
            let case_expression = if is_case {
                let expr_str = child_block.parameters[0].expression.as_str();
                let parsed =
                    self.binding_parser.parse_binding(expr_str, child_block.parameters[0].span);
                Some(parsed.ast)
            } else {
                None
            };

            // Create the SwitchBlockCase (without children - those go in the group)
            let switch_case = R3SwitchBlockCase {
                expression: case_expression,
                source_span: child_block.span,
                start_source_span: child_block.start_span,
                end_source_span: child_block.end_span,
                name_span: child_block.name_span,
            };
            collected_cases.push(switch_case);

            // Check if this case has an empty body (fallthrough case)
            // Empty body = no children AND end_span is zero-length (start == end)
            let case_without_body = child_block.children.is_empty()
                && child_block.end_span.is_some_and(|end_span| end_span.start == end_span.end);

            if case_without_body {
                // Track the start of the first case in this group
                if first_case_start.is_none() {
                    first_case_start = Some(child_block.span);
                }
                // Continue collecting cases until we find one with a body
                continue;
            }

            // This case has a body - create a group with all collected cases
            let (group_source_span, group_start_source_span) =
                if let Some(first_start) = first_case_start {
                    // Group spans from first case to end of this case
                    (
                        Span::new(first_start.start, child_block.span.end),
                        Span::new(first_start.start, child_block.start_span.end),
                    )
                } else {
                    // Single case - use its spans
                    (child_block.span, child_block.start_span)
                };

            // Move collected cases into allocator vector
            let mut cases = Vec::new_in(self.allocator);
            for case in collected_cases.drain(..) {
                cases.push(case);
            }

            // Visit children of the final case (which has the body)
            let children = self.visit_children(&child_block.children);

            let group = R3SwitchBlockCaseGroup {
                cases,
                children,
                source_span: group_source_span,
                start_source_span: group_start_source_span,
                end_source_span: child_block.end_span,
                name_span: child_block.name_span,
                i18n: None,
            };
            groups.push(group);

            // Reset for next group
            first_case_start = None;
        }

        let switch_block = R3SwitchBlock {
            expression,
            groups,
            unknown_blocks,
            source_span: block.span,
            start_source_span: block.start_span,
            end_source_span: block.end_span,
            name_span: block.name_span,
        };
        Some(R3Node::SwitchBlock(Box::new_in(switch_block, self.allocator)))
    }

    /// Visits a @defer block with connected @placeholder, @loading, @error blocks.
    fn visit_defer_block(
        &mut self,
        block: &HtmlBlock<'a>,
        connected_blocks: &[&HtmlBlock<'a>],
    ) -> Option<R3Node<'a>> {
        use crate::ast::r3::{
            R3DeferredBlockError, R3DeferredBlockLoading, R3DeferredBlockPlaceholder,
        };

        let children = self.visit_children(&block.children);

        // Parse defer triggers from block parameters
        let trigger_result =
            parse_defer_triggers(self.allocator, &block.parameters, &self.binding_parser);

        // Report trigger parsing errors via diagnostics
        for error in trigger_result.errors {
            self.errors.push(crate::util::ParseError {
                span: crate::util::ParseSourceSpan::new(
                    crate::util::ParseLocation::new(
                        std::sync::Arc::new(crate::util::ParseSourceFile::new(
                            self.source_text,
                            "template",
                        )),
                        block.span.start,
                        0,
                        0,
                    ),
                    crate::util::ParseLocation::new(
                        std::sync::Arc::new(crate::util::ParseSourceFile::new(
                            self.source_text,
                            "template",
                        )),
                        block.span.end,
                        0,
                        0,
                    ),
                ),
                msg: error,
                level: crate::util::ParseErrorLevel::Error,
            });
        }

        // Process connected blocks
        let mut placeholder = None;
        let mut loading = None;
        let mut error = None;

        for connected in connected_blocks {
            let connected_children = self.visit_children(&connected.children);

            match connected.block_type {
                BlockType::Placeholder => {
                    // Parse minimum time from parameters like "minimum 500ms"
                    let params: std::vec::Vec<&str> =
                        connected.parameters.iter().map(|p| p.expression.as_str()).collect();
                    let minimum_time = parse_placeholder_parameters(&params);

                    placeholder = Some(R3DeferredBlockPlaceholder {
                        children: connected_children,
                        minimum_time,
                        source_span: connected.span,
                        name_span: connected.name_span,
                        start_source_span: connected.start_span,
                        end_source_span: connected.end_span,
                        i18n: None,
                    });
                }
                BlockType::Loading => {
                    // Parse after and minimum time from parameters like "after 100ms; minimum 1s"
                    let params: std::vec::Vec<&str> =
                        connected.parameters.iter().map(|p| p.expression.as_str()).collect();
                    let (after_time, minimum_time) = parse_loading_parameters(&params);

                    loading = Some(R3DeferredBlockLoading {
                        children: connected_children,
                        after_time,
                        minimum_time,
                        source_span: connected.span,
                        name_span: connected.name_span,
                        start_source_span: connected.start_span,
                        end_source_span: connected.end_span,
                        i18n: None,
                    });
                }
                BlockType::Error => {
                    error = Some(R3DeferredBlockError {
                        children: connected_children,
                        source_span: connected.span,
                        name_span: connected.name_span,
                        start_source_span: connected.start_span,
                        end_source_span: connected.end_span,
                        i18n: None,
                    });
                }
                _ => {}
            }
        }

        // Calculate the outer span to encompass all connected blocks
        let end_source_span =
            if let Some(last) = connected_blocks.last() { last.end_span } else { block.end_span };

        let source_span = if let Some(last) = connected_blocks.last() {
            Span::new(block.span.start, last.span.end)
        } else {
            block.span
        };

        let defer_block = R3DeferredBlock {
            children,
            triggers: trigger_result.triggers,
            prefetch_triggers: trigger_result.prefetch_triggers,
            hydrate_triggers: trigger_result.hydrate_triggers,
            placeholder,
            loading,
            error,
            source_span,
            main_block_span: block.span,
            name_span: block.name_span,
            start_source_span: block.start_span,
            end_source_span,
            i18n: None,
        };
        Some(R3Node::DeferredBlock(Box::new_in(defer_block, self.allocator)))
    }

    /// Visits all children of a node (uses sibling-aware traversal).
    fn visit_children(&mut self, children: &[HtmlNode<'a>]) -> Vec<'a, R3Node<'a>> {
        self.visit_siblings(children)
    }

    /// Parses element attributes into their respective categories.
    #[expect(clippy::type_complexity)]
    fn parse_attributes(
        &mut self,
        attrs: &[HtmlAttribute<'a>],
        element_name: &str,
        is_template: bool,
    ) -> (
        Vec<'a, R3TextAttribute<'a>>,  // Static attributes
        Vec<'a, R3BoundAttribute<'a>>, // Inputs
        Vec<'a, R3BoundEvent<'a>>,     // Outputs
        Vec<'a, R3Reference<'a>>,      // References
        Vec<'a, R3Variable<'a>>,       // Variables
        Option<TemplateAttrInfo<'a>>,  // Template attribute info
    ) {
        let mut attributes = Vec::new_in(self.allocator);
        let mut inputs = Vec::new_in(self.allocator);
        let mut outputs = Vec::new_in(self.allocator);
        let mut references = Vec::new_in(self.allocator);
        let mut variables = Vec::new_in(self.allocator);
        let mut template_attr_info: Option<TemplateAttrInfo<'a>> = None;

        // First pass: collect i18n-* attribute metadata
        // Maps attribute name (without i18n- prefix) to its i18n metadata
        let mut i18n_attrs_meta: std::collections::HashMap<&str, I18nMeta<'a>> =
            std::collections::HashMap::new();
        for attr in attrs {
            let name = attr.name.as_str();
            if let Some(target_attr) = name.strip_prefix("i18n-") {
                let instance_id = self.allocate_i18n_message_instance_id();
                let meta = parse_i18n_meta(self.allocator, attr.value.as_str(), instance_id);
                i18n_attrs_meta.insert(target_attr, meta);
            }
        }

        for attr in attrs {
            let raw_name = attr.name.as_str();
            // Normalize name early (case-insensitive data- prefix stripping)
            // This must happen before ANY binding syntax checks
            let name = self.normalize_attribute_name(raw_name);

            // Skip i18n-* attributes early - they are metadata for other attributes, not bindings.
            // In Angular's TypeScript compiler, these are filtered out by I18nMetaVisitor before
            // r3_template_transform runs. The metadata was already collected in the first pass above.
            if name == "i18n" || name.starts_with("i18n-") {
                continue;
            }

            // Inside ngNonBindable, all bindings become text attributes.
            // The only exception is ngNonBindable itself which is handled elsewhere.
            // TypeScript NonBindableVisitor.visitElement does this transformation.
            if self.non_bindable_depth > 0 && raw_name != "ngNonBindable" {
                attributes.push(R3TextAttribute {
                    name: attr.name.clone(),
                    value: attr.value.clone(),
                    source_span: attr.span,
                    key_span: Some(attr.name_span),
                    value_span: attr.value_span,
                    i18n: None,
                });
                continue;
            }

            // Check for structural directive (*ngFor, *ngIf, etc.)
            if name.starts_with(TEMPLATE_ATTR_PREFIX) {
                if template_attr_info.is_some() {
                    self.report_error(
                        "Can't have multiple template bindings on one element. Use only one attribute prefixed with *",
                        attr.span,
                    );
                } else {
                    template_attr_info = Some(TemplateAttrInfo {
                        name: attr.name.clone(),
                        value: attr.value.clone(),
                        span: attr.span,
                        name_span: attr.name_span,
                        value_span: attr.value_span,
                    });
                }
                continue;
            }

            // Check for binding prefixes (normalization already applied to name)
            if let Some((prefix, rest)) = self.parse_binding_prefix(name) {
                match prefix {
                    BindingPrefix::Bind => {
                        // Check for animation bindings: bind-@prop or bind-animate-prop
                        if let Some(anim_name) = rest.strip_prefix('@') {
                            inputs.push(self.create_bound_attribute(
                                element_name,
                                anim_name,
                                attr,
                                BindingType::Animation,
                                None,
                            ));
                        } else if let Some(anim_name) = rest.strip_prefix("animate-") {
                            inputs.push(self.create_bound_attribute(
                                element_name,
                                anim_name,
                                attr,
                                BindingType::Animation,
                                None,
                            ));
                        } else {
                            let i18n = i18n_attrs_meta.remove(rest);
                            let mut bound_attr = self.create_bound_attribute(
                                element_name,
                                rest,
                                attr,
                                BindingType::Property,
                                None,
                            );
                            bound_attr.i18n = i18n;
                            inputs.push(bound_attr);
                        }
                    }
                    BindingPrefix::Let => {
                        if is_template {
                            // Validate and create variable
                            if let Some(var) = self.create_validated_variable(rest, attr) {
                                variables.push(var);
                            }
                        } else {
                            // let- is only supported on ng-template elements
                            self.report_error(
                                "\"let-\" is only supported on ng-template elements.",
                                attr.span,
                            );
                        }
                    }
                    BindingPrefix::Ref => {
                        // Validate and create reference (with duplicate detection)
                        if let Some(reference) =
                            self.create_validated_reference(rest, attr, &references)
                        {
                            references.push(reference);
                        }
                    }
                    BindingPrefix::On => {
                        outputs.push(self.create_bound_event(rest, attr));
                    }
                    BindingPrefix::BindOn => {
                        // Two-way binding creates both input and output
                        inputs.push(self.create_bound_attribute(
                            element_name,
                            rest,
                            attr,
                            BindingType::TwoWay,
                            None,
                        ));
                        // Create the output event (propChange)
                        let event_name = format!("{}Change", rest);
                        outputs.push(self.create_two_way_event(&event_name, attr));
                    }
                    BindingPrefix::At => {
                        // Legacy animation trigger syntax (@prop)
                        // If there's a value, report an error - expressions require [@prop]
                        if !attr.value.is_empty() {
                            self.report_error(
                                "Assigning animation triggers via @prop=\"exp\" attributes with an expression is invalid. Use property bindings (e.g. [@prop]=\"exp\") or use an attribute without a value (e.g. @prop) instead.",
                                attr.span,
                            );
                        }
                        // Still create as a legacy animation input for error recovery
                        // Use create_legacy_animation_attribute to default empty values to "undefined"
                        inputs.push(self.create_legacy_animation_attribute(
                            element_name,
                            rest,
                            attr,
                        ));
                    }
                }
                continue;
            }

            // Check for banana-in-a-box [(prop)]
            if name.starts_with(BANANA_BOX_START) && name.ends_with(BANANA_BOX_END) {
                let prop_name = &name[2..name.len() - 2];
                // Two-way binding creates both input and output
                inputs.push(self.create_bound_attribute(
                    element_name,
                    prop_name,
                    attr,
                    BindingType::TwoWay,
                    None,
                ));
                // Create the output event (propChange)
                let event_name = format!("{}Change", prop_name);
                outputs.push(self.create_two_way_event(&event_name, attr));
                continue;
            }

            // Check for property binding [prop]
            if name.starts_with(PROPERTY_START) && name.ends_with(PROPERTY_END) {
                let prop_name = &name[1..name.len() - 1];
                let (binding_type, final_name, unit) =
                    if let Some(stripped) = prop_name.strip_prefix("attr.") {
                        (BindingType::Attribute, stripped, None)
                    } else if let Some(stripped) = prop_name.strip_prefix("class.") {
                        (BindingType::Class, stripped, None)
                    } else if let Some(stripped) = prop_name.strip_prefix("style.") {
                        // Check for unit suffix: style.width.px -> property="width", unit="px"
                        if let Some(dot_pos) = stripped.find('.') {
                            let prop = &stripped[..dot_pos];
                            let unit_str = &stripped[dot_pos + 1..];
                            (BindingType::Style, prop, Some(unit_str))
                        } else {
                            (BindingType::Style, stripped, None)
                        }
                    } else if prop_name.starts_with("animate.") {
                        // [animate.xxx] keeps the full name "animate.xxx"
                        (BindingType::Animation, prop_name, None)
                    } else if prop_name.starts_with('@') {
                        // [@xxx] is legacy animation binding - uses ɵɵproperty
                        // TypeScript: isLegacyAnimationLabel checks name[0] == '@'
                        (BindingType::LegacyAnimation, prop_name, None)
                    } else {
                        (BindingType::Property, prop_name, None)
                    };
                // Look up i18n metadata for this property binding (e.g., i18n-heading for [heading])
                // Ported from Angular's categorizePropertyAttributes in r3_template_transform.ts
                let i18n = i18n_attrs_meta.remove(final_name);
                let mut bound_attr =
                    self.create_bound_attribute(element_name, final_name, attr, binding_type, unit);
                bound_attr.i18n = i18n;
                inputs.push(bound_attr);
                continue;
            }

            // Check for event binding (event)
            if name.starts_with(EVENT_START) && name.ends_with(EVENT_END) {
                let event_name = &name[1..name.len() - 1];
                // Check for legacy animation syntax (@) or (@name)
                if event_name.starts_with('@') {
                    let anim_name = event_name.strip_prefix('@').unwrap_or("");
                    outputs.push(self.create_legacy_animation_event(anim_name, attr));
                } else {
                    outputs.push(self.create_bound_event_with_target(event_name, attr));
                }
                continue;
            }

            // Check for reference #ref
            if name.starts_with('#') {
                if let Some(reference) =
                    self.create_validated_reference(&name[1..], attr, &references)
                {
                    references.push(reference);
                }
                continue;
            }

            // NOTE: animate.* without brackets is a TextAttribute, NOT a BoundAttribute.
            // Only [animate.xxx] or (animate.xxx) are special bindings.

            // Check for interpolation in attribute value - convert to BoundAttribute
            // Note: name is already normalized at the start of the loop
            if self.has_interpolation(attr.value.as_str()) {
                // Look up i18n metadata for this attribute (e.g., i18n-title for title="{{ name }}")
                let i18n = i18n_attrs_meta.remove(name);
                if let Some(bound_attr) =
                    self.create_interpolated_bound_attribute(element_name, name, attr, i18n)
                {
                    inputs.push(bound_attr);
                    continue;
                }
            }

            // Static attribute - look up i18n metadata
            let i18n = i18n_attrs_meta.remove(name);
            attributes.push(self.create_text_attribute_with_i18n(attr, i18n));
        }

        (attributes, inputs, outputs, references, variables, template_attr_info)
    }

    /// Normalizes an attribute name by stripping the data- prefix (case-insensitive).
    /// This matches TypeScript's behavior: /^data-/i.test(attrName) ? attrName.substring(5) : attrName
    fn normalize_attribute_name<'b>(&self, name: &'b str) -> &'b str {
        // Case-insensitive data- prefix stripping
        if name.len() > 5 && name[..5].eq_ignore_ascii_case("data-") { &name[5..] } else { name }
    }

    /// Parses a binding prefix from an attribute name.
    /// Handles the data- prefix as per Angular's normalization:
    /// `data-bind-*`, `data-on-*`, `data-ref-*`, `data-let-*`, `data-bindon-*`
    fn parse_binding_prefix<'b>(&self, name: &'b str) -> Option<(BindingPrefix, &'b str)> {
        // Strip data- prefix if present (case-insensitive)
        let name = self.normalize_attribute_name(name);

        for (prefix, kind) in BIND_NAME_PREFIXES {
            if let Some(rest) = name.strip_prefix(prefix) {
                return Some((*kind, rest));
            }
        }
        None
    }

    /// Creates a text attribute with optional i18n metadata.
    fn create_text_attribute_with_i18n(
        &self,
        attr: &HtmlAttribute<'a>,
        i18n: Option<I18nMeta<'a>>,
    ) -> R3TextAttribute<'a> {
        R3TextAttribute {
            name: attr.name.clone(),
            value: attr.value.clone(),
            source_span: attr.span,
            key_span: Some(attr.name_span),
            value_span: attr.value_span,
            i18n,
        }
    }

    /// Creates a bound attribute.
    fn create_bound_attribute(
        &mut self,
        element_name: &str,
        property_name: &str,
        attr: &HtmlAttribute<'a>,
        binding_type: BindingType,
        unit: Option<&str>,
    ) -> R3BoundAttribute<'a> {
        let value_span = attr.value_span.unwrap_or(attr.span);
        let value = self.parse_binding_expression(&attr.value, value_span);
        let name_atom = Atom::from(self.allocator.alloc_str(property_name));

        // Look up security context based on element and property
        let security_context = get_security_context(element_name, property_name);

        R3BoundAttribute {
            name: name_atom,
            binding_type,
            security_context,
            value,
            unit: unit.map(|u| Atom::from(self.allocator.alloc_str(u))),
            source_span: attr.span,
            key_span: attr.name_span,
            value_span: attr.value_span,
            i18n: None,
        }
    }

    /// Parses a binding expression from an attribute value.
    fn parse_binding_expression(&mut self, value: &Atom<'a>, span: Span) -> AngularExpression<'a> {
        let value_str = value.as_str();
        if value_str.is_empty() {
            return self.create_empty_expression(span);
        }

        let result = self.binding_parser.parse_binding(value_str, span);

        // Collect any parse errors
        for error in result.errors {
            self.errors.push(error);
        }

        result.ast
    }

    /// Creates a bound attribute for legacy animation triggers (@prop).
    /// When there's no expression, uses "undefined" as the default value.
    /// TypeScript reference: binding_parser.ts _parseLegacyAnimation (line 505-506)
    fn create_legacy_animation_attribute(
        &mut self,
        _element_name: &str,
        property_name: &str,
        attr: &HtmlAttribute<'a>,
    ) -> R3BoundAttribute<'a> {
        let value_span = attr.value_span.unwrap_or(attr.span);
        // For legacy animation triggers, parse "undefined" when there's no expression.
        // This matches TypeScript's behavior: `expression || 'undefined'`
        let value_str = if attr.value.is_empty() { "undefined" } else { attr.value.as_str() };
        let result = self.binding_parser.parse_binding(value_str, value_span);

        // Collect any parse errors
        for error in result.errors {
            self.errors.push(error);
        }

        let name_atom = Atom::from(self.allocator.alloc_str(property_name));

        R3BoundAttribute {
            name: name_atom,
            binding_type: BindingType::LegacyAnimation,
            security_context: crate::ast::r3::SecurityContext::None,
            value: result.ast,
            unit: None,
            source_span: attr.span,
            key_span: attr.name_span,
            value_span: attr.value_span,
            i18n: None,
        }
    }

    /// Creates a bound event with target parsing.
    /// Parses target:event syntax (e.g., window:click -> target=window, event=click)
    /// Detects animation events (animate.xxx -> Animation type)
    fn create_bound_event_with_target(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
    ) -> R3BoundEvent<'a> {
        let handler_span = self.calculate_event_handler_span(attr);
        let handler = self.parse_event_expression(&attr.value, handler_span);

        // Check for animation events (animate.xxx)
        let event_type = if name.starts_with("animate.") {
            ParsedEventType::Animation
        } else {
            ParsedEventType::Regular
        };

        // Parse target:event syntax
        let (event_name, target) = if let Some(colon_pos) = name.find(':') {
            let target = &name[..colon_pos];
            let event = &name[colon_pos + 1..];
            (event, Some(Atom::from(self.allocator.alloc_str(target))))
        } else {
            (name, None)
        };

        let name_atom = Atom::from(self.allocator.alloc_str(event_name));

        R3BoundEvent {
            name: name_atom,
            event_type,
            handler,
            target,
            phase: None,
            source_span: attr.span,
            handler_span,
            key_span: attr.name_span,
        }
    }

    /// Creates a bound event (without target parsing).
    fn create_bound_event(&mut self, name: &str, attr: &HtmlAttribute<'a>) -> R3BoundEvent<'a> {
        let handler_span = self.calculate_event_handler_span(attr);
        let handler = self.parse_event_expression(&attr.value, handler_span);
        let name_atom = Atom::from(self.allocator.alloc_str(name));

        R3BoundEvent {
            name: name_atom,
            event_type: ParsedEventType::Regular,
            handler,
            target: None,
            phase: None,
            source_span: attr.span,
            handler_span,
            key_span: attr.name_span,
        }
    }

    /// Creates a legacy animation event binding for `(@name)` or `(@name.phase)` syntax.
    ///
    /// Parses the event name to extract the animation trigger name and optional phase.
    /// Per TypeScript's `parseLegacyAnimationEventName` in binding_parser.ts:
    /// - `openClose.start` -> eventName: "openClose", phase: "start"
    /// - `openClose.done` -> eventName: "openClose", phase: "done"
    /// - `openClose` -> eventName: "openClose", phase: null
    fn create_legacy_animation_event(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
    ) -> R3BoundEvent<'a> {
        let handler_span = self.calculate_event_handler_span(attr);
        let handler = self.parse_event_expression(&attr.value, handler_span);

        // Split the name at the period to extract event name and phase
        // Per TypeScript's parseLegacyAnimationEventName: splitAtPeriod(rawName, [rawName, null])
        // returns {eventName: matches[0], phase: matches[1]}
        // e.g., "openClose.done" -> eventName: "openClose", phase: "done"
        let (event_name, phase) = if let Some(dot_pos) = name.find('.') {
            let event_name = &name[..dot_pos];
            let phase_str = &name[dot_pos + 1..];
            // Phase is lowercased per TypeScript: matches[1].toLowerCase()
            let phase_lower = phase_str.to_lowercase();
            (event_name, Some(Atom::from(self.allocator.alloc_str(&phase_lower))))
        } else {
            (name, None)
        };

        // Use only the event name (trigger name), not including the phase
        let name_atom = Atom::from(self.allocator.alloc_str(event_name));

        R3BoundEvent {
            name: name_atom,
            event_type: ParsedEventType::LegacyAnimation,
            handler,
            target: None,
            phase,
            source_span: attr.span,
            handler_span,
            key_span: attr.name_span,
        }
    }

    /// Creates a bound event for two-way binding.
    /// The handler is just the target expression (e.g., `name`), not the full assignment.
    /// The `$event` is added separately in the ingest phase.
    fn create_two_way_event(&mut self, name: &str, attr: &HtmlAttribute<'a>) -> R3BoundEvent<'a> {
        let handler_span = self.calculate_event_handler_span(attr);
        let name_atom = Atom::from(self.allocator.alloc_str(name));

        // For two-way binding, the handler is just the target expression (e.g., `name`)
        // NOT the full assignment `name = $event`.
        // The $event reference and TwoWayBindingSetExpr are created in the ingest phase.
        let handler = self.parse_binding_expression(&attr.value, handler_span);

        R3BoundEvent {
            name: name_atom,
            event_type: ParsedEventType::TwoWay,
            handler,
            target: None,
            phase: None,
            source_span: attr.span,
            handler_span,
            key_span: attr.name_span,
        }
    }

    /// Parses an event handler expression.
    fn parse_event_expression(&mut self, value: &Atom<'a>, span: Span) -> AngularExpression<'a> {
        let value_str = value.as_str();
        if value_str.is_empty() {
            return self.create_empty_expression(span);
        }

        let result = self.binding_parser.parse_event(value_str, span);

        // Collect any parse errors
        for error in result.errors {
            self.errors.push(error);
        }

        result.ast
    }

    /// Calculates the handler span for an event binding, adjusting for comments.
    ///
    /// Angular strips `//` comments from action bindings before parsing.
    /// The handler span should exclude the comment portion and trailing whitespace.
    fn calculate_event_handler_span(&self, attr: &HtmlAttribute<'a>) -> Span {
        let base_span = attr.value_span.unwrap_or(attr.span);
        let value_str = attr.value.as_str();

        // Check if there's a comment to strip
        if let Some(comment_pos) = find_comment_start(value_str) {
            // Get the portion before the comment and trim trailing whitespace
            let before_comment = &value_str[..comment_pos];
            let trimmed_len = before_comment.trim_end().len();
            Span::new(base_span.start, base_span.start + trimmed_len as u32)
        } else {
            base_span
        }
    }

    /// Creates a variable with validation.
    /// Returns None if validation fails (error is reported).
    fn create_validated_variable(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
    ) -> Option<R3Variable<'a>> {
        // Validate variable name
        if name.contains('-') {
            self.report_error("\"-\" is not allowed in variable names", attr.span);
        } else if name.is_empty() {
            self.report_error("Variable does not have a name", attr.span);
        }

        // Still create the variable even if invalid (for error recovery)
        let name_atom = Atom::from(self.allocator.alloc_str(name));
        Some(R3Variable {
            name: name_atom,
            value: attr.value.clone(),
            source_span: attr.span,
            key_span: attr.name_span,
            value_span: attr.value_span,
        })
    }

    /// Creates a reference with validation and duplicate detection.
    /// Returns None if validation fails (error is reported).
    fn create_validated_reference(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
        existing_refs: &Vec<'a, R3Reference<'a>>,
    ) -> Option<R3Reference<'a>> {
        // Validate reference name
        if name.contains('-') {
            self.report_error("\"-\" is not allowed in reference names", attr.span);
        } else if name.is_empty() {
            self.report_error("Reference does not have a name", attr.span);
        } else if existing_refs.iter().any(|r| r.name.as_str() == name) {
            self.report_error(
                &format!("Reference \"#{}\" is defined more than once", name),
                attr.span,
            );
        }

        // Still create the reference even if invalid (for error recovery)
        let name_atom = Atom::from(self.allocator.alloc_str(name));
        Some(R3Reference {
            name: name_atom,
            value: attr.value.clone(),
            source_span: attr.span,
            key_span: attr.name_span,
            value_span: attr.value_span,
        })
    }

    /// Wraps a node in a template for structural directives.
    fn wrap_in_template(
        &mut self,
        node: R3Node<'a>,
        element: &HtmlElement<'a>,
        template_attr: TemplateAttrInfo<'a>,
    ) -> R3Node<'a> {
        use crate::ast::expression::TemplateBinding;
        use crate::ast::r3::SecurityContext;

        // Generate i18n metadata if we're inside an i18n block.
        // When inside an i18n block (i18n_depth > 0), each element gets a TagPlaceholder.
        // The wrapper template inherits this metadata.
        // Reference: r3_template_transform.ts line 1017 and i18n/meta.ts
        let i18n = if self.i18n_depth > 0 {
            Some(self.create_i18n_tag_placeholder_for_element(element))
        } else {
            // Not in i18n block - clone from wrapped node if it has i18n
            self.get_node_i18n(&node).map(|meta| meta.clone_in(self.allocator))
        };

        let mut children = Vec::new_in(self.allocator);
        children.push(node);

        let mut attributes = Vec::new_in(self.allocator);
        let mut inputs = Vec::new_in(self.allocator);
        let mut variables = Vec::new_in(self.allocator);

        // Extract the directive name (strip the * prefix)
        let directive_name: &str =
            template_attr.name.strip_prefix('*').unwrap_or(&template_attr.name);
        // Allocate the directive name in the arena for long-lived reference
        let directive_name = self.allocator.alloc_str(directive_name);
        let directive_name_atom = Atom::from(directive_name);

        // Get the value span (if present)
        let value_span = template_attr.value_span.unwrap_or(template_attr.span);
        // Key span excludes the `*` prefix
        let key_span = Span::new(template_attr.name_span.start + 1, template_attr.name_span.end);

        // Parse template bindings if there's a value
        let value_str = template_attr.value.as_str();
        if !value_str.is_empty() {
            // Parse the structural directive value
            let result = self.binding_parser.parse_template_bindings(
                directive_name,
                value_str,
                key_span,
                value_span,
            );

            // Forward errors from template binding parsing
            for error in result.errors {
                self.report_error(&error, value_span);
            }

            // Forward warnings from template binding parsing
            for warning in result.warnings {
                self.errors.push(crate::util::ParseError {
                    span: crate::util::ParseSourceSpan::new(
                        crate::util::ParseLocation::new(
                            std::sync::Arc::new(crate::util::ParseSourceFile::new(
                                self.source_text,
                                "template",
                            )),
                            value_span.start,
                            0,
                            0,
                        ),
                        crate::util::ParseLocation::new(
                            std::sync::Arc::new(crate::util::ParseSourceFile::new(
                                self.source_text,
                                "template",
                            )),
                            value_span.end,
                            0,
                            0,
                        ),
                    ),
                    msg: warning,
                    level: crate::util::ParseErrorLevel::Warning,
                });
            }

            // Track if we should add a TextAttribute (when value is empty/null for directive)
            let mut first_is_text_attr = false;

            // Check the first binding to determine if we need a TextAttribute or BoundAttribute
            if let Some(first) = result.bindings.first() {
                match first {
                    TemplateBinding::Variable(_) => {
                        first_is_text_attr = true;
                    }
                    TemplateBinding::Expression(expr) => {
                        if expr.key.source.as_str() == directive_name {
                            // If the expression has no value (e.g., "*ngFor="let item...""),
                            // treat it as a TextAttribute
                            if expr.value.is_none() {
                                first_is_text_attr = true;
                            }
                        }
                    }
                }
            }

            // Add TextAttribute or BoundAttribute for the directive
            if first_is_text_attr {
                // Add a TextAttribute for the directive name
                let directive_source_span = Span::new(
                    template_attr.name_span.start + 1, // Skip the *
                    template_attr.name_span.end,
                );
                attributes.push(R3TextAttribute {
                    name: directive_name_atom.clone(),
                    value: Atom::from(""),
                    source_span: directive_source_span,
                    key_span: Some(directive_source_span),
                    value_span: None,
                    i18n: None,
                });
            } else if let Some(TemplateBinding::Expression(expr)) = result.bindings.first() {
                if expr.key.source.as_str() == directive_name {
                    // Add as BoundAttribute with the expression value
                    let source_span =
                        Span::new(expr.source_span.start as u32, expr.source_span.end as u32);
                    // Parse the expression value from source
                    let expr_value = if let Some(v) = &expr.value {
                        if let Some(src) = &v.source {
                            let val_span =
                                Span::new(v.absolute_offset, v.absolute_offset + src.len() as u32);
                            self.parse_binding_expression(src, val_span)
                        } else {
                            self.create_empty_expression(source_span)
                        }
                    } else {
                        self.create_empty_expression(source_span)
                    };

                    let value_span_opt = expr.value.as_ref().map(|v| {
                        Span::new(
                            v.absolute_offset,
                            v.absolute_offset + v.source.as_ref().map_or(0, |s| s.len() as u32),
                        )
                    });

                    inputs.push(R3BoundAttribute {
                        name: directive_name_atom.clone(),
                        binding_type: BindingType::Property,
                        security_context: SecurityContext::None,
                        value: expr_value,
                        unit: None,
                        source_span,
                        key_span,
                        value_span: value_span_opt,
                        i18n: None,
                    });
                }
            }

            // Process all bindings
            let mut first_expr_handled = false;
            for binding in result.bindings.iter() {
                match binding {
                    TemplateBinding::Variable(var) => {
                        variables.push(self.create_template_variable(var));
                    }
                    TemplateBinding::Expression(expr) => {
                        // Skip the first expression if it matches the directive name
                        // (it was already handled above as either TextAttribute or BoundAttribute)
                        if !first_expr_handled && expr.key.source.as_str() == directive_name {
                            first_expr_handled = true;
                            continue;
                        }

                        // Use the full binding name (e.g., "ngForOf", not "of")
                        let full_name = expr.key.source.as_str();
                        let binding_name_atom = Atom::from(self.allocator.alloc_str(full_name));

                        let source_span =
                            Span::new(expr.source_span.start as u32, expr.source_span.end as u32);
                        // Parse the expression value from source
                        let expr_value = if let Some(v) = &expr.value {
                            if let Some(src) = &v.source {
                                let val_span = Span::new(
                                    v.absolute_offset,
                                    v.absolute_offset + src.len() as u32,
                                );
                                self.parse_binding_expression(src, val_span)
                            } else {
                                self.create_empty_expression(source_span)
                            }
                        } else {
                            self.create_empty_expression(source_span)
                        };

                        let value_span_opt = expr.value.as_ref().map(|v| {
                            Span::new(
                                v.absolute_offset,
                                v.absolute_offset + v.source.as_ref().map_or(0, |s| s.len() as u32),
                            )
                        });

                        inputs.push(R3BoundAttribute {
                            name: binding_name_atom,
                            binding_type: BindingType::Property,
                            security_context: SecurityContext::None,
                            value: expr_value,
                            unit: None,
                            source_span,
                            key_span: Span::new(
                                expr.key.span.start as u32,
                                expr.key.span.end as u32,
                            ),
                            value_span: value_span_opt,
                            i18n: None,
                        });
                    }
                }
            }
        } else {
            // No value - just add the directive name as a TextAttribute
            // The source span should exclude the `*` prefix
            let directive_source_span =
                Span::new(template_attr.name_span.start + 1, template_attr.name_span.end);
            attributes.push(R3TextAttribute {
                name: directive_name_atom,
                value: Atom::from(""),
                source_span: directive_source_span,
                key_span: Some(directive_source_span),
                value_span: None,
                i18n: None,
            });
        }

        // Get tag_name from the wrapped node (Element -> name, Template -> None)
        let tag_name = self.get_wrapped_tag_name(&children[0]);

        // Hoist attributes, inputs, and outputs from wrapped element (with animation filtering)
        // These go on the outer template in addition to any directive-related attrs
        let (hoisted_attributes, hoisted_inputs, hoisted_outputs) =
            self.get_hoisted_attrs_from_node(&children[0]);

        // The directive-related attributes go into template_attrs
        // The hoisted element attributes become the template's main attributes
        let mut template_attrs: Vec<'a, R3TemplateAttr<'a>> = Vec::new_in(self.allocator);
        for attr in attributes.into_iter() {
            template_attrs.push(R3TemplateAttr::Text(attr));
        }
        for input in inputs.into_iter() {
            template_attrs.push(R3TemplateAttr::Bound(input));
        }

        let template = R3Template {
            tag_name,
            attributes: hoisted_attributes,
            inputs: hoisted_inputs,
            outputs: hoisted_outputs,
            directives: Vec::new_in(self.allocator),
            template_attrs,
            children,
            references: Vec::new_in(self.allocator),
            variables,
            is_self_closing: false,
            source_span: element.span,
            start_source_span: element.start_span,
            end_source_span: element.end_span,
            i18n,
        };
        R3Node::Template(Box::new_in(template, self.allocator))
    }

    /// Wraps a component node in a template for structural directives.
    /// This is similar to `wrap_in_template` but uses component spans.
    fn wrap_component_in_template(
        &mut self,
        node: R3Node<'a>,
        component: &HtmlComponent<'a>,
        template_attr: TemplateAttrInfo<'a>,
    ) -> R3Node<'a> {
        use crate::ast::expression::TemplateBinding;
        use crate::ast::r3::SecurityContext;

        // Generate i18n metadata if we're inside an i18n block.
        // When inside an i18n block (i18n_depth > 0), each element gets a TagPlaceholder.
        // The wrapper template inherits this metadata.
        // Reference: r3_template_transform.ts line 1017 and i18n/meta.ts
        let i18n = if self.i18n_depth > 0 {
            Some(self.create_i18n_tag_placeholder_for_component(component))
        } else {
            // Not in i18n block - clone from wrapped node if it has i18n
            self.get_node_i18n(&node).map(|meta| meta.clone_in(self.allocator))
        };

        let mut children = Vec::new_in(self.allocator);
        children.push(node);

        let mut attributes = Vec::new_in(self.allocator);
        let mut inputs = Vec::new_in(self.allocator);
        let mut variables = Vec::new_in(self.allocator);

        // Extract the directive name (strip the * prefix)
        let directive_name: &str =
            template_attr.name.strip_prefix('*').unwrap_or(&template_attr.name);
        let directive_name = self.allocator.alloc_str(directive_name);
        let directive_name_atom = Atom::from(directive_name);

        let value_span = template_attr.value_span.unwrap_or(template_attr.span);
        let key_span = Span::new(template_attr.name_span.start + 1, template_attr.name_span.end);

        let value_str = template_attr.value.as_str();
        if !value_str.is_empty() {
            let result = self.binding_parser.parse_template_bindings(
                directive_name,
                value_str,
                key_span,
                value_span,
            );

            for error in result.errors {
                self.report_error(&error, value_span);
            }

            for warning in result.warnings {
                self.errors.push(crate::util::ParseError {
                    span: crate::util::ParseSourceSpan::new(
                        crate::util::ParseLocation::new(
                            std::sync::Arc::new(crate::util::ParseSourceFile::new(
                                self.source_text,
                                "template",
                            )),
                            value_span.start,
                            0,
                            0,
                        ),
                        crate::util::ParseLocation::new(
                            std::sync::Arc::new(crate::util::ParseSourceFile::new(
                                self.source_text,
                                "template",
                            )),
                            value_span.end,
                            0,
                            0,
                        ),
                    ),
                    msg: warning,
                    level: crate::util::ParseErrorLevel::Warning,
                });
            }

            let mut first_is_text_attr = false;
            if let Some(first) = result.bindings.first() {
                match first {
                    TemplateBinding::Variable(_) => {
                        first_is_text_attr = true;
                    }
                    TemplateBinding::Expression(expr) => {
                        if expr.key.source.as_str() == directive_name && expr.value.is_none() {
                            first_is_text_attr = true;
                        }
                    }
                }
            }

            if first_is_text_attr {
                let directive_source_span =
                    Span::new(template_attr.name_span.start + 1, template_attr.name_span.end);
                attributes.push(R3TextAttribute {
                    name: directive_name_atom.clone(),
                    value: Atom::from(""),
                    source_span: directive_source_span,
                    key_span: Some(directive_source_span),
                    value_span: None,
                    i18n: None,
                });
            } else if let Some(TemplateBinding::Expression(expr)) = result.bindings.first() {
                if expr.key.source.as_str() == directive_name {
                    let source_span =
                        Span::new(expr.source_span.start as u32, expr.source_span.end as u32);
                    let expr_value = if let Some(v) = &expr.value {
                        if let Some(src) = &v.source {
                            let val_span =
                                Span::new(v.absolute_offset, v.absolute_offset + src.len() as u32);
                            self.parse_binding_expression(src, val_span)
                        } else {
                            self.create_empty_expression(source_span)
                        }
                    } else {
                        self.create_empty_expression(source_span)
                    };

                    let value_span_opt = expr.value.as_ref().map(|v| {
                        Span::new(
                            v.absolute_offset,
                            v.absolute_offset + v.source.as_ref().map_or(0, |s| s.len() as u32),
                        )
                    });

                    inputs.push(R3BoundAttribute {
                        name: directive_name_atom.clone(),
                        binding_type: BindingType::Property,
                        security_context: SecurityContext::None,
                        value: expr_value,
                        unit: None,
                        source_span,
                        key_span,
                        value_span: value_span_opt,
                        i18n: None,
                    });
                }
            }

            let mut first_expr_handled = false;
            for binding in result.bindings.iter() {
                match binding {
                    TemplateBinding::Variable(var) => {
                        variables.push(self.create_template_variable(var));
                    }
                    TemplateBinding::Expression(expr) => {
                        if !first_expr_handled && expr.key.source.as_str() == directive_name {
                            first_expr_handled = true;
                            continue;
                        }

                        let full_name = expr.key.source.as_str();
                        let binding_name_atom = Atom::from(self.allocator.alloc_str(full_name));

                        let source_span =
                            Span::new(expr.source_span.start as u32, expr.source_span.end as u32);
                        let expr_value = if let Some(v) = &expr.value {
                            if let Some(src) = &v.source {
                                let val_span = Span::new(
                                    v.absolute_offset,
                                    v.absolute_offset + src.len() as u32,
                                );
                                self.parse_binding_expression(src, val_span)
                            } else {
                                self.create_empty_expression(source_span)
                            }
                        } else {
                            self.create_empty_expression(source_span)
                        };

                        let value_span_opt = expr.value.as_ref().map(|v| {
                            Span::new(
                                v.absolute_offset,
                                v.absolute_offset + v.source.as_ref().map_or(0, |s| s.len() as u32),
                            )
                        });

                        inputs.push(R3BoundAttribute {
                            name: binding_name_atom,
                            binding_type: BindingType::Property,
                            security_context: SecurityContext::None,
                            value: expr_value,
                            unit: None,
                            source_span,
                            key_span: Span::new(
                                expr.key.span.start as u32,
                                expr.key.span.end as u32,
                            ),
                            value_span: value_span_opt,
                            i18n: None,
                        });
                    }
                }
            }
        } else {
            let directive_source_span =
                Span::new(template_attr.name_span.start + 1, template_attr.name_span.end);
            attributes.push(R3TextAttribute {
                name: directive_name_atom,
                value: Atom::from(""),
                source_span: directive_source_span,
                key_span: Some(directive_source_span),
                value_span: None,
                i18n: None,
            });
        }

        let tag_name = self.get_wrapped_tag_name(&children[0]);
        let (hoisted_attributes, hoisted_inputs, hoisted_outputs) =
            self.get_hoisted_attrs_from_node(&children[0]);

        let mut template_attrs: Vec<'a, R3TemplateAttr<'a>> = Vec::new_in(self.allocator);
        for attr in attributes.into_iter() {
            template_attrs.push(R3TemplateAttr::Text(attr));
        }
        for input in inputs.into_iter() {
            template_attrs.push(R3TemplateAttr::Bound(input));
        }

        let template = R3Template {
            tag_name,
            attributes: hoisted_attributes,
            inputs: hoisted_inputs,
            outputs: hoisted_outputs,
            directives: Vec::new_in(self.allocator),
            template_attrs,
            children,
            references: Vec::new_in(self.allocator),
            variables,
            is_self_closing: false,
            source_span: component.span,
            start_source_span: component.start_span,
            end_source_span: component.end_span,
            i18n,
        };
        R3Node::Template(Box::new_in(template, self.allocator))
    }

    /// Filters out animation attributes (those starting with "animate.").
    /// These should remain on the inner element, not be hoisted to the template.
    fn filter_animation_attributes(
        &self,
        attributes: &Vec<'a, R3TextAttribute<'a>>,
    ) -> Vec<'a, R3TextAttribute<'a>> {
        let mut result = Vec::new_in(self.allocator);
        for attr in attributes.iter() {
            if !attr.name.as_str().starts_with("animate.") {
                result.push(R3TextAttribute {
                    name: attr.name.clone(),
                    value: attr.value.clone(),
                    source_span: attr.source_span,
                    key_span: attr.key_span,
                    value_span: attr.value_span,
                    i18n: None,
                });
            }
        }
        result
    }

    /// Filters out animation inputs (those with BindingType::Animation).
    /// These should remain on the inner element, not be hoisted to the template.
    fn filter_animation_inputs(
        &self,
        inputs: &Vec<'a, R3BoundAttribute<'a>>,
    ) -> Vec<'a, R3BoundAttribute<'a>> {
        let mut result = Vec::new_in(self.allocator);
        for input in inputs.iter() {
            if input.binding_type != BindingType::Animation {
                result.push(R3BoundAttribute {
                    name: input.name.clone(),
                    binding_type: input.binding_type,
                    security_context: input.security_context,
                    value: self.clone_angular_expression(&input.value),
                    unit: input.unit.clone(),
                    source_span: input.source_span,
                    key_span: input.key_span,
                    value_span: input.value_span,
                    i18n: None,
                });
            }
        }
        result
    }

    /// Gets the tag name from a wrapped R3Node for template wrapping.
    /// Returns the element name for R3Element, "ng-content" for Content, None for R3Template.
    /// Reference: r3_template_transform.ts lines 1018-1026
    fn get_wrapped_tag_name(&self, node: &R3Node<'a>) -> Option<Atom<'a>> {
        match node {
            R3Node::Element(elem) => Some(elem.name.clone()),
            R3Node::Template(_) => None,
            // Content has a readonly name = 'ng-content' in TypeScript
            R3Node::Content(_) => Some(Atom::from("ng-content")),
            _ => None,
        }
    }

    /// Gets a reference to the i18n metadata from an R3Node for template wrapping.
    /// The wrapper template inherits the i18n metadata from the wrapped node.
    /// Reference: r3_template_transform.ts line 1017
    fn get_node_i18n<'b>(&self, node: &'b R3Node<'a>) -> Option<&'b I18nMeta<'a>> {
        match node {
            R3Node::Element(elem) => elem.i18n.as_ref(),
            R3Node::Template(tmpl) => tmpl.i18n.as_ref(),
            R3Node::Content(content) => content.i18n.as_ref(),
            R3Node::Component(comp) => comp.i18n.as_ref(),
            _ => None,
        }
    }

    /// Creates a TagPlaceholder i18n metadata for an element inside an i18n block.
    /// This is called when an element is inside an i18n region but doesn't have its own i18n attribute.
    /// Reference: i18n_parser.ts line 266-277
    fn create_i18n_tag_placeholder_for_element(
        &mut self,
        element: &HtmlElement<'a>,
    ) -> I18nMeta<'a> {
        use indexmap::IndexMap;

        let tag_name = element.name.as_str();
        let is_void = crate::parser::html::is_void_element(tag_name);

        // Collect attributes for placeholder name generation
        let mut attrs: IndexMap<String, String> = element
            .attrs
            .iter()
            .map(|attr| (attr.name.to_string(), attr.value.to_string()))
            .collect();
        // Include selectorless directive attrs in the signature
        for directive in &element.directives {
            for attr in &directive.attrs {
                attrs.insert(attr.name.to_string(), attr.value.to_string());
            }
        }

        // Generate placeholder names
        let start_name = self
            .i18n_placeholder_registry
            .get_start_tag_placeholder_name(tag_name, &attrs, is_void);
        let close_name = if is_void {
            String::new()
        } else {
            self.i18n_placeholder_registry.get_close_tag_placeholder_name(tag_name)
        };

        // Create TagPlaceholder
        let mut placeholder_attrs = HashMap::new_in(self.allocator);
        for (k, v) in attrs {
            placeholder_attrs.insert(
                Atom::from(self.allocator.alloc_str(&k)),
                Atom::from(self.allocator.alloc_str(&v)),
            );
        }

        I18nMeta::Node(I18nNode::TagPlaceholder(I18nTagPlaceholder {
            tag: Atom::from(self.allocator.alloc_str(tag_name)),
            attrs: placeholder_attrs,
            start_name: Atom::from(self.allocator.alloc_str(&start_name)),
            close_name: Atom::from(self.allocator.alloc_str(&close_name)),
            children: Vec::new_in(self.allocator),
            is_void,
            source_span: element.span,
            start_source_span: Some(element.start_span),
            end_source_span: element.end_span,
        }))
    }

    /// Creates a TagPlaceholder i18n metadata for a component inside an i18n block.
    /// This is called when a component is inside an i18n region but doesn't have its own i18n attribute.
    /// Reference: i18n_parser.ts line 266-277
    fn create_i18n_tag_placeholder_for_component(
        &mut self,
        component: &HtmlComponent<'a>,
    ) -> I18nMeta<'a> {
        use indexmap::IndexMap;

        let tag_name = component.full_name.as_str();
        let is_void = false; // Components are never void

        // Collect attributes for placeholder name generation
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

        // Generate placeholder names
        let start_name = self
            .i18n_placeholder_registry
            .get_start_tag_placeholder_name(tag_name, &attrs, is_void);
        let close_name = self.i18n_placeholder_registry.get_close_tag_placeholder_name(tag_name);

        // Create TagPlaceholder
        let mut placeholder_attrs = HashMap::new_in(self.allocator);
        for (k, v) in attrs {
            placeholder_attrs.insert(
                Atom::from(self.allocator.alloc_str(&k)),
                Atom::from(self.allocator.alloc_str(&v)),
            );
        }

        I18nMeta::Node(I18nNode::TagPlaceholder(I18nTagPlaceholder {
            tag: Atom::from(self.allocator.alloc_str(tag_name)),
            attrs: placeholder_attrs,
            start_name: Atom::from(self.allocator.alloc_str(&start_name)),
            close_name: Atom::from(self.allocator.alloc_str(&close_name)),
            children: Vec::new_in(self.allocator),
            is_void,
            source_span: component.span,
            start_source_span: Some(component.start_span),
            end_source_span: component.end_span,
        }))
    }

    /// Extracts hoisted attributes, inputs, and outputs from a wrapped R3Node.
    /// Animation attributes and inputs are filtered out (they stay on the inner element).
    fn get_hoisted_attrs_from_node(
        &self,
        node: &R3Node<'a>,
    ) -> (Vec<'a, R3TextAttribute<'a>>, Vec<'a, R3BoundAttribute<'a>>, Vec<'a, R3BoundEvent<'a>>)
    {
        match node {
            R3Node::Element(elem) => {
                let attrs = self.filter_animation_attributes(&elem.attributes);
                let inputs = self.filter_animation_inputs(&elem.inputs);
                let outputs = self.copy_bound_events(&elem.outputs);
                (attrs, inputs, outputs)
            }
            _ => (
                Vec::new_in(self.allocator),
                Vec::new_in(self.allocator),
                Vec::new_in(self.allocator),
            ),
        }
    }

    /// Creates a shallow copy of bound events.
    fn copy_bound_events(&self, events: &Vec<'a, R3BoundEvent<'a>>) -> Vec<'a, R3BoundEvent<'a>> {
        let mut result = Vec::new_in(self.allocator);
        for event in events.iter() {
            result.push(R3BoundEvent {
                name: event.name.clone(),
                event_type: event.event_type,
                handler: self.clone_angular_expression(&event.handler),
                target: event.target,
                phase: event.phase,
                source_span: event.source_span,
                handler_span: event.handler_span,
                key_span: event.key_span,
            });
        }
        result
    }

    /// Creates a deep copy of an AngularExpression.
    fn clone_angular_expression(&self, expr: &AngularExpression<'a>) -> AngularExpression<'a> {
        expr.clone_in(self.allocator)
    }

    /// Creates a variable from a template variable binding.
    fn create_template_variable(
        &self,
        var: &crate::ast::expression::VariableBinding<'a>,
    ) -> R3Variable<'a> {
        let name = var.key.source.clone();
        // When value is None, Angular uses "$implicit" as the default binding
        let value = var.value.as_ref().map_or(Atom::from("$implicit"), |v| v.source.clone());
        let value_span = var.value.as_ref().map(|v| Span::new(v.span.start, v.span.end));

        R3Variable {
            name,
            value,
            source_span: Span::new(var.source_span.start, var.source_span.end),
            key_span: Span::new(var.key.span.start, var.key.span.end),
            value_span,
        }
    }

    /// Checks if text contains interpolation.
    /// Ensures that `{{` appears before `}}` in the string.
    fn has_interpolation(&self, text: &str) -> bool {
        if let Some(open_pos) = text.find("{{") {
            if let Some(close_pos) = text.find("}}") {
                // Ensure the opening delimiter comes before the closing delimiter
                return open_pos < close_pos;
            }
        }
        false
    }

    /// Creates i18n metadata (Container with Text and Placeholder nodes) for text with interpolation.
    /// This is used when bound text appears inside an i18n block.
    ///
    /// The generated structure contains:
    /// - I18nText nodes for literal text parts
    /// - I18nPlaceholder nodes for interpolation expressions
    ///
    /// Example: "Hello {{name}}!" produces:
    /// - Container([Text("Hello "), Placeholder("name", "INTERPOLATION"), Text("!")])
    ///
    /// Ported from Angular's I18nMetaVisitor._visitTextWithInterpolation in i18n/meta.ts.
    fn create_i18n_meta_for_text_with_interpolation(
        &mut self,
        text: &str,
        span: Span,
    ) -> I18nMeta<'a> {
        let mut children = Vec::new_in(self.allocator);
        let mut current_pos = 0;

        // Find all interpolations {{ expr }}
        while let Some(start) = text[current_pos..].find("{{") {
            let abs_start = current_pos + start;

            // Add text before interpolation
            if start > 0 {
                let text_before = &text[current_pos..abs_start];
                if !text_before.is_empty() {
                    let text_atom = Atom::from_in(text_before, self.allocator);
                    children.push(I18nNode::Text(I18nText { value: text_atom, source_span: span }));
                }
            }

            // Find the closing }}
            if let Some(end) = text[abs_start + 2..].find("}}") {
                let abs_end = abs_start + 2 + end;
                let expr = text[abs_start + 2..abs_end].trim();

                if !expr.is_empty() {
                    // Generate placeholder name using the i18n placeholder registry
                    let placeholder_name =
                        self.i18n_placeholder_registry.get_placeholder_name("INTERPOLATION", expr);
                    let name_atom = Atom::from_in(&placeholder_name, self.allocator);
                    let value_atom = Atom::from_in(expr, self.allocator);

                    children.push(I18nNode::Placeholder(I18nPlaceholder {
                        value: value_atom,
                        name: name_atom,
                        source_span: span,
                    }));
                }

                current_pos = abs_end + 2;
            } else {
                // No closing }}, treat rest as text
                break;
            }
        }

        // Add remaining text after last interpolation
        if current_pos < text.len() {
            let remaining = &text[current_pos..];
            if !remaining.is_empty() {
                let text_atom = Atom::from_in(remaining, self.allocator);
                children.push(I18nNode::Text(I18nText { value: text_atom, source_span: span }));
            }
        }

        // Return Container if multiple children, otherwise single node
        if children.len() == 1 {
            let single_child = children.pop().unwrap();
            I18nMeta::Node(single_child)
        } else {
            I18nMeta::Node(I18nNode::Container(I18nContainer { children, source_span: span }))
        }
    }

    /// Parses an interpolation expression.
    fn parse_interpolation(&mut self, text: &'a str, span: Span) -> Option<AngularExpression<'a>> {
        if let Some(result) = self.binding_parser.parse_default_interpolation(text, span) {
            // Collect any parse errors
            for error in result.errors {
                self.errors.push(error);
            }
            Some(result.ast)
        } else {
            None
        }
    }

    /// Parses interpolation from tokens, using token spans for correct source positions.
    /// This handles the case where HTML entities affect span calculations.
    fn parse_interpolation_from_tokens(
        &mut self,
        text: &HtmlText<'a>,
    ) -> Option<AngularExpression<'a>> {
        use crate::ast::expression::Interpolation;
        use crate::parser::html::NGSP_UNICODE;

        let mut strings = Vec::new_in(self.allocator);
        let mut expressions = Vec::new_in(self.allocator);

        // Helper to accumulate text/entity content before committing to strings array.
        // The invariant we maintain: strings.len() == expressions.len() after processing text/entity,
        // and strings.len() == expressions.len() + 1 after processing interpolation.
        // This ensures the final result has strings.len() == expressions.len() + 1.
        let mut current_string = String::new();

        for token in text.tokens.iter() {
            match token.token_type {
                InterpolatedTokenType::Text => {
                    // Add text part (possibly with ngsp replaced)
                    if let Some(text_value) = token.parts.first() {
                        let text_str = text_value.as_str();
                        if text_str.contains(NGSP_UNICODE) {
                            current_string.push_str(&text_str.replace(NGSP_UNICODE, " "));
                        } else {
                            current_string.push_str(text_str);
                        }
                    }
                }
                InterpolatedTokenType::Interpolation => {
                    // Interpolation token has parts = [startMarker, expression, endMarker]
                    if token.parts.len() >= 3 {
                        // Before adding an expression, commit the current string buffer
                        // (even if empty, we need a string before each expression)
                        strings.push(Atom::from_in(current_string.as_str(), self.allocator));
                        current_string.clear();

                        let start_marker = &token.parts[0];
                        let expr_content = &token.parts[1];
                        let end_marker = &token.parts[2];

                        // Calculate the expression span from the token span
                        // Token span covers the whole interpolation: {{expr}}
                        // Expression span is token.span.start + start_marker.len() to
                        // token.span.end - end_marker.len()
                        let expr_start = token.span.start + start_marker.len() as u32;
                        let expr_end = token.span.end - end_marker.len() as u32;
                        let expr_span = Span::new(expr_start, expr_end);

                        // For backward compatibility, decode HTML entities in interpolation
                        // expressions. This matches Angular's behavior where entities like
                        // &times; inside string literals in interpolations are decoded.
                        let decoded_expr = decode_entities_in_string(expr_content.as_str());
                        let expr_str = self.allocator.alloc_str(&decoded_expr);
                        let parse_result = self.binding_parser.parse_binding(expr_str, expr_span);

                        // Collect any parse errors
                        for error in parse_result.errors {
                            self.errors.push(error);
                        }
                        expressions.push(parse_result.ast);
                    }
                }
                InterpolatedTokenType::EncodedEntity => {
                    // Encoded entity token has parts = [decoded, encoded]
                    // Append the decoded value to the current string buffer
                    if let Some(decoded) = token.parts.first() {
                        current_string.push_str(decoded.as_str());
                    }
                }
            }
        }

        if expressions.is_empty() {
            return None;
        }

        // Commit the trailing string (after the last expression)
        strings.push(Atom::from_in(current_string.as_str(), self.allocator));

        // Create the Interpolation expression
        let span = ParseSpan::new(0, (text.span.end - text.span.start) as u32);
        let source_span = AbsoluteSourceSpan { start: text.span.start, end: text.span.end };
        let interpolation = Interpolation { span, source_span, strings, expressions };
        Some(AngularExpression::Interpolation(Box::new_in(interpolation, self.allocator)))
    }

    /// Parses interpolation from attribute tokens, using token spans for correct source positions.
    /// This handles the case where HTML entities affect span calculations.
    fn parse_interpolation_from_attr_tokens(
        &mut self,
        attr: &HtmlAttribute<'a>,
        value_span: Span,
    ) -> Option<AngularExpression<'a>> {
        use crate::ast::expression::Interpolation;
        use crate::parser::html::NGSP_UNICODE;

        let tokens = attr.value_tokens.as_ref()?;

        let mut strings = Vec::new_in(self.allocator);
        let mut expressions = Vec::new_in(self.allocator);

        // Helper to accumulate text/entity content before committing to strings array.
        // The invariant we maintain: strings.len() == expressions.len() after processing text/entity,
        // and strings.len() == expressions.len() + 1 after processing interpolation.
        // This ensures the final result has strings.len() == expressions.len() + 1.
        let mut current_string = String::new();

        for token in tokens.iter() {
            match token.token_type {
                InterpolatedTokenType::Text => {
                    // Add text part (possibly with ngsp replaced)
                    if let Some(text_value) = token.parts.first() {
                        let text_str = text_value.as_str();
                        if text_str.contains(NGSP_UNICODE) {
                            current_string.push_str(&text_str.replace(NGSP_UNICODE, " "));
                        } else {
                            current_string.push_str(text_str);
                        }
                    }
                }
                InterpolatedTokenType::Interpolation => {
                    // Interpolation token has parts = [startMarker, expression, endMarker]
                    if token.parts.len() >= 3 {
                        // Before adding an expression, commit the current string buffer
                        // (even if empty, we need a string before each expression)
                        strings.push(Atom::from_in(current_string.as_str(), self.allocator));
                        current_string.clear();

                        let start_marker = &token.parts[0];
                        let expr_content = &token.parts[1];
                        let end_marker = &token.parts[2];

                        // Calculate the expression span from the token span
                        let expr_start = token.span.start + start_marker.len() as u32;
                        let expr_end = token.span.end - end_marker.len() as u32;
                        let expr_span = Span::new(expr_start, expr_end);

                        // For backward compatibility, decode HTML entities in interpolation
                        // expressions. This matches Angular's behavior where entities like
                        // &times; inside string literals in interpolations are decoded.
                        let decoded_expr = decode_entities_in_string(expr_content.as_str());
                        let expr_str = self.allocator.alloc_str(&decoded_expr);
                        let parse_result = self.binding_parser.parse_binding(expr_str, expr_span);

                        // Collect any parse errors
                        for error in parse_result.errors {
                            self.errors.push(error);
                        }
                        expressions.push(parse_result.ast);
                    }
                }
                InterpolatedTokenType::EncodedEntity => {
                    // Encoded entity token has parts = [decoded, encoded]
                    // Append the decoded value to the current string buffer
                    if let Some(decoded) = token.parts.first() {
                        current_string.push_str(decoded.as_str());
                    }
                }
            }
        }

        if expressions.is_empty() {
            return None;
        }

        // Commit the trailing string (after the last expression)
        strings.push(Atom::from_in(current_string.as_str(), self.allocator));

        // Create the Interpolation expression
        let span = ParseSpan::new(0, value_span.size());
        let source_span = AbsoluteSourceSpan { start: value_span.start, end: value_span.end };
        let interpolation = Interpolation { span, source_span, strings, expressions };
        Some(AngularExpression::Interpolation(Box::new_in(interpolation, self.allocator)))
    }

    /// Creates a bound attribute from an attribute with interpolation value.
    /// Handles special prefixes like `attr.`, `class.`, `style.` similar to Angular's
    /// `createBoundElementProperty` in binding_parser.ts.
    fn create_interpolated_bound_attribute(
        &mut self,
        element_name: &str,
        name: &str,
        attr: &HtmlAttribute<'a>,
        i18n: Option<I18nMeta<'a>>,
    ) -> Option<R3BoundAttribute<'a>> {
        let value_span = attr.value_span.unwrap_or(attr.span);

        // Check if there are HTML entities in the attribute value that could affect span calculations.
        // If so, use the tokens which have correct source spans.
        let has_entities = attr.value_tokens.as_ref().is_some_and(|tokens| {
            tokens.iter().any(|t| matches!(t.token_type, InterpolatedTokenType::EncodedEntity))
        });

        let expr = if has_entities {
            // Use tokens to parse interpolations with correct source spans
            self.parse_interpolation_from_attr_tokens(attr, value_span)?
        } else {
            // Use attr.value which has HTML entities already decoded by the HTML parser.
            // This matches Angular's behavior where the ml_parser decodes entities in interpolation
            // expressions for backward compatibility.
            // See: Angular's parser.ts _consumeAttr which decodes entities in ATTR_VALUE_INTERPOLATION.
            self.parse_interpolation(attr.value.as_str(), value_span)?
        };

        // Determine binding type, final name, and security context based on prefix
        // This matches Angular's binding_parser.ts createBoundElementProperty
        let (binding_type, final_name, unit, security_context) =
            if let Some(stripped) = name.strip_prefix("attr.") {
                // Attribute bindings use the attribute security context
                let security_context = get_security_context(element_name, stripped);
                (BindingType::Attribute, stripped, None, security_context)
            } else if let Some(stripped) = name.strip_prefix("class.") {
                (BindingType::Class, stripped, None, SecurityContext::None)
            } else if let Some(stripped) = name.strip_prefix("style.") {
                // Check for unit suffix: style.width.px -> property="width", unit="px"
                if let Some(dot_pos) = stripped.find('.') {
                    let prop = &stripped[..dot_pos];
                    let unit_str = &stripped[dot_pos + 1..];
                    (BindingType::Style, prop, Some(unit_str), SecurityContext::Style)
                } else {
                    (BindingType::Style, stripped, None, SecurityContext::Style)
                }
            } else {
                // Property bindings use the property security context
                let security_context = get_security_context(element_name, name);
                (BindingType::Property, name, None, security_context)
            };

        let name_atom = Atom::from(self.allocator.alloc_str(final_name));

        Some(R3BoundAttribute {
            name: name_atom,
            binding_type,
            security_context,
            value: expr,
            unit: unit.map(|u| Atom::from(self.allocator.alloc_str(u))),
            source_span: attr.span,
            key_span: attr.name_span,
            value_span: attr.value_span,
            i18n,
        })
    }

    /// Creates an empty expression placeholder.
    fn create_empty_expression(&self, span: Span) -> AngularExpression<'a> {
        AngularExpression::Empty(Box::new_in(
            crate::ast::expression::EmptyExpr {
                span: ParseSpan::new(0, span.size()),
                source_span: AbsoluteSourceSpan::new(span.start, span.end),
            },
            self.allocator,
        ))
    }

    /// Reports a parse error at the given span.
    fn report_error(&mut self, msg: &str, span: Span) {
        self.errors.push(crate::util::ParseError {
            span: crate::util::ParseSourceSpan::new(
                crate::util::ParseLocation::new(
                    std::sync::Arc::new(crate::util::ParseSourceFile::new(
                        self.source_text,
                        "template",
                    )),
                    span.start,
                    0,
                    0,
                ),
                crate::util::ParseLocation::new(
                    std::sync::Arc::new(crate::util::ParseSourceFile::new(
                        self.source_text,
                        "template",
                    )),
                    span.end,
                    0,
                    0,
                ),
            ),
            msg: msg.to_string(),
            level: crate::util::ParseErrorLevel::Error,
        });
    }

    /// Validates references for selectorless components.
    ///
    /// Selectorless references must not have values, and duplicate names are not allowed.
    /// This matches TypeScript's `validateSelectorlessReferences` in r3_template_transform.ts.
    fn validate_selectorless_references(&mut self, references: &[R3Reference<'a>]) {
        if references.is_empty() {
            return;
        }

        let mut seen_names: FxHashSet<&str> = FxHashSet::default();

        for reference in references {
            if !reference.value.is_empty() {
                // References must not have values in selectorless context
                self.report_error(
                    "Cannot specify a value for a local reference in this context",
                    reference.value_span.unwrap_or(reference.source_span),
                );
            } else if seen_names.contains(reference.name.as_str()) {
                // Duplicate reference names not allowed
                self.report_error(
                    "Duplicate reference names are not allowed",
                    reference.source_span,
                );
            } else {
                seen_names.insert(reference.name.as_str());
            }
        }
    }

    /// Gets text content from an element.
    fn get_text_content(&self, element: &HtmlElement<'a>) -> Option<Atom<'a>> {
        if element.children.len() == 1 {
            if let HtmlNode::Text(text) = &element.children[0] {
                return Some(text.value.clone());
            }
        }
        None
    }

    /// Gets the stylesheet href from a link element.
    /// Only returns resolvable URLs per Angular's `isStyleUrlResolvable`.
    /// Reference: style_url_resolver.ts lines 12-18
    fn get_stylesheet_href(&self, element: &HtmlElement<'a>) -> Option<Atom<'a>> {
        let mut is_stylesheet = false;
        let mut href = None;

        for attr in &element.attrs {
            if attr.name.as_str() == "rel" {
                is_stylesheet = attr.value.as_str() == "stylesheet";
            }
            if attr.name.as_str() == "href" {
                let href_value = attr.value.as_str();
                // Only include resolvable URLs
                if is_style_url_resolvable(href_value) {
                    href = Some(attr.value.clone());
                }
            }
        }

        if is_stylesheet { href } else { None }
    }

    /// Checks if an element is a `<link rel="stylesheet">`.
    fn is_stylesheet_link(&self, element: &HtmlElement<'a>) -> bool {
        element
            .attrs
            .iter()
            .any(|attr| attr.name.as_str() == "rel" && attr.value.as_str() == "stylesheet")
    }

    /// Gets the ng-content selector.
    fn get_ng_content_selector(&self, element: &HtmlElement<'a>) -> Atom<'a> {
        for attr in &element.attrs {
            if attr.name.as_str() == "select" && !attr.value.is_empty() {
                return attr.value.clone();
            }
        }
        Atom::from("*")
    }

    /// Checks if an element is a void element.
    fn is_void_element(&self, name: &str) -> bool {
        matches!(
            name,
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
    }
}

/// Checks if a stylesheet URL is resolvable.
/// Angular only processes relative URLs and 'package:'/'asset:' scheme URLs.
/// Reference: style_url_resolver.ts lines 12-18
fn is_style_url_resolvable(url: &str) -> bool {
    // Empty URLs are not resolvable
    if url.is_empty() {
        return false;
    }

    // Absolute paths starting with '/' are not resolvable
    if url.starts_with('/') {
        return false;
    }

    // Check for URL scheme
    if let Some(colon_pos) = url.find(':') {
        let scheme = &url[..colon_pos];
        // Only allow 'package' and 'asset' schemes
        scheme == "package" || scheme == "asset"
    } else {
        // No scheme = relative URL = resolvable
        true
    }
}

/// Parses i18n metadata from an attribute value (for i18n-* attributes).
///
/// Format: `meaning|description@@customId`
/// Examples:
/// - `"Save button tooltip|Click to save@@SAVE_BTN"` -> meaning: "Save button tooltip", description: "Click to save", customId: "SAVE_BTN"
/// - `"Click to save@@SAVE_BTN"` -> description: "Click to save", customId: "SAVE_BTN"
/// - `"Click to save"` -> description: "Click to save"
/// - `"@@SAVE_BTN"` -> customId: "SAVE_BTN"
///
/// This variant is used for i18n-* attributes where the message content comes from
/// the attribute value itself, not from children.
fn parse_i18n_meta<'a>(allocator: &'a Allocator, value: &str, instance_id: u32) -> I18nMeta<'a> {
    parse_i18n_meta_with_message(allocator, value, instance_id, "")
}

/// Parses i18n metadata from an attribute value.
///
/// Format: `meaning|description@@customId`
/// Examples:
/// - `"Save button tooltip|Click to save@@SAVE_BTN"` -> meaning: "Save button tooltip", description: "Click to save", customId: "SAVE_BTN"
/// - `"Click to save@@SAVE_BTN"` -> description: "Click to save", customId: "SAVE_BTN"
/// - `"Click to save"` -> description: "Click to save"
/// - `"@@SAVE_BTN"` -> customId: "SAVE_BTN"
///
/// The `message_string` parameter should contain the serialized i18n message text with
/// placeholders like `"Your balance is {$interpolation}"`. This is generated by the
/// `I18nMessageFactory` from the element's children.
fn parse_i18n_meta_with_message<'a>(
    allocator: &'a Allocator,
    value: &str,
    instance_id: u32,
    message_string: &str,
) -> I18nMeta<'a> {
    // Split on @@ first to get the custom ID
    let (before_id, custom_id) = if let Some(pos) = value.find("@@") {
        (&value[..pos], &value[pos + 2..])
    } else {
        (value, "")
    };

    // Split the remaining part on | to get meaning and description
    let (meaning, description) = if let Some(pipe_pos) = before_id.find('|') {
        (before_id[..pipe_pos].trim(), before_id[pipe_pos + 1..].trim())
    } else {
        ("", before_id.trim())
    };

    I18nMeta::Message(I18nMessage {
        instance_id,
        nodes: Vec::new_in(allocator),
        meaning: Atom::from_in(meaning, allocator),
        description: Atom::from_in(description, allocator),
        custom_id: Atom::from_in(custom_id, allocator),
        id: Atom::from(""),
        legacy_ids: Vec::new_in(allocator),
        message_string: Atom::from_in(message_string, allocator),
    })
}

/// Transforms HTML AST to R3 AST.
pub fn html_ast_to_r3_ast<'a>(
    allocator: &'a Allocator,
    source_text: &'a str,
    html_nodes: &[HtmlNode<'a>],
    options: TransformOptions,
) -> R3ParseResult<'a> {
    let transformer = HtmlToR3Transform::new(allocator, source_text, options);
    transformer.transform(html_nodes)
}

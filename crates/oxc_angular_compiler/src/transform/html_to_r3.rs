//! HTML AST to R3 AST transformation.
//!
//! This module transforms parsed HTML template AST nodes into R3 AST nodes,
//! which are the intermediate representation used by the IR pipeline.
//!
//! Ported from Angular's `render3/r3_template_transform.ts`.

use oxc_allocator::{Allocator, Box, FromIn, HashMap, Vec};
use oxc_span::Span;
use oxc_str::Ident;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::AngularVersion;
use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, BindingType, ParseSpan, ParsedEventType,
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
    R3Variable, SecurityContext,
};
use crate::i18n::parser::I18nMessageFactory;
use crate::i18n::placeholder::PlaceholderRegistry;
use crate::parser::expression::{BindingParser, blank_interpolation_error, find_comment_start};
use crate::parser::html::{decode_entities_in_string, get_html_tag_definition, split_ns_name};
use crate::schema::{
    element_security_context_for, is_known_element, is_trusted_types_sink_at,
    strips_namespaced_svg_script, strips_namespaced_svg_style, uses_namespaced_schema,
};
use crate::transform::control_flow::{
    DeferTriggerError, parse_conditional_params, parse_defer_triggers,
};
use crate::util::ParseError;

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

/// The prefix of a legacy animation property: `[animate-x]`, `bind-animate-x`.
/// Reference: binding_parser.ts line 48 (`LEGACY_ANIMATE_PROP_PREFIX`)
const LEGACY_ANIMATE_PROP_PREFIX: &str = "animate-";

/// The key span of a binding: `identifier`, which starts `prefix` into the attribute.
/// For `[user]="x"` it is `user`, for `(click)="f()"` it is `click`.
/// Reference: r3_template_transform.ts lines 751-757 (`createKeySpan` in `parseAttribute`)
fn create_key_span(attr: &HtmlAttribute<'_>, prefix: &str, identifier: &str) -> Span {
    let start = attr.span.start + prefix.len() as u32;
    Span::new(start, start + identifier.len() as u32)
}

/// The key span of a property binding named `name` (the identifier `parseAttribute` passes
/// to `parsePropertyBinding`): a legacy animation's `animate-` or `@` is not part of it.
/// Reference: binding_parser.ts lines 365-386 (`parsePropertyBinding`)
fn property_binding_key_span(name: &str, key_span: Span) -> Span {
    if name.starts_with(LEGACY_ANIMATE_PROP_PREFIX) {
        Span::new(key_span.start + LEGACY_ANIMATE_PROP_PREFIX.len() as u32, key_span.end)
    } else if name.starts_with('@') {
        Span::new(key_span.start + 1, key_span.end)
    } else {
        key_span
    }
}

/// The key span of an event named `name` (the identifier `parseAttribute` passes to
/// `parseEvent`): a legacy animation event's `@` is not part of it.
/// Reference: binding_parser.ts lines 660-667 (`parseEvent`)
fn event_key_span(name: &str, key_span: Span) -> Span {
    if name.starts_with('@') { Span::new(key_span.start + 1, key_span.end) } else { key_span }
}

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
    name: Ident<'a>,
    /// The attribute value (e.g., `let item of items`).
    value: Ident<'a>,
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
    /// Angular version being compiled. `None` uses the latest (v22) security schema.
    pub angular_version: Option<AngularVersion>,
}

/// Inserts or updates a var entry in an ordered Vec, preserving first-insertion order.
/// This matches JS object semantics where reassigning an existing key keeps its position.
fn ordered_insert_var<'a>(
    vec: &mut Vec<'a, (Ident<'a>, R3BoundText<'a>)>,
    key: Ident<'a>,
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
    vec: &mut Vec<'a, (Ident<'a>, R3IcuPlaceholder<'a>)>,
    key: Ident<'a>,
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
    styles: Vec<'a, Ident<'a>>,
    style_urls: Vec<'a, Ident<'a>>,
    ng_content_selectors: Vec<'a, Ident<'a>>,
    comment_nodes: Option<Vec<'a, R3Comment<'a>>>,
    processed_nodes: FxHashSet<usize>,
    /// Prefix each open element passes to its children (`svg`, `math`, `xml`,
    /// or empty), matching `getNsPrefix(parentName)` in `_getPrefix`.
    namespace_stack: std::vec::Vec<String>,
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
    /// Angular version for security-schema and script-stripping compatibility.
    angular_version: Option<AngularVersion>,
    /// Counter for generating unique i18n message instance IDs.
    ///
    /// Each i18n message gets a unique instance ID that's used to track message identity
    /// across moves/copies. This ensures that when the same attribute is processed twice
    /// (e.g., by `ingestControlFlowInsertionPoint` and `ingestStaticAttributes`), both
    /// can share the same i18n context.
    i18n_message_instance_counter: u32,
    /// The `i18n` attribute value and message instance id of an element whose only child is
    /// an ICU. That ICU is the element's message rather than a sub-message, so it reuses the
    /// element's message (Angular's I18nMetaVisitor: `currentMessage || meta`).
    sole_icu_message: Option<(String, u32)>,
    /// Placeholder names from the enclosing i18n message.
    message_names: MessagePlaceholderNames,
    /// Placeholder names from the message of the ICU being visited.
    icu_names: IcuPlaceholderNames,
    /// Full names (`:svg:svg`) of the enclosing elements, for i18n placeholder names.
    element_full_names: std::vec::Vec<String>,
    /// The template as a source file for i18n messages, built on first use and shared by
    /// every message so the template text is copied once.
    i18n_source_file: std::cell::OnceCell<std::sync::Arc<crate::util::ParseSourceFile>>,
}

/// Placeholder names taken from an i18n message, keyed by source offset, so that the r3 AST
/// uses the same names as the message: Angular's placeholder registry numbers them per message.
#[derive(Default)]
struct MessagePlaceholderNames {
    /// ICU placeholder names (`ICU`, `ICU_1`, ...), keyed by the ICU's start offset.
    icus: FxHashMap<u32, String>,
    /// Block start and close names (`START_BLOCK_IF`, `CLOSE_BLOCK_IF`, ...), keyed by the
    /// block's start offset.
    blocks: FxHashMap<u32, (String, String)>,
    /// Tag start and close names, keyed by the element's start offset.
    tags: FxHashMap<u32, (String, String)>,
}

/// Placeholder names taken from the message of the ICU being visited.
#[derive(Default)]
struct IcuPlaceholderNames {
    /// `VAR_*` names of the ICU and its nested ICUs, keyed by the start offset of each switch
    /// expression.
    vars: FxHashMap<u32, String>,
    /// Interpolation placeholder names, keyed by their interpolation text (`{{count}}`).
    interpolations: FxHashMap<String, String>,
}

impl<'a> HtmlToR3Transform<'a> {
    /// Creates a new HTML to R3 transformer.
    pub fn new(allocator: &'a Allocator, source_text: &'a str, options: TransformOptions) -> Self {
        let comment_nodes =
            if options.collect_comment_nodes { Some(Vec::new_in(&allocator)) } else { None };

        Self {
            allocator,
            source_text,
            binding_parser: BindingParser::new(allocator),
            errors: std::vec::Vec::new(),
            styles: Vec::new_in(&allocator),
            style_urls: Vec::new_in(&allocator),
            ng_content_selectors: Vec::new_in(&allocator),
            comment_nodes,
            processed_nodes: FxHashSet::default(),
            namespace_stack: std::vec::Vec::new(),
            non_bindable_depth: 0,
            i18n_depth: 0,
            block_placeholder_counter: 0,
            icu_placeholder_counts: FxHashMap::default(),
            i18n_placeholder_registry: PlaceholderRegistry::new(),
            i18n_message_instance_counter: 0,
            angular_version: options.angular_version,
            sole_icu_message: None,
            message_names: MessagePlaceholderNames::default(),
            icu_names: IcuPlaceholderNames::default(),
            element_full_names: std::vec::Vec::new(),
            i18n_source_file: std::cell::OnceCell::new(),
        }
    }

    /// Returns the template source file that i18n messages point into.
    fn i18n_source_file(&self) -> std::sync::Arc<crate::util::ParseSourceFile> {
        std::sync::Arc::clone(self.i18n_source_file.get_or_init(|| {
            std::sync::Arc::new(crate::util::ParseSourceFile::new(self.source_text, "<template>"))
        }))
    }

    fn security_context(&self, element: &str, property: &str) -> SecurityContext {
        // Element bindings go through `calcPossibleSecurityContexts` upstream,
        // which promotes a bare element missing from the DOM schema to its
        // `:svg:`/`:math:` form (`<animate>` → `:svg:animate`) on the
        // namespaced schema.
        element_security_context_for(element, property, self.angular_version)
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
        let mut result = Vec::new_in(&self.allocator);

        for (index, node) in siblings.iter().enumerate() {
            // Skip nodes that were already processed as connected blocks
            let node_id = std::ptr::from_ref(node) as usize;
            if self.processed_nodes.contains(&node_id) {
                continue;
            }

            // In ngNonBindable context, blocks should be flattened into text nodes + children
            // (not wrapped in a Template). TypeScript returns [startText, ...children, endText].flat()
            if self.non_bindable_depth > 0
                && let HtmlNode::Block(block) = node
            {
                let block_nodes = self.visit_block_as_text_flat(block);
                for child in block_nodes {
                    result.push(child);
                }
                continue;
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
                    let text_value = Ident::from_in(reconstructed.as_str(), &self.allocator);
                    let r3_text = R3Text { value: text_value, source_span: decl.span };
                    return Some(R3Node::Text(Box::new_in(r3_text, &self.allocator)));
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

        // Angular's parser bakes the namespace into the element name with
        // `_getPrefix` + `mergeNsAndName` (`:svg:rect`, `:xml:div`). The parser
        // here keeps the local name plus an explicit `:ns:` prefix, so the
        // upstream name is reconstructed and the inherited prefix is tracked
        // on a stack.
        let parent_prefix = self.current_prefix();
        // `<MyComp:iframe>` keeps the class in `name`. The host tag is what
        // `isTrustedTypesSink` and the binding security lookup see, including an
        // explicit prefix and a prefix inherited from the parent
        // (`:svg:ng-component`, `:svg:iframe`). No prefix and no local tag is
        // `tagName === null`.
        let host_tag = if element.is_component {
            Self::selectorless_host_tag(element, parent_prefix)
        } else {
            None
        };
        // `_getPrefix`: the explicit `:ns:` prefix wins, then the tag's implicit
        // namespace (`svg`, `math`, `foreignObject`), then the parent's prefix —
        // kept verbatim, so arbitrary prefixes like `:xml:` inherit too.
        let resolved_name = if element.is_component {
            String::new()
        } else {
            Self::resolve_element_name(raw_name, parent_prefix)
        };
        let security_name = if element.is_component {
            Self::component_security_name(host_tag.as_deref(), self.angular_version)
        } else {
            // The security lookup gets the verbatim resolved name;
            // `get_security_context_for` decides per target version whether
            // `normalizeTagName` strips non-svg/math prefixes.
            resolved_name.clone()
        };
        // Trusted Types and the script/style sets use the parser's full name.
        let qualified_name = resolved_name.to_ascii_lowercase();
        // Children inherit this element's own resolved prefix, or nothing when
        // its tag definition prevents namespace inheritance (`foreignObject`) or
        // the host tag is `tagName === null`.
        let child_prefix = if element.is_component {
            Self::component_child_prefix(host_tag.as_deref())
        } else {
            Self::inheritable_prefix(&resolved_name)
        };

        if element.is_component {
            // `visitComponent` does not run the element preparser. A class named
            // `Script` is not an HTML script. Unsupported hosts are the host tag
            // (`<MyComp:script>`), not the class.
            if let Some(tag) = host_tag.as_deref()
                && UNSUPPORTED_SELECTORLESS_TAGS.contains(&tag)
            {
                self.report_error(
                    &format!("Tag name \"{tag}\" cannot be used as a component tag"),
                    element.start_span,
                );
                return None;
            }
        } else {
            // HTML `<script>` is always stripped. `:svg:script` is stripped from
            // v22 (`SCRIPT_ELEMENTS`). Other prefixes, such as `:xml:script`, stay.
            if qualified_name == "script"
                || (strips_namespaced_svg_script(self.angular_version)
                    && qualified_name == ":svg:script")
            {
                return None;
            }
            // The preparser classified `:svg:style` as a style element only on
            // 20.3.22 and 21.2.14 (`STYLE_ELEMENTS`); elsewhere it stays an
            // ordinary element.
            if qualified_name == "style"
                || (strips_namespaced_svg_style(self.angular_version)
                    && qualified_name == ":svg:style")
            {
                if let Some(content) = self.get_text_content(element) {
                    self.styles.push(content);
                }
                return None;
            }
            // Like `script` and `style` above, the preparser lowercases the name before
            // it looks for `link` and `ng-content` (`preparseElement`).
            if raw_name.eq_ignore_ascii_case("link") {
                // A stylesheet `<link>` with a resolvable URL is compiled into
                // `styles` and dropped from the tree (`r3_template_transform.ts`
                // returns `null`). Inside ngNonBindable the preparser still drops
                // stylesheet links (`NonBindableVisitor`), but the URL is discarded
                // because that visitor never reaches `styleUrls`.
                if let Some(href) = self.get_stylesheet_href(element) {
                    if self.non_bindable_depth == 0 {
                        self.style_urls.push(href);
                    }
                    return None;
                }
                // Inside ngNonBindable a stylesheet `<link>` is dropped even when
                // its URL is unresolvable or missing — `NonBindableVisitor` checks
                // only `type === STYLESHEET`, not `isStyleUrlResolvable`.
                if self.non_bindable_depth > 0 && self.is_stylesheet_link(element) {
                    return None;
                }
                // Every other `<link>` stays an element.
            }
        }

        let i18n_element_name =
            if element.is_component { host_tag.as_deref() } else { Some(qualified_name.as_str()) };

        // Parse attributes
        let (attributes, inputs, outputs, references, variables, template_attr) = self
            .parse_attributes(
                &element.attrs,
                &security_name,
                i18n_element_name,
                // `isNgTemplate` sees the namespace-stripped local name.
                split_ns_name(raw_name).1 == "ng-template",
            );

        // `child_prefix` is what children inherit; `foreignObject` already
        // resolves to `:svg:foreignObject` and passes nothing on.
        self.namespace_stack.push(child_prefix);

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
            if element.children.is_empty() {
                String::new()
            } else {
                // Use I18nMessageFactory to convert children to i18n AST and serialize
                let factory = I18nMessageFactory::new(false, true);
                let source_file = self.i18n_source_file();
                let meta = parse_i18n_meta(attr.value.as_str());

                let message = factory.create_message(
                    &element.children,
                    Some(self.element_i18n_name(&resolved_name, host_tag.as_deref())),
                    Some(meta.meaning),
                    Some(meta.description),
                    Some(meta.custom_id),
                    None,
                    source_file,
                );
                collect_placeholder_names(&message.nodes, &mut self.message_names);
                message.serialize()
            }
        } else {
            String::new()
        };

        // Now create the i18n metadata with the generated message string
        let i18n_meta = if let Some(attr) = i18n_attr {
            // Element has its own i18n attribute - parse it as a Message with message string
            let instance_id = self.allocate_i18n_message_instance_id();
            Some(parse_i18n_meta_with_message(
                &self.allocator,
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

        if let (Some(attr), Some(I18nMeta::Message(message))) = (i18n_attr, &i18n_meta)
            && matches!(element.children.as_slice(), [HtmlNode::Expansion(_)])
        {
            self.sole_icu_message = Some((attr.value.to_string(), message.instance_id));
        }

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
        self.element_full_names
            .push(self.element_i18n_name(&resolved_name, host_tag.as_deref()).to_string());
        let children = self.visit_children(&element.children);
        self.element_full_names.pop();
        // Consumed by the ICU child, if any; do not leak it to a later ICU.
        self.sole_icu_message = None;

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
        let directives = self.transform_directives(&element.directives, &security_name);

        // Determine if element is self-closing (explicitly closed with />)
        let is_self_closing = element.is_self_closing;

        // Check for ng-content
        // Reference: r3_template_transform.ts lines 191-204
        // Children are passed directly without extra whitespace filtering
        // `preparseElement` lowercases the name and `isNgContent` reads the local
        // name, so namespaced and capitalised spellings both count.
        if split_ns_name(raw_name).1.eq_ignore_ascii_case("ng-content") {
            let selector = self.get_ng_content_selector(element);
            self.ng_content_selectors.push(selector);

            // For ng-content, Angular converts ALL raw HTML attributes to TextAttributes.
            // Reference: r3_template_transform.ts line 193:
            //   const attrs: t.TextAttribute[] = element.attrs.map((attr) => this.visitAttribute(attr));
            // This includes bound attributes like [select]="..." which get serialized with
            // their raw names (e.g., "[select]") and values into the projection instruction.
            //
            // However, i18n/i18n-* attributes are excluded because Angular's I18nMetaVisitor
            // strips them from element.attrs before r3_template_transform runs.
            let mut content_attributes: Vec<'a, R3TextAttribute<'a>> =
                Vec::with_capacity_in(element.attrs.len(), &self.allocator);
            for attr in &element.attrs {
                let name = attr.name.as_str();
                if name == "i18n" || name.starts_with("i18n-") {
                    continue;
                }
                content_attributes.push(R3TextAttribute {
                    name: attr.name,
                    value: attr.value,
                    source_span: attr.span,
                    key_span: Some(attr.name_span),
                    value_span: attr.value_span,
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
                // Inside an i18n block, <ng-content> is a tag placeholder like any element.
                i18n: i18n_meta,
            };
            let mut result = R3Node::Content(Box::new_in(content, &self.allocator));

            // Wrap in template if has structural directive (*ngIf, etc.)
            // Reference: r3_template_transform.ts lines 266-277
            if let Some(template_attr_info) = template_attr {
                result = self.wrap_in_template(result, element, template_attr_info);
            }

            return Some(result);
        }

        // Check for ng-template. `isNgTemplate` reads the local name without
        // lowering it: a namespaced `<svg:ng-template>` is a template, but a
        // capitalised `<NG-TEMPLATE>` is an ordinary element.
        if split_ns_name(raw_name).1 == "ng-template" {
            let name = Ident::from_in(resolved_name.as_str(), &self.allocator);
            let template = R3Template {
                tag_name: Some(name),
                attributes,
                inputs,
                outputs,
                directives,
                template_attrs: Vec::new_in(&self.allocator),
                children,
                references,
                variables,
                is_self_closing,
                source_span: element.span,
                start_source_span: element.start_span,
                end_source_span: element.end_span,
                i18n: i18n_meta,
            };
            let mut result = R3Node::Template(Box::new_in(template, &self.allocator));

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
            for input in &inputs {
                if input.binding_type == BindingType::Attribute {
                    self.report_error(
                        "Attribute bindings are not supported on ng-container. Use property bindings instead.",
                        input.source_span,
                    );
                }
            }
        }

        let name = Ident::from_in(resolved_name.as_str(), &self.allocator);

        // Only the lexer decides that a tag is a selectorless component, and only when
        // selectorless parsing is enabled. An element name is otherwise kept as written,
        // whatever its case: `<View>` is an element named "View".
        let mut result = if element.is_component {
            // Validate selectorless references
            self.validate_selectorless_references(&references);

            // `tagName` is the resolved host tag (`_getComponentTagName`), so it
            // carries an explicit, implicit, or inherited prefix.
            let tag_name = host_tag.map(|tag| Ident::from_in(tag.as_str(), &self.allocator));

            // Compute full_name: "ComponentName:prefix:tag_name" or "ComponentName:tag_name"
            let full_name = match &tag_name {
                Some(tag) if tag.starts_with(':') => {
                    // Namespace format: "MyComp:svg:rect" (tag_name already has :prefix:)
                    Ident::from_in(&format!("{}{}", element.name, tag), &self.allocator)
                }
                Some(tag) => {
                    // Simple format: "MyComp:div"
                    Ident::from_in(&format!("{}:{}", element.name, tag), &self.allocator)
                }
                None => element.name,
            };

            // Create R3Component for uppercase element names
            let r3_component = R3Component {
                component_name: element.name,
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
            R3Node::Component(Box::new_in(r3_component, &self.allocator))
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
                // `element.isVoid` in TypeScript is the parser's own tag-definition
                // lookup; `HtmlElement.is_void` already holds it and is
                // case-insensitive (`<BR>` is void upstream).
                is_void: element.is_void,
                i18n: i18n_meta,
            };
            R3Node::Element(Box::new_in(r3_element, &self.allocator))
        };

        // Wrap in template if has structural directive
        if let Some(template_attr_info) = template_attr {
            result = self.wrap_in_template(result, element, template_attr_info);
        }

        Some(result)
    }

    /// Visits an HTML component (selectorless component AST node).
    fn visit_html_component(&mut self, component: &HtmlComponent<'a>) -> Option<R3Node<'a>> {
        let parent_prefix = self.current_prefix();
        let host_tag = Self::component_node_host_tag(component, parent_prefix);
        if let Some(tag) = host_tag.as_deref()
            && UNSUPPORTED_SELECTORLESS_TAGS.contains(&tag)
        {
            self.report_error(
                &format!("Tag name \"{tag}\" cannot be used as a component tag"),
                component.start_span,
            );
            return None;
        }

        // `tagName === null` is not a Trusted Types sink. `full_name` is the class.
        let i18n_element_name = host_tag.as_deref();
        let security_name =
            Self::component_security_name(host_tag.as_deref(), self.angular_version);
        let (attributes, inputs, outputs, references, _variables, template_attr) =
            self.parse_attributes(&component.attrs, &security_name, i18n_element_name, false);

        // Children inherit the host tag's prefix verbatim (`_getPrefix` uses
        // `component.tagName` as the parent name), honoring
        // `preventNamespaceInheritance` on hosts like `foreignObject`.
        self.namespace_stack.push(Self::component_child_prefix(host_tag.as_deref()));

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
        // `security_name` is empty when the host tag is `tagName === null`.
        let directives = self.transform_directives(&component.directives, &security_name);

        // Validate selectorless references
        self.validate_selectorless_references(&references);

        // Create R3Component
        let r3_component = R3Component {
            component_name: component.component_name,
            tag_name: component.tag_name,
            full_name: component.full_name,
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
        let mut result = R3Node::Component(Box::new_in(r3_component, &self.allocator));

        // Wrap in template if has structural directive
        if let Some(template_attr_info) = template_attr {
            result = self.wrap_component_in_template(result, component, template_attr_info);
        }

        Some(result)
    }

    /// The name Angular's i18n placeholders are built from: the resolved element name
    /// (`:svg:circle`), or the host tag for a selectorless component.
    fn element_i18n_name<'n>(&self, resolved_name: &'n str, host_tag: Option<&'n str>) -> &'n str {
        if resolved_name.is_empty() { host_tag.unwrap_or("") } else { resolved_name }
    }

    /// Prefix inherited from the innermost open element (`""` at the root).
    fn current_prefix(&self) -> &str {
        self.namespace_stack.last().map_or("", String::as_str)
    }

    /// `_getPrefix` + `mergeNsAndName`: explicit `:ns:` prefix, then the tag's
    /// `implicitNamespacePrefix`, then the parent's prefix kept verbatim so
    /// arbitrary prefixes (`:xml:div`) inherit.
    fn resolve_element_name(raw_name: &str, parent_prefix: &str) -> String {
        let (explicit_ns, local) = split_ns_name(raw_name);
        if let Some(prefix) = explicit_ns {
            return format!(":{prefix}:{local}");
        }
        let prefix = get_html_tag_definition(local)
            .implicit_namespace_prefix
            .map_or_else(|| parent_prefix.to_string(), str::to_string);
        if prefix.is_empty() { local.to_string() } else { format!(":{prefix}:{local}") }
    }

    /// Prefix this element passes on to its children. `foreignObject` (and any
    /// other tag with `preventNamespaceInheritance`) passes nothing.
    fn inheritable_prefix(resolved_name: &str) -> String {
        let (ns, local) = split_ns_name(resolved_name);
        if get_html_tag_definition(local).prevent_namespace_inheritance {
            return String::new();
        }
        ns.unwrap_or("").to_string()
    }

    /// Host tag of a selectorless component, matching Angular's `tagName`.
    ///
    /// Prefix order matches `_getPrefix`: explicit prefix, then the host tag's
    /// implicit namespace (`svg`, `math`, `foreignObject`), then the parent's
    /// prefix verbatim. A prefix with no local tag becomes `ng-component`. No
    /// prefix and no local tag is `tagName === null`.
    fn selectorless_host_tag(element: &HtmlElement<'a>, parent_prefix: &str) -> Option<String> {
        Self::canonical_host_tag(
            element.component_prefix.as_ref().map(|prefix| prefix.as_str()),
            element.component_tag_name.as_ref().map(|tag| tag.as_str()),
            parent_prefix,
        )
    }

    /// `HtmlComponent.tag_name` is either already `:ns:local` or a local name.
    fn component_node_host_tag(
        component: &HtmlComponent<'a>,
        parent_prefix: &str,
    ) -> Option<String> {
        match component.tag_name.as_ref().map(|tag| tag.as_str()) {
            Some(tag) if tag.starts_with(':') => Some(tag.to_string()),
            other => Self::canonical_host_tag(None, other, parent_prefix),
        }
    }

    fn canonical_host_tag(
        explicit_prefix: Option<&str>,
        local_tag: Option<&str>,
        parent_prefix: &str,
    ) -> Option<String> {
        let mut prefix = explicit_prefix.unwrap_or("").to_string();
        if prefix.is_empty()
            && let Some(tag) = local_tag
            && let Some(implicit) = get_html_tag_definition(tag).implicit_namespace_prefix
        {
            prefix = implicit.to_string();
        }
        if prefix.is_empty() {
            prefix = parent_prefix.to_string();
        }
        match (prefix.is_empty(), local_tag) {
            (true, None) => None,
            (true, Some(tag)) => Some(tag.to_string()),
            (false, local) => {
                let local = local.unwrap_or("ng-component");
                Some(format!(":{prefix}:{local}"))
            }
        }
    }

    /// Prefix a selectorless component passes to its children. `tagName ===
    /// null` and hosts with `preventNamespaceInheritance` pass nothing.
    fn component_child_prefix(host_tag: Option<&str>) -> String {
        let Some(tag) = host_tag else {
            return String::new();
        };
        let (ns, local) = split_ns_name(tag);
        if get_html_tag_definition(local).prevent_namespace_inheritance {
            return String::new();
        }
        ns.unwrap_or("").to_string()
    }

    /// Security-schema name for a selectorless component, matching
    /// `calcPossibleSecurityContexts(component.tagName, ...)`.
    ///
    /// On the namespaced schema a bare host tag that is not an HTML element is
    /// rewritten to its known `:svg:`/`:math:` form (`animate` →
    /// `:svg:animate`); pre-v22 versions look up the bare tag (`animate|to`).
    /// `tagName === null` resolves over every known element upstream; the empty
    /// name reproduces that through the `*|attr` fallback (`src` → `NONE`,
    /// `innerHTML` → `HTML`).
    fn component_security_name(host_tag: Option<&str>, version: Option<AngularVersion>) -> String {
        let Some(tag) = host_tag else {
            return String::new();
        };
        if !uses_namespaced_schema(version) {
            return tag.to_string();
        }
        let lower = tag.to_ascii_lowercase();
        let (ns, local) = split_ns_name(&lower);
        if ns.is_none() && !is_known_element(local) {
            let svg = format!(":svg:{local}");
            if is_known_element(&svg) {
                return svg;
            }
            let math = format!(":math:{local}");
            if is_known_element(&math) {
                return math;
            }
        }
        tag.to_string()
    }

    /// Transforms HTML directives to R3 directives.
    /// Implements validation from Angular's `extractDirectives()` and `validateSelectorlessReferences()`.
    fn transform_directives(
        &mut self,
        html_directives: &[HtmlDirective<'a>],
        element_name: &str,
    ) -> Vec<'a, R3Directive<'a>> {
        let mut directives = Vec::new_in(&self.allocator);
        let mut seen_directives: FxHashSet<&str> = FxHashSet::default();

        for html_dir in html_directives {
            let directive_name = html_dir.name.as_str();

            // Check for duplicate directives
            // Reference: r3_template_transform.ts lines 924-930
            if seen_directives.contains(directive_name) {
                self.report_error(
                    &format!(
                        "Cannot apply directive \"{directive_name}\" multiple times on the same element"
                    ),
                    html_dir.span,
                );
                continue;
            }
            seen_directives.insert(directive_name);

            // Parse directive attributes similar to element attributes
            let mut attributes = Vec::new_in(&self.allocator);
            let mut inputs = Vec::new_in(&self.allocator);
            let mut outputs = Vec::new_in(&self.allocator);
            let mut references = Vec::new_in(&self.allocator);
            let mut seen_reference_names: FxHashSet<&str> = FxHashSet::default();
            let mut invalid = false;

            for attr in &html_dir.attrs {
                let attr_name = attr.name.as_str();
                let attr_value = attr.value.as_str();

                // Check for unsupported attributes
                // Reference: r3_template_transform.ts lines 908-922
                if attr_name.starts_with(TEMPLATE_ATTR_PREFIX) {
                    self.report_error(
                        &format!(
                            "Shorthand template syntax \"{attr_name}\" is not supported inside a directive context"
                        ),
                        attr.span,
                    );
                    invalid = true;
                    continue;
                }

                if attr_name == "ngProjectAs" || attr_name == "ngNonBindable" {
                    self.report_error(
                        &format!(
                            "Attribute \"{attr_name}\" is not supported in a directive context"
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
                    // Reference: r3_template_transform.ts line 789
                    let key_span = create_key_span(
                        attr,
                        &attr_name[..attr_name.len() - ref_name.len()],
                        ref_name,
                    );

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
                            name: Ident::from_in(ref_name, &self.allocator),
                            value: Ident::from_in("", &self.allocator),
                            source_span: attr.span,
                            key_span,
                            value_span: None,
                        });
                    }
                    continue;
                }

                // Check for binding prefix syntax (bind-, on-, bindon-)
                if let Some((prefix, rest)) = self.parse_binding_prefix(attr_name) {
                    // Reference: r3_template_transform.ts lines 765-811 (`createKeySpan(srcSpan,
                    // bindParts[KW_*_IDX], identifier)`)
                    let key_span =
                        create_key_span(attr, &attr_name[..attr_name.len() - rest.len()], rest);
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
                                name: Ident::from_in(prop_name, &self.allocator),
                                binding_type,
                                value: parse_result.ast,
                                unit: None,
                                source_span: attr.span,
                                key_span: property_binding_key_span(rest, key_span),
                                value_span: Some(value_span),
                                i18n: None,
                                security_context: self.security_context(element_name, prop_name),
                            });
                        }
                        BindingPrefix::On => {
                            // on-event="handler" - event binding
                            let value_span = attr.value_span.unwrap_or(attr.span);
                            let parse_result =
                                self.binding_parser.parse_event(attr_value, value_span);

                            outputs.push(R3BoundEvent {
                                name: Ident::from_in(rest, &self.allocator),
                                handler: parse_result.ast,
                                target: None,
                                event_type: ParsedEventType::Regular,
                                phase: None,
                                source_span: attr.span,
                                key_span: event_key_span(rest, key_span),
                                handler_span: value_span,
                            });
                        }
                        BindingPrefix::BindOn => {
                            // bindon-prop="value" - two-way binding
                            let value_span = attr.value_span.unwrap_or(attr.span);
                            let parse_result =
                                self.binding_parser.parse_binding(attr_value, value_span);

                            inputs.push(R3BoundAttribute {
                                name: Ident::from_in(rest, &self.allocator),
                                binding_type: BindingType::TwoWay,
                                value: parse_result.ast,
                                unit: None,
                                source_span: attr.span,
                                key_span: property_binding_key_span(rest, key_span),
                                value_span: Some(value_span),
                                i18n: None,
                                security_context: self.security_context(element_name, rest),
                            });

                            // Two-way binding also creates an output event
                            let event_name =
                                Ident::from_in(&format!("{rest}Change"), &self.allocator);
                            let event_parse_result =
                                self.binding_parser.parse_event(attr_value, value_span);

                            outputs.push(R3BoundEvent {
                                key_span: event_key_span(&event_name, key_span),
                                name: event_name,
                                handler: event_parse_result.ast,
                                target: None,
                                event_type: ParsedEventType::TwoWay,
                                phase: None,
                                source_span: attr.span,
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
                                name: Ident::from_in(rest, &self.allocator),
                                binding_type: BindingType::LegacyAnimation,
                                value: parse_result.ast,
                                unit: None,
                                source_span: attr.span,
                                // `parseLiteralAttr` drops the `@` from the key span.
                                // Reference: binding_parser.ts lines 308-314
                                key_span: create_key_span(attr, "@", rest),
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
                    // Reference: r3_template_transform.ts line 871
                    let key_span = create_key_span(attr, BANANA_BOX_START, prop_name);
                    let value_str = self.allocator.alloc_str(attr_value);
                    let value_span = attr.value_span.unwrap_or(attr.span);
                    let parse_result = self.binding_parser.parse_binding(value_str, value_span);

                    inputs.push(R3BoundAttribute {
                        name: Ident::from_in(prop_name, &self.allocator),
                        binding_type: BindingType::TwoWay,
                        value: parse_result.ast,
                        unit: None,
                        source_span: attr.span,
                        key_span: property_binding_key_span(prop_name, key_span),
                        value_span: Some(value_span),
                        i18n: None,
                        security_context: self.security_context(element_name, prop_name),
                    });

                    // Two-way binding also creates an output event
                    let event_name = Ident::from_in(&format!("{prop_name}Change"), &self.allocator);
                    let event_value_str = self.allocator.alloc_str(attr_value);
                    let event_parse_result =
                        self.binding_parser.parse_event(event_value_str, value_span);

                    outputs.push(R3BoundEvent {
                        key_span: event_key_span(&event_name, key_span),
                        name: event_name,
                        handler: event_parse_result.ast,
                        target: None,
                        event_type: ParsedEventType::TwoWay,
                        phase: None,
                        source_span: attr.span,
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
                        name: Ident::from_in(prop_name, &self.allocator),
                        binding_type: BindingType::Property,
                        value: parse_result.ast,
                        unit: None,
                        source_span: attr.span,
                        // Reference: r3_template_transform.ts line 871
                        key_span: property_binding_key_span(
                            prop_name,
                            create_key_span(attr, "[", prop_name),
                        ),
                        value_span: Some(value_span),
                        i18n: None,
                        security_context: self.security_context(element_name, prop_name),
                    });
                } else if attr_name.starts_with('(') && attr_name.ends_with(')') {
                    // Event binding: (event)="handler"
                    let event_name = &attr_name[1..attr_name.len() - 1];
                    // Reference: r3_template_transform.ts line 871
                    let key_span =
                        event_key_span(event_name, create_key_span(attr, "(", event_name));

                    // Check for legacy animation syntax (@) or (@name.phase)
                    if event_name.starts_with('@') {
                        let anim_name = event_name.strip_prefix('@').unwrap_or("");
                        outputs.push(self.create_legacy_animation_event(anim_name, attr, key_span));
                    } else {
                        // Regular event handling
                        let value_str = self.allocator.alloc_str(attr_value);
                        let value_span = attr.value_span.unwrap_or(attr.span);
                        let parse_result = self.binding_parser.parse_event(value_str, value_span);

                        outputs.push(R3BoundEvent {
                            name: Ident::from_in(event_name, &self.allocator),
                            handler: parse_result.ast,
                            target: None,
                            event_type: ParsedEventType::Regular,
                            phase: None,
                            source_span: attr.span,
                            key_span,
                            handler_span: value_span,
                        });
                    }
                } else {
                    // Plain text attribute
                    attributes.push(R3TextAttribute {
                        name: attr.name,
                        value: attr.value,
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
                name: html_dir.name,
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
    /// The `VAR_*` name of the ICU whose switch expression starts at `start`, from the ICU's
    /// message, or a newly generated one.
    fn icu_var_name(&mut self, start: u32, base_name: &str) -> String {
        match self.icu_names.vars.remove(&start) {
            // Angular's visitExpansion trims the key: `{count, select , ...}` is named
            // "VAR_SELECT " in the message but the var is VAR_SELECT.
            Some(name) => name.trim().to_string(),
            None => self.generate_unique_icu_placeholder(base_name),
        }
    }

    fn generate_unique_icu_placeholder(&mut self, base_name: &str) -> String {
        let count = self.icu_placeholder_counts.entry(base_name.to_string()).or_insert(0);
        let result =
            if *count == 0 { base_name.to_string() } else { format!("{base_name}_{count}") };
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
        // Every ICU gets an i18n message, including ICUs outside an i18n block: Angular's
        // I18nMetaVisitor visits all expansions, and the wrap_icus phase later gives such an ICU
        // its own i18n block.

        // Reset ICU placeholder counters for this new top-level ICU.
        // This ensures unique placeholder names within each ICU context.
        // Ported from Angular's PlaceholderRegistry usage in i18n_parser.ts.
        self.reset_icu_placeholder_counts();

        // The ICU's message is built from the expansion itself (Angular's I18nMetaVisitor:
        // `_generateI18nMessage([expansion], currentMessage || meta)`). An ICU that is the only
        // child of an i18n element shares that element's message, so it is the message rather
        // than a sub-message of it.
        let source_file = self.i18n_source_file();
        let icu_message = I18nMessageFactory::new(false, true).create_icu_message(
            expansion,
            self.element_full_names.last().map(String::as_str),
            source_file,
        );
        let message_string = icu_message.serialize();
        // Angular's meta visitor gives an ICU sub-message no meaning.
        let associated_message_id = crate::i18n::compute_msg_id(
            &crate::i18n::ast::serialize_message_for_id(&icu_message.nodes),
            "",
        );
        // Angular builds the ICU's placeholders from the message's placeholders: `VAR_*` become
        // vars, interpolations become bound text, and element markup becomes plain text.
        let (vars, others): (std::vec::Vec<_>, std::vec::Vec<_>) =
            icu_message.placeholders.into_iter().partition(|(name, _)| name.starts_with("VAR_"));
        self.icu_names.vars = vars
            .into_iter()
            .map(|(name, placeholder)| (placeholder.source_span.start.offset, name))
            .collect();
        let (interpolations, mut tags): (std::vec::Vec<_>, std::vec::Vec<_>) =
            others.into_iter().partition(|(_, placeholder)| placeholder.text.starts_with("{{"));
        self.icu_names.interpolations = interpolations
            .into_iter()
            .map(|(name, placeholder)| (placeholder.text, name))
            .collect();
        tags.sort_by(|a, b| a.0.cmp(&b.0));
        let (meta_value, instance_id, is_sub_message) = match self.sole_icu_message.take() {
            Some((meta_value, instance_id)) => (meta_value, instance_id, false),
            None => (String::new(), self.allocate_i18n_message_instance_id(), true),
        };
        let I18nMeta::Message(mut i18n_message) =
            parse_i18n_meta_with_message(self.allocator, &meta_value, instance_id, &message_string)
        else {
            unreachable!("parse_i18n_meta_with_message always returns a Message")
        };
        if is_sub_message {
            i18n_message.associated_message_id =
                Ident::from_in(associated_message_id.as_str(), self.allocator);
        }

        // The pipeline identifies the ICU by a message holding a single IcuPlaceholder.
        let icu_type_upper = expansion.expansion_type.as_str().to_uppercase();
        let base_name = format!("VAR_{icu_type_upper}");
        let expression_placeholder = Ident::from_in(
            &self.icu_var_name(expansion.switch_value_span.start, &base_name),
            self.allocator,
        );
        i18n_message.nodes.push(I18nNode::IcuPlaceholder(I18nIcuPlaceholder {
            value: Box::new_in(
                I18nIcu {
                    expression: expansion.switch_value,
                    icu_type: expansion.expansion_type,
                    cases: HashMap::new_in(self.allocator),
                    source_span: expansion.span,
                    expression_placeholder: Some(expression_placeholder),
                },
                &self.allocator,
            ),
            name: Ident::from_in(
                self.message_names.icus.remove(&expansion.span.start).as_deref().unwrap_or("ICU"),
                self.allocator,
            ),
            source_span: expansion.span,
        }));

        // Create variable for the switch value (using VAR_* placeholder name)
        let mut vars = Vec::new_in(&self.allocator);
        let switch_value_str = expansion.switch_value.as_str();
        let switch_value_span = expansion.switch_value_span;

        // Parse the switch value as a binding expression
        let value_str = self.allocator.alloc_str(switch_value_str);
        let parse_result = self.binding_parser.parse_binding(value_str, switch_value_span);

        // Parse placeholders from expansion cases FIRST (before adding the outer VAR).
        // This matches Angular's visitExpansion behavior where nested ICUs are visited first,
        // and their VAR_* placeholders are added before the outer ICU's VAR_*.
        // Ported from Angular's i18n_parser.ts:137-159
        let mut placeholders = Vec::new_in(&self.allocator);
        for case in &expansion.cases {
            self.extract_placeholders_from_nodes(&case.expansion, &mut placeholders, &mut vars);
        }
        for (name, placeholder) in tags {
            // Angular: `_visitTextWithInterpolation(value.text, ...)`, so markup such as
            // `<span title="{{a}}">` becomes bound text with expression placeholders.
            let span =
                Span::new(placeholder.source_span.start.offset, placeholder.source_span.end.offset);
            let text = self.allocator.alloc_str(&placeholder.text);
            let value = self.parse_interpolation(text, span).map(|value| {
                R3IcuPlaceholder::BoundText(R3BoundText { value, source_span: span, i18n: None })
            });
            let value = value.unwrap_or_else(|| {
                R3IcuPlaceholder::Text(R3Text { value: Ident::from(text), source_span: span })
            });
            ordered_insert_placeholder(
                &mut placeholders,
                Ident::from_in(name.as_str(), self.allocator),
                value,
            );
        }

        // Add the outer ICU's var AFTER processing all children.
        // This ensures the correct order: nested ICU vars first, then outer ICU var.
        // Use the unique VAR_* placeholder name as the key, matching Angular's behavior.
        // The expression_placeholder was already generated above with getUniquePlaceholder.
        ordered_insert_var(
            &mut vars,
            expression_placeholder,
            R3BoundText { value: parse_result.ast, source_span: switch_value_span, i18n: None },
        );

        let icu = R3Icu {
            vars,
            placeholders,
            source_span: expansion.span,
            i18n: Some(I18nMeta::Message(i18n_message)),
        };

        Some(R3Node::Icu(Box::new_in(icu, &self.allocator)))
    }

    /// Extracts placeholders and nested ICU vars from expansion case nodes.
    fn extract_placeholders_from_nodes(
        &mut self,
        nodes: &[HtmlNode<'a>],
        placeholders: &mut Vec<'a, (Ident<'a>, R3IcuPlaceholder<'a>)>,
        vars: &mut Vec<'a, (Ident<'a>, R3BoundText<'a>)>,
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

                    // Name the nested ICU's VAR_* placeholder as the ICU's message does
                    // (Angular numbers nested ICUs before their parent).
                    let icu_type_upper = nested_expansion.expansion_type.as_str().to_uppercase();
                    let base_name = format!("VAR_{icu_type_upper}");
                    let unique_name = self.icu_var_name(switch_value_span.start, &base_name);
                    let var_placeholder_name = Ident::from_in(&unique_name, self.allocator);

                    // Recursively extract from nested expansion cases FIRST.
                    // This ensures placeholders and further nested ICU vars are processed
                    // before adding this ICU's VAR_*.
                    for case in &nested_expansion.cases {
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
                HtmlNode::Element(element) => {
                    // Recurse into element children to extract interpolations inside HTML
                    // elements like `<strong>{{ expr }}</strong>` within ICU branches.
                    // Angular's i18n_parser.ts visitElement recursively visits children,
                    // so interpolations inside elements are correctly registered as
                    // placeholders. Without this recursion, these interpolations are lost.
                    self.extract_placeholders_from_nodes(&element.children, placeholders, vars);
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
        placeholders: &mut Vec<'a, (Ident<'a>, R3IcuPlaceholder<'a>)>,
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

                    let placeholder_key = Ident::from_in(
                        self.icu_names
                            .interpolations
                            .get(interpolation)
                            .map_or(interpolation, String::as_str),
                        self.allocator,
                    );
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

        // Like Angular's `_visitTextWithInterpolation`, always run the interpolation parser
        // (outside ngNonBindable) and keep the text static when it finds no interpolation.
        if self.non_bindable_depth == 0 {
            // Check if there are HTML entities that could affect span calculations.
            // If so, use the tokens which have correct source spans.
            let has_entities = text
                .tokens
                .iter()
                .any(|t| matches!(t.token_type, InterpolatedTokenType::EncodedEntity));

            let expr = if has_entities {
                // Use tokens to parse interpolations with correct source spans
                self.parse_interpolation_from_tokens(text)
            } else {
                // No entities - use the simple path with decoded text
                let interpolation_text = if has_ngsp {
                    let replaced = value_str.replace(NGSP_UNICODE, " ");
                    self.allocator.alloc_str(&replaced)
                } else {
                    value_str
                };
                self.parse_interpolation(interpolation_text, text.span)
            };

            if let Some(expr) = expr {
                // Create i18n metadata for bound text when inside an i18n block.
                // This creates a Container with Text and Placeholder nodes that match
                // the structure expected by the ingest phase.
                // Ported from Angular's r3_template_transform.ts I18nMetaVisitor.visitText
                let i18n_meta =
                    if self.i18n_depth > 0 {
                        let processed_text = if has_ngsp {
                            value_str.replace(NGSP_UNICODE, " ")
                        } else {
                            value_str.to_string()
                        };
                        Some(self.create_i18n_meta_for_text_with_interpolation(
                            &processed_text,
                            text.span,
                        ))
                    } else {
                        None
                    };
                let bound_text =
                    R3BoundText { value: expr, source_span: text.span, i18n: i18n_meta };
                return Some(R3Node::BoundText(Box::new_in(bound_text, &self.allocator)));
            }
        }

        // Static text - use value with ngsp replaced
        let value_atom = if has_ngsp {
            let value_no_ngsp = value_str.replace(NGSP_UNICODE, " ");
            Ident::from_in(&value_no_ngsp, &self.allocator)
        } else {
            text.value
        };
        let r3_text = R3Text { value: value_atom, source_span: text.span };
        Some(R3Node::Text(Box::new_in(r3_text, &self.allocator)))
    }

    /// Visits a comment node.
    /// Angular's visitComment returns null - comments are collected separately, not in AST.
    fn visit_comment(&mut self, comment: &crate::ast::html::HtmlComment<'a>) -> Option<R3Node<'a>> {
        // Only collect comment nodes when the flag is set
        // Angular's visitComment (r3_template_transform.ts:344-349) always returns null
        // Comments are NOT included in the AST output - they're collected separately
        if let Some(ref mut comments) = self.comment_nodes {
            let r3_comment = R3Comment { value: comment.value, source_span: comment.span };
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
        let mut nodes = Vec::new_in(&self.allocator);

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
                value: Ident::from(self.allocator.alloc_str(start_text)),
                source_span: block.start_span,
            },
            &self.allocator,
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
                    value: Ident::from(self.allocator.alloc_str(end_text)),
                    source_span: end_span,
                },
                &self.allocator,
            )));
        }

        // Since we can only return a single node but TypeScript returns an array,
        // we wrap everything in an implicit template (with no tag name).
        // This preserves all the content while matching the single-node return type.
        Some(R3Node::Template(Box::new_in(
            R3Template {
                tag_name: None,
                attributes: Vec::new_in(&self.allocator),
                inputs: Vec::new_in(&self.allocator),
                outputs: Vec::new_in(&self.allocator),
                directives: Vec::new_in(&self.allocator),
                template_attrs: Vec::new_in(&self.allocator),
                children: nodes,
                references: Vec::new_in(&self.allocator),
                variables: Vec::new_in(&self.allocator),
                source_span: block.span,
                start_source_span: block.start_span,
                end_source_span: block.end_span,
                i18n: None,
                is_self_closing: false,
            },
            &self.allocator,
        )))
    }

    /// Visits a block inside ngNonBindable context, returning a flat Vec of nodes.
    /// This is the correct implementation matching TypeScript's NonBindableVisitor.visitBlock().
    /// TypeScript returns [startText, ...children, endText].flat(Infinity).
    fn visit_block_as_text_flat(&mut self, block: &HtmlBlock<'a>) -> Vec<'a, R3Node<'a>> {
        let mut nodes = Vec::new_in(&self.allocator);

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
                value: Ident::from(self.allocator.alloc_str(start_text)),
                source_span: block.start_span,
            },
            &self.allocator,
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
                    value: Ident::from(self.allocator.alloc_str(end_text)),
                    source_span: end_span,
                },
                &self.allocator,
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
                    let node_id = &raw const siblings[idx] as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Mark whitespace between blocks as processed
                for &idx in &whitespace_indices {
                    let node_id = &raw const siblings[idx] as usize;
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
                // Standalone @else/@else if without a preceding @if - emit error and create UnknownBlock
                // Reference: r3_template_transform.ts:515-517
                self.report_error(
                    &format!(
                        "@{} block can only be used after an @if or @else if block.",
                        block.name
                    ),
                    block.span,
                );
                Some(R3Node::UnknownBlock(Box::new_in(
                    crate::ast::r3::R3UnknownBlock {
                        name: block.name,
                        source_span: block.span,
                        name_span: block.name_span,
                    },
                    &self.allocator,
                )))
            }
            BlockType::For => {
                // First, find and collect connected block indices and whitespace
                let (connected_indices, whitespace_indices) =
                    Self::find_connected_block_indices(index, siblings, |bt| {
                        matches!(bt, BlockType::Empty)
                    });
                // Mark connected blocks as processed
                for &idx in &connected_indices {
                    let node_id = &raw const siblings[idx] as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Mark whitespace between blocks as processed
                for &idx in &whitespace_indices {
                    let node_id = &raw const siblings[idx] as usize;
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
                // Standalone @empty without a preceding @for - emit error and create UnknownBlock
                // Reference: r3_template_transform.ts:512-514
                self.report_error("@empty block can only be used after an @for block.", block.span);
                Some(R3Node::UnknownBlock(Box::new_in(
                    crate::ast::r3::R3UnknownBlock {
                        name: block.name,
                        source_span: block.span,
                        name_span: block.name_span,
                    },
                    &self.allocator,
                )))
            }
            BlockType::Switch => self.visit_switch_block(block),
            BlockType::Case | BlockType::Default => {
                // Standalone @case/@default outside @switch - emit error and create UnknownBlock
                // Reference: r3_template_transform.ts:518 - falls through to "Unrecognized block"
                self.report_error(&format!("Unrecognized block @{}.", block.name), block.span);
                Some(R3Node::UnknownBlock(Box::new_in(
                    crate::ast::r3::R3UnknownBlock {
                        name: block.name,
                        source_span: block.span,
                        name_span: block.name_span,
                    },
                    &self.allocator,
                )))
            }
            BlockType::Defer => {
                // First, find and collect connected block indices and whitespace
                let (connected_indices, whitespace_indices) =
                    Self::find_connected_block_indices(index, siblings, |bt| {
                        matches!(bt, BlockType::Placeholder | BlockType::Loading | BlockType::Error)
                    });
                // Mark connected blocks as processed
                for &idx in &connected_indices {
                    let node_id = &raw const siblings[idx] as usize;
                    self.processed_nodes.insert(node_id);
                }
                // Mark whitespace between blocks as processed
                for &idx in &whitespace_indices {
                    let node_id = &raw const siblings[idx] as usize;
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
                // Standalone @placeholder/@loading/@error without a preceding @defer - emit error and create UnknownBlock
                // Reference: r3_template_transform.ts:509-511
                self.report_error(
                    &format!("@{} block can only be used after an @defer block.", block.name),
                    block.span,
                );
                Some(R3Node::UnknownBlock(Box::new_in(
                    crate::ast::r3::R3UnknownBlock {
                        name: block.name,
                        source_span: block.span,
                        name_span: block.name_span,
                    },
                    &self.allocator,
                )))
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

            // Skip over comments without marking them as processed
            // Reference: r3_template_transform.ts:543-546 - Angular skips comments
            // but does NOT add them to processedNodes
            if matches!(node, HtmlNode::Comment(_)) {
                continue;
            }

            // Skip empty text nodes between blocks and mark them as consumed
            if let HtmlNode::Text(text) = node
                && text.value.trim().is_empty()
            {
                whitespace_indices.push(i);
                continue;
            }

            // Check if this is a connected block
            if let HtmlNode::Block(block) = node
                && is_connected(block.block_type)
            {
                connected_indices.push(i);
                continue;
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
            name: decl.name,
            value,
            source_span: decl.span,
            name_span: decl.name_span,
            value_span: decl.value_span,
        };
        Some(R3Node::LetDeclaration(Box::new_in(r3_decl, &self.allocator)))
    }

    /// Creates an i18n BlockPlaceholder for a control flow block when inside an i18n context.
    /// Returns Some if we're inside an i18n region (i18n_depth > 0), None otherwise.
    /// Ported from Angular's i18n_parser.ts:visitBlock which creates BlockPlaceholder nodes.
    fn create_block_placeholder(
        &mut self,
        block_name: &str,
        parameters: &[Ident<'a>],
        source_span: Span,
        start_source_span: Span,
        end_source_span: Option<Span>,
    ) -> Option<I18nMeta<'a>> {
        // Only create placeholders when inside an i18n context
        if self.i18n_depth == 0 {
            return None;
        }

        // Use the names from the enclosing i18n message, as Angular does.
        if let Some((start, close)) = self.message_names.blocks.remove(&source_span.start) {
            let mut params = Vec::new_in(&self.allocator);
            params.extend(parameters.iter().copied());
            return Some(I18nMeta::BlockPlaceholder(I18nBlockPlaceholder {
                name: Ident::from_in(block_name, self.allocator),
                parameters: params,
                start_name: Ident::from_in(start.as_str(), self.allocator),
                close_name: Ident::from_in(close.as_str(), self.allocator),
                children: Vec::new_in(&self.allocator),
                source_span,
                start_source_span: Some(start_source_span),
                end_source_span,
            }));
        }

        // Generate unique placeholder names (following Angular's placeholder naming convention)
        // Angular uses START_BLOCK_IF, START_BLOCK_IF_1, etc.
        let block_upper = block_name.to_uppercase().replace(' ', "_");
        let count = self.block_placeholder_counter;
        self.block_placeholder_counter += 1;

        let start_name = if count == 0 {
            Ident::from_in(format!("START_BLOCK_{block_upper}").as_str(), &self.allocator)
        } else {
            Ident::from_in(format!("START_BLOCK_{block_upper}_{count}").as_str(), &self.allocator)
        };

        let close_name = if count == 0 {
            Ident::from_in(format!("CLOSE_BLOCK_{block_upper}").as_str(), &self.allocator)
        } else {
            Ident::from_in(format!("CLOSE_BLOCK_{block_upper}_{count}").as_str(), &self.allocator)
        };

        // Convert parameters to Atom vec
        let mut params = Vec::new_in(&self.allocator);
        for p in parameters {
            params.push(*p);
        }

        let placeholder = I18nBlockPlaceholder {
            name: Ident::from_in(block_name, &self.allocator),
            parameters: params,
            start_name,
            close_name,
            children: Vec::new_in(&self.allocator),
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
        let mut branches = Vec::new_in(&self.allocator);

        // Parse the main @if branch parameters (condition and optional "as" alias)
        let main_params = parse_conditional_params(
            &self.allocator,
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

        // Match Angular's createIfBlock: only push the main branch when
        // parseConditionalBlockParameters succeeds (returns non-null).
        // When parameters are empty, Angular returns null and skips the branch.
        if main_params.expression.is_some() {
            let children = self.visit_children(&block.children);

            // Create i18n placeholder if inside an i18n context
            let i18n = self.create_block_placeholder(
                "if",
                &[],
                block.span,
                block.start_span,
                block.end_span,
            );

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
        }

        // Validate connected blocks and process @else if and @else blocks
        let mut has_else = false;

        for (i, connected) in connected_blocks.iter().enumerate() {
            match connected.block_type {
                BlockType::ElseIf => {
                    // Parse @else if parameters (condition and optional "as" alias)
                    let params = parse_conditional_params(
                        &self.allocator,
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

                    // Match Angular: only push the branch when params are non-null
                    // (i.e., when an expression was successfully parsed).
                    if params.expression.is_some() {
                        let children = self.visit_children(&connected.children);

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

                    // @else has no condition (null expression) and no alias
                    let children = self.visit_children(&connected.children);

                    // Create i18n placeholder if inside an i18n context
                    // (must be after visit_children to match @if/@else if ordering
                    // and preserve i18n placeholder numbering)
                    let i18n = self.create_block_placeholder(
                        "else",
                        &[],
                        connected.span,
                        connected.start_span,
                        connected.end_span,
                    );
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
        Some(R3Node::IfBlock(Box::new_in(if_block, &self.allocator)))
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
            &self.allocator,
            &block.parameters,
            &self.binding_parser,
            block.start_span,
        );

        // Track whether the core expression failed to parse (to skip track validation).
        let expression_parse_failed = params.expression_parse_failed;

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

        // Process and validate connected @empty block.
        // Angular processes connected blocks before checking params, so we do too.
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

        // Match Angular's createForLoop: if expression parsing failed entirely
        // (params === null in Angular), return None — no ForLoopBlock is emitted.
        if expression_parse_failed {
            return None;
        }

        let item = params.item;
        let expression = params.expression;
        let context_variables = params.context_variables;

        // Match Angular's createForLoop: if track is missing and expression parsed OK,
        // report the error and return None (Angular only creates the node when
        // params !== null AND params.trackBy !== null).
        let (track_by, track_keyword_span) = if let Some(track_info) = params.track_by {
            (track_info.expression, track_info.keyword_span)
        } else {
            self.report_error("@for loop must have a \"track\" expression", block.start_span);
            return None;
        };

        let children = self.visit_children(&block.children);

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
        Some(R3Node::ForLoopBlock(Box::new_in(for_block, &self.allocator)))
    }

    /// Visits a @switch block.
    ///
    /// This implements case grouping logic from Angular's `createSwitchBlock` in r3_control_flow.ts.
    /// Consecutive cases without bodies are grouped together into a single `SwitchBlockCaseGroup`.
    fn visit_switch_block(&mut self, block: &HtmlBlock<'a>) -> Option<R3Node<'a>> {
        use crate::ast::html::{BlockType, HtmlNode};
        use crate::ast::r3::{R3SwitchBlockCase, R3SwitchBlockCaseGroup, R3SwitchExhaustiveCheck};

        // Validation: @switch must have exactly one parameter.
        // Match Angular's createSwitchBlock: always parse the first parameter when present
        // (even if there are extra parameters), only use empty when there are no parameters.
        // Validation errors are reported by validateSwitchBlock in Angular; we report inline.
        if block.parameters.len() != 1 {
            self.report_error("@switch block must have exactly one parameter", block.start_span);
        }
        let expression = if !block.parameters.is_empty() {
            let expr_str = block.parameters[0].expression.as_str();
            let parsed = self.binding_parser.parse_binding(expr_str, block.parameters[0].span);
            parsed.ast
        } else {
            self.binding_parser.parse_binding("", block.span).ast
        };

        let mut groups = Vec::new_in(&self.allocator);
        let mut unknown_blocks = Vec::new_in(&self.allocator);
        let mut collected_cases: std::vec::Vec<R3SwitchBlockCase<'a>> = std::vec::Vec::new();
        let mut first_case_start: Option<Span> = None;
        let mut has_default = false;
        let mut exhaustive_check: Option<R3SwitchExhaustiveCheck<'a>> = None;

        for child in &block.children {
            // Skip comments and whitespace-only text nodes (same as Angular)
            match child {
                HtmlNode::Comment(_) => continue,
                HtmlNode::Text(t) if t.value.trim().is_empty() => continue,
                _ => {}
            }

            // Non-block children (elements, non-whitespace text, etc.) are invalid
            let child_block = if let HtmlNode::Block(b) = child {
                b
            } else {
                self.report_error(
                    "@switch block can only contain @case and @default blocks",
                    child.span(),
                );
                continue;
            };

            // v22: `@default never;` is an exhaustive check, not a case. Its block
            // name is the literal "default never". It carries an optional expression
            // (`@default never(expr);`), no body, and produces no runtime output.
            if child_block.name.as_str() == "default never" {
                let check_expression = if child_block.parameters.is_empty() {
                    None
                } else {
                    let expr_str = child_block.parameters[0].expression.as_str();
                    Some(
                        self.binding_parser
                            .parse_binding(expr_str, child_block.parameters[0].span)
                            .ast,
                    )
                };
                if exhaustive_check.is_some() {
                    self.report_error(
                        "@default block with \"never\" parameter must be the last case in a switch",
                        child_block.span,
                    );
                } else {
                    exhaustive_check = Some(R3SwitchExhaustiveCheck {
                        expression: check_expression,
                        source_span: child_block.span,
                        start_source_span: child_block.start_span,
                        end_source_span: child_block.end_span,
                        name_span: child_block.name_span,
                    });
                }
                continue;
            }

            // Validate: only @case and @default are allowed inside @switch
            if child_block.block_type != BlockType::Case
                && child_block.block_type != BlockType::Default
            {
                self.report_error(
                    "@switch block can only contain @case and @default blocks",
                    child_block.span,
                );
                unknown_blocks.push(crate::ast::r3::R3UnknownBlock {
                    name: child_block.name,
                    source_span: child_block.span,
                    name_span: child_block.name_span,
                });
                continue;
            }

            let is_default = child_block.block_type == BlockType::Default;

            // Validate @default
            if is_default {
                if has_default {
                    self.report_error(
                        "@switch block can only have one @default block",
                        child_block.start_span,
                    );
                } else if !child_block.parameters.is_empty() {
                    self.report_error(
                        "@default block cannot have parameters",
                        child_block.start_span,
                    );
                }
                has_default = true;
            }

            // Validate @case: must have exactly one parameter
            // Reference: r3_control_flow.ts line 242, 250, 589
            // Angular pushes @case with 0 params into unknownBlocks, but @case with >1 params
            // still gets parsed using the first parameter (only a validation error is reported).
            let is_case = child_block.block_type == BlockType::Case;
            if is_case && child_block.parameters.is_empty() {
                self.report_error(
                    "@case block must have exactly one parameter",
                    child_block.start_span,
                );
                unknown_blocks.push(crate::ast::r3::R3UnknownBlock {
                    name: child_block.name,
                    source_span: child_block.span,
                    name_span: child_block.name_span,
                });
                continue;
            }

            // The `@default never;` exhaustive marker must be the last case in the
            // switch. Any recognized @case/@default that follows it is an error
            // (reference: r3_control_flow.ts reports this for every block once the
            // exhaustive check has been seen). A second `@default never;` is rejected
            // in its own branch above.
            if exhaustive_check.is_some() {
                self.report_error(
                    "@default block with \"never\" parameter must be the last case in a switch",
                    child_block.span,
                );
            }

            if is_case && child_block.parameters.len() > 1 {
                self.report_error(
                    "@case block must have exactly one parameter",
                    child_block.start_span,
                );
                // Continue parsing with the first parameter (matching Angular behavior)
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
            let mut cases = Vec::new_in(&self.allocator);
            for case in collected_cases.drain(..) {
                cases.push(case);
            }

            // Visit children of the final case (which has the body)
            let children = self.visit_children(&child_block.children);

            // Propagate i18n metadata for the case group
            // Reference: r3_control_flow.ts:293 - Angular passes node.i18n to SwitchBlockCaseGroup
            let i18n = self.create_block_placeholder(
                &child_block.name,
                &[],
                group_source_span,
                group_start_source_span,
                child_block.end_span,
            );

            let group = R3SwitchBlockCaseGroup {
                cases,
                children,
                source_span: group_source_span,
                start_source_span: group_start_source_span,
                end_source_span: child_block.end_span,
                name_span: child_block.name_span,
                i18n,
            };
            groups.push(group);

            // Reset for next group
            first_case_start = None;
        }

        let switch_block = R3SwitchBlock {
            expression,
            groups,
            unknown_blocks,
            exhaustive_check,
            source_span: block.span,
            start_source_span: block.start_span,
            end_source_span: block.end_span,
            name_span: block.name_span,
        };
        Some(R3Node::SwitchBlock(Box::new_in(switch_block, &self.allocator)))
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

        // Parse defer triggers from block parameters. `idle(<time>)` is v22+;
        // an unknown version is treated as the latest.
        let trigger_result = parse_defer_triggers(
            self.allocator,
            &block.parameters,
            &self.binding_parser,
            block.start_span,
            self.angular_version.is_none_or(|v| v.major >= 22),
        );

        // Report trigger parsing errors via diagnostics
        for error in trigger_result.errors {
            match error {
                DeferTriggerError::Trigger { span, msg } => self.report_error(&msg, span),
                DeferTriggerError::Expression(error) => self.errors.push(error),
            }
        }

        // Process connected blocks with validation
        // Reference: r3_deferred_blocks.ts parseConnectedBlocks (lines 106-164)
        let mut placeholder = None;
        let mut loading = None;
        let mut error = None;

        for connected in connected_blocks {
            let connected_children = self.visit_children(&connected.children);

            match connected.block_type {
                BlockType::Placeholder => {
                    if placeholder.is_some() {
                        // Reference: r3_deferred_blocks.ts:124-131
                        self.report_error(
                            "@defer block can only have one @placeholder block",
                            connected.start_span,
                        );
                    } else {
                        // Parse and validate placeholder parameters
                        // Reference: r3_deferred_blocks.ts parsePlaceholderBlock (lines 167-198)
                        let mut minimum_time = None;
                        let mut has_minimum = false;
                        for param in &connected.parameters {
                            let expr = param.expression.as_str();
                            if expr.trim_start().starts_with("minimum")
                                && expr.trim_start().get(7..8).is_none_or(|c| {
                                    c.chars().next().is_none_or(char::is_whitespace)
                                })
                            {
                                if has_minimum {
                                    self.report_error(
                                        "@placeholder block can only have one \"minimum\" parameter",
                                        connected.start_span,
                                    );
                                } else {
                                    has_minimum = true;
                                    let time_str =
                                        &expr[crate::util::get_trigger_parameters_start(expr)..];
                                    let parsed = crate::util::parse_deferred_time(time_str);
                                    if parsed.is_none() {
                                        self.report_error(
                                            "Could not parse time value of parameter \"minimum\"",
                                            param.span,
                                        );
                                    }
                                    minimum_time = parsed;
                                }
                            } else {
                                self.report_error(
                                    &format!(
                                        "Unrecognized parameter in @placeholder block: \"{expr}\""
                                    ),
                                    param.span,
                                );
                            }
                        }

                        // Create i18n placeholder if inside an i18n context
                        let i18n = self.create_block_placeholder(
                            "placeholder",
                            &[],
                            connected.span,
                            connected.start_span,
                            connected.end_span,
                        );

                        placeholder = Some(R3DeferredBlockPlaceholder {
                            children: connected_children,
                            minimum_time,
                            source_span: connected.span,
                            name_span: connected.name_span,
                            start_source_span: connected.start_span,
                            end_source_span: connected.end_span,
                            i18n,
                        });
                    }
                }
                BlockType::Loading => {
                    if loading.is_some() {
                        // Reference: r3_deferred_blocks.ts:137-143
                        self.report_error(
                            "@defer block can only have one @loading block",
                            connected.start_span,
                        );
                    } else {
                        // Parse and validate loading parameters
                        // Reference: r3_deferred_blocks.ts parseLoadingBlock (lines 201-248)
                        let mut after_time = None;
                        let mut has_after = false;
                        let mut minimum_time = None;
                        let mut has_minimum = false;
                        for param in &connected.parameters {
                            let expr = param.expression.as_str();
                            let trimmed = expr.trim_start();
                            if trimmed.starts_with("after")
                                && trimmed.get(5..6).is_none_or(|c| {
                                    c.chars().next().is_none_or(char::is_whitespace)
                                })
                            {
                                if has_after {
                                    self.report_error(
                                        "@loading block can only have one \"after\" parameter",
                                        connected.start_span,
                                    );
                                } else {
                                    has_after = true;
                                    let time_str =
                                        &expr[crate::util::get_trigger_parameters_start(expr)..];
                                    let parsed = crate::util::parse_deferred_time(time_str);
                                    if parsed.is_none() {
                                        self.report_error(
                                            "Could not parse time value of parameter \"after\"",
                                            param.span,
                                        );
                                    }
                                    after_time = parsed;
                                }
                            } else if trimmed.starts_with("minimum")
                                && trimmed.get(7..8).is_none_or(|c| {
                                    c.chars().next().is_none_or(char::is_whitespace)
                                })
                            {
                                if has_minimum {
                                    self.report_error(
                                        "@loading block can only have one \"minimum\" parameter",
                                        connected.start_span,
                                    );
                                } else {
                                    has_minimum = true;
                                    let time_str =
                                        &expr[crate::util::get_trigger_parameters_start(expr)..];
                                    let parsed = crate::util::parse_deferred_time(time_str);
                                    if parsed.is_none() {
                                        self.report_error(
                                            "Could not parse time value of parameter \"minimum\"",
                                            param.span,
                                        );
                                    }
                                    minimum_time = parsed;
                                }
                            } else {
                                self.report_error(
                                    &format!(
                                        "Unrecognized parameter in @loading block: \"{expr}\""
                                    ),
                                    param.span,
                                );
                            }
                        }

                        // Create i18n placeholder if inside an i18n context
                        let i18n = self.create_block_placeholder(
                            "loading",
                            &[],
                            connected.span,
                            connected.start_span,
                            connected.end_span,
                        );

                        loading = Some(R3DeferredBlockLoading {
                            children: connected_children,
                            after_time,
                            minimum_time,
                            source_span: connected.span,
                            name_span: connected.name_span,
                            start_source_span: connected.start_span,
                            end_source_span: connected.end_span,
                            i18n,
                        });
                    }
                }
                BlockType::Error => {
                    if error.is_some() {
                        // Reference: r3_deferred_blocks.ts:149-152
                        self.report_error(
                            "@defer block can only have one @error block",
                            connected.start_span,
                        );
                    } else {
                        // Reference: r3_deferred_blocks.ts:252-253
                        if !connected.parameters.is_empty() {
                            self.report_error(
                                "@error block cannot have parameters",
                                connected.start_span,
                            );
                        }

                        // Create i18n placeholder if inside an i18n context
                        let i18n = self.create_block_placeholder(
                            "error",
                            &[],
                            connected.span,
                            connected.start_span,
                            connected.end_span,
                        );

                        error = Some(R3DeferredBlockError {
                            children: connected_children,
                            source_span: connected.span,
                            name_span: connected.name_span,
                            start_source_span: connected.start_span,
                            end_source_span: connected.end_span,
                            i18n,
                        });
                    }
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

        // Create i18n placeholder for @defer block if inside i18n context
        let i18n = self.create_block_placeholder(
            "defer",
            &[],
            source_span,
            block.start_span,
            end_source_span,
        );

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
            i18n,
        };
        Some(R3Node::DeferredBlock(Box::new_in(defer_block, &self.allocator)))
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
        i18n_element_name: Option<&str>,
        is_template: bool,
    ) -> (
        Vec<'a, R3TextAttribute<'a>>,  // Static attributes
        Vec<'a, R3BoundAttribute<'a>>, // Inputs
        Vec<'a, R3BoundEvent<'a>>,     // Outputs
        Vec<'a, R3Reference<'a>>,      // References
        Vec<'a, R3Variable<'a>>,       // Variables
        Option<TemplateAttrInfo<'a>>,  // Template attribute info
    ) {
        let mut attributes = Vec::new_in(&self.allocator);
        let mut inputs = Vec::new_in(&self.allocator);
        let mut outputs = Vec::new_in(&self.allocator);
        let mut references = Vec::new_in(&self.allocator);
        let mut variables = Vec::new_in(&self.allocator);
        let mut template_attr_info: Option<TemplateAttrInfo<'a>> = None;

        // First pass: collect i18n-* attribute metadata
        // Maps attribute name (without i18n- prefix) to its i18n metadata
        let mut i18n_attrs_meta: std::collections::HashMap<&str, I18nMeta<'a>> =
            std::collections::HashMap::new();
        for attr in attrs {
            let name = attr.name.as_str();
            if let Some(target_attr) = name.strip_prefix("i18n-") {
                // `isTrustedTypesSink` lowercases the parser's full name and does
                // not strip a namespace prefix. `None` is a selectorless component
                // with no host tag (`tagName === null`), which is not a sink.
                if i18n_element_name.is_some_and(|element_name| {
                    is_trusted_types_sink_at(element_name, target_attr, self.angular_version)
                }) {
                    self.report_error(
                        &format!(
                            "Translating attribute '{target_attr}' is disallowed for security reasons."
                        ),
                        attr.span,
                    );
                    continue;
                }
                // Angular's I18nMetaVisitor only gives a message to a plain attribute with that
                // name and a value, built from the value: `_generateI18nMessage([attr], meta)`.
                let Some(target) =
                    attrs.iter().find(|a| a.name.as_str() == target_attr && !a.value.is_empty())
                else {
                    continue;
                };
                let instance_id = self.allocate_i18n_message_instance_id();
                let (message_string, nodes) = self.create_attribute_i18n_message(target);
                let mut meta = parse_i18n_meta_with_message(
                    self.allocator,
                    attr.value.as_str(),
                    instance_id,
                    &message_string,
                );
                if let I18nMeta::Message(message) = &mut meta {
                    message.nodes = nodes;
                }
                i18n_attrs_meta.insert(target_attr, meta);
            }
        }

        for attr in attrs {
            let raw_name = attr.name.as_str();
            // Angular v22 removed the `data-` prefix normalization: binding syntax
            // (`bind-`, `on-`, `bindon-`, `ref-`, `let-`, `*`, `@`) is matched against
            // the raw attribute name, so e.g. `data-ref-a` is a plain text attribute.
            let name = raw_name;

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
                    name: attr.name,
                    value: attr.value,
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
                        name: attr.name,
                        value: attr.value,
                        span: attr.span,
                        name_span: attr.name_span,
                        value_span: attr.value_span,
                    });
                }
                continue;
            }

            // Check for binding prefixes (normalization already applied to name)
            if let Some((prefix, rest)) = self.parse_binding_prefix(name) {
                // The key span covers the identifier after the keyword prefix.
                // Reference: r3_template_transform.ts lines 765-811 (`createKeySpan(srcSpan,
                // bindParts[KW_*_IDX], identifier)`)
                let key_span = create_key_span(attr, &name[..name.len() - rest.len()], rest);
                match prefix {
                    BindingPrefix::Bind => {
                        // Check for animation bindings: bind-@prop or bind-animate-prop
                        let key_span = property_binding_key_span(rest, key_span);
                        if let Some(anim_name) = rest.strip_prefix('@') {
                            inputs.push(self.create_bound_attribute(
                                element_name,
                                anim_name,
                                attr,
                                BindingType::Animation,
                                None,
                                key_span,
                            ));
                        } else if let Some(anim_name) = rest.strip_prefix("animate-") {
                            inputs.push(self.create_bound_attribute(
                                element_name,
                                anim_name,
                                attr,
                                BindingType::Animation,
                                None,
                                key_span,
                            ));
                        } else {
                            let i18n = i18n_attrs_meta.remove(rest);
                            let mut bound_attr = self.create_bound_attribute(
                                element_name,
                                rest,
                                attr,
                                BindingType::Property,
                                None,
                                key_span,
                            );
                            bound_attr.i18n = i18n;
                            inputs.push(bound_attr);
                        }
                    }
                    BindingPrefix::Let => {
                        if is_template {
                            // Validate and create variable
                            if let Some(var) = self.create_validated_variable(rest, attr, key_span)
                            {
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
                            self.create_validated_reference(rest, attr, key_span, &references)
                        {
                            references.push(reference);
                        }
                    }
                    BindingPrefix::On => {
                        outputs.push(self.create_bound_event(
                            rest,
                            attr,
                            event_key_span(rest, key_span),
                        ));
                    }
                    BindingPrefix::BindOn => {
                        // Two-way binding creates both input and output
                        inputs.push(self.create_bound_attribute(
                            element_name,
                            rest,
                            attr,
                            BindingType::TwoWay,
                            None,
                            property_binding_key_span(rest, key_span),
                        ));
                        // Create the output event (propChange)
                        let event_name = format!("{rest}Change");
                        let event_key = event_key_span(&event_name, key_span);
                        outputs.push(self.create_two_way_event(&event_name, attr, event_key));
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
                        // `parseLiteralAttr` takes the key span of the whole name and drops the
                        // `@`. Reference: r3_template_transform.ts line 836, binding_parser.ts
                        // lines 308-314
                        let key_span = create_key_span(attr, "@", rest);
                        inputs.push(self.create_legacy_animation_attribute(
                            element_name,
                            rest,
                            attr,
                            key_span,
                        ));
                    }
                }
                continue;
            }

            // Check for banana-in-a-box [(prop)]
            if name.starts_with(BANANA_BOX_START) && name.ends_with(BANANA_BOX_END) {
                let prop_name = &name[2..name.len() - 2];
                // Reference: r3_template_transform.ts line 871 (`createKeySpan(srcSpan,
                // delims.start, identifier)`)
                let key_span = create_key_span(attr, BANANA_BOX_START, prop_name);
                // Two-way binding creates both input and output
                inputs.push(self.create_bound_attribute(
                    element_name,
                    prop_name,
                    attr,
                    BindingType::TwoWay,
                    None,
                    property_binding_key_span(prop_name, key_span),
                ));
                // Create the output event (propChange)
                let event_name = format!("{prop_name}Change");
                let event_key = event_key_span(&event_name, key_span);
                outputs.push(self.create_two_way_event(&event_name, attr, event_key));
                continue;
            }

            // Check for property binding [prop]
            if name.starts_with(PROPERTY_START) && name.ends_with(PROPERTY_END) {
                let prop_name = &name[1..name.len() - 1];
                // Reference: r3_template_transform.ts line 871 (`createKeySpan(srcSpan,
                // delims.start, identifier)`)
                let key_span =
                    property_binding_key_span(prop_name, create_key_span(attr, "[", prop_name));
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
                let mut bound_attr = self.create_bound_attribute(
                    element_name,
                    final_name,
                    attr,
                    binding_type,
                    unit,
                    key_span,
                );
                bound_attr.i18n = i18n;
                inputs.push(bound_attr);
                continue;
            }

            // Check for event binding (event)
            if name.starts_with(EVENT_START) && name.ends_with(EVENT_END) {
                let event_name = &name[1..name.len() - 1];
                // Reference: r3_template_transform.ts line 871 (`createKeySpan(srcSpan,
                // delims.start, identifier)`)
                let key_span = event_key_span(event_name, create_key_span(attr, "(", event_name));
                // Check for legacy animation syntax (@) or (@name)
                if event_name.starts_with('@') {
                    let anim_name = event_name.strip_prefix('@').unwrap_or("");
                    outputs.push(self.create_legacy_animation_event(anim_name, attr, key_span));
                } else {
                    outputs.push(self.create_bound_event_with_target(event_name, attr, key_span));
                }
                continue;
            }

            // Check for reference #ref
            if name.starts_with('#') {
                if let Some(reference) = self.create_validated_reference(
                    &name[1..],
                    attr,
                    create_key_span(attr, "#", &name[1..]),
                    &references,
                ) {
                    references.push(reference);
                }
                continue;
            }

            // NOTE: animate.* without brackets is a TextAttribute, NOT a BoundAttribute.
            // Only [animate.xxx] or (animate.xxx) are special bindings.

            // An attribute value with interpolation becomes a BoundAttribute (Angular's
            // `parsePropertyInterpolation`); without one it stays a text attribute.
            // Note: name is already normalized at the start of the loop
            if let Some(expr) = self.parse_attribute_interpolation(attr) {
                // Look up i18n metadata for this attribute (e.g., i18n-title for title="{{ name }}")
                let i18n = i18n_attrs_meta.remove(name);
                inputs.push(self.create_interpolated_bound_attribute(
                    element_name,
                    name,
                    attr,
                    expr,
                    i18n,
                ));
                continue;
            }

            // Static attribute - look up i18n metadata
            let i18n = i18n_attrs_meta.remove(name);
            attributes.push(self.create_text_attribute_with_i18n(attr, i18n));
        }

        (attributes, inputs, outputs, references, variables, template_attr_info)
    }

    /// Builds the i18n message for a translated attribute from the attribute's value.
    ///
    /// Returns the serialized message and its top-level nodes. An attribute message only
    /// contains text and interpolation placeholders; the placeholder nodes give ingest the
    /// names Angular reads from `Object.keys(message.placeholders)`.
    fn create_attribute_i18n_message(
        &self,
        attr: &HtmlAttribute<'a>,
    ) -> (String, Vec<'a, I18nNode<'a>>) {
        let message = I18nMessageFactory::new(false, true)
            .create_attribute_message(attr, self.i18n_source_file());
        let span = |s: &crate::util::ParseSourceSpan| Span::new(s.start.offset, s.end.offset);
        let mut nodes = Vec::new_in(&self.allocator);
        let mut push = |node: &crate::i18n::ast::Node| match node {
            crate::i18n::ast::Node::Text(text) => nodes.push(I18nNode::Text(I18nText {
                value: Ident::from_in(text.value.as_str(), self.allocator),
                source_span: span(&text.source_span),
            })),
            crate::i18n::ast::Node::Placeholder(ph) => {
                nodes.push(I18nNode::Placeholder(I18nPlaceholder {
                    value: Ident::from_in(ph.value.as_str(), self.allocator),
                    name: Ident::from_in(ph.name.as_str(), self.allocator),
                    source_span: span(&ph.source_span),
                }));
            }
            _ => {}
        };
        for node in &message.nodes {
            match node {
                crate::i18n::ast::Node::Container(container) => {
                    container.children.iter().for_each(&mut push);
                }
                node => push(node),
            }
        }
        (message.serialize(), nodes)
    }

    /// Normalizes an attribute name by stripping the data- prefix (case-insensitive).
    /// This matches TypeScript's behavior: /^data-/i.test(attrName) ? attrName.substring(5) : attrName
    /// Parses a binding prefix (`bind-`, `let-`, `ref-`, `on-`, `bindon-`) from an
    /// attribute name. Angular v22 matches these against the raw name; a `data-`
    /// prefix is no longer stripped, so `data-on-x` is not an event binding.
    fn parse_binding_prefix<'b>(&self, name: &'b str) -> Option<(BindingPrefix, &'b str)> {
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
            name: attr.name,
            value: attr.value,
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
        key_span: Span,
    ) -> R3BoundAttribute<'a> {
        let value_span = attr.value_span.unwrap_or(attr.span);
        let value = self.parse_binding_expression(&attr.value, value_span);
        let name_atom = Ident::from(self.allocator.alloc_str(property_name));

        // Look up security context based on element and property
        let security_context = self.security_context(element_name, property_name);

        R3BoundAttribute {
            name: name_atom,
            binding_type,
            security_context,
            value,
            unit: unit.map(|u| Ident::from(self.allocator.alloc_str(u))),
            source_span: attr.span,
            key_span,
            value_span: attr.value_span,
            i18n: None,
        }
    }

    /// Parses a binding expression from an attribute value.
    fn parse_binding_expression(&mut self, value: &Ident<'a>, span: Span) -> AngularExpression<'a> {
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
        key_span: Span,
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

        let name_atom = Ident::from(self.allocator.alloc_str(property_name));

        R3BoundAttribute {
            name: name_atom,
            binding_type: BindingType::LegacyAnimation,
            security_context: crate::ast::r3::SecurityContext::None,
            value: result.ast,
            unit: None,
            source_span: attr.span,
            key_span,
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
        key_span: Span,
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
            (event, Some(Ident::from(self.allocator.alloc_str(target))))
        } else {
            (name, None)
        };

        let name_atom = Ident::from(self.allocator.alloc_str(event_name));

        R3BoundEvent {
            name: name_atom,
            event_type,
            handler,
            target,
            phase: None,
            source_span: attr.span,
            handler_span,
            key_span,
        }
    }

    /// Creates a bound event (without target parsing).
    fn create_bound_event(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
        key_span: Span,
    ) -> R3BoundEvent<'a> {
        let handler_span = self.calculate_event_handler_span(attr);
        let handler = self.parse_event_expression(&attr.value, handler_span);
        let name_atom = Ident::from(self.allocator.alloc_str(name));
        // `on-animate.enter` is an animation event, like `(animate.enter)`
        let event_type = if name.starts_with("animate.") {
            ParsedEventType::Animation
        } else {
            ParsedEventType::Regular
        };

        R3BoundEvent {
            name: name_atom,
            event_type,
            handler,
            target: None,
            phase: None,
            source_span: attr.span,
            handler_span,
            key_span,
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
        key_span: Span,
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
            (event_name, Some(Ident::from(self.allocator.alloc_str(&phase_lower))))
        } else {
            (name, None)
        };

        // Use only the event name (trigger name), not including the phase
        let name_atom = Ident::from(self.allocator.alloc_str(event_name));

        R3BoundEvent {
            name: name_atom,
            event_type: ParsedEventType::LegacyAnimation,
            handler,
            target: None,
            phase,
            source_span: attr.span,
            handler_span,
            key_span,
        }
    }

    /// Creates a bound event for two-way binding.
    /// The handler is just the target expression (e.g., `name`), not the full assignment.
    /// The `$event` is added separately in the ingest phase.
    fn create_two_way_event(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
        key_span: Span,
    ) -> R3BoundEvent<'a> {
        let handler_span = self.calculate_event_handler_span(attr);
        let name_atom = Ident::from(self.allocator.alloc_str(name));

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
            key_span,
        }
    }

    /// Parses an event handler expression.
    fn parse_event_expression(&mut self, value: &Ident<'a>, span: Span) -> AngularExpression<'a> {
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
        key_span: Span,
    ) -> Option<R3Variable<'a>> {
        // Validate variable name
        if name.contains('-') {
            self.report_error("\"-\" is not allowed in variable names", attr.span);
        } else if name.is_empty() {
            self.report_error("Variable does not have a name", attr.span);
        }

        // Still create the variable even if invalid (for error recovery)
        let name_atom = Ident::from(self.allocator.alloc_str(name));
        Some(R3Variable {
            name: name_atom,
            value: attr.value,
            source_span: attr.span,
            key_span,
            value_span: attr.value_span,
        })
    }

    /// Creates a reference with validation and duplicate detection.
    /// Returns None if validation fails (error is reported).
    fn create_validated_reference(
        &mut self,
        name: &str,
        attr: &HtmlAttribute<'a>,
        key_span: Span,
        existing_refs: &Vec<'a, R3Reference<'a>>,
    ) -> Option<R3Reference<'a>> {
        // Validate reference name
        if name.contains('-') {
            self.report_error("\"-\" is not allowed in reference names", attr.span);
        } else if name.is_empty() {
            self.report_error("Reference does not have a name", attr.span);
        } else if existing_refs.iter().any(|r| r.name.as_str() == name) {
            self.report_error(
                &format!("Reference \"#{name}\" is defined more than once"),
                attr.span,
            );
        }

        // Still create the reference even if invalid (for error recovery)
        let name_atom = Ident::from(self.allocator.alloc_str(name));
        Some(R3Reference {
            name: name_atom,
            value: attr.value,
            source_span: attr.span,
            key_span,
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

        let mut children = Vec::new_in(&self.allocator);
        children.push(node);

        let mut attributes = Vec::new_in(&self.allocator);
        let mut inputs = Vec::new_in(&self.allocator);
        let mut variables = Vec::new_in(&self.allocator);

        // Extract the directive name (strip the * prefix)
        let directive_name: &str =
            template_attr.name.strip_prefix('*').unwrap_or(&template_attr.name);
        // Allocate the directive name in the arena for long-lived reference
        let directive_name = self.allocator.alloc_str(directive_name);
        let directive_name_atom = Ident::from(directive_name);

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
                    name: directive_name_atom,
                    value: Ident::from(""),
                    source_span: directive_source_span,
                    key_span: Some(directive_source_span),
                    value_span: None,
                    i18n: None,
                });
            } else if let Some(TemplateBinding::Expression(expr)) = result.bindings.first()
                && expr.key.source.as_str() == directive_name
            {
                // Add as BoundAttribute with the expression value
                let source_span = Span::new(expr.source_span.start, expr.source_span.end);
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
                    name: directive_name_atom,
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

            // Process all bindings
            let mut first_expr_handled = false;
            for binding in &result.bindings {
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
                        let binding_name_atom = Ident::from(self.allocator.alloc_str(full_name));

                        let source_span = Span::new(expr.source_span.start, expr.source_span.end);
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
                            key_span: Span::new(expr.key.span.start, expr.key.span.end),
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
                value: Ident::from(""),
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
        let mut template_attrs: Vec<'a, R3TemplateAttr<'a>> = Vec::new_in(&self.allocator);
        for attr in attributes {
            template_attrs.push(R3TemplateAttr::Text(attr));
        }
        for input in inputs {
            template_attrs.push(R3TemplateAttr::Bound(input));
        }

        let template = R3Template {
            tag_name,
            attributes: hoisted_attributes,
            inputs: hoisted_inputs,
            outputs: hoisted_outputs,
            directives: Vec::new_in(&self.allocator),
            template_attrs,
            children,
            references: Vec::new_in(&self.allocator),
            variables,
            is_self_closing: false,
            source_span: element.span,
            start_source_span: element.start_span,
            end_source_span: element.end_span,
            i18n,
        };
        R3Node::Template(Box::new_in(template, &self.allocator))
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

        let mut children = Vec::new_in(&self.allocator);
        children.push(node);

        let mut attributes = Vec::new_in(&self.allocator);
        let mut inputs = Vec::new_in(&self.allocator);
        let mut variables = Vec::new_in(&self.allocator);

        // Extract the directive name (strip the * prefix)
        let directive_name: &str =
            template_attr.name.strip_prefix('*').unwrap_or(&template_attr.name);
        let directive_name = self.allocator.alloc_str(directive_name);
        let directive_name_atom = Ident::from(directive_name);

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
                    name: directive_name_atom,
                    value: Ident::from(""),
                    source_span: directive_source_span,
                    key_span: Some(directive_source_span),
                    value_span: None,
                    i18n: None,
                });
            } else if let Some(TemplateBinding::Expression(expr)) = result.bindings.first()
                && expr.key.source.as_str() == directive_name
            {
                let source_span = Span::new(expr.source_span.start, expr.source_span.end);
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
                    name: directive_name_atom,
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

            let mut first_expr_handled = false;
            for binding in &result.bindings {
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
                        let binding_name_atom = Ident::from(self.allocator.alloc_str(full_name));

                        let source_span = Span::new(expr.source_span.start, expr.source_span.end);
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
                            key_span: Span::new(expr.key.span.start, expr.key.span.end),
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
                value: Ident::from(""),
                source_span: directive_source_span,
                key_span: Some(directive_source_span),
                value_span: None,
                i18n: None,
            });
        }

        let tag_name = self.get_wrapped_tag_name(&children[0]);
        let (hoisted_attributes, hoisted_inputs, hoisted_outputs) =
            self.get_hoisted_attrs_from_node(&children[0]);

        let mut template_attrs: Vec<'a, R3TemplateAttr<'a>> = Vec::new_in(&self.allocator);
        for attr in attributes {
            template_attrs.push(R3TemplateAttr::Text(attr));
        }
        for input in inputs {
            template_attrs.push(R3TemplateAttr::Bound(input));
        }

        let template = R3Template {
            tag_name,
            attributes: hoisted_attributes,
            inputs: hoisted_inputs,
            outputs: hoisted_outputs,
            directives: Vec::new_in(&self.allocator),
            template_attrs,
            children,
            references: Vec::new_in(&self.allocator),
            variables,
            is_self_closing: false,
            source_span: component.span,
            start_source_span: component.start_span,
            end_source_span: component.end_span,
            i18n,
        };
        R3Node::Template(Box::new_in(template, &self.allocator))
    }

    /// Filters out animation attributes (those starting with "animate.").
    /// These should remain on the inner element, not be hoisted to the template.
    fn filter_animation_attributes(
        &self,
        attributes: &Vec<'a, R3TextAttribute<'a>>,
    ) -> Vec<'a, R3TextAttribute<'a>> {
        let mut result = Vec::new_in(&self.allocator);
        for attr in attributes {
            if !attr.name.as_str().starts_with("animate.") {
                result.push(R3TextAttribute {
                    name: attr.name,
                    value: attr.value,
                    source_span: attr.source_span,
                    key_span: attr.key_span,
                    value_span: attr.value_span,
                    i18n: attr.i18n.as_ref().map(|meta| meta.clone_in(self.allocator)),
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
        let mut result = Vec::new_in(&self.allocator);
        for input in inputs {
            if input.binding_type != BindingType::Animation {
                result.push(R3BoundAttribute {
                    name: input.name,
                    binding_type: input.binding_type,
                    security_context: input.security_context,
                    value: self.clone_angular_expression(&input.value),
                    unit: input.unit,
                    source_span: input.source_span,
                    key_span: input.key_span,
                    value_span: input.value_span,
                    i18n: input.i18n.as_ref().map(|meta| meta.clone_in(self.allocator)),
                });
            }
        }
        result
    }

    /// Gets the tag name from a wrapped R3Node for template wrapping.
    /// Returns the element name for R3Element, "ng-content" for Content, None for R3Template.
    /// Reference: r3_template_transform.ts lines 1018-1026
    fn get_wrapped_tag_name(&self, node: &R3Node<'a>) -> Option<Ident<'a>> {
        match node {
            R3Node::Element(elem) => Some(elem.name),
            R3Node::Template(_) => None,
            // Content has a readonly name = 'ng-content' in TypeScript
            R3Node::Content(_) => Some(Ident::from("ng-content")),
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

        // Use the names from the enclosing i18n message, as Angular does.
        let (start_name, close_name) =
            self.message_names.tags.remove(&element.span.start).unwrap_or_else(|| {
                let start_name = self
                    .i18n_placeholder_registry
                    .get_start_tag_placeholder_name(tag_name, &attrs, is_void);
                let close_name = if is_void {
                    String::new()
                } else {
                    self.i18n_placeholder_registry.get_close_tag_placeholder_name(tag_name)
                };
                (start_name, close_name)
            });

        // Create TagPlaceholder
        let mut placeholder_attrs = HashMap::new_in(&self.allocator);
        for (k, v) in attrs {
            placeholder_attrs.insert(
                Ident::from(self.allocator.alloc_str(&k)),
                Ident::from(self.allocator.alloc_str(&v)),
            );
        }

        I18nMeta::Node(I18nNode::TagPlaceholder(I18nTagPlaceholder {
            tag: Ident::from(self.allocator.alloc_str(tag_name)),
            attrs: placeholder_attrs,
            start_name: Ident::from(self.allocator.alloc_str(&start_name)),
            close_name: Ident::from(self.allocator.alloc_str(&close_name)),
            children: Vec::new_in(&self.allocator),
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
        let mut placeholder_attrs = HashMap::new_in(&self.allocator);
        for (k, v) in attrs {
            placeholder_attrs.insert(
                Ident::from(self.allocator.alloc_str(&k)),
                Ident::from(self.allocator.alloc_str(&v)),
            );
        }

        I18nMeta::Node(I18nNode::TagPlaceholder(I18nTagPlaceholder {
            tag: Ident::from(self.allocator.alloc_str(tag_name)),
            attrs: placeholder_attrs,
            start_name: Ident::from(self.allocator.alloc_str(&start_name)),
            close_name: Ident::from(self.allocator.alloc_str(&close_name)),
            children: Vec::new_in(&self.allocator),
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
                Vec::new_in(&self.allocator),
                Vec::new_in(&self.allocator),
                Vec::new_in(&self.allocator),
            ),
        }
    }

    /// Creates a shallow copy of bound events.
    fn copy_bound_events(&self, events: &Vec<'a, R3BoundEvent<'a>>) -> Vec<'a, R3BoundEvent<'a>> {
        let mut result = Vec::new_in(&self.allocator);
        for event in events {
            result.push(R3BoundEvent {
                name: event.name,
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
        let name = var.key.source;
        // When value is None, Angular uses "$implicit" as the default binding
        let value = var.value.as_ref().map_or(Ident::from("$implicit"), |v| v.source);
        let value_span = var.value.as_ref().map(|v| Span::new(v.span.start, v.span.end));

        R3Variable {
            name,
            value,
            source_span: Span::new(var.source_span.start, var.source_span.end),
            key_span: Span::new(var.key.span.start, var.key.span.end),
            value_span,
        }
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
        let mut children = Vec::new_in(&self.allocator);
        let mut current_pos = 0;

        // Find all interpolations {{ expr }}
        while let Some(start) = text[current_pos..].find("{{") {
            let abs_start = current_pos + start;

            // Add text before interpolation
            if start > 0 {
                let text_before = &text[current_pos..abs_start];
                if !text_before.is_empty() {
                    let text_atom = Ident::from_in(text_before, &self.allocator);
                    children.push(I18nNode::Text(I18nText { value: text_atom, source_span: span }));
                }
            }

            // Find the closing }}
            if let Some(end) = text[abs_start + 2..].find("}}") {
                let abs_end = abs_start + 2 + end;
                let expr = text[abs_start + 2..abs_end].trim();

                if !expr.is_empty() {
                    // Generate placeholder name using the i18n placeholder registry, honouring a
                    // custom name from `// i18n(ph="NAME")` as Angular's i18n parser does.
                    let base_name = crate::i18n::parser::extract_placeholder_name(expr);
                    let placeholder_name = self.i18n_placeholder_registry.get_placeholder_name(
                        base_name.as_deref().unwrap_or("INTERPOLATION"),
                        expr,
                    );
                    let name_atom = Ident::from_in(&placeholder_name, self.allocator);
                    let value_atom = Ident::from_in(expr, self.allocator);

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
                let text_atom = Ident::from_in(remaining, &self.allocator);
                children.push(I18nNode::Text(I18nText { value: text_atom, source_span: span }));
            }
        }

        // Text with an interpolation is always a Container, even `{{ x }}` alone, as in
        // Angular's _visitTextWithInterpolation; ingest reads placeholder names from it.
        I18nMeta::Node(I18nNode::Container(I18nContainer { children, source_span: span }))
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

        let input = text.value.as_str();

        let mut strings = Vec::new_in(&self.allocator);
        let mut expressions = Vec::new_in(&self.allocator);

        // Helper to accumulate text/entity content before committing to strings array.
        // The invariant we maintain: strings.len() == expressions.len() after processing text/entity,
        // and strings.len() == expressions.len() + 1 after processing interpolation.
        // This ensures the final result has strings.len() == expressions.len() + 1.
        let mut current_string = String::new();
        // Length of the decoded input before `current_string`, for error columns.
        let mut committed_len = 0;

        for token in &text.tokens {
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
                        let start_marker = &token.parts[0];
                        let expr_content = &token.parts[1];
                        let end_marker = &token.parts[2];
                        let column = committed_len + current_string.len();

                        // Before adding an expression, commit the current string buffer
                        // (even if empty, we need a string before each expression)
                        strings.push(Ident::from_in(current_string.as_str(), &self.allocator));
                        current_string.clear();

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
                        committed_len =
                            column + start_marker.len() + decoded_expr.len() + end_marker.len();
                        // Angular's `splitInterpolation` reports `{{ }}` and still parses it.
                        if decoded_expr.trim().is_empty() {
                            self.report_error(
                                &blank_interpolation_error(column, input),
                                token.span,
                            );
                        }
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
        strings.push(Ident::from_in(current_string.as_str(), &self.allocator));

        // Create the Interpolation expression
        let span = ParseSpan::new(0, text.span.end - text.span.start);
        let source_span = AbsoluteSourceSpan { start: text.span.start, end: text.span.end };
        let interpolation = Interpolation { span, source_span, strings, expressions };
        Some(AngularExpression::Interpolation(Box::new_in(interpolation, &self.allocator)))
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
        let input = attr.value.as_str();

        let mut strings = Vec::new_in(&self.allocator);
        let mut expressions = Vec::new_in(&self.allocator);

        // Helper to accumulate text/entity content before committing to strings array.
        // The invariant we maintain: strings.len() == expressions.len() after processing text/entity,
        // and strings.len() == expressions.len() + 1 after processing interpolation.
        // This ensures the final result has strings.len() == expressions.len() + 1.
        let mut current_string = String::new();
        // Length of the decoded input before `current_string`, for error columns.
        let mut committed_len = 0;

        for token in tokens {
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
                        let start_marker = &token.parts[0];
                        let expr_content = &token.parts[1];
                        let end_marker = &token.parts[2];
                        let column = committed_len + current_string.len();

                        // Before adding an expression, commit the current string buffer
                        // (even if empty, we need a string before each expression)
                        strings.push(Ident::from_in(current_string.as_str(), &self.allocator));
                        current_string.clear();

                        // Calculate the expression span from the token span
                        let expr_start = token.span.start + start_marker.len() as u32;
                        let expr_end = token.span.end - end_marker.len() as u32;
                        let expr_span = Span::new(expr_start, expr_end);

                        // For backward compatibility, decode HTML entities in interpolation
                        // expressions. This matches Angular's behavior where entities like
                        // &times; inside string literals in interpolations are decoded.
                        let decoded_expr = decode_entities_in_string(expr_content.as_str());
                        committed_len =
                            column + start_marker.len() + decoded_expr.len() + end_marker.len();
                        // Angular's `splitInterpolation` reports `{{ }}` and still parses it.
                        if decoded_expr.trim().is_empty() {
                            self.report_error(
                                &blank_interpolation_error(column, input),
                                token.span,
                            );
                        }
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
        strings.push(Ident::from_in(current_string.as_str(), &self.allocator));

        // Create the Interpolation expression
        let span = ParseSpan::new(0, value_span.size());
        let source_span = AbsoluteSourceSpan { start: value_span.start, end: value_span.end };
        let interpolation = Interpolation { span, source_span, strings, expressions };
        Some(AngularExpression::Interpolation(Box::new_in(interpolation, &self.allocator)))
    }

    /// Parses the interpolation in an attribute value, or returns `None` when the value has
    /// none.
    fn parse_attribute_interpolation(
        &mut self,
        attr: &HtmlAttribute<'a>,
    ) -> Option<AngularExpression<'a>> {
        let value_span = attr.value_span.unwrap_or(attr.span);

        // Check if there are HTML entities in the attribute value that could affect span calculations.
        // If so, use the tokens which have correct source spans.
        let has_entities = attr.value_tokens.as_ref().is_some_and(|tokens| {
            tokens.iter().any(|t| matches!(t.token_type, InterpolatedTokenType::EncodedEntity))
        });

        if has_entities {
            // Use tokens to parse interpolations with correct source spans
            self.parse_interpolation_from_attr_tokens(attr, value_span)
        } else {
            // Use attr.value which has HTML entities already decoded by the HTML parser.
            // This matches Angular's behavior where the ml_parser decodes entities in interpolation
            // expressions for backward compatibility.
            // See: Angular's parser.ts _consumeAttr which decodes entities in ATTR_VALUE_INTERPOLATION.
            self.parse_interpolation(attr.value.as_str(), value_span)
        }
    }

    /// Creates a bound attribute from an attribute with interpolation value.
    /// Handles special prefixes like `attr.`, `class.`, `style.` similar to Angular's
    /// `createBoundElementProperty` in binding_parser.ts.
    fn create_interpolated_bound_attribute(
        &self,
        element_name: &str,
        name: &str,
        attr: &HtmlAttribute<'a>,
        expr: AngularExpression<'a>,
        i18n: Option<I18nMeta<'a>>,
    ) -> R3BoundAttribute<'a> {
        // Determine binding type, final name, and security context based on prefix
        // This matches Angular's binding_parser.ts createBoundElementProperty
        let (binding_type, final_name, unit, security_context) =
            if let Some(stripped) = name.strip_prefix("attr.") {
                // Attribute bindings use the attribute security context
                let security_context = self.security_context(element_name, stripped);
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
                let security_context = self.security_context(element_name, name);
                (BindingType::Property, name, None, security_context)
            };

        let name_atom = Ident::from(self.allocator.alloc_str(final_name));

        R3BoundAttribute {
            name: name_atom,
            binding_type,
            security_context,
            value: expr,
            unit: unit.map(|u| Ident::from(self.allocator.alloc_str(u))),
            source_span: attr.span,
            // Reference: r3_template_transform.ts line 927 (`createKeySpan(srcSpan, '', name)`)
            key_span: create_key_span(attr, "", name),
            value_span: attr.value_span,
            i18n,
        }
    }

    /// Creates an empty expression placeholder.
    fn create_empty_expression(&self, span: Span) -> AngularExpression<'a> {
        AngularExpression::Empty(Box::new_in(
            crate::ast::expression::EmptyExpr {
                span: ParseSpan::new(0, span.size()),
                source_span: AbsoluteSourceSpan::new(span.start, span.end),
            },
            &self.allocator,
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
    fn get_text_content(&self, element: &HtmlElement<'a>) -> Option<Ident<'a>> {
        if element.children.len() == 1
            && let HtmlNode::Text(text) = &element.children[0]
        {
            return Some(text.value);
        }
        None
    }

    /// Gets the stylesheet href from a link element.
    /// Only returns resolvable URLs per Angular's `isStyleUrlResolvable`.
    /// Reference: style_url_resolver.ts lines 12-18
    ///
    /// `preparseElement` lowercases attribute names and keeps the last
    /// `rel`/`href` when they repeat; the rel value itself is matched as
    /// written (`relAttr == 'stylesheet'`).
    fn get_stylesheet_href(&self, element: &HtmlElement<'a>) -> Option<Ident<'a>> {
        let mut rel = None;
        let mut href = None;

        for attr in &element.attrs {
            if attr.name.as_str().eq_ignore_ascii_case("rel") {
                rel = Some(attr.value.as_str());
            } else if attr.name.as_str().eq_ignore_ascii_case("href") {
                href = Some(attr.value);
            }
        }

        if rel == Some("stylesheet") && href.is_some_and(|h| is_style_url_resolvable(h.as_str())) {
            href
        } else {
            None
        }
    }

    /// Checks whether the preparser would classify the element as
    /// `PreparsedElementType.STYLESHEET`: `rel == "stylesheet"` on the last
    /// case-insensitively-named `rel` attribute, with no resolvability check.
    /// Only used inside ngNonBindable, where `NonBindableVisitor` drops every
    /// stylesheet link regardless of its URL.
    fn is_stylesheet_link(&self, element: &HtmlElement<'a>) -> bool {
        element
            .attrs
            .iter()
            .rev()
            .find(|attr| attr.name.as_str().eq_ignore_ascii_case("rel"))
            .is_some_and(|attr| attr.value.as_str() == "stylesheet")
    }

    /// Gets the ng-content selector.
    /// `preparseElement` lowercases the attribute name, keeps the last
    /// `select`, and defaults a missing or empty one to `*`.
    fn get_ng_content_selector(&self, element: &HtmlElement<'a>) -> Ident<'a> {
        let mut selector = Ident::from("*");
        for attr in &element.attrs {
            if attr.name.as_str().eq_ignore_ascii_case("select") {
                selector = attr.value;
            }
        }
        if selector.is_empty() { Ident::from("*") } else { selector }
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

/// Records the placeholder names of each ICU and block in an i18n message, keyed by their start
/// offset. Angular takes these names from the message (`getPlaceholderName('ICU', ...)`,
/// `getStartBlockPlaceholderName(...)`), giving `ICU_1`, `START_BLOCK_IF_1`, ... as needed.
fn collect_placeholder_names(
    nodes: &[crate::i18n::ast::Node],
    names: &mut MessagePlaceholderNames,
) {
    use crate::i18n::ast::Node;
    for node in nodes {
        match node {
            Node::IcuPlaceholder(ph) => {
                names.icus.insert(ph.source_span.start.offset, ph.name.clone());
            }
            Node::Container(container) => {
                collect_placeholder_names(&container.children, names);
            }
            Node::TagPlaceholder(tag) => {
                names.tags.insert(
                    tag.source_span.start.offset,
                    (tag.start_name.clone(), tag.close_name.clone()),
                );
                collect_placeholder_names(&tag.children, names);
            }
            Node::BlockPlaceholder(block) => {
                names.blocks.insert(
                    block.source_span.start.offset,
                    (block.start_name.clone(), block.close_name.clone()),
                );
                collect_placeholder_names(&block.children, names);
            }
            Node::Text(_) | Node::Icu(_) | Node::Placeholder(_) => {}
        }
    }
}

/// The meaning, description and custom id of an `i18n` attribute value.
struct ParsedI18nMeta<'s> {
    meaning: &'s str,
    description: &'s str,
    custom_id: &'s str,
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
/// Ported from Angular's `parseI18nMeta` in `render3/view/i18n/meta.ts`: the value is trimmed
/// once and the parts are not, so `"  m | d  "` has meaning `"m "` and description `" d"`.
/// The parts are emitted in `$localize` and JSDoc and the meaning feeds the message id, so
/// they must match exactly. Like Angular, the `|` is searched in the whole value, and a `|`
/// after `@@` leaves the description empty.
fn parse_i18n_meta(meta: &str) -> ParsedI18nMeta<'_> {
    // `String.prototype.trim`: Unicode white space and line terminators, plus U+FEFF but not
    // U+0085.
    let meta = meta.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{FEFF}');
    if meta.is_empty() {
        return ParsedI18nMeta { meaning: "", description: "", custom_id: "" };
    }
    let desc_index = meta.find('|');
    let (meaning_and_desc, custom_id) = match meta.find("@@") {
        Some(id_index) => (&meta[..id_index], &meta[id_index + 2..]),
        None => (meta, ""),
    };
    // `String.prototype.slice` clamps out-of-range indices.
    let (meaning, description) = match desc_index {
        Some(desc_index) => (
            meaning_and_desc.get(..desc_index).unwrap_or(meaning_and_desc),
            meaning_and_desc.get(desc_index + 1..).unwrap_or(""),
        ),
        None => ("", meaning_and_desc),
    };
    ParsedI18nMeta { meaning, description, custom_id }
}

/// Builds the i18n message metadata of an `i18n` / `i18n-*` attribute value.
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
    let ParsedI18nMeta { meaning, description, custom_id } = parse_i18n_meta(value);

    I18nMeta::Message(I18nMessage {
        instance_id,
        nodes: Vec::new_in(&allocator),
        meaning: Ident::from_in(meaning, &allocator),
        description: Ident::from_in(description, &allocator),
        custom_id: Ident::from_in(custom_id, &allocator),
        id: Ident::from(""),
        legacy_ids: Vec::new_in(&allocator),
        message_string: Ident::from_in(message_string, allocator),
        associated_message_id: Ident::from(""),
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

#[cfg(test)]
mod security_tests {
    use super::*;
    use crate::parser::html::HtmlParser;
    use oxc_allocator::Allocator;

    fn compile(
        source: &str,
    ) -> (std::vec::Vec<String>, std::vec::Vec<(String, SecurityContext)>, std::vec::Vec<String>)
    {
        compile_at(source, None)
    }

    fn compile_at(
        source: &str,
        angular_version: Option<AngularVersion>,
    ) -> (std::vec::Vec<String>, std::vec::Vec<(String, SecurityContext)>, std::vec::Vec<String>)
    {
        compile_with(source, angular_version, false)
    }

    fn compile_selectorless(
        source: &str,
    ) -> (std::vec::Vec<String>, std::vec::Vec<(String, SecurityContext)>, std::vec::Vec<String>)
    {
        compile_with(source, None, true)
    }

    fn compile_selectorless_at(
        source: &str,
        angular_version: Option<AngularVersion>,
    ) -> (std::vec::Vec<String>, std::vec::Vec<(String, SecurityContext)>, std::vec::Vec<String>)
    {
        compile_with(source, angular_version, true)
    }

    fn compile_with(
        source: &str,
        angular_version: Option<AngularVersion>,
        selectorless: bool,
    ) -> (std::vec::Vec<String>, std::vec::Vec<(String, SecurityContext)>, std::vec::Vec<String>)
    {
        let allocator = Allocator::default();
        let parsed = if selectorless {
            HtmlParser::with_selectorless(&allocator, source, "test.html").parse()
        } else {
            HtmlParser::new(&allocator, source, "test.html").parse()
        };
        let result = html_ast_to_r3_ast(
            &allocator,
            source,
            &parsed.nodes,
            TransformOptions { angular_version, ..TransformOptions::default() },
        );
        let mut names = std::vec::Vec::new();
        let mut contexts = std::vec::Vec::new();
        fn walk<'a>(
            nodes: &[R3Node<'a>],
            names: &mut std::vec::Vec<String>,
            contexts: &mut std::vec::Vec<(String, SecurityContext)>,
        ) {
            for node in nodes {
                match node {
                    R3Node::Element(element) => {
                        names.push(element.name.as_str().to_string());
                        for input in &element.inputs {
                            contexts
                                .push((input.name.as_str().to_string(), input.security_context));
                        }
                        walk(&element.children, names, contexts);
                    }
                    R3Node::Component(component) => {
                        names.push(format!("component:{}", component.component_name.as_str()));
                        if let Some(tag) = component.tag_name {
                            names.push(format!("host:{}", tag.as_str()));
                        }
                        for input in &component.inputs {
                            contexts
                                .push((input.name.as_str().to_string(), input.security_context));
                        }
                        walk(&component.children, names, contexts);
                    }
                    R3Node::Template(template) => walk(&template.children, names, contexts),
                    R3Node::Content(content) => walk(&content.children, names, contexts),
                    _ => {}
                }
            }
        }
        walk(&result.nodes, &mut names, &mut contexts);
        let errors = result.errors.iter().map(|err| err.msg.clone()).collect::<std::vec::Vec<_>>();
        (names, contexts, errors)
    }

    #[test]
    fn svg_script_is_stripped_and_animate_to_is_validated() {
        let (names, contexts, errors) =
            compile(r#"<svg><script>alert(1)</script><animate [attr.to]="url"></animate></svg>"#);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(names.iter().any(|name| name == ":svg:svg" || name == "svg"));
        assert!(!names.iter().any(|name| name.contains("script")));
        assert!(names.iter().any(|name| name.contains("animate")));
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "to" && *ctx == SecurityContext::AttributeNoBinding),
            "{names:?} {contexts:?}"
        );
    }

    #[test]
    fn math_href_is_a_url() {
        let (names, contexts, _) = compile(r#"<math><mi [attr.href]="url"></mi></math>"#);
        assert!(names.iter().any(|name| name.contains("mi")), "{names:?}");
        assert!(
            contexts.iter().any(|(name, ctx)| name == "href" && *ctx == SecurityContext::Url),
            "{contexts:?}"
        );
    }

    #[test]
    fn iframe_i18n_src_is_rejected() {
        let (_, _, errors) = compile(r#"<iframe i18n-src src="https://example.com"></iframe>"#);
        assert!(
            errors.iter().any(|msg| msg.contains("disallowed for security reasons")),
            "{errors:?}"
        );
    }

    #[test]
    fn namespaced_iframe_i18n_src_stays_allowed() {
        let (_, _, errors) =
            compile(r#"<svg><iframe i18n-src src="https://example.com"></iframe></svg>"#);
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");
    }

    #[test]
    fn v21_keeps_svg_script_and_validates_namespaced_animate() {
        // `:svg:animate` parses to element `animate` on the pre-namespaced
        // selector path (upstream has no `splitNsName` there), so `to` hits
        // the bare `animate|to` attributeNoBinding key at 21.2.7.
        let version = Some(AngularVersion::new(21, 2, 7));
        let (names, contexts, _) = compile_at(
            r#"<svg><script>alert(1)</script><animate [attr.to]="url"></animate></svg>"#,
            version,
        );
        assert!(names.iter().any(|name| name.contains("script")), "{names:?}");
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "to" && *ctx == SecurityContext::AttributeNoBinding),
            "{contexts:?}"
        );
    }

    #[test]
    fn xml_script_is_kept_and_xml_iframe_src_stays_translatable() {
        let (names, _, _) = compile(r#"<xml:script>alert(1)</xml:script>"#);
        assert!(names.iter().any(|name| name.contains("script")), "{names:?}");
        let (_, _, errors) =
            compile(r#"<xml:iframe i18n-src src="https://example.com"></xml:iframe>"#);
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");
    }

    #[test]
    fn v21_2_3_allows_iframe_src_translation() {
        let (_, _, errors) = compile_at(
            r#"<iframe i18n-src src="https://example.com"></iframe>"#,
            Some(AngularVersion::new(21, 2, 3)),
        );
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");
    }

    #[test]
    fn selectorless_iframe_host_rejects_i18n_src() {
        let (_, _, errors) = compile_selectorless(
            r#"<MyComp:iframe i18n-src src="https://example.com"></MyComp:iframe>"#,
        );
        assert!(
            errors.iter().any(|msg| msg.contains("disallowed for security reasons")),
            "{errors:?}"
        );
    }

    #[test]
    fn selectorless_svg_iframe_host_is_not_the_iframe_sink() {
        let (_, _, errors) = compile_selectorless(
            r#"<MyComp:svg:iframe i18n-src src="https://example.com"></MyComp:svg:iframe>"#,
        );
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");
    }

    #[test]
    fn selectorless_without_host_tag_is_not_a_sink() {
        let (_, _, errors) =
            compile_selectorless(r#"<MyComp i18n-innerHTML innerHTML="<b>x</b>"></MyComp>"#);
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");
    }

    #[test]
    fn selectorless_script_class_is_kept_and_script_host_is_rejected() {
        let (names, _, errors) = compile_selectorless(r#"<Script>alert(1)</Script>"#);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(names.iter().any(|name| name == "component:Script"), "{names:?}");

        let (names, _, errors) = compile_selectorless(
            r#"<Script:iframe i18n-src src="https://example.com"></Script:iframe>"#,
        );
        assert!(
            errors.iter().any(|msg| msg.contains("disallowed for security reasons")),
            "{errors:?} {names:?}"
        );
        assert!(names.iter().any(|name| name == "component:Script"), "{names:?}");

        let (_, _, errors) = compile_selectorless(r#"<MyComp:script></MyComp:script>"#);
        assert!(
            errors
                .iter()
                .any(|msg| msg.contains("Tag name \"script\" cannot be used as a component tag")),
            "{errors:?}"
        );
    }

    #[test]
    fn element_script_class_is_still_stripped() {
        let (names, _, _) = compile(r#"<Script>alert(1)</Script>"#);
        assert!(
            !names.iter().any(|name| name.to_ascii_lowercase().contains("script")),
            "{names:?}"
        );
    }

    #[test]
    fn selectorless_inside_svg_inherits_the_namespace() {
        let (_, _, errors) = compile_selectorless(
            r#"<svg><MyComp i18n-innerHTML innerHTML="<b>x</b>"></MyComp></svg>"#,
        );
        assert!(
            errors.iter().any(|msg| msg.contains("disallowed for security reasons")),
            "{errors:?}"
        );

        let (_, _, errors) = compile_selectorless(
            r#"<svg><MyComp:iframe i18n-src src="https://example.com"></MyComp:iframe></svg>"#,
        );
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");

        let (names, _, errors) =
            compile_selectorless(r#"<svg><MyComp:script></MyComp:script></svg>"#);
        assert!(
            !errors.iter().any(|msg| msg.contains("cannot be used as a component tag")),
            "{errors:?}"
        );
        assert!(names.iter().any(|name| name == "component:MyComp"), "{names:?}");

        let (_, _, errors) = compile_selectorless(
            r#"<math><MyComp i18n-innerHTML innerHTML="<b>x</b>"></MyComp></math>"#,
        );
        assert!(
            errors.iter().any(|msg| msg.contains("disallowed for security reasons")),
            "{errors:?}"
        );
    }

    #[test]
    fn selectorless_inside_foreign_object_does_not_inherit_svg() {
        let (_, _, errors) = compile_selectorless(
            r#"<svg><foreignObject><MyComp i18n-innerHTML innerHTML="<b>x</b>"></MyComp></foreignObject></svg>"#,
        );
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");

        let (_, _, errors) = compile_selectorless(
            r#"<svg><foreignObject><MyComp:iframe i18n-src src="https://example.com"></MyComp:iframe></foreignObject></svg>"#,
        );
        assert!(
            errors.iter().any(|msg| msg.contains("disallowed for security reasons")),
            "{errors:?}"
        );
    }

    #[test]
    fn selectorless_iframe_src_i18n_follows_version() {
        let (_, _, errors) = compile_selectorless_at(
            r#"<MyComp:iframe i18n-src src="https://example.com"></MyComp:iframe>"#,
            Some(AngularVersion::new(21, 2, 3)),
        );
        assert!(!errors.iter().any(|msg| msg.contains("disallowed")), "{errors:?}");
    }

    #[test]
    fn selectorless_binding_security_uses_the_host_tag() {
        // `iframe|src` is a resource URL on the resolved host tag.
        let (_, contexts, _) =
            compile_selectorless(r#"<MyComp:iframe [src]="url"></MyComp:iframe>"#);
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "src" && *ctx == SecurityContext::ResourceUrl),
            "{contexts:?}"
        );

        // The `:svg:` host hits the namespaced animation schema.
        let (_, contexts, _) =
            compile_selectorless(r#"<MyComp:svg:animate [attr.to]="v"></MyComp:svg:animate>"#);
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "to" && *ctx == SecurityContext::AttributeNoBinding),
            "{contexts:?}"
        );

        // A namespaced host does not fall back to the bare iframe sink.
        let (_, contexts, _) =
            compile_selectorless(r#"<MyComp:svg:iframe [src]="url"></MyComp:svg:iframe>"#);
        assert!(
            contexts.iter().all(|(name, ctx)| name != "src" || *ctx == SecurityContext::None),
            "{contexts:?}"
        );

        // `tagName === null` resolves over every element: `*|innerHTML`.
        let (_, contexts, _) = compile_selectorless(r#"<MyComp [innerHTML]="html"></MyComp>"#);
        assert!(
            contexts.iter().any(|(name, ctx)| name == "innerHTML" && *ctx == SecurityContext::Html),
            "{contexts:?}"
        );
    }

    #[test]
    fn prefixed_element_does_not_inherit_the_parent_namespace_for_security() {
        // `normalizeTagName` drops a non-svg/math prefix, so `<xml:iframe>`
        // inside `<svg>` still requires the resource-URL sanitizer.
        let (_, contexts, _) = compile(r#"<svg><xml:iframe [src]="url"></xml:iframe></svg>"#);
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "src" && *ctx == SecurityContext::ResourceUrl),
            "{contexts:?}"
        );
    }

    #[test]
    fn children_inherit_an_arbitrary_prefix() {
        // `_getPrefix` inherits `getNsPrefix(parentName)` verbatim, so `<iframe>`
        // inside `<xml:div>` resolves to `:xml:iframe`. `normalizeTagName` drops
        // the `xml` prefix and the `iframe|src` resource-URL sanitizer applies.
        let (names, contexts, _) =
            compile(r#"<svg><xml:div><iframe [src]="url"></iframe></xml:div></svg>"#);
        assert!(names.iter().any(|name| name == ":xml:iframe"), "{names:?}");
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "src" && *ctx == SecurityContext::ResourceUrl),
            "{names:?} {contexts:?}"
        );

        // The same inheritance drives selectorless host tags (`tagName`).
        let (names, _, _) =
            compile_selectorless(r#"<xml:div><MyComp [innerHTML]="html"></MyComp></xml:div>"#);
        assert!(names.iter().any(|name| name == "host::xml:ng-component"), "{names:?}");
    }

    #[test]
    fn selectorless_children_inherit_the_host_tag_namespace() {
        // Children of `<MyComp:math>` are MathML: `mi` is only an href sink in
        // the `:math:` namespace.
        let (_, contexts, _) =
            compile_selectorless(r#"<MyComp:math><mi [href]="url"></mi></MyComp:math>"#);
        assert!(
            contexts.iter().any(|(name, ctx)| name == "href" && *ctx == SecurityContext::Url),
            "{contexts:?}"
        );

        // `foreignObject` prevents namespace inheritance, so children of
        // `<MyComp:foreignObject>` inside `<svg>` are HTML again.
        let (_, contexts, _) = compile_selectorless(
            r#"<svg><MyComp:foreignObject><iframe [src]="url"></iframe></MyComp:foreignObject></svg>"#,
        );
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "src" && *ctx == SecurityContext::ResourceUrl),
            "{contexts:?}"
        );

        // `<MyComp>` inside `<svg>` resolves to `:svg:ng-component`, so its
        // children stay namespaced.
        let (_, contexts, _) =
            compile_selectorless(r#"<svg><MyComp><iframe [src]="url"></iframe></MyComp></svg>"#);
        assert!(
            contexts.iter().all(|(name, ctx)| name != "src" || *ctx == SecurityContext::None),
            "{contexts:?}"
        );

        // At the HTML root `tagName === null`: children are plain HTML.
        let (_, contexts, _) =
            compile_selectorless(r#"<MyComp><iframe [src]="url"></iframe></MyComp>"#);
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "src" && *ctx == SecurityContext::ResourceUrl),
            "{contexts:?}"
        );
    }

    #[test]
    fn selectorless_bare_host_stays_bare_before_the_namespaced_schema() {
        // Pre-v22 `calcPossibleSecurityContexts` looks the bare tag up:
        // `animate|to` is `AttributeNoBinding` on 21.2.7 while `:svg:animate|to`
        // is not a key.
        let (_, contexts, _) = compile_selectorless_at(
            r#"<MyComp:animate [attr.to]="value"></MyComp:animate>"#,
            Some(AngularVersion::new(21, 2, 7)),
        );
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "to" && *ctx == SecurityContext::AttributeNoBinding),
            "{contexts:?}"
        );

        // Same for bare MathML hosts: `mi|href` is a bare URL key pre-v22.
        let (_, contexts, _) = compile_selectorless_at(
            r#"<MyComp:mi [attr.href]="value"></MyComp:mi>"#,
            Some(AngularVersion::new(21, 2, 7)),
        );
        assert!(
            contexts.iter().any(|(name, ctx)| name == "href" && *ctx == SecurityContext::Url),
            "{contexts:?}"
        );

        // v22 promotes the bare host to `:svg:animate` like upstream.
        let (_, contexts, _) =
            compile_selectorless(r#"<MyComp:animate [attr.to]="value"></MyComp:animate>"#);
        assert!(
            contexts
                .iter()
                .any(|(name, ctx)| name == "to" && *ctx == SecurityContext::AttributeNoBinding),
            "{contexts:?}"
        );
    }

    #[test]
    fn prefixed_element_lookup_matches_the_versions_normalize_tag_name() {
        // Pre-namespaced versions feed `:xml:iframe` to `CssSelector.parse`
        // verbatim, which yields element `iframe`, so `src` resolves to
        // `iframe|src` — the sink still applies. The namespaced schema splits
        // the `:xml:` prefix; only versions with `normalizeTagName` (19.2.23 /
        // 20.3.22 / 21.2.15+) strip it back to `iframe`, while 21.2.14 looks
        // up `:xml:iframe|src` verbatim and misses.
        for (major, minor, patch, expected) in [
            (21, 2, 13, SecurityContext::ResourceUrl),
            (21, 2, 14, SecurityContext::None),
            (21, 2, 15, SecurityContext::ResourceUrl),
            (19, 2, 22, SecurityContext::ResourceUrl),
            (19, 2, 23, SecurityContext::ResourceUrl),
            (20, 3, 21, SecurityContext::ResourceUrl),
            (20, 3, 22, SecurityContext::ResourceUrl),
        ] {
            let (_, contexts, _) = compile_at(
                r#"<xml:iframe [src]="url"></xml:iframe>"#,
                Some(AngularVersion::new(major, minor, patch)),
            );
            assert!(
                contexts.iter().any(|(name, ctx)| name == "src" && *ctx == expected),
                "v{major}.{minor}.{patch}: {contexts:?}"
            );
        }
    }
}

#[cfg(test)]
mod template_parser_tests {
    use super::*;
    use crate::parser::html::HtmlParser;
    use oxc_allocator::Allocator;

    fn meta(value: &str) -> (&str, &str, &str) {
        let ParsedI18nMeta { meaning, description, custom_id } = parse_i18n_meta(value);
        (meaning, description, custom_id)
    }

    #[test]
    fn i18n_meta_trims_the_value_once() {
        // Angular's `parseI18nMeta` trims the whole value and leaves the parts as they are.
        assert_eq!(meta("  m | d  "), ("m ", " d", ""));
        assert_eq!(meta("  only description  "), ("", "only description", ""));
        assert_eq!(meta(" m | d @@ custom "), ("m ", " d ", " custom"));
        assert_eq!(meta("@@id"), ("", "", "id"));
        assert_eq!(meta("   "), ("", "", ""));
        // `String.prototype.trim` strips U+FEFF but not U+0085.
        assert_eq!(meta("\u{FEFF}d\u{FEFF}"), ("", "d", ""));
        assert_eq!(meta("\u{85}d\u{85}"), ("", "\u{85}d\u{85}", ""));
    }

    #[test]
    fn i18n_meta_finds_the_pipe_in_the_whole_value() {
        // The `|` index comes from the whole value, so a `|` after `@@` slices past the end
        // of the meaning-and-description part: meaning is all of it, description is empty.
        assert_eq!(meta("d@@id|x"), ("d", "", "id|x"));
        assert_eq!(meta("m|d@@id|x"), ("m", "d", "id|x"));
    }

    /// Transforms `source` and returns a flat description of its nodes and the errors.
    fn transform(source: &str) -> (std::vec::Vec<String>, std::vec::Vec<String>) {
        let allocator = Allocator::default();
        let parsed = HtmlParser::new(&allocator, source, "test.html").parse();
        let result =
            html_ast_to_r3_ast(&allocator, source, &parsed.nodes, TransformOptions::default());
        fn describe(node: &R3Node<'_>, out: &mut std::vec::Vec<String>) {
            match node {
                R3Node::Text(text) => out.push(format!("text:{}", text.value.as_str())),
                R3Node::BoundText(_) => out.push("bound-text".to_string()),
                R3Node::Element(element) => {
                    for attr in &element.attributes {
                        out.push(format!("attr:{}", attr.name.as_str()));
                    }
                    for input in &element.inputs {
                        out.push(format!("input:{}", input.name.as_str()));
                    }
                    for child in &element.children {
                        describe(child, out);
                    }
                }
                _ => {}
            }
        }
        let mut nodes = std::vec::Vec::new();
        for node in &result.nodes {
            describe(node, &mut nodes);
        }
        let errors = result.errors.iter().map(|err| err.msg.clone()).collect();
        (nodes, errors)
    }

    #[test]
    fn closing_braces_before_an_interpolation_still_bind() {
        // A bare `}` in text closes a block, so text spells `}}` with entities.
        let (nodes, errors) = transform(r#"<div title="}} {{a}}" i18n-title></div>"#);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(nodes, ["input:title"]);

        let (nodes, errors) = transform("<p>&#125;&#125; {{a}}</p>");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(nodes, ["bound-text"]);

        let (nodes, errors) = transform("<p i18n>&#125;&#125; {{a}}</p>");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(nodes, ["bound-text"]);
    }

    #[test]
    fn text_without_an_interpolation_stays_static() {
        let (nodes, errors) = transform(
            r#"<div title="{{ a" i18n-title></div><p title="{{ '}}' ">&#123;&#123; a</p>"#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(nodes, ["attr:title", "attr:title", "text:{{ a"]);
    }

    #[test]
    fn blank_interpolations_are_reported() {
        for source in ["<p>a {{ }}</p>", r#"<p title="a {{}}"></p>"#, "<p>&amp; {{ }}</p>"] {
            let (_, errors) = transform(source);
            assert!(
                errors.iter().any(|msg| msg
                    .starts_with("Blank expressions are not allowed in interpolated strings")),
                "{source}: {errors:?}"
            );
        }
        let (_, errors) = transform("<p>a {{ }}</p>");
        assert_eq!(
            errors,
            ["Blank expressions are not allowed in interpolated strings at column 2 in [a {{ }}]"]
        );
    }
}

/// Key spans, ported from Angular's `test/render3/r3_ast_spans_spec.ts`: the key span of a binding
/// is its identifier, without the binding syntax around it.
#[cfg(test)]
mod key_span_tests {
    use super::*;
    use crate::parser::html::HtmlParser;
    use oxc_allocator::Allocator;

    type Keys = std::vec::Vec<(&'static str, String)>;

    /// `(node kind, key span text)` for every attribute, binding, reference and variable.
    fn key_spans(source: &str) -> Keys {
        let allocator = Allocator::default();
        let parsed = HtmlParser::new(&allocator, source, "test.html").parse();
        let result =
            html_ast_to_r3_ast(&allocator, source, &parsed.nodes, TransformOptions::default());
        let mut out = Keys::new();
        walk(source, &result.nodes, &mut out);
        out
    }

    fn text(source: &str, span: Span) -> String {
        source[span.start as usize..span.end as usize].to_string()
    }

    fn push_attrs(
        source: &str,
        out: &mut Keys,
        attributes: &[R3TextAttribute<'_>],
        inputs: &[R3BoundAttribute<'_>],
        outputs: &[R3BoundEvent<'_>],
        references: &[R3Reference<'_>],
    ) {
        for attr in attributes {
            out.push(("TextAttribute", attr.key_span.map(|s| text(source, s)).unwrap_or_default()));
        }
        for input in inputs {
            out.push(("BoundAttribute", text(source, input.key_span)));
        }
        for output in outputs {
            out.push(("BoundEvent", text(source, output.key_span)));
        }
        for reference in references {
            out.push(("Reference", text(source, reference.key_span)));
        }
    }

    fn walk(source: &str, nodes: &[R3Node<'_>], out: &mut Keys) {
        for node in nodes {
            match node {
                R3Node::Element(el) => {
                    push_attrs(
                        source,
                        out,
                        &el.attributes,
                        &el.inputs,
                        &el.outputs,
                        &el.references,
                    );
                    walk(source, &el.children, out);
                }
                R3Node::Template(tpl) => {
                    for attr in &tpl.template_attrs {
                        match attr {
                            R3TemplateAttr::Text(attr) => out.push((
                                "TextAttribute",
                                attr.key_span.map(|s| text(source, s)).unwrap_or_default(),
                            )),
                            R3TemplateAttr::Bound(input) => {
                                out.push(("BoundAttribute", text(source, input.key_span)));
                            }
                        }
                    }
                    for variable in &tpl.variables {
                        out.push(("Variable", text(source, variable.key_span)));
                    }
                    // A `*` template's own attributes are hoisted from the element it wraps,
                    // which reports them itself.
                    if tpl.template_attrs.is_empty() {
                        push_attrs(
                            source,
                            out,
                            &tpl.attributes,
                            &tpl.inputs,
                            &tpl.outputs,
                            &tpl.references,
                        );
                    }
                    walk(source, &tpl.children, out);
                }
                _ => {}
            }
        }
    }

    fn pairs(expected: &[(&'static str, &str)]) -> Keys {
        expected.iter().map(|(kind, key)| (*kind, (*key).to_string())).collect()
    }

    #[test]
    fn bound_attribute() {
        assert_eq!(
            key_spans(r#"<div [someProp]="v"></div>"#),
            pairs(&[("BoundAttribute", "someProp")])
        );
        assert_eq!(key_spans("<div [someProp]></div>"), pairs(&[("BoundAttribute", "someProp")]));
        // `attr.`, `class.` and `style.` stay in the key span: `parseAttribute` does not split them.
        assert_eq!(
            key_spans(r#"<div [attr.role]="r" [class.on]="c" [style.width.px]="w"></div>"#),
            pairs(&[
                ("BoundAttribute", "attr.role"),
                ("BoundAttribute", "class.on"),
                ("BoundAttribute", "style.width.px"),
            ])
        );
        assert_eq!(
            key_spans(r#"<div [@trigger]="v"></div>"#),
            pairs(&[("BoundAttribute", "trigger")])
        );
    }

    #[test]
    fn bind_prefix() {
        assert_eq!(key_spans(r#"<div bind-prop="v"></div>"#), pairs(&[("BoundAttribute", "prop")]));
        assert_eq!(
            key_spans(r#"<div bind-@animation="v"></div>"#),
            pairs(&[("BoundAttribute", "animation")])
        );
        assert_eq!(
            key_spans(r#"<div bind-animate-animationName="v"></div>"#),
            pairs(&[("BoundAttribute", "animationName")])
        );
        assert_eq!(key_spans("<div @animation></div>"), pairs(&[("BoundAttribute", "animation")]));
    }

    #[test]
    fn interpolated_attribute() {
        assert_eq!(key_spans(r#"<div prop="{{v}}"></div>"#), pairs(&[("BoundAttribute", "prop")]));
        assert_eq!(
            key_spans(r#"<div data-prop="{{v}}"></div>"#),
            pairs(&[("BoundAttribute", "data-prop")])
        );
    }

    #[test]
    fn event() {
        assert_eq!(
            key_spans(r#"<div (someEvent)="v"></div>"#),
            pairs(&[("BoundEvent", "someEvent")])
        );
        assert_eq!(key_spans(r#"<div on-event="v"></div>"#), pairs(&[("BoundEvent", "event")]));
        assert_eq!(
            key_spans(r#"<div (window:resize)="v"></div>"#),
            pairs(&[("BoundEvent", "window:resize")])
        );
        assert_eq!(
            key_spans(r#"<div (@name.done)="v"></div>"#),
            pairs(&[("BoundEvent", "name.done")])
        );
        assert_eq!(
            key_spans(r#"<div data-on-event="v"></div>"#),
            pairs(&[("TextAttribute", "data-on-event")])
        );
    }

    #[test]
    fn two_way_binding() {
        assert_eq!(
            key_spans(r#"<div [(prop)]="v"></div>"#),
            pairs(&[("BoundAttribute", "prop"), ("BoundEvent", "prop")])
        );
        assert_eq!(
            key_spans(r#"<div bindon-prop="v"></div>"#),
            pairs(&[("BoundAttribute", "prop"), ("BoundEvent", "prop")])
        );
    }

    #[test]
    fn references_and_variables() {
        assert_eq!(key_spans("<div #a></div>"), pairs(&[("Reference", "a")]));
        assert_eq!(key_spans(r#"<div #a="b"></div>"#), pairs(&[("Reference", "a")]));
        assert_eq!(key_spans("<div ref-a></div>"), pairs(&[("Reference", "a")]));
        assert_eq!(
            key_spans(r#"<ng-template let-a="b"></ng-template>"#),
            pairs(&[("Variable", "a")])
        );
    }

    #[test]
    fn template_attribute() {
        assert_eq!(key_spans("<div *ngIf></div>"), pairs(&[("TextAttribute", "ngIf")]));
        assert_eq!(
            key_spans(r#"<div *ngFor="let item of items; trackBy: trackByFn"></div>"#),
            pairs(&[
                ("TextAttribute", "ngFor"),
                ("BoundAttribute", "of"),
                ("BoundAttribute", "trackBy"),
                ("Variable", "item"),
            ])
        );
        assert_eq!(
            key_spans(r#"<div *ngFor="item of items"></div>"#),
            pairs(&[("BoundAttribute", "ngFor"), ("BoundAttribute", "of")])
        );
        assert_eq!(
            key_spans(r#"<div *ngIf="expr as local"></div>"#),
            pairs(&[("BoundAttribute", "ngIf"), ("Variable", "local")])
        );
        assert_eq!(
            key_spans(r#"<div *ngIf="let a=b"></div>"#),
            pairs(&[("TextAttribute", "ngIf"), ("Variable", "a")])
        );
    }
}

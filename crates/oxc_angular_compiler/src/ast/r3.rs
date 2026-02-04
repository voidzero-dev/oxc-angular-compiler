//! R3 AST nodes for the Angular template compiler.
//!
//! This module contains the R3 (Render3) AST, which is an intermediate
//! representation between the raw HTML AST and the IR operations.
//!
//! Ported from Angular's `render3/r3_ast.ts`.

use oxc_allocator::{Allocator, Box, HashMap, Vec};
use oxc_span::{Atom, Span};

use crate::ast::expression::{ASTWithSource, AngularExpression, BindingType, ParsedEventType};
use crate::i18n::serializer::format_i18n_placeholder_name;

// ============================================================================
// i18n Metadata
// ============================================================================

/// i18n metadata attached to R3 nodes.
#[derive(Debug)]
pub enum I18nMeta<'a> {
    /// Root of an i18n message.
    Message(I18nMessage<'a>),
    /// Part of a containing message.
    Node(I18nNode<'a>),
    /// Control flow block placeholder (for @if/@else/@for etc. inside i18n blocks).
    /// Ported from Angular's `i18n.BlockPlaceholder`.
    BlockPlaceholder(I18nBlockPlaceholder<'a>),
}

/// An i18n message containing translatable content.
#[derive(Debug)]
pub struct I18nMessage<'a> {
    /// Unique instance ID for this message.
    ///
    /// This ID is used to track message identity across moves/copies. It ensures that
    /// when an i18n attribute is copied (e.g., from element to conditional via
    /// `ingestControlFlowInsertionPoint`), both references can be linked to the same
    /// i18n context. Without this, Rust's move semantics would cause pointer-based
    /// identity checks to fail.
    ///
    /// Assigned during parsing and must be unique per compilation unit.
    pub instance_id: u32,
    /// Message AST nodes.
    pub nodes: Vec<'a, I18nNode<'a>>,
    /// The meaning of the message (for disambiguation).
    pub meaning: Atom<'a>,
    /// Description of the message for translators.
    pub description: Atom<'a>,
    /// Custom ID specified by the developer.
    pub custom_id: Atom<'a>,
    /// The computed message ID.
    pub id: Atom<'a>,
    /// Legacy IDs for backwards compatibility.
    pub legacy_ids: Vec<'a, Atom<'a>>,
    /// The serialized message string for goog.getMsg and $localize.
    /// Contains the message text with placeholder markers like "{$interpolation}".
    pub message_string: Atom<'a>,
}

/// i18n AST node.
#[derive(Debug)]
pub enum I18nNode<'a> {
    /// Plain text content.
    Text(I18nText<'a>),
    /// Container for child nodes.
    Container(I18nContainer<'a>),
    /// ICU expression (plural, select, selectordinal).
    Icu(I18nIcu<'a>),
    /// HTML tag placeholder.
    TagPlaceholder(I18nTagPlaceholder<'a>),
    /// Expression placeholder.
    Placeholder(I18nPlaceholder<'a>),
    /// ICU placeholder (nested ICU reference).
    IcuPlaceholder(I18nIcuPlaceholder<'a>),
    /// Control flow block placeholder.
    BlockPlaceholder(I18nBlockPlaceholder<'a>),
}

/// Plain text content.
#[derive(Debug)]
pub struct I18nText<'a> {
    /// The text value.
    pub value: Atom<'a>,
    /// Source span.
    pub source_span: Span,
}

/// Container for child nodes.
#[derive(Debug)]
pub struct I18nContainer<'a> {
    /// Child nodes.
    pub children: Vec<'a, I18nNode<'a>>,
    /// Source span.
    pub source_span: Span,
}

/// ICU expression (plural, select, selectordinal).
#[derive(Debug)]
pub struct I18nIcu<'a> {
    /// The expression being evaluated.
    pub expression: Atom<'a>,
    /// ICU type string (plural, select, selectordinal, or custom).
    pub icu_type: Atom<'a>,
    /// Case branches.
    pub cases: HashMap<'a, Atom<'a>, I18nNode<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Expression placeholder name (for message serialization).
    pub expression_placeholder: Option<Atom<'a>>,
}

/// HTML tag placeholder.
#[derive(Debug)]
pub struct I18nTagPlaceholder<'a> {
    /// Tag name.
    pub tag: Atom<'a>,
    /// Tag attributes.
    pub attrs: HashMap<'a, Atom<'a>, Atom<'a>>,
    /// Start tag placeholder name.
    pub start_name: Atom<'a>,
    /// Close tag placeholder name.
    pub close_name: Atom<'a>,
    /// Child nodes.
    pub children: Vec<'a, I18nNode<'a>>,
    /// Whether this is a void element.
    pub is_void: bool,
    /// Source span (overall).
    pub source_span: Span,
    /// Start tag source span.
    pub start_source_span: Option<Span>,
    /// End tag source span.
    pub end_source_span: Option<Span>,
}

/// Expression placeholder.
#[derive(Debug)]
pub struct I18nPlaceholder<'a> {
    /// The expression value.
    pub value: Atom<'a>,
    /// Placeholder name.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Span,
}

/// ICU placeholder (reference to a nested ICU).
#[derive(Debug)]
pub struct I18nIcuPlaceholder<'a> {
    /// The ICU expression.
    pub value: Box<'a, I18nIcu<'a>>,
    /// Placeholder name.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Span,
}

/// Control flow block placeholder.
#[derive(Debug)]
pub struct I18nBlockPlaceholder<'a> {
    /// Block name.
    pub name: Atom<'a>,
    /// Block parameters.
    pub parameters: Vec<'a, Atom<'a>>,
    /// Start block placeholder name.
    pub start_name: Atom<'a>,
    /// End block placeholder name.
    pub close_name: Atom<'a>,
    /// Child nodes.
    pub children: Vec<'a, I18nNode<'a>>,
    /// Source span (overall).
    pub source_span: Span,
    /// Start block source span.
    pub start_source_span: Option<Span>,
    /// End block source span.
    pub end_source_span: Option<Span>,
}

// ============================================================================
// i18n Clone implementations
// ============================================================================

impl<'a> I18nMeta<'a> {
    /// Creates a deep clone of this i18n metadata using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        match self {
            I18nMeta::Message(msg) => I18nMeta::Message(msg.clone_in(allocator)),
            I18nMeta::Node(node) => I18nMeta::Node(node.clone_in(allocator)),
            I18nMeta::BlockPlaceholder(bp) => I18nMeta::BlockPlaceholder(bp.clone_in(allocator)),
        }
    }
}

impl<'a> I18nMessage<'a> {
    /// Creates a deep clone of this i18n message using the provided allocator.
    ///
    /// Note: This preserves the `instance_id` so that cloned messages maintain
    /// their identity for i18n context sharing.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        let mut nodes = Vec::new_in(allocator);
        for node in self.nodes.iter() {
            nodes.push(node.clone_in(allocator));
        }
        let mut legacy_ids = Vec::new_in(allocator);
        for id in self.legacy_ids.iter() {
            legacy_ids.push(id.clone());
        }
        I18nMessage {
            instance_id: self.instance_id,
            nodes,
            meaning: self.meaning.clone(),
            description: self.description.clone(),
            custom_id: self.custom_id.clone(),
            id: self.id.clone(),
            legacy_ids,
            message_string: self.message_string.clone(),
        }
    }
}

impl<'a> I18nNode<'a> {
    /// Creates a deep clone of this i18n node using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        match self {
            I18nNode::Text(t) => I18nNode::Text(t.clone_in()),
            I18nNode::Container(c) => I18nNode::Container(c.clone_in(allocator)),
            I18nNode::Icu(i) => I18nNode::Icu(i.clone_in(allocator)),
            I18nNode::TagPlaceholder(tp) => I18nNode::TagPlaceholder(tp.clone_in(allocator)),
            I18nNode::Placeholder(p) => I18nNode::Placeholder(p.clone_in()),
            I18nNode::IcuPlaceholder(ip) => I18nNode::IcuPlaceholder(ip.clone_in(allocator)),
            I18nNode::BlockPlaceholder(bp) => I18nNode::BlockPlaceholder(bp.clone_in(allocator)),
        }
    }
}

impl<'a> I18nText<'a> {
    /// Creates a clone of this i18n text (no allocator needed - only atoms and spans).
    pub fn clone_in(&self) -> Self {
        I18nText { value: self.value.clone(), source_span: self.source_span }
    }
}

impl<'a> I18nContainer<'a> {
    /// Creates a deep clone of this i18n container using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        let mut children = Vec::new_in(allocator);
        for child in self.children.iter() {
            children.push(child.clone_in(allocator));
        }
        I18nContainer { children, source_span: self.source_span }
    }
}

impl<'a> I18nIcu<'a> {
    /// Creates a deep clone of this ICU expression using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        let mut cases = HashMap::new_in(allocator);
        for (key, value) in self.cases.iter() {
            cases.insert(key.clone(), value.clone_in(allocator));
        }
        I18nIcu {
            expression: self.expression.clone(),
            icu_type: self.icu_type.clone(),
            cases,
            source_span: self.source_span,
            expression_placeholder: self.expression_placeholder.clone(),
        }
    }
}

impl<'a> I18nTagPlaceholder<'a> {
    /// Creates a deep clone of this tag placeholder using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        let mut attrs = HashMap::new_in(allocator);
        for (key, value) in self.attrs.iter() {
            attrs.insert(key.clone(), value.clone());
        }
        let mut children = Vec::new_in(allocator);
        for child in self.children.iter() {
            children.push(child.clone_in(allocator));
        }
        I18nTagPlaceholder {
            tag: self.tag.clone(),
            attrs,
            start_name: self.start_name.clone(),
            close_name: self.close_name.clone(),
            children,
            is_void: self.is_void,
            source_span: self.source_span,
            start_source_span: self.start_source_span,
            end_source_span: self.end_source_span,
        }
    }
}

impl<'a> I18nPlaceholder<'a> {
    /// Creates a clone of this placeholder (no allocator needed - only atoms and spans).
    pub fn clone_in(&self) -> Self {
        I18nPlaceholder {
            value: self.value.clone(),
            name: self.name.clone(),
            source_span: self.source_span,
        }
    }
}

impl<'a> I18nIcuPlaceholder<'a> {
    /// Creates a deep clone of this ICU placeholder using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        I18nIcuPlaceholder {
            value: Box::new_in(self.value.clone_in(allocator), allocator),
            name: self.name.clone(),
            source_span: self.source_span,
        }
    }
}

impl<'a> I18nBlockPlaceholder<'a> {
    /// Creates a deep clone of this block placeholder using the provided allocator.
    pub fn clone_in(&self, allocator: &'a Allocator) -> Self {
        let mut parameters = Vec::new_in(allocator);
        for param in self.parameters.iter() {
            parameters.push(param.clone());
        }
        let mut children = Vec::new_in(allocator);
        for child in self.children.iter() {
            children.push(child.clone_in(allocator));
        }
        I18nBlockPlaceholder {
            name: self.name.clone(),
            parameters,
            start_name: self.start_name.clone(),
            close_name: self.close_name.clone(),
            children,
            source_span: self.source_span,
            start_source_span: self.start_source_span,
            end_source_span: self.end_source_span,
        }
    }
}

// ============================================================================
// i18n Message Serialization
// ============================================================================

/// Serialize i18n nodes to the $localize / goog.getMsg message format.
///
/// This produces a message string with placeholder markers like "{$interpolation}"
/// for expression placeholders and "{$startTag}/{$closeTag}" for element boundaries.
///
/// Ported from Angular's `serialize_message` in `i18n/i18n_ast.ts`.
pub fn serialize_i18n_nodes(nodes: &[I18nNode<'_>]) -> String {
    let mut result = String::new();
    for node in nodes {
        serialize_i18n_node(node, &mut result);
    }
    result
}

/// Serialize a single i18n node to the message string format.
///
/// Placeholder names are formatted to camelCase for goog.getMsg compatibility.
/// For example: `INTERPOLATION` -> `{$interpolation}`, `START_TAG_DIV` -> `{$startTagDiv}`
fn serialize_i18n_node(node: &I18nNode<'_>, result: &mut String) {
    match node {
        I18nNode::Text(text) => {
            result.push_str(text.value.as_str());
        }
        I18nNode::Container(container) => {
            for child in container.children.iter() {
                serialize_i18n_node(child, result);
            }
        }
        I18nNode::Icu(icu) => {
            serialize_i18n_icu(icu, result);
        }
        I18nNode::TagPlaceholder(ph) => {
            let start_name = format_i18n_placeholder_name(ph.start_name.as_str(), true);
            let close_name = format_i18n_placeholder_name(ph.close_name.as_str(), true);
            result.push_str(&format!("{{${start_name}}}"));
            for child in ph.children.iter() {
                serialize_i18n_node(child, result);
            }
            result.push_str(&format!("{{${close_name}}}"));
        }
        I18nNode::Placeholder(ph) => {
            let name = format_i18n_placeholder_name(ph.name.as_str(), true);
            result.push_str(&format!("{{${name}}}"));
        }
        I18nNode::IcuPlaceholder(ph) => {
            let name = format_i18n_placeholder_name(ph.name.as_str(), true);
            result.push_str(&format!("{{${name}}}"));
        }
        I18nNode::BlockPlaceholder(ph) => {
            let start_name = format_i18n_placeholder_name(ph.start_name.as_str(), true);
            let close_name = format_i18n_placeholder_name(ph.close_name.as_str(), true);
            result.push_str(&format!("{{${start_name}}}"));
            for child in ph.children.iter() {
                serialize_i18n_node(child, result);
            }
            result.push_str(&format!("{{${close_name}}}"));
        }
    }
}

/// Serialize an ICU expression to the message string format.
fn serialize_i18n_icu(icu: &I18nIcu<'_>, result: &mut String) {
    // Use expression_placeholder if available, otherwise use expression directly
    let expr =
        icu.expression_placeholder.as_ref().map_or_else(|| icu.expression.as_str(), |p| p.as_str());

    result.push('{');
    result.push_str(expr);
    result.push_str(", ");
    result.push_str(icu.icu_type.as_str());
    result.push_str(", ");

    // Serialize cases - must be sorted for deterministic output
    let mut cases: std::vec::Vec<_> = icu.cases.iter().collect();
    cases.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

    for (i, (key, value)) in cases.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push_str(key.as_str());
        result.push_str(" {");
        serialize_i18n_node(value, result);
        result.push('}');
    }

    result.push('}');
}

// ============================================================================
// Core Node Enum
// ============================================================================

/// The main R3 node enum containing all R3 AST node types.
#[derive(Debug)]
pub enum R3Node<'a> {
    /// A comment node.
    Comment(Box<'a, R3Comment<'a>>),
    /// A static text node.
    Text(Box<'a, R3Text<'a>>),
    /// A bound text node with interpolation.
    BoundText(Box<'a, R3BoundText<'a>>),
    /// An HTML element.
    Element(Box<'a, R3Element<'a>>),
    /// A template element (`<ng-template>` or structural directive).
    Template(Box<'a, R3Template<'a>>),
    /// A content projection slot (`<ng-content>`).
    Content(Box<'a, R3Content<'a>>),
    /// A template variable (`let x`).
    Variable(Box<'a, R3Variable<'a>>),
    /// A template reference (`#ref`).
    Reference(Box<'a, R3Reference<'a>>),
    /// An ICU message.
    Icu(Box<'a, R3Icu<'a>>),
    /// A deferred block (`@defer`).
    DeferredBlock(Box<'a, R3DeferredBlock<'a>>),
    /// A deferred block placeholder (`@placeholder`).
    DeferredBlockPlaceholder(Box<'a, R3DeferredBlockPlaceholder<'a>>),
    /// A deferred block loading state (`@loading`).
    DeferredBlockLoading(Box<'a, R3DeferredBlockLoading<'a>>),
    /// A deferred block error state (`@error`).
    DeferredBlockError(Box<'a, R3DeferredBlockError<'a>>),
    /// A switch block (`@switch`).
    SwitchBlock(Box<'a, R3SwitchBlock<'a>>),
    /// A switch block case group (containing cases and children).
    SwitchBlockCaseGroup(Box<'a, R3SwitchBlockCaseGroup<'a>>),
    /// A for loop block (`@for`).
    ForLoopBlock(Box<'a, R3ForLoopBlock<'a>>),
    /// A for loop empty block (`@empty`).
    ForLoopBlockEmpty(Box<'a, R3ForLoopBlockEmpty<'a>>),
    /// An if block (`@if`).
    IfBlock(Box<'a, R3IfBlock<'a>>),
    /// An if block branch (`@if`, `@else if`, `@else`).
    IfBlockBranch(Box<'a, R3IfBlockBranch<'a>>),
    /// An unknown block (for error recovery).
    UnknownBlock(Box<'a, R3UnknownBlock<'a>>),
    /// A let declaration (`@let`).
    LetDeclaration(Box<'a, R3LetDeclaration<'a>>),
    /// A component reference.
    Component(Box<'a, R3Component<'a>>),
    /// A directive reference.
    Directive(Box<'a, R3Directive<'a>>),
    /// A host element (for type checking only, cannot be visited).
    HostElement(Box<'a, R3HostElement<'a>>),
}

impl<'a> R3Node<'a> {
    /// Returns the source span of this node.
    pub fn source_span(&self) -> Span {
        match self {
            Self::Comment(n) => n.source_span,
            Self::Text(n) => n.source_span,
            Self::BoundText(n) => n.source_span,
            Self::Element(n) => n.source_span,
            Self::Template(n) => n.source_span,
            Self::Content(n) => n.source_span,
            Self::Variable(n) => n.source_span,
            Self::Reference(n) => n.source_span,
            Self::Icu(n) => n.source_span,
            Self::DeferredBlock(n) => n.source_span,
            Self::DeferredBlockPlaceholder(n) => n.source_span,
            Self::DeferredBlockLoading(n) => n.source_span,
            Self::DeferredBlockError(n) => n.source_span,
            Self::SwitchBlock(n) => n.source_span,
            Self::SwitchBlockCaseGroup(n) => n.source_span,
            Self::ForLoopBlock(n) => n.source_span,
            Self::ForLoopBlockEmpty(n) => n.source_span,
            Self::IfBlock(n) => n.source_span,
            Self::IfBlockBranch(n) => n.source_span,
            Self::UnknownBlock(n) => n.source_span,
            Self::LetDeclaration(n) => n.source_span,
            Self::Component(n) => n.source_span,
            Self::Directive(n) => n.source_span,
            Self::HostElement(n) => n.source_span,
        }
    }

    /// Visit this node with the given visitor.
    pub fn visit<V: R3Visitor<'a> + ?Sized>(&self, visitor: &mut V) {
        match self {
            Self::Comment(n) => visitor.visit_comment(n),
            Self::Text(n) => visitor.visit_text(n),
            Self::BoundText(n) => visitor.visit_bound_text(n),
            Self::Element(n) => visitor.visit_element(n),
            Self::Template(n) => visitor.visit_template(n),
            Self::Content(n) => visitor.visit_content(n),
            Self::Variable(n) => visitor.visit_variable(n),
            Self::Reference(n) => visitor.visit_reference(n),
            Self::Icu(n) => visitor.visit_icu(n),
            Self::DeferredBlock(n) => visitor.visit_deferred_block(n),
            Self::DeferredBlockPlaceholder(n) => visitor.visit_deferred_block_placeholder(n),
            Self::DeferredBlockLoading(n) => visitor.visit_deferred_block_loading(n),
            Self::DeferredBlockError(n) => visitor.visit_deferred_block_error(n),
            Self::SwitchBlock(n) => visitor.visit_switch_block(n),
            Self::SwitchBlockCaseGroup(n) => visitor.visit_switch_block_case_group(n),
            Self::ForLoopBlock(n) => visitor.visit_for_loop_block(n),
            Self::ForLoopBlockEmpty(n) => visitor.visit_for_loop_block_empty(n),
            Self::IfBlock(n) => visitor.visit_if_block(n),
            Self::IfBlockBranch(n) => visitor.visit_if_block_branch(n),
            Self::UnknownBlock(n) => visitor.visit_unknown_block(n),
            Self::LetDeclaration(n) => visitor.visit_let_declaration(n),
            Self::Component(n) => visitor.visit_component(n),
            Self::Directive(n) => visitor.visit_directive(n),
            // HostElement cannot be visited (used only for type checking)
            Self::HostElement(_) => {}
        }
    }
}

// ============================================================================
// Basic Nodes
// ============================================================================

/// A comment node.
#[derive(Debug, Clone)]
pub struct R3Comment<'a> {
    /// The comment text.
    pub value: Atom<'a>,
    /// Source span.
    pub source_span: Span,
}

/// A static text node.
#[derive(Debug)]
pub struct R3Text<'a> {
    /// The text content.
    pub value: Atom<'a>,
    /// Source span.
    pub source_span: Span,
}

/// A bound text node containing interpolation.
#[derive(Debug)]
pub struct R3BoundText<'a> {
    /// The interpolation expression.
    pub value: AngularExpression<'a>,
    /// Source span.
    pub source_span: Span,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

// ============================================================================
// Attributes
// ============================================================================

/// A static text attribute.
#[derive(Debug)]
pub struct R3TextAttribute<'a> {
    /// Attribute name.
    pub name: Atom<'a>,
    /// Attribute value.
    pub value: Atom<'a>,
    /// Source span.
    pub source_span: Span,
    /// Key span (the attribute name).
    pub key_span: Option<Span>,
    /// Value span.
    pub value_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// Security context for DOM sanitization.
///
/// In Angular, this can be a single context or an array of contexts
/// (e.g., when an attribute could match multiple element types).
/// The special case `[URL, RESOURCE_URL]` is represented here as
/// `UrlOrResourceUrl` for simplicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityContext {
    /// No security context needed.
    None,
    /// HTML content.
    Html,
    /// Style content.
    Style,
    /// Script content.
    Script,
    /// URL content.
    Url,
    /// Resource URL content.
    ResourceUrl,
    /// Attribute that should not be bound (security-sensitive).
    AttributeNoBinding,
    /// Ambiguous URL/ResourceURL context - resolved at runtime.
    ///
    /// This is used when the host element isn't known and the attribute
    /// (like `src` or `href`) could require either URL or ResourceURL
    /// sanitization depending on the element type. The runtime function
    /// `ɵɵsanitizeUrlOrResourceUrl` selects the appropriate sanitizer
    /// based on the element tag name.
    UrlOrResourceUrl,
}

impl Default for SecurityContext {
    fn default() -> Self {
        Self::None
    }
}

/// A bound attribute with a dynamic expression.
#[derive(Debug)]
pub struct R3BoundAttribute<'a> {
    /// Attribute name.
    pub name: Atom<'a>,
    /// Binding type (Property, Attribute, Class, Style, etc.).
    pub binding_type: BindingType,
    /// Security context for sanitization.
    pub security_context: SecurityContext,
    /// The binding expression.
    pub value: AngularExpression<'a>,
    /// Unit for style bindings (e.g., "px").
    pub unit: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Key span.
    pub key_span: Span,
    /// Value span.
    pub value_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A bound event.
#[derive(Debug)]
pub struct R3BoundEvent<'a> {
    /// Event name.
    pub name: Atom<'a>,
    /// Event type.
    pub event_type: ParsedEventType,
    /// Handler expression.
    pub handler: AngularExpression<'a>,
    /// Target element (for `window:` or `document:` events).
    pub target: Option<Atom<'a>>,
    /// Animation phase.
    pub phase: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Handler span.
    pub handler_span: Span,
    /// Key span.
    pub key_span: Span,
}

// ============================================================================
// Elements
// ============================================================================

/// An HTML element.
#[derive(Debug)]
pub struct R3Element<'a> {
    /// Element tag name.
    pub name: Atom<'a>,
    /// Static attributes.
    pub attributes: Vec<'a, R3TextAttribute<'a>>,
    /// Bound input properties.
    pub inputs: Vec<'a, R3BoundAttribute<'a>>,
    /// Bound output events.
    pub outputs: Vec<'a, R3BoundEvent<'a>>,
    /// Directives applied to this element.
    pub directives: Vec<'a, R3Directive<'a>>,
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Template references.
    pub references: Vec<'a, R3Reference<'a>>,
    /// Whether the element is self-closing.
    pub is_self_closing: bool,
    /// Source span.
    pub source_span: Span,
    /// Start tag span.
    pub start_source_span: Span,
    /// End tag span (None for self-closing).
    pub end_source_span: Option<Span>,
    /// Whether the element is void (e.g., `<br>`).
    pub is_void: bool,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A template element (`<ng-template>` or structural directive).
#[derive(Debug)]
pub struct R3Template<'a> {
    /// Tag name (None for structural directives on `ng-template`).
    pub tag_name: Option<Atom<'a>>,
    /// Static attributes.
    pub attributes: Vec<'a, R3TextAttribute<'a>>,
    /// Bound inputs.
    pub inputs: Vec<'a, R3BoundAttribute<'a>>,
    /// Bound outputs.
    pub outputs: Vec<'a, R3BoundEvent<'a>>,
    /// Directives.
    pub directives: Vec<'a, R3Directive<'a>>,
    /// Template attributes (from structural directive microsyntax).
    pub template_attrs: Vec<'a, R3TemplateAttr<'a>>,
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Template references.
    pub references: Vec<'a, R3Reference<'a>>,
    /// Template variables.
    pub variables: Vec<'a, R3Variable<'a>>,
    /// Whether self-closing.
    pub is_self_closing: bool,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A template attribute (either bound or static).
#[derive(Debug)]
pub enum R3TemplateAttr<'a> {
    /// A bound attribute.
    Bound(R3BoundAttribute<'a>),
    /// A text attribute.
    Text(R3TextAttribute<'a>),
}

/// A content projection slot (`<ng-content>`).
#[derive(Debug)]
pub struct R3Content<'a> {
    /// The selector for content projection.
    pub selector: Atom<'a>,
    /// Static attributes.
    pub attributes: Vec<'a, R3TextAttribute<'a>>,
    /// Child nodes (usually empty).
    pub children: Vec<'a, R3Node<'a>>,
    /// Whether self-closing.
    pub is_self_closing: bool,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A template variable.
#[derive(Debug)]
pub struct R3Variable<'a> {
    /// Variable name.
    pub name: Atom<'a>,
    /// Variable value (for `let x = value`).
    pub value: Atom<'a>,
    /// Source span.
    pub source_span: Span,
    /// Key span.
    pub key_span: Span,
    /// Value span.
    pub value_span: Option<Span>,
}

/// A template reference.
#[derive(Debug)]
pub struct R3Reference<'a> {
    /// Reference name.
    pub name: Atom<'a>,
    /// Reference value (directive name or empty).
    pub value: Atom<'a>,
    /// Source span.
    pub source_span: Span,
    /// Key span.
    pub key_span: Span,
    /// Value span.
    pub value_span: Option<Span>,
}

// ============================================================================
// Control Flow Blocks
// ============================================================================

/// An if block (`@if`).
#[derive(Debug)]
pub struct R3IfBlock<'a> {
    /// The branches of the if block.
    pub branches: Vec<'a, R3IfBlockBranch<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
}

/// An if block branch.
#[derive(Debug)]
pub struct R3IfBlockBranch<'a> {
    /// Condition expression (None for `@else`).
    pub expression: Option<AngularExpression<'a>>,
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Expression alias (`as` variable).
    pub expression_alias: Option<R3Variable<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A for loop block (`@for`).
#[derive(Debug)]
pub struct R3ForLoopBlock<'a> {
    /// The loop item variable.
    pub item: R3Variable<'a>,
    /// The iterable expression.
    pub expression: ASTWithSource<'a>,
    /// The track expression.
    pub track_by: ASTWithSource<'a>,
    /// Track keyword span.
    pub track_keyword_span: Span,
    /// Context variables ($index, $first, etc.).
    pub context_variables: Vec<'a, R3Variable<'a>>,
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Empty block.
    pub empty: Option<R3ForLoopBlockEmpty<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Main block span.
    pub main_block_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A for loop empty block (`@empty`).
#[derive(Debug)]
pub struct R3ForLoopBlockEmpty<'a> {
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A switch block (`@switch`).
#[derive(Debug)]
pub struct R3SwitchBlock<'a> {
    /// The switch expression.
    pub expression: AngularExpression<'a>,
    /// The switch case groups.
    pub groups: Vec<'a, R3SwitchBlockCaseGroup<'a>>,
    /// Unknown blocks for error recovery.
    pub unknown_blocks: Vec<'a, R3UnknownBlock<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
}

/// A switch block case (`@case` or `@default`).
///
/// Note: In the R3 AST, `SwitchBlockCase` does NOT have children.
/// Children are stored in `SwitchBlockCaseGroup`.
#[derive(Debug)]
pub struct R3SwitchBlockCase<'a> {
    /// Case expression (None for `@default`).
    pub expression: Option<AngularExpression<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
}

/// A switch block case group.
///
/// Groups consecutive case blocks that fall through to the same body.
/// For example: `@case (1) @case (2) { body }` creates one group with
/// two cases and the body as children.
#[derive(Debug)]
pub struct R3SwitchBlockCaseGroup<'a> {
    /// The switch cases in this group.
    pub cases: Vec<'a, R3SwitchBlockCase<'a>>,
    /// Child nodes (the body of the group).
    pub children: Vec<'a, R3Node<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// Name span.
    pub name_span: Span,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

// ============================================================================
// Deferred Blocks
// ============================================================================

/// A deferred trigger.
#[derive(Debug)]
pub enum R3DeferredTrigger<'a> {
    /// A bound trigger (`when condition`).
    Bound(R3BoundDeferredTrigger<'a>),
    /// Never trigger.
    Never(R3NeverDeferredTrigger),
    /// Idle trigger.
    Idle(R3IdleDeferredTrigger),
    /// Immediate trigger.
    Immediate(R3ImmediateDeferredTrigger),
    /// Hover trigger.
    Hover(R3HoverDeferredTrigger<'a>),
    /// Timer trigger.
    Timer(R3TimerDeferredTrigger),
    /// Interaction trigger.
    Interaction(R3InteractionDeferredTrigger<'a>),
    /// Viewport trigger.
    Viewport(R3ViewportDeferredTrigger<'a>),
}

impl<'a> R3DeferredTrigger<'a> {
    /// Returns the source span of the trigger.
    pub fn source_span(&self) -> Span {
        match self {
            Self::Bound(t) => t.source_span,
            Self::Never(t) => t.source_span,
            Self::Idle(t) => t.source_span,
            Self::Immediate(t) => t.source_span,
            Self::Hover(t) => t.source_span,
            Self::Timer(t) => t.source_span,
            Self::Interaction(t) => t.source_span,
            Self::Viewport(t) => t.source_span,
        }
    }
}

/// A bound deferred trigger (`when condition`).
#[derive(Debug)]
pub struct R3BoundDeferredTrigger<'a> {
    /// The condition expression.
    pub value: AngularExpression<'a>,
    /// Source span.
    pub source_span: Span,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// When source span.
    pub when_source_span: Span,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// A never deferred trigger.
#[derive(Debug)]
pub struct R3NeverDeferredTrigger {
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Option<Span>,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// When/on source span.
    pub when_or_on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// An idle deferred trigger.
#[derive(Debug)]
pub struct R3IdleDeferredTrigger {
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Option<Span>,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// When/on source span.
    pub when_or_on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// An immediate deferred trigger.
#[derive(Debug)]
pub struct R3ImmediateDeferredTrigger {
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Option<Span>,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// When/on source span.
    pub when_or_on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// A hover deferred trigger.
#[derive(Debug)]
pub struct R3HoverDeferredTrigger<'a> {
    /// Reference to the element to hover.
    pub reference: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// On source span.
    pub on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// A timer deferred trigger.
#[derive(Debug)]
pub struct R3TimerDeferredTrigger {
    /// Delay in milliseconds.
    pub delay: u32,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// On source span.
    pub on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// An interaction deferred trigger.
#[derive(Debug)]
pub struct R3InteractionDeferredTrigger<'a> {
    /// Reference to the element to interact with.
    pub reference: Option<Atom<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// On source span.
    pub on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// A viewport deferred trigger.
#[derive(Debug)]
pub struct R3ViewportDeferredTrigger<'a> {
    /// Reference to the element to observe.
    pub reference: Option<Atom<'a>>,
    /// Viewport options (margin, etc.).
    pub options: Option<AngularExpression<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Prefetch span.
    pub prefetch_span: Option<Span>,
    /// On source span.
    pub on_source_span: Option<Span>,
    /// Hydrate span.
    pub hydrate_span: Option<Span>,
}

/// Deferred block triggers.
#[derive(Debug, Default)]
pub struct R3DeferredBlockTriggers<'a> {
    /// When trigger.
    pub when: Option<R3BoundDeferredTrigger<'a>>,
    /// Idle trigger.
    pub idle: Option<R3IdleDeferredTrigger>,
    /// Immediate trigger.
    pub immediate: Option<R3ImmediateDeferredTrigger>,
    /// Hover trigger.
    pub hover: Option<R3HoverDeferredTrigger<'a>>,
    /// Timer trigger.
    pub timer: Option<R3TimerDeferredTrigger>,
    /// Interaction trigger.
    pub interaction: Option<R3InteractionDeferredTrigger<'a>>,
    /// Viewport trigger.
    pub viewport: Option<R3ViewportDeferredTrigger<'a>>,
    /// Never trigger.
    pub never: Option<R3NeverDeferredTrigger>,
}

impl R3DeferredBlockTriggers<'_> {
    /// Returns true if any trigger is set.
    pub fn has_any(&self) -> bool {
        self.when.is_some()
            || self.idle.is_some()
            || self.immediate.is_some()
            || self.hover.is_some()
            || self.timer.is_some()
            || self.interaction.is_some()
            || self.viewport.is_some()
            || self.never.is_some()
    }
}

/// A deferred block (`@defer`).
#[derive(Debug)]
pub struct R3DeferredBlock<'a> {
    /// Child nodes (main content).
    pub children: Vec<'a, R3Node<'a>>,
    /// Load triggers.
    pub triggers: R3DeferredBlockTriggers<'a>,
    /// Prefetch triggers.
    pub prefetch_triggers: R3DeferredBlockTriggers<'a>,
    /// Hydrate triggers.
    pub hydrate_triggers: R3DeferredBlockTriggers<'a>,
    /// Placeholder block.
    pub placeholder: Option<R3DeferredBlockPlaceholder<'a>>,
    /// Loading block.
    pub loading: Option<R3DeferredBlockLoading<'a>>,
    /// Error block.
    pub error: Option<R3DeferredBlockError<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Main block span.
    pub main_block_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A deferred block placeholder (`@placeholder`).
#[derive(Debug)]
pub struct R3DeferredBlockPlaceholder<'a> {
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Minimum time to show.
    pub minimum_time: Option<u32>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A deferred block loading state (`@loading`).
#[derive(Debug)]
pub struct R3DeferredBlockLoading<'a> {
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Minimum time before showing.
    pub after_time: Option<u32>,
    /// Minimum time to show.
    pub minimum_time: Option<u32>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A deferred block error state (`@error`).
#[derive(Debug)]
pub struct R3DeferredBlockError<'a> {
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

// ============================================================================
// Miscellaneous
// ============================================================================

/// An unknown block (for error recovery).
#[derive(Debug)]
pub struct R3UnknownBlock<'a> {
    /// Block name.
    pub name: Atom<'a>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
}

/// A let declaration (`@let`).
#[derive(Debug)]
pub struct R3LetDeclaration<'a> {
    /// Variable name.
    pub name: Atom<'a>,
    /// Value expression.
    pub value: AngularExpression<'a>,
    /// Source span.
    pub source_span: Span,
    /// Name span.
    pub name_span: Span,
    /// Value span.
    pub value_span: Span,
}

/// A component reference in the template.
#[derive(Debug)]
pub struct R3Component<'a> {
    /// Component class name.
    pub component_name: Atom<'a>,
    /// Tag name in template.
    pub tag_name: Option<Atom<'a>>,
    /// Full component name.
    pub full_name: Atom<'a>,
    /// Static attributes.
    pub attributes: Vec<'a, R3TextAttribute<'a>>,
    /// Bound inputs.
    pub inputs: Vec<'a, R3BoundAttribute<'a>>,
    /// Bound outputs.
    pub outputs: Vec<'a, R3BoundEvent<'a>>,
    /// Directives.
    pub directives: Vec<'a, R3Directive<'a>>,
    /// Child nodes.
    pub children: Vec<'a, R3Node<'a>>,
    /// References.
    pub references: Vec<'a, R3Reference<'a>>,
    /// Whether self-closing.
    pub is_self_closing: bool,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// A directive reference in the template.
#[derive(Debug)]
pub struct R3Directive<'a> {
    /// Directive class name.
    pub name: Atom<'a>,
    /// Static attributes.
    pub attributes: Vec<'a, R3TextAttribute<'a>>,
    /// Bound inputs.
    pub inputs: Vec<'a, R3BoundAttribute<'a>>,
    /// Bound outputs.
    pub outputs: Vec<'a, R3BoundEvent<'a>>,
    /// References.
    pub references: Vec<'a, R3Reference<'a>>,
    /// Source span.
    pub source_span: Span,
    /// Start span.
    pub start_source_span: Span,
    /// End span.
    pub end_source_span: Option<Span>,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// Represents the host element of a directive.
/// This node is used only for type checking purposes and cannot be produced
/// from a user's template. HostElement nodes should NOT be visited.
#[derive(Debug)]
pub struct R3HostElement<'a> {
    /// Possible tag names for the host element. Must have at least one.
    pub tag_names: Vec<'a, Atom<'a>>,
    /// Attribute and property bindings.
    pub bindings: Vec<'a, R3BoundAttribute<'a>>,
    /// Event listeners.
    pub listeners: Vec<'a, R3BoundEvent<'a>>,
    /// Source span.
    pub source_span: Span,
}

/// An ICU message for internationalization.
#[derive(Debug)]
pub struct R3Icu<'a> {
    /// Variable expressions (ordered: must preserve insertion order like JS objects).
    pub vars: Vec<'a, (Atom<'a>, R3BoundText<'a>)>,
    /// Placeholder expressions (ordered: must preserve insertion order like JS objects).
    pub placeholders: Vec<'a, (Atom<'a>, R3IcuPlaceholder<'a>)>,
    /// Source span.
    pub source_span: Span,
    /// i18n metadata.
    pub i18n: Option<I18nMeta<'a>>,
}

/// An ICU placeholder (text or bound text).
#[derive(Debug)]
pub enum R3IcuPlaceholder<'a> {
    /// A static text placeholder.
    Text(R3Text<'a>),
    /// A bound text placeholder.
    BoundText(R3BoundText<'a>),
}

// ============================================================================
// Visitor
// ============================================================================

/// Visitor trait for R3 AST nodes.
pub trait R3Visitor<'a> {
    /// Visit a comment node.
    fn visit_comment(&mut self, _comment: &R3Comment<'a>) {}

    /// Visit a text node.
    fn visit_text(&mut self, _text: &R3Text<'a>) {}

    /// Visit a bound text node.
    fn visit_bound_text(&mut self, _text: &R3BoundText<'a>) {}

    /// Visit an element.
    fn visit_element(&mut self, element: &R3Element<'a>) {
        self.visit_element_children(element);
    }

    /// Visit element children.
    fn visit_element_children(&mut self, element: &R3Element<'a>) {
        for child in &element.children {
            child.visit(self);
        }
    }

    /// Visit a template.
    fn visit_template(&mut self, template: &R3Template<'a>) {
        self.visit_template_children(template);
    }

    /// Visit template children.
    fn visit_template_children(&mut self, template: &R3Template<'a>) {
        for child in &template.children {
            child.visit(self);
        }
    }

    /// Visit a content projection slot.
    fn visit_content(&mut self, content: &R3Content<'a>) {
        for child in &content.children {
            child.visit(self);
        }
    }

    /// Visit a variable.
    fn visit_variable(&mut self, _variable: &R3Variable<'a>) {}

    /// Visit a reference.
    fn visit_reference(&mut self, _reference: &R3Reference<'a>) {}

    /// Visit an ICU message.
    fn visit_icu(&mut self, _icu: &R3Icu<'a>) {}

    /// Visit a deferred block.
    fn visit_deferred_block(&mut self, block: &R3DeferredBlock<'a>) {
        for child in &block.children {
            child.visit(self);
        }
        if let Some(placeholder) = &block.placeholder {
            self.visit_deferred_block_placeholder(placeholder);
        }
        if let Some(loading) = &block.loading {
            self.visit_deferred_block_loading(loading);
        }
        if let Some(error) = &block.error {
            self.visit_deferred_block_error(error);
        }
    }

    /// Visit a deferred block placeholder.
    fn visit_deferred_block_placeholder(&mut self, block: &R3DeferredBlockPlaceholder<'a>) {
        for child in &block.children {
            child.visit(self);
        }
    }

    /// Visit a deferred block loading state.
    fn visit_deferred_block_loading(&mut self, block: &R3DeferredBlockLoading<'a>) {
        for child in &block.children {
            child.visit(self);
        }
    }

    /// Visit a deferred block error state.
    fn visit_deferred_block_error(&mut self, block: &R3DeferredBlockError<'a>) {
        for child in &block.children {
            child.visit(self);
        }
    }

    /// Visit a switch block.
    fn visit_switch_block(&mut self, block: &R3SwitchBlock<'a>) {
        for group in &block.groups {
            self.visit_switch_block_case_group(group);
        }
    }

    /// Visit a switch block case group.
    fn visit_switch_block_case_group(&mut self, group: &R3SwitchBlockCaseGroup<'a>) {
        for case in &group.cases {
            self.visit_switch_block_case(case);
        }
        for child in &group.children {
            child.visit(self);
        }
    }

    /// Visit a switch block case.
    fn visit_switch_block_case(&mut self, _case: &R3SwitchBlockCase<'a>) {}

    /// Visit a for loop block.
    fn visit_for_loop_block(&mut self, block: &R3ForLoopBlock<'a>) {
        for child in &block.children {
            child.visit(self);
        }
        if let Some(empty) = &block.empty {
            self.visit_for_loop_block_empty(empty);
        }
    }

    /// Visit a for loop empty block.
    fn visit_for_loop_block_empty(&mut self, block: &R3ForLoopBlockEmpty<'a>) {
        for child in &block.children {
            child.visit(self);
        }
    }

    /// Visit an if block.
    fn visit_if_block(&mut self, block: &R3IfBlock<'a>) {
        for branch in &block.branches {
            self.visit_if_block_branch(branch);
        }
    }

    /// Visit an if block branch.
    fn visit_if_block_branch(&mut self, branch: &R3IfBlockBranch<'a>) {
        for child in &branch.children {
            child.visit(self);
        }
    }

    /// Visit an unknown block.
    fn visit_unknown_block(&mut self, _block: &R3UnknownBlock<'a>) {}

    /// Visit a let declaration.
    fn visit_let_declaration(&mut self, _decl: &R3LetDeclaration<'a>) {}

    /// Visit a component.
    fn visit_component(&mut self, component: &R3Component<'a>) {
        for child in &component.children {
            child.visit(self);
        }
    }

    /// Visit a directive.
    fn visit_directive(&mut self, _directive: &R3Directive<'a>) {}
}

/// Visit all nodes in a slice.
pub fn visit_all<'a, V: R3Visitor<'a> + ?Sized>(visitor: &mut V, nodes: &[R3Node<'a>]) {
    for node in nodes {
        node.visit(visitor);
    }
}

// ============================================================================
// Parse Result
// ============================================================================

/// Result of parsing a template to R3 AST.
#[derive(Debug)]
pub struct R3ParseResult<'a> {
    /// The parsed R3 nodes.
    pub nodes: Vec<'a, R3Node<'a>>,
    /// Parse errors.
    /// Uses std::vec::Vec since ParseError contains Drop types (Arc, String).
    pub errors: std::vec::Vec<crate::util::ParseError>,
    /// Extracted styles.
    pub styles: Vec<'a, Atom<'a>>,
    /// Extracted style URLs.
    pub style_urls: Vec<'a, Atom<'a>>,
    /// Content projection selectors.
    pub ng_content_selectors: Vec<'a, Atom<'a>>,
    /// Comment nodes (if collected).
    pub comment_nodes: Option<Vec<'a, R3Comment<'a>>>,
}

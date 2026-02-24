//! I18n AST node definitions.
//!
//! Ported from Angular's `i18n/i18n_ast.ts`.

use indexmap::IndexMap;
use rustc_hash::FxHashMap;

use crate::i18n::serializer::format_i18n_placeholder_name;
use crate::util::ParseSourceSpan;

/// Describes the text contents of a placeholder as it appears in an ICU expression.
#[derive(Debug, Clone)]
pub struct MessagePlaceholder {
    /// The text contents of the placeholder.
    pub text: String,
    /// The source span of the placeholder.
    pub source_span: ParseSourceSpan,
}

/// Source location information for a message (1-based line/column).
#[derive(Debug, Clone)]
pub struct MessageSpan {
    /// File path.
    pub file_path: String,
    /// Start line (1-based).
    pub start_line: u32,
    /// Start column (1-based).
    pub start_col: u32,
    /// End line (1-based).
    pub end_line: u32,
    /// End column (1-based).
    pub end_col: u32,
}

/// An i18n message containing translatable content.
#[derive(Debug, Clone)]
pub struct Message {
    /// Message AST nodes.
    pub nodes: Vec<Node>,
    /// Maps placeholder names to static content and their source spans.
    pub placeholders: FxHashMap<String, MessagePlaceholder>,
    /// Maps placeholder names to messages (used for nested ICU messages).
    pub placeholder_to_message: FxHashMap<String, Message>,
    /// The meaning of the message (for disambiguation).
    pub meaning: String,
    /// Description of the message for translators.
    pub description: String,
    /// Custom ID specified by the developer.
    pub custom_id: String,
    /// The computed message ID.
    pub id: String,
    /// The serialized message string for $localize.
    pub message_string: String,
    /// Source location information.
    pub sources: Vec<MessageSpan>,
    /// Legacy IDs for backwards compatibility.
    pub legacy_ids: Vec<String>,
}

impl Message {
    /// Creates a new i18n message.
    pub fn new(
        nodes: Vec<Node>,
        placeholders: FxHashMap<String, MessagePlaceholder>,
        placeholder_to_message: FxHashMap<String, Message>,
        meaning: String,
        description: String,
        custom_id: String,
    ) -> Self {
        let id = custom_id.clone();
        let message_string = serialize_message(&nodes);

        let sources = if !nodes.is_empty() {
            let first_span = nodes.first().and_then(|n| n.source_span());
            let last_span = nodes.last().and_then(|n| n.source_span());
            if let (Some(first), Some(last)) = (first_span, last_span) {
                vec![MessageSpan {
                    file_path: first.start.file.url.to_string(),
                    start_line: first.start.line + 1,
                    start_col: first.start.col + 1,
                    end_line: last.end.line + 1,
                    // Match Angular's behavior (end col is derived from first start col).
                    end_col: first.start.col + 1,
                }]
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        Self {
            nodes,
            placeholders,
            placeholder_to_message,
            meaning,
            description,
            custom_id,
            id,
            message_string,
            sources,
            legacy_ids: Vec::new(),
        }
    }

    /// Returns the serialized message string for goog.getMsg and $localize.
    /// Contains the message text with placeholder markers like "{$interpolation}".
    pub fn serialize(&self) -> String {
        self.message_string.clone()
    }
}

/// I18n AST node.
#[derive(Debug, Clone)]
pub enum Node {
    /// Plain text content.
    Text(Text),
    /// Container for child nodes.
    Container(Container),
    /// ICU expression (plural, select, selectordinal).
    Icu(Icu),
    /// HTML tag placeholder.
    TagPlaceholder(TagPlaceholder),
    /// Expression placeholder.
    Placeholder(Placeholder),
    /// ICU placeholder (nested ICU reference).
    IcuPlaceholder(IcuPlaceholder),
    /// Control flow block placeholder.
    BlockPlaceholder(BlockPlaceholder),
}

impl Node {
    /// Returns the source span for this node.
    pub fn source_span(&self) -> Option<&ParseSourceSpan> {
        match self {
            Node::Text(n) => Some(&n.source_span),
            Node::Container(n) => Some(&n.source_span),
            Node::Icu(n) => Some(&n.source_span),
            Node::TagPlaceholder(n) => Some(&n.source_span),
            Node::Placeholder(n) => Some(&n.source_span),
            Node::IcuPlaceholder(n) => Some(&n.source_span),
            Node::BlockPlaceholder(n) => Some(&n.source_span),
        }
    }

    /// Accept a visitor.
    pub fn visit<V: Visitor>(&self, visitor: &mut V, context: &mut V::Context) -> V::Result {
        match self {
            Node::Text(n) => visitor.visit_text(n, context),
            Node::Container(n) => visitor.visit_container(n, context),
            Node::Icu(n) => visitor.visit_icu(n, context),
            Node::TagPlaceholder(n) => visitor.visit_tag_placeholder(n, context),
            Node::Placeholder(n) => visitor.visit_placeholder(n, context),
            Node::IcuPlaceholder(n) => visitor.visit_icu_placeholder(n, context),
            Node::BlockPlaceholder(n) => visitor.visit_block_placeholder(n, context),
        }
    }
}

/// Plain text content.
#[derive(Debug, Clone)]
pub struct Text {
    /// The text value.
    pub value: String,
    /// Source span.
    pub source_span: ParseSourceSpan,
}

impl Text {
    /// Creates a new text node.
    pub fn new(value: String, source_span: ParseSourceSpan) -> Self {
        Self { value, source_span }
    }
}

/// Container for child nodes.
#[derive(Debug, Clone)]
pub struct Container {
    /// Child nodes.
    pub children: Vec<Node>,
    /// Source span.
    pub source_span: ParseSourceSpan,
}

impl Container {
    /// Creates a new container.
    pub fn new(children: Vec<Node>, source_span: ParseSourceSpan) -> Self {
        Self { children, source_span }
    }
}

/// ICU expression (plural, select, selectordinal).
#[derive(Debug, Clone)]
pub struct Icu {
    /// The expression being evaluated.
    pub expression: String,
    /// ICU type string (plural, select, selectordinal, or custom).
    /// Stored as-is from the source for 1:1 parity with Angular.
    pub icu_type: String,
    /// Case branches (ordered for consistent serialization).
    pub cases: IndexMap<String, Node>,
    /// Source span.
    pub source_span: ParseSourceSpan,
    /// Expression placeholder name (for message serialization).
    pub expression_placeholder: Option<String>,
}

impl Icu {
    /// Creates a new ICU expression.
    pub fn new(
        expression: String,
        icu_type: String,
        cases: IndexMap<String, Node>,
        source_span: ParseSourceSpan,
        expression_placeholder: Option<String>,
    ) -> Self {
        Self { expression, icu_type, cases, source_span, expression_placeholder }
    }
}

/// HTML tag placeholder.
#[derive(Debug, Clone)]
pub struct TagPlaceholder {
    /// Tag name.
    pub tag: String,
    /// Tag attributes (ordered for consistent serialization).
    pub attrs: IndexMap<String, String>,
    /// Start tag placeholder name.
    pub start_name: String,
    /// Close tag placeholder name.
    pub close_name: String,
    /// Child nodes.
    pub children: Vec<Node>,
    /// Whether this is a void element.
    pub is_void: bool,
    /// Source span (overall).
    pub source_span: ParseSourceSpan,
    /// Start tag source span.
    pub start_source_span: Option<ParseSourceSpan>,
    /// End tag source span.
    pub end_source_span: Option<ParseSourceSpan>,
}

impl TagPlaceholder {
    /// Creates a new tag placeholder.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        tag: String,
        attrs: IndexMap<String, String>,
        start_name: String,
        close_name: String,
        children: Vec<Node>,
        is_void: bool,
        source_span: ParseSourceSpan,
        start_source_span: Option<ParseSourceSpan>,
        end_source_span: Option<ParseSourceSpan>,
    ) -> Self {
        Self {
            tag,
            attrs,
            start_name,
            close_name,
            children,
            is_void,
            source_span,
            start_source_span,
            end_source_span,
        }
    }
}

/// Expression placeholder.
#[derive(Debug, Clone)]
pub struct Placeholder {
    /// The expression value.
    pub value: String,
    /// Placeholder name.
    pub name: String,
    /// Source span.
    pub source_span: ParseSourceSpan,
}

impl Placeholder {
    /// Creates a new placeholder.
    pub fn new(value: String, name: String, source_span: ParseSourceSpan) -> Self {
        Self { value, name, source_span }
    }
}

/// ICU placeholder (reference to a nested ICU).
#[derive(Debug, Clone)]
pub struct IcuPlaceholder {
    /// The ICU expression.
    pub value: Box<Icu>,
    /// Placeholder name.
    pub name: String,
    /// Source span.
    pub source_span: ParseSourceSpan,
    /// Message computed from a previous processing pass.
    pub previous_message: Option<Box<Message>>,
}

impl IcuPlaceholder {
    /// Creates a new ICU placeholder.
    pub fn new(value: Icu, name: String, source_span: ParseSourceSpan) -> Self {
        Self { value: Box::new(value), name, source_span, previous_message: None }
    }
}

/// Control flow block placeholder (@if, @for, etc.).
#[derive(Debug, Clone)]
pub struct BlockPlaceholder {
    /// Block name (if, for, switch, etc.).
    pub name: String,
    /// Block parameters.
    pub parameters: Vec<String>,
    /// Start block placeholder name.
    pub start_name: String,
    /// Close block placeholder name.
    pub close_name: String,
    /// Child nodes.
    pub children: Vec<Node>,
    /// Source span (overall).
    pub source_span: ParseSourceSpan,
    /// Start block source span.
    pub start_source_span: Option<ParseSourceSpan>,
    /// End block source span.
    pub end_source_span: Option<ParseSourceSpan>,
}

impl BlockPlaceholder {
    /// Creates a new block placeholder.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        parameters: Vec<String>,
        start_name: String,
        close_name: String,
        children: Vec<Node>,
        source_span: ParseSourceSpan,
        start_source_span: Option<ParseSourceSpan>,
        end_source_span: Option<ParseSourceSpan>,
    ) -> Self {
        Self {
            name,
            parameters,
            start_name,
            close_name,
            children,
            source_span,
            start_source_span,
            end_source_span,
        }
    }
}

/// Each HTML node that is affected by an i18n tag will also have an `i18n` property
/// that is of type `I18nMeta`.
#[derive(Debug, Clone)]
pub enum I18nMeta {
    /// Root of an i18n message.
    Message(Message),
    /// Part of a containing message.
    Node(Node),
}

/// Visitor trait for i18n AST nodes.
pub trait Visitor {
    /// Context type passed through visitation.
    type Context;
    /// Result type returned from visit methods.
    type Result;

    /// Visit a text node.
    fn visit_text(&mut self, text: &Text, context: &mut Self::Context) -> Self::Result;

    /// Visit a container node.
    fn visit_container(
        &mut self,
        container: &Container,
        context: &mut Self::Context,
    ) -> Self::Result;

    /// Visit an ICU expression.
    fn visit_icu(&mut self, icu: &Icu, context: &mut Self::Context) -> Self::Result;

    /// Visit a tag placeholder.
    fn visit_tag_placeholder(
        &mut self,
        ph: &TagPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result;

    /// Visit a placeholder.
    fn visit_placeholder(&mut self, ph: &Placeholder, context: &mut Self::Context) -> Self::Result;

    /// Visit an ICU placeholder.
    fn visit_icu_placeholder(
        &mut self,
        ph: &IcuPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result;

    /// Visit a block placeholder.
    fn visit_block_placeholder(
        &mut self,
        ph: &BlockPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result;
}

// ============================================================================
// Serialization
// ============================================================================

/// Serialize the message to the $localize backtick string format.
fn serialize_message(nodes: &[Node]) -> String {
    let mut visitor = LocalizeMessageStringVisitor;
    let mut ctx = ();
    nodes.iter().map(|n| n.visit(&mut visitor, &mut ctx)).collect::<Vec<_>>().join("")
}

/// Visitor that serializes i18n nodes to $localize format.
struct LocalizeMessageStringVisitor;

impl Visitor for LocalizeMessageStringVisitor {
    type Context = ();
    type Result = String;

    fn visit_text(&mut self, text: &Text, _context: &mut Self::Context) -> Self::Result {
        text.value.clone()
    }

    fn visit_container(
        &mut self,
        container: &Container,
        context: &mut Self::Context,
    ) -> Self::Result {
        container
            .children
            .iter()
            .map(|child| child.visit(self, context))
            .collect::<Vec<_>>()
            .join("")
    }

    fn visit_icu(&mut self, icu: &Icu, context: &mut Self::Context) -> Self::Result {
        let cases: Vec<String> = icu
            .cases
            .iter()
            .map(|(k, v)| format!("{} {{{}}}", k, v.visit(self, context)))
            .collect();
        let expr_placeholder = icu.expression_placeholder.as_deref().unwrap_or(&icu.expression);
        format!("{{{}, {}, {}}}", expr_placeholder, icu.icu_type, cases.join(" "))
    }

    fn visit_tag_placeholder(
        &mut self,
        ph: &TagPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        let children: String =
            ph.children.iter().map(|child| child.visit(self, context)).collect::<Vec<_>>().join("");
        let start_name = format_i18n_placeholder_name(&ph.start_name, true);
        let close_name = format_i18n_placeholder_name(&ph.close_name, true);
        format!("{{${start_name}}}{children}{{${close_name}}}")
    }

    fn visit_placeholder(
        &mut self,
        ph: &Placeholder,
        _context: &mut Self::Context,
    ) -> Self::Result {
        let name = format_i18n_placeholder_name(&ph.name, true);
        format!("{{${name}}}")
    }

    fn visit_icu_placeholder(
        &mut self,
        ph: &IcuPlaceholder,
        _context: &mut Self::Context,
    ) -> Self::Result {
        let name = format_i18n_placeholder_name(&ph.name, true);
        format!("{{${name}}}")
    }

    fn visit_block_placeholder(
        &mut self,
        ph: &BlockPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        let children: String =
            ph.children.iter().map(|child| child.visit(self, context)).collect::<Vec<_>>().join("");
        let start_name = format_i18n_placeholder_name(&ph.start_name, true);
        let close_name = format_i18n_placeholder_name(&ph.close_name, true);
        format!("{{${start_name}}}{children}{{${close_name}}}")
    }
}

// ============================================================================
// Clone Visitor
// ============================================================================

/// Visitor that clones i18n AST nodes.
pub struct CloneVisitor;

impl Visitor for CloneVisitor {
    type Context = ();
    type Result = Node;

    fn visit_text(&mut self, text: &Text, _context: &mut Self::Context) -> Self::Result {
        Node::Text(Text::new(text.value.clone(), text.source_span.clone()))
    }

    fn visit_container(
        &mut self,
        container: &Container,
        context: &mut Self::Context,
    ) -> Self::Result {
        let children: Vec<Node> =
            container.children.iter().map(|n| n.visit(self, context)).collect();
        Node::Container(Container::new(children, container.source_span.clone()))
    }

    fn visit_icu(&mut self, icu: &Icu, context: &mut Self::Context) -> Self::Result {
        let cases: IndexMap<String, Node> =
            icu.cases.iter().map(|(k, v)| (k.clone(), v.visit(self, context))).collect();
        Node::Icu(Icu::new(
            icu.expression.clone(),
            icu.icu_type.clone(),
            cases,
            icu.source_span.clone(),
            icu.expression_placeholder.clone(),
        ))
    }

    fn visit_tag_placeholder(
        &mut self,
        ph: &TagPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        let children: Vec<Node> = ph.children.iter().map(|n| n.visit(self, context)).collect();
        Node::TagPlaceholder(TagPlaceholder::new(
            ph.tag.clone(),
            ph.attrs.clone(),
            ph.start_name.clone(),
            ph.close_name.clone(),
            children,
            ph.is_void,
            ph.source_span.clone(),
            ph.start_source_span.clone(),
            ph.end_source_span.clone(),
        ))
    }

    fn visit_placeholder(
        &mut self,
        ph: &Placeholder,
        _context: &mut Self::Context,
    ) -> Self::Result {
        Node::Placeholder(Placeholder::new(
            ph.value.clone(),
            ph.name.clone(),
            ph.source_span.clone(),
        ))
    }

    fn visit_icu_placeholder(
        &mut self,
        ph: &IcuPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        let icu_node = self.visit_icu(&ph.value, context);
        if let Node::Icu(icu) = icu_node {
            Node::IcuPlaceholder(IcuPlaceholder::new(icu, ph.name.clone(), ph.source_span.clone()))
        } else {
            // visit_icu should always return Node::Icu by design.
            // Return the original placeholder unchanged as a safe fallback.
            Node::IcuPlaceholder(ph.clone())
        }
    }

    fn visit_block_placeholder(
        &mut self,
        ph: &BlockPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        let children: Vec<Node> = ph.children.iter().map(|n| n.visit(self, context)).collect();
        Node::BlockPlaceholder(BlockPlaceholder::new(
            ph.name.clone(),
            ph.parameters.clone(),
            ph.start_name.clone(),
            ph.close_name.clone(),
            children,
            ph.source_span.clone(),
            ph.start_source_span.clone(),
            ph.end_source_span.clone(),
        ))
    }
}

// ============================================================================
// Recurse Visitor
// ============================================================================

/// Visitor that recursively visits all nodes.
pub struct RecurseVisitor;

impl Visitor for RecurseVisitor {
    type Context = ();
    type Result = ();

    fn visit_text(&mut self, _text: &Text, _context: &mut Self::Context) -> Self::Result {}

    fn visit_container(
        &mut self,
        container: &Container,
        context: &mut Self::Context,
    ) -> Self::Result {
        for child in &container.children {
            child.visit(self, context);
        }
    }

    fn visit_icu(&mut self, icu: &Icu, context: &mut Self::Context) -> Self::Result {
        for (_, case) in &icu.cases {
            case.visit(self, context);
        }
    }

    fn visit_tag_placeholder(
        &mut self,
        ph: &TagPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        for child in &ph.children {
            child.visit(self, context);
        }
    }

    fn visit_placeholder(
        &mut self,
        _ph: &Placeholder,
        _context: &mut Self::Context,
    ) -> Self::Result {
    }

    fn visit_icu_placeholder(
        &mut self,
        _ph: &IcuPlaceholder,
        _context: &mut Self::Context,
    ) -> Self::Result {
    }

    fn visit_block_placeholder(
        &mut self,
        ph: &BlockPlaceholder,
        context: &mut Self::Context,
    ) -> Self::Result {
        for child in &ph.children {
            child.visit(self, context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_serialization() {
        let nodes =
            vec![Node::Text(Text::new("Hello World".to_string(), ParseSourceSpan::default()))];
        let msg = Message::new(
            nodes,
            FxHashMap::default(),
            FxHashMap::default(),
            String::new(),
            String::new(),
            String::new(),
        );
        assert_eq!(msg.message_string, "Hello World");
    }

    #[test]
    fn test_placeholder_serialization() {
        let nodes = vec![
            Node::Text(Text::new("Hello ".to_string(), ParseSourceSpan::default())),
            Node::Placeholder(Placeholder::new(
                "name".to_string(),
                "INTERPOLATION".to_string(),
                ParseSourceSpan::default(),
            )),
            Node::Text(Text::new("!".to_string(), ParseSourceSpan::default())),
        ];
        let msg = Message::new(
            nodes,
            FxHashMap::default(),
            FxHashMap::default(),
            String::new(),
            String::new(),
            String::new(),
        );
        assert_eq!(msg.message_string, "Hello {$interpolation}!");
    }
}

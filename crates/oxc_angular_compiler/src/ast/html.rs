//! HTML AST nodes for Angular templates.
//!
//! This module contains AST node types for HTML templates,
//! ported from Angular's `ml_parser/ast.ts`.

use oxc_allocator::{Box, Vec};
use oxc_span::Span;
use oxc_str::Ident;

use super::expression::AngularExpression;

// ============================================================================
// Token Types (ported from Angular's ml_parser/tokens.ts)
// ============================================================================

/// Token types for text and attribute value tokens.
/// These are used to preserve the original token structure for transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpolatedTokenType {
    /// Plain text: parts = [text]
    Text,
    /// Interpolation: parts = [startMarker, expression, endMarker]
    Interpolation,
    /// Encoded entity: parts = [decoded, encoded]
    EncodedEntity,
}

/// A token within interpolated text or attribute values.
/// Preserves the original lexer tokens for transforms like whitespace removal.
#[derive(Debug)]
pub struct InterpolatedToken<'a> {
    /// The token type.
    pub token_type: InterpolatedTokenType,
    /// The token parts (structure depends on token type).
    pub parts: Vec<'a, Ident<'a>>,
    /// The source span.
    pub span: Span,
}

/// An HTML AST node.
#[derive(Debug)]
pub enum HtmlNode<'a> {
    /// A text node.
    Text(Box<'a, HtmlText<'a>>),
    /// An element node.
    Element(Box<'a, HtmlElement<'a>>),
    /// A selectorless component node.
    Component(Box<'a, HtmlComponent<'a>>),
    /// An attribute node.
    Attribute(Box<'a, HtmlAttribute<'a>>),
    /// A comment node.
    Comment(Box<'a, HtmlComment<'a>>),
    /// An ICU expansion.
    Expansion(Box<'a, HtmlExpansion<'a>>),
    /// An ICU expansion case.
    ExpansionCase(Box<'a, HtmlExpansionCase<'a>>),
    /// A block node (@if, @for, @switch, @defer).
    Block(Box<'a, HtmlBlock<'a>>),
    /// A block parameter.
    BlockParameter(Box<'a, HtmlBlockParameter<'a>>),
    /// A let declaration.
    LetDeclaration(Box<'a, HtmlLetDeclaration<'a>>),
}

impl<'a> HtmlNode<'a> {
    /// Returns the span of this node.
    pub fn span(&self) -> Span {
        match self {
            HtmlNode::Text(t) => t.span,
            HtmlNode::Element(e) => e.span,
            HtmlNode::Component(c) => c.span,
            HtmlNode::Attribute(a) => a.span,
            HtmlNode::Comment(c) => c.span,
            HtmlNode::Expansion(e) => e.span,
            HtmlNode::ExpansionCase(e) => e.span,
            HtmlNode::Block(b) => b.span,
            HtmlNode::BlockParameter(p) => p.span,
            HtmlNode::LetDeclaration(d) => d.span,
        }
    }
}

/// A text node in the HTML AST.
#[derive(Debug)]
pub struct HtmlText<'a> {
    /// The decoded text value (entities resolved, interpolations joined).
    pub value: Ident<'a>,
    /// The source span (after stripping leading trivia).
    pub span: Span,
    /// The full start offset before stripping leading trivia (for source maps).
    /// If None, there was no leading trivia stripped.
    pub full_start: Option<u32>,
    /// The original tokens (text, interpolation, encoded entity).
    /// Used by transforms like whitespace removal.
    pub tokens: Vec<'a, InterpolatedToken<'a>>,
}

/// An element node in the HTML AST.
#[derive(Debug)]
pub struct HtmlElement<'a> {
    /// The element tag name.
    /// For regular elements: the tag name (e.g., "div", ":svg:rect").
    /// For selectorless components: the component name (e.g., "MyComp").
    pub name: Ident<'a>,
    /// For selectorless components: the namespace prefix (e.g., "svg" in `<ng-component:MyComp:svg:rect>`).
    /// None for regular elements or selectorless components without namespace.
    pub component_prefix: Option<Ident<'a>>,
    /// For selectorless components: the HTML tag name (e.g., "rect" in `<ng-component:MyComp:svg:rect>`).
    /// None for regular elements or selectorless components without tag name.
    pub component_tag_name: Option<Ident<'a>>,
    /// The element attributes.
    pub attrs: Vec<'a, HtmlAttribute<'a>>,
    /// Selectorless directives (e.g., @Dir, @Dir(attr="value")).
    pub directives: Vec<'a, HtmlDirective<'a>>,
    /// The child nodes.
    pub children: Vec<'a, HtmlNode<'a>>,
    /// The source span.
    pub span: Span,
    /// The start tag source span.
    pub start_span: Span,
    /// The end tag source span (None for void/self-closing/incomplete elements).
    pub end_span: Option<Span>,
    /// Whether this element was explicitly self-closing (e.g., `<div/>`).
    pub is_self_closing: bool,
    /// Whether this is a void element (area, base, br, col, embed, hr, img, input, link, meta, param, source, track, wbr).
    /// Void elements cannot have content and do not have end tags.
    pub is_void: bool,
}

/// A selectorless component in the HTML AST.
///
/// This represents a component used with selectorless syntax: `<MyComp>` or `<MyComp:button>`.
/// This is a first-class node type that matches Angular's Component AST node.
#[derive(Debug)]
pub struct HtmlComponent<'a> {
    /// The component class name (e.g., "MyComp").
    pub component_name: Ident<'a>,
    /// The HTML tag name (e.g., "button" in `<MyComp:button>`).
    /// None for component-only syntax like `<MyComp>`.
    pub tag_name: Option<Ident<'a>>,
    /// The full qualified name (e.g., "MyComp:svg:rect").
    pub full_name: Ident<'a>,
    /// The element attributes.
    pub attrs: Vec<'a, HtmlAttribute<'a>>,
    /// Selectorless directives (e.g., @Dir).
    pub directives: Vec<'a, HtmlDirective<'a>>,
    /// The child nodes.
    pub children: Vec<'a, HtmlNode<'a>>,
    /// Whether this component was explicitly self-closing (e.g., `<MyComp/>`).
    pub is_self_closing: bool,
    /// The source span.
    pub span: Span,
    /// The start tag source span.
    pub start_span: Span,
    /// The end tag source span (None for self-closing/incomplete components).
    pub end_span: Option<Span>,
}

/// A selectorless directive in the HTML AST (e.g., @Dir or @Dir(attr="value")).
#[derive(Debug)]
pub struct HtmlDirective<'a> {
    /// The directive name (without the @ prefix).
    pub name: Ident<'a>,
    /// The directive attributes (inside parentheses, if any).
    pub attrs: Vec<'a, HtmlAttribute<'a>>,
    /// The source span for the entire directive.
    pub span: Span,
    /// The span for @DirectiveName.
    pub name_span: Span,
    /// The span for the opening paren (if present).
    pub start_paren_span: Option<Span>,
    /// The span for the closing paren (if present).
    pub end_paren_span: Option<Span>,
}

/// An attribute node in the HTML AST.
#[derive(Debug)]
pub struct HtmlAttribute<'a> {
    /// The attribute name.
    pub name: Ident<'a>,
    /// The decoded attribute value (entities resolved, interpolations joined).
    pub value: Ident<'a>,
    /// The source span.
    pub span: Span,
    /// The name span.
    pub name_span: Span,
    /// The value span (if value exists).
    pub value_span: Option<Span>,
    /// The original value tokens (text, interpolation, encoded entity).
    /// Used by transforms. None for valueless attributes.
    pub value_tokens: Option<Vec<'a, InterpolatedToken<'a>>>,
}

/// A comment node in the HTML AST.
#[derive(Debug)]
pub struct HtmlComment<'a> {
    /// The comment value.
    pub value: Ident<'a>,
    /// The source span.
    pub span: Span,
}

/// An ICU message expansion.
#[derive(Debug)]
pub struct HtmlExpansion<'a> {
    /// The switch value.
    pub switch_value: Ident<'a>,
    /// The expansion type (e.g., "plural", "select").
    pub expansion_type: Ident<'a>,
    /// The expansion cases.
    pub cases: Vec<'a, HtmlExpansionCase<'a>>,
    /// The source span.
    pub span: Span,
    /// The switch value source span.
    pub switch_value_span: Span,
    /// Whether this expansion is inside an i18n block.
    /// ICU expansions are only emitted to R3 AST when this is true.
    /// The full I18nMeta is populated during i18n processing phase.
    pub in_i18n_block: bool,
}

/// A case in an ICU expansion.
#[derive(Debug)]
pub struct HtmlExpansionCase<'a> {
    /// The case value (e.g., "one", "other").
    pub value: Ident<'a>,
    /// The expansion nodes.
    pub expansion: Vec<'a, HtmlNode<'a>>,
    /// The source span.
    pub span: Span,
    /// The value source span.
    pub value_span: Span,
    /// The expansion source span.
    pub expansion_span: Span,
}

/// The type of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// An @if block.
    If,
    /// An @else block.
    Else,
    /// An @else if block.
    ElseIf,
    /// A @for block.
    For,
    /// An @empty block (inside @for).
    Empty,
    /// A @switch block.
    Switch,
    /// A @case block.
    Case,
    /// A @default block.
    Default,
    /// A @defer block.
    Defer,
    /// A @placeholder block.
    Placeholder,
    /// A @loading block.
    Loading,
    /// An @error block.
    Error,
}

/// A block node (@if, @for, @switch, @defer).
#[derive(Debug)]
pub struct HtmlBlock<'a> {
    /// The block type.
    pub block_type: BlockType,
    /// The block name.
    pub name: Ident<'a>,
    /// The block parameters.
    pub parameters: Vec<'a, HtmlBlockParameter<'a>>,
    /// The child nodes.
    pub children: Vec<'a, HtmlNode<'a>>,
    /// The source span.
    pub span: Span,
    /// The name span.
    pub name_span: Span,
    /// The start source span.
    pub start_span: Span,
    /// The end source span.
    pub end_span: Option<Span>,
}

/// A parameter in a block.
///
/// Following Angular's approach, the expression is stored as raw text.
/// Higher-level transforms parse this string based on context
/// (e.g., timing parameters for @loading/@placeholder, conditions for @if).
#[derive(Debug)]
pub struct HtmlBlockParameter<'a> {
    /// The raw expression text (e.g., "minimum 500ms", "track item.id").
    pub expression: Ident<'a>,
    /// The source span.
    pub span: Span,
}

/// A @let declaration.
#[derive(Debug)]
pub struct HtmlLetDeclaration<'a> {
    /// The variable name.
    pub name: Ident<'a>,
    /// The value expression.
    pub value: AngularExpression<'a>,
    /// The source span.
    pub span: Span,
    /// The name span.
    pub name_span: Span,
    /// The value span.
    pub value_span: Span,
}

// ============================================================================
// Visitor Pattern
// ============================================================================

/// A visitor for HTML AST nodes.
///
/// This trait provides default implementations that do nothing.
/// Override the methods you're interested in to process specific node types.
///
/// # Example
/// ```ignore
/// struct MyVisitor;
///
/// impl<'a> Visitor<'a> for MyVisitor {
///     fn visit_element(&mut self, element: &HtmlElement<'a>) {
///         println!("Found element: {}", element.name);
///         // Call the default to continue traversal
///         visit_element(self, element);
///     }
/// }
/// ```
pub trait Visitor<'a>: Sized {
    /// Called when visiting a text node.
    fn visit_text(&mut self, _text: &HtmlText<'a>) {}

    /// Called when visiting an element node.
    fn visit_element(&mut self, element: &HtmlElement<'a>) {
        visit_element(self, element);
    }

    /// Called when visiting a component node.
    fn visit_component(&mut self, component: &HtmlComponent<'a>) {
        visit_component(self, component);
    }

    /// Called when visiting an attribute node.
    fn visit_attribute(&mut self, _attr: &HtmlAttribute<'a>) {}

    /// Called when visiting a comment node.
    fn visit_comment(&mut self, _comment: &HtmlComment<'a>) {}

    /// Called when visiting an expansion node.
    fn visit_expansion(&mut self, expansion: &HtmlExpansion<'a>) {
        visit_expansion(self, expansion);
    }

    /// Called when visiting an expansion case node.
    fn visit_expansion_case(&mut self, case: &HtmlExpansionCase<'a>) {
        visit_expansion_case(self, case);
    }

    /// Called when visiting a block node.
    fn visit_block(&mut self, block: &HtmlBlock<'a>) {
        visit_block(self, block);
    }

    /// Called when visiting a block parameter node.
    fn visit_block_parameter(&mut self, _param: &HtmlBlockParameter<'a>) {}

    /// Called when visiting a let declaration node.
    fn visit_let_declaration(&mut self, _decl: &HtmlLetDeclaration<'a>) {}

    /// Called when visiting any node. Dispatches to specific visit methods.
    fn visit_node(&mut self, node: &HtmlNode<'a>) {
        match node {
            HtmlNode::Text(text) => self.visit_text(text),
            HtmlNode::Element(element) => self.visit_element(element),
            HtmlNode::Component(component) => self.visit_component(component),
            HtmlNode::Attribute(attr) => self.visit_attribute(attr),
            HtmlNode::Comment(comment) => self.visit_comment(comment),
            HtmlNode::Expansion(expansion) => self.visit_expansion(expansion),
            HtmlNode::ExpansionCase(case) => self.visit_expansion_case(case),
            HtmlNode::Block(block) => self.visit_block(block),
            HtmlNode::BlockParameter(param) => self.visit_block_parameter(param),
            HtmlNode::LetDeclaration(decl) => self.visit_let_declaration(decl),
        }
    }
}

/// Visits all nodes in a slice.
pub fn visit_all<'a, V: Visitor<'a>>(visitor: &mut V, nodes: &[HtmlNode<'a>]) {
    for node in nodes {
        visitor.visit_node(node);
    }
}

/// Default traversal for an element node.
/// Visits attributes and children.
pub fn visit_element<'a, V: Visitor<'a>>(visitor: &mut V, element: &HtmlElement<'a>) {
    for attr in &element.attrs {
        visitor.visit_attribute(attr);
    }
    visit_all(visitor, &element.children);
}

/// Default traversal for a component node.
/// Visits attributes and children.
pub fn visit_component<'a, V: Visitor<'a>>(visitor: &mut V, component: &HtmlComponent<'a>) {
    for attr in &component.attrs {
        visitor.visit_attribute(attr);
    }
    visit_all(visitor, &component.children);
}

/// Default traversal for an expansion node.
/// Visits all cases.
pub fn visit_expansion<'a, V: Visitor<'a>>(visitor: &mut V, expansion: &HtmlExpansion<'a>) {
    for case in &expansion.cases {
        visitor.visit_expansion_case(case);
    }
}

/// Default traversal for an expansion case node.
/// Visits the expansion nodes.
pub fn visit_expansion_case<'a, V: Visitor<'a>>(visitor: &mut V, case: &HtmlExpansionCase<'a>) {
    visit_all(visitor, &case.expansion);
}

/// Default traversal for a block node.
/// Visits parameters and children.
pub fn visit_block<'a, V: Visitor<'a>>(visitor: &mut V, block: &HtmlBlock<'a>) {
    for param in &block.parameters {
        visitor.visit_block_parameter(param);
    }
    visit_all(visitor, &block.children);
}

/// A recursive visitor that traverses all nodes in the tree.
///
/// This provides a convenient base for visitors that need to traverse
/// the entire tree but only care about specific node types.
pub struct RecursiveVisitor<F> {
    callback: F,
}

impl<F> RecursiveVisitor<F> {
    /// Creates a new recursive visitor with the given callback.
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<'a, F> Visitor<'a> for RecursiveVisitor<F>
where
    F: FnMut(&HtmlNode<'a>),
{
    fn visit_node(&mut self, node: &HtmlNode<'a>) {
        (self.callback)(node);
        // Continue traversal
        match node {
            HtmlNode::Text(text) => self.visit_text(text),
            HtmlNode::Element(element) => self.visit_element(element),
            HtmlNode::Component(component) => self.visit_component(component),
            HtmlNode::Attribute(attr) => self.visit_attribute(attr),
            HtmlNode::Comment(comment) => self.visit_comment(comment),
            HtmlNode::Expansion(expansion) => self.visit_expansion(expansion),
            HtmlNode::ExpansionCase(case) => self.visit_expansion_case(case),
            HtmlNode::Block(block) => self.visit_block(block),
            HtmlNode::BlockParameter(param) => self.visit_block_parameter(param),
            HtmlNode::LetDeclaration(decl) => self.visit_let_declaration(decl),
        }
    }
}

/// Traverses all nodes in the tree, calling the callback for each node.
pub fn traverse_all<'a, F>(nodes: &[HtmlNode<'a>], mut callback: F)
where
    F: FnMut(&HtmlNode<'a>),
{
    let mut visitor = RecursiveVisitor::new(&mut callback);
    visit_all(&mut visitor, nodes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;

    struct ElementCounter {
        count: usize,
    }

    impl<'a> Visitor<'a> for ElementCounter {
        fn visit_element(&mut self, element: &HtmlElement<'a>) {
            self.count += 1;
            visit_element(self, element);
        }
    }

    #[test]
    fn test_visitor_counts_elements() {
        let allocator = Allocator::default();

        // Create a simple tree: root element with two child elements
        let child1 = HtmlElement {
            name: Ident::from("span"),
            component_prefix: None,
            component_tag_name: None,
            attrs: Vec::new_in(&allocator),
            directives: Vec::new_in(&allocator),
            children: Vec::new_in(&allocator),
            span: Span::default(),
            start_span: Span::default(),
            end_span: None,
            is_self_closing: false,
            is_void: false,
        };

        let child2 = HtmlElement {
            name: Ident::from("p"),
            component_prefix: None,
            component_tag_name: None,
            attrs: Vec::new_in(&allocator),
            directives: Vec::new_in(&allocator),
            children: Vec::new_in(&allocator),
            span: Span::default(),
            start_span: Span::default(),
            end_span: None,
            is_self_closing: false,
            is_void: false,
        };

        let mut children = Vec::new_in(&allocator);
        children.push(HtmlNode::Element(Box::new_in(child1, &allocator)));
        children.push(HtmlNode::Element(Box::new_in(child2, &allocator)));

        let root = HtmlElement {
            name: Ident::from("div"),
            component_prefix: None,
            component_tag_name: None,
            attrs: Vec::new_in(&allocator),
            directives: Vec::new_in(&allocator),
            children,
            span: Span::default(),
            start_span: Span::default(),
            end_span: None,
            is_self_closing: false,
            is_void: false,
        };

        let mut nodes = Vec::new_in(&allocator);
        nodes.push(HtmlNode::Element(Box::new_in(root, &allocator)));

        let mut counter = ElementCounter { count: 0 };
        visit_all(&mut counter, &nodes);

        assert_eq!(counter.count, 3); // root + 2 children
    }

    #[test]
    fn test_traverse_all() {
        let allocator = Allocator::default();

        let text = HtmlText {
            value: Ident::from("Hello"),
            span: Span::default(),
            full_start: None,
            tokens: Vec::new_in(&allocator),
        };

        let mut nodes = Vec::new_in(&allocator);
        nodes.push(HtmlNode::Text(Box::new_in(text, &allocator)));

        let mut visited = 0;
        traverse_all(&nodes, |_node| {
            visited += 1;
        });

        assert_eq!(visited, 1);
    }
}

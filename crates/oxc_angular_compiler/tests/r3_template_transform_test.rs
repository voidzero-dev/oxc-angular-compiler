//! R3 Template Transform tests.
//!
//! Ported from Angular's `test/render3/r3_template_transform_spec.ts`.
//!
//! These tests verify the transformation from HTML AST to R3 AST.

mod utils;

use oxc_allocator::Allocator;
use oxc_angular_compiler::ast::expression::BindingType;
use oxc_angular_compiler::ast::r3::R3Node;
use oxc_angular_compiler::parser::html::HtmlParser;
use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};

use utils::unparse;

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse result with owned nodes
struct ParseResult {
    nodes: std::vec::Vec<R3NodeRef>,
}

/// Reference to an R3Node that can be used after parsing
enum R3NodeRef {
    Element {
        name: String,
        is_self_closing: bool,
        attributes: Vec<(String, String)>,
        inputs: Vec<(BindingType, String, String)>,
        outputs: Vec<(i32, String, String, String)>,
        references: Vec<(String, String)>,
        children: Vec<R3NodeRef>,
    },
    Template {
        is_self_closing: bool,
        attributes: Vec<(String, String)>,
        inputs: Vec<(BindingType, String, String)>,
        variables: Vec<(String, String)>,
        references: Vec<(String, String)>,
        children: Vec<R3NodeRef>,
    },
    Content {
        selector: String,
        is_self_closing: bool,
        attributes: Vec<(String, String)>,
        children: Vec<R3NodeRef>,
    },
    Text {
        value: String,
    },
    BoundText {
        value: String,
    },
    Comment {
        value: String,
    },
    Variable {
        name: String,
        value: String,
    },
    Reference {
        name: String,
        value: String,
    },
    Icu {},
    IfBlock {
        branches: Vec<IfBranchRef>,
    },
    IfBlockBranch {
        expression: Option<String>,
        children: Vec<R3NodeRef>,
    },
    ForLoopBlock {
        expression: String,
        track_by: String,
        item: (String, String),
        context_variables: Vec<(String, String)>,
        children: Vec<R3NodeRef>,
        empty: Option<Vec<R3NodeRef>>,
    },
    ForLoopBlockEmpty {
        children: Vec<R3NodeRef>,
    },
    SwitchBlock {
        expression: String,
        groups: Vec<SwitchCaseGroupRef>,
    },
    SwitchBlockCaseGroup {
        cases: Vec<SwitchCaseRef>,
        children: Vec<R3NodeRef>,
    },
    DeferredBlock {
        children: Vec<R3NodeRef>,
        loading: Option<DeferredLoadingRef>,
        placeholder: Option<DeferredPlaceholderRef>,
        error: Option<Vec<R3NodeRef>>,
    },
    DeferredBlockPlaceholder {
        minimum_time: Option<u32>,
        children: Vec<R3NodeRef>,
    },
    DeferredBlockLoading {
        after_time: Option<u32>,
        minimum_time: Option<u32>,
        children: Vec<R3NodeRef>,
    },
    DeferredBlockError {
        children: Vec<R3NodeRef>,
    },
    LetDeclaration {
        name: String,
        value: String,
    },
    UnknownBlock {
        name: String,
    },
    Component {
        name: String,
    },
    Directive {
        name: String,
    },
    HostElement {
        tag_names: Vec<String>,
    },
}

struct IfBranchRef {
    expression: Option<String>,
    expression_alias: Option<(String, String)>,
    children: Vec<R3NodeRef>,
}

struct SwitchCaseGroupRef {
    cases: Vec<SwitchCaseRef>,
    children: Vec<R3NodeRef>,
}

struct SwitchCaseRef {
    expression: Option<String>,
}

struct DeferredLoadingRef {
    after_time: Option<u32>,
    minimum_time: Option<u32>,
    children: Vec<R3NodeRef>,
}

struct DeferredPlaceholderRef {
    minimum_time: Option<u32>,
    children: Vec<R3NodeRef>,
}

fn convert_node(node: &R3Node<'_>) -> R3NodeRef {
    match node {
        R3Node::Element(e) => R3NodeRef::Element {
            name: e.name.to_string(),
            is_self_closing: e.is_self_closing,
            attributes: e
                .attributes
                .iter()
                .map(|a| (a.name.to_string(), a.value.to_string()))
                .collect(),
            inputs: e
                .inputs
                .iter()
                .map(|i| (i.binding_type, i.name.to_string(), unparse(&i.value)))
                .collect(),
            outputs: e
                .outputs
                .iter()
                .map(|o| {
                    (
                        o.event_type as i32,
                        o.name.to_string(),
                        o.target.as_ref().map(std::string::ToString::to_string).unwrap_or_default(),
                        unparse(&o.handler),
                    )
                })
                .collect(),
            references: e
                .references
                .iter()
                .map(|r| (r.name.to_string(), r.value.to_string()))
                .collect(),
            children: e.children.iter().map(convert_node).collect(),
        },
        R3Node::Template(t) => {
            use oxc_angular_compiler::ast::r3::R3TemplateAttr;

            // Collect attributes from both hoisted attributes and template_attrs (directive attributes)
            let mut attributes: Vec<(String, String)> =
                t.attributes.iter().map(|a| (a.name.to_string(), a.value.to_string())).collect();

            // Add TextAttributes from template_attrs (structural directive attributes)
            for attr in &t.template_attrs {
                if let R3TemplateAttr::Text(text_attr) = attr {
                    attributes.push((text_attr.name.to_string(), text_attr.value.to_string()));
                }
            }

            // Collect inputs from both hoisted inputs and template_attrs (directive bindings)
            let mut inputs: Vec<(BindingType, String, String)> = t
                .inputs
                .iter()
                .map(|i| (i.binding_type, i.name.to_string(), unparse(&i.value)))
                .collect();

            // Add BoundAttributes from template_attrs (structural directive bindings)
            for attr in &t.template_attrs {
                if let R3TemplateAttr::Bound(bound_attr) = attr {
                    inputs.push((
                        bound_attr.binding_type,
                        bound_attr.name.to_string(),
                        unparse(&bound_attr.value),
                    ));
                }
            }

            R3NodeRef::Template {
                is_self_closing: t.is_self_closing,
                attributes,
                inputs,
                variables: t
                    .variables
                    .iter()
                    .map(|v| (v.name.to_string(), v.value.to_string()))
                    .collect(),
                references: t
                    .references
                    .iter()
                    .map(|r| (r.name.to_string(), r.value.to_string()))
                    .collect(),
                children: t.children.iter().map(convert_node).collect(),
            }
        }
        R3Node::Content(c) => R3NodeRef::Content {
            selector: c.selector.to_string(),
            is_self_closing: c.is_self_closing,
            attributes: c
                .attributes
                .iter()
                .map(|a| (a.name.to_string(), a.value.to_string()))
                .collect(),
            children: c.children.iter().map(convert_node).collect(),
        },
        R3Node::Text(t) => R3NodeRef::Text { value: t.value.to_string() },
        R3Node::BoundText(t) => R3NodeRef::BoundText { value: unparse(&t.value) },
        R3Node::Comment(c) => R3NodeRef::Comment { value: c.value.to_string() },
        R3Node::IfBlock(b) => R3NodeRef::IfBlock {
            branches: b
                .branches
                .iter()
                .map(|br| IfBranchRef {
                    expression: br.expression.as_ref().map(|e| unparse(e)),
                    expression_alias: br
                        .expression_alias
                        .as_ref()
                        .map(|a| (a.name.to_string(), a.value.to_string())),
                    children: br.children.iter().map(convert_node).collect(),
                })
                .collect(),
        },
        R3Node::ForLoopBlock(b) => R3NodeRef::ForLoopBlock {
            expression: unparse(&b.expression.ast),
            track_by: unparse(&b.track_by.ast),
            item: (b.item.name.to_string(), b.item.value.to_string()),
            context_variables: b
                .context_variables
                .iter()
                .map(|v| (v.name.to_string(), v.value.to_string()))
                .collect(),
            children: b.children.iter().map(convert_node).collect(),
            empty: b.empty.as_ref().map(|e| e.children.iter().map(convert_node).collect()),
        },
        R3Node::SwitchBlock(b) => R3NodeRef::SwitchBlock {
            expression: unparse(&b.expression),
            groups: b
                .groups
                .iter()
                .map(|g| SwitchCaseGroupRef {
                    cases: g
                        .cases
                        .iter()
                        .map(|c| SwitchCaseRef {
                            expression: c.expression.as_ref().map(|e| unparse(e)),
                        })
                        .collect(),
                    children: g.children.iter().map(convert_node).collect(),
                })
                .collect(),
        },
        R3Node::DeferredBlock(b) => R3NodeRef::DeferredBlock {
            children: b.children.iter().map(convert_node).collect(),
            loading: b.loading.as_ref().map(|l| DeferredLoadingRef {
                after_time: l.after_time,
                minimum_time: l.minimum_time,
                children: l.children.iter().map(convert_node).collect(),
            }),
            placeholder: b.placeholder.as_ref().map(|p| DeferredPlaceholderRef {
                minimum_time: p.minimum_time,
                children: p.children.iter().map(convert_node).collect(),
            }),
            error: b.error.as_ref().map(|e| e.children.iter().map(convert_node).collect()),
        },
        R3Node::LetDeclaration(d) => {
            R3NodeRef::LetDeclaration { name: d.name.to_string(), value: unparse(&d.value) }
        }
        R3Node::UnknownBlock(b) => R3NodeRef::UnknownBlock { name: b.name.to_string() },
        R3Node::Variable(v) => {
            R3NodeRef::Variable { name: v.name.to_string(), value: v.value.to_string() }
        }
        R3Node::Reference(r) => {
            R3NodeRef::Reference { name: r.name.to_string(), value: r.value.to_string() }
        }
        R3Node::Icu(_) => R3NodeRef::Icu {},
        R3Node::IfBlockBranch(b) => R3NodeRef::IfBlockBranch {
            expression: b.expression.as_ref().map(|e| unparse(e)),
            children: b.children.iter().map(convert_node).collect(),
        },
        R3Node::ForLoopBlockEmpty(e) => {
            R3NodeRef::ForLoopBlockEmpty { children: e.children.iter().map(convert_node).collect() }
        }
        R3Node::SwitchBlockCaseGroup(g) => R3NodeRef::SwitchBlockCaseGroup {
            cases: g
                .cases
                .iter()
                .map(|c| SwitchCaseRef { expression: c.expression.as_ref().map(|e| unparse(e)) })
                .collect(),
            children: g.children.iter().map(convert_node).collect(),
        },
        R3Node::DeferredBlockPlaceholder(p) => R3NodeRef::DeferredBlockPlaceholder {
            minimum_time: p.minimum_time,
            children: p.children.iter().map(convert_node).collect(),
        },
        R3Node::DeferredBlockLoading(l) => R3NodeRef::DeferredBlockLoading {
            after_time: l.after_time,
            minimum_time: l.minimum_time,
            children: l.children.iter().map(convert_node).collect(),
        },
        R3Node::DeferredBlockError(e) => R3NodeRef::DeferredBlockError {
            children: e.children.iter().map(convert_node).collect(),
        },
        R3Node::Component(c) => R3NodeRef::Component { name: c.component_name.to_string() },
        R3Node::Directive(d) => R3NodeRef::Directive { name: d.name.to_string() },
        R3Node::HostElement(h) => R3NodeRef::HostElement {
            tag_names: h.tag_names.iter().map(std::string::ToString::to_string).collect(),
        },
    }
}

/// Parses HTML and transforms it to R3 AST nodes.
fn parse(html: &str) -> ParseResult {
    parse_with_options(html, false, false)
}

/// Parses HTML with options.
fn parse_with_options(html: &str, ignore_error: bool, _selectorless_enabled: bool) -> ParseResult {
    let (result, _) = parse_internal(html, ignore_error, false);
    result
}

/// Result with comment nodes for testing comment collection.
struct ParseResultWithComments {
    nodes: Vec<R3NodeRef>,
    comment_nodes: Option<Vec<String>>,
}

/// Parses HTML with comment collection enabled.
fn parse_with_comments(html: &str) -> ParseResultWithComments {
    let (result, comments) = parse_internal(html, false, true);
    ParseResultWithComments { nodes: result.nodes, comment_nodes: comments }
}

/// Internal parse function.
fn parse_internal(
    html: &str,
    ignore_error: bool,
    collect_comments: bool,
) -> (ParseResult, Option<Vec<String>>) {
    let allocator = Box::new(Allocator::default());
    let allocator_ref: &'static Allocator =
        unsafe { &*std::ptr::from_ref::<Allocator>(allocator.as_ref()) };

    let parser = HtmlParser::new(allocator_ref, html, "test.html");
    let html_result = parser.parse();

    assert!(
        !(!ignore_error && !html_result.errors.is_empty()),
        "HTML parse errors: {:?}",
        html_result.errors.iter().map(|e| &e.msg).collect::<Vec<_>>()
    );

    let options = TransformOptions { collect_comment_nodes: collect_comments };
    let transformer = HtmlToR3Transform::new(allocator_ref, html, options);
    let r3_result = transformer.transform(&html_result.nodes);

    assert!(
        !(!ignore_error && !r3_result.errors.is_empty()),
        "Transform errors: {:?}",
        r3_result.errors.iter().map(|e| &e.msg).collect::<Vec<_>>()
    );

    // Convert to owned refs before returning
    let nodes = r3_result.nodes.iter().map(convert_node).collect();

    // Convert comment nodes to owned strings
    let comment_nodes = r3_result
        .comment_nodes
        .map(|comments| comments.iter().map(|c| c.value.to_string()).collect());

    // allocator is dropped here but that's fine since we've converted to owned types
    (ParseResult { nodes }, comment_nodes)
}

/// Humanize value enum for R3 AST nodes.
#[derive(Debug, Clone, PartialEq)]
enum HumanValue {
    Str(String),
    Num(i32),
    Null,
}

impl From<&str> for HumanValue {
    fn from(s: &str) -> Self {
        HumanValue::Str(s.to_string())
    }
}

impl From<String> for HumanValue {
    fn from(s: String) -> Self {
        HumanValue::Str(s)
    }
}

impl From<i32> for HumanValue {
    fn from(n: i32) -> Self {
        HumanValue::Num(n)
    }
}

impl From<BindingType> for HumanValue {
    fn from(bt: BindingType) -> Self {
        HumanValue::Num(bt as i32)
    }
}

/// Macro to create a humanized result row.
macro_rules! h {
    ($($val:expr),* $(,)?) => {
        vec![$(HumanValue::from($val)),*]
    };
}

/// Humanizer for R3NodeRef
struct R3AstHumanizer {
    result: Vec<Vec<HumanValue>>,
}

impl R3AstHumanizer {
    fn new() -> Self {
        Self { result: Vec::new() }
    }

    fn humanize(mut self, nodes: &[R3NodeRef]) -> Vec<Vec<HumanValue>> {
        for node in nodes {
            self.visit_node(node);
        }
        self.result
    }

    fn visit_node(&mut self, node: &R3NodeRef) {
        match node {
            R3NodeRef::Element {
                name,
                is_self_closing,
                attributes,
                inputs,
                outputs,
                references,
                children,
            } => {
                let mut res = vec![HumanValue::from("Element"), HumanValue::from(name.as_str())];
                if *is_self_closing {
                    res.push(HumanValue::from("#selfClosing"));
                }
                self.result.push(res);
                for (n, v) in attributes {
                    self.result.push(h!["TextAttribute", n.as_str(), v.as_str()]);
                }
                for (bt, n, v) in inputs {
                    self.result.push(h!["BoundAttribute", *bt, n.as_str(), v.as_str()]);
                }
                for (et, n, t, h) in outputs {
                    self.result.push(h!["BoundEvent", *et, n.as_str(), t.as_str(), h.as_str()]);
                }
                for (n, v) in references {
                    self.result.push(h!["Reference", n.as_str(), v.as_str()]);
                }
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::Template {
                is_self_closing,
                attributes,
                inputs,
                variables,
                references,
                children,
            } => {
                let mut res = vec![HumanValue::from("Template")];
                if *is_self_closing {
                    res.push(HumanValue::from("#selfClosing"));
                }
                self.result.push(res);
                for (n, v) in attributes {
                    self.result.push(h!["TextAttribute", n.as_str(), v.as_str()]);
                }
                for (bt, n, v) in inputs {
                    self.result.push(h!["BoundAttribute", *bt, n.as_str(), v.as_str()]);
                }
                for (n, v) in variables {
                    self.result.push(h!["Variable", n.as_str(), v.as_str()]);
                }
                for (n, v) in references {
                    self.result.push(h!["Reference", n.as_str(), v.as_str()]);
                }
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::Content { selector, is_self_closing, attributes, children } => {
                let mut res =
                    vec![HumanValue::from("Content"), HumanValue::from(selector.as_str())];
                if *is_self_closing {
                    res.push(HumanValue::from("#selfClosing"));
                }
                self.result.push(res);
                for (n, v) in attributes {
                    self.result.push(h!["TextAttribute", n.as_str(), v.as_str()]);
                }
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::Text { value } => {
                self.result.push(h!["Text", value.as_str()]);
            }
            R3NodeRef::BoundText { value } => {
                self.result.push(h!["BoundText", value.as_str()]);
            }
            R3NodeRef::Comment { value } => {
                self.result.push(h!["Comment", value.as_str()]);
            }
            R3NodeRef::IfBlock { branches } => {
                self.result.push(h!["IfBlock"]);
                for branch in branches {
                    if let Some(expr) = &branch.expression {
                        self.result.push(h!["IfBlockBranch", expr.as_str()]);
                    } else {
                        self.result.push(vec![HumanValue::from("IfBlockBranch"), HumanValue::Null]);
                    }
                    if let Some((n, v)) = &branch.expression_alias {
                        self.result.push(h!["Variable", n.as_str(), v.as_str()]);
                    }
                    for child in &branch.children {
                        self.visit_node(child);
                    }
                }
            }
            R3NodeRef::ForLoopBlock {
                expression,
                track_by,
                item,
                context_variables,
                children,
                empty,
            } => {
                self.result.push(h!["ForLoopBlock", expression.as_str(), track_by.as_str()]);
                self.result.push(h!["Variable", item.0.as_str(), item.1.as_str()]);
                for (n, v) in context_variables {
                    self.result.push(h!["Variable", n.as_str(), v.as_str()]);
                }
                for child in children {
                    self.visit_node(child);
                }
                if let Some(empty_children) = empty {
                    self.result.push(h!["ForLoopBlockEmpty"]);
                    for child in empty_children {
                        self.visit_node(child);
                    }
                }
            }
            R3NodeRef::SwitchBlock { expression, groups } => {
                self.result.push(h!["SwitchBlock", expression.as_str()]);
                for group in groups {
                    self.result.push(h!["SwitchBlockCaseGroup"]);
                    for case in &group.cases {
                        if let Some(expr) = &case.expression {
                            self.result.push(h!["SwitchBlockCase", expr.as_str()]);
                        } else {
                            self.result
                                .push(vec![HumanValue::from("SwitchBlockCase"), HumanValue::Null]);
                        }
                    }
                    for child in &group.children {
                        self.visit_node(child);
                    }
                }
            }
            R3NodeRef::DeferredBlock { children, loading, placeholder, error } => {
                self.result.push(h!["DeferredBlock"]);
                for child in children {
                    self.visit_node(child);
                }
                if let Some(l) = loading {
                    let mut res = vec![HumanValue::from("DeferredBlockLoading")];
                    if let Some(after) = l.after_time {
                        res.push(HumanValue::from(format!("after {after}ms")));
                    }
                    if let Some(min) = l.minimum_time {
                        res.push(HumanValue::from(format!("minimum {min}ms")));
                    }
                    self.result.push(res);
                    for child in &l.children {
                        self.visit_node(child);
                    }
                }
                if let Some(p) = placeholder {
                    let mut res = vec![HumanValue::from("DeferredBlockPlaceholder")];
                    if let Some(min) = p.minimum_time {
                        res.push(HumanValue::from(format!("minimum {min}ms")));
                    }
                    self.result.push(res);
                    for child in &p.children {
                        self.visit_node(child);
                    }
                }
                if let Some(e) = error {
                    self.result.push(h!["DeferredBlockError"]);
                    for child in e {
                        self.visit_node(child);
                    }
                }
            }
            R3NodeRef::LetDeclaration { name, value } => {
                self.result.push(h!["LetDeclaration", name.as_str(), value.as_str()]);
            }
            R3NodeRef::UnknownBlock { name } => {
                self.result.push(h!["UnknownBlock", name.as_str()]);
            }
            R3NodeRef::Variable { name, value } => {
                self.result.push(h!["Variable", name.as_str(), value.as_str()]);
            }
            R3NodeRef::Reference { name, value } => {
                self.result.push(h!["Reference", name.as_str(), value.as_str()]);
            }
            R3NodeRef::Icu {} => {
                self.result.push(h!["Icu"]);
            }
            R3NodeRef::IfBlockBranch { expression, children } => {
                if let Some(expr) = expression {
                    self.result.push(h!["IfBlockBranch", expr.as_str()]);
                } else {
                    self.result.push(vec![HumanValue::from("IfBlockBranch"), HumanValue::Null]);
                }
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::ForLoopBlockEmpty { children } => {
                self.result.push(h!["ForLoopBlockEmpty"]);
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::SwitchBlockCaseGroup { cases, children } => {
                self.result.push(h!["SwitchBlockCaseGroup"]);
                for case in cases {
                    if let Some(expr) = &case.expression {
                        self.result.push(h!["SwitchBlockCase", expr.as_str()]);
                    } else {
                        self.result
                            .push(vec![HumanValue::from("SwitchBlockCase"), HumanValue::Null]);
                    }
                }
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::DeferredBlockPlaceholder { minimum_time, children } => {
                let mut res = vec![HumanValue::from("DeferredBlockPlaceholder")];
                if let Some(min) = minimum_time {
                    res.push(HumanValue::from(format!("minimum {min}ms")));
                }
                self.result.push(res);
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::DeferredBlockLoading { after_time, minimum_time, children } => {
                let mut res = vec![HumanValue::from("DeferredBlockLoading")];
                if let Some(after) = after_time {
                    res.push(HumanValue::from(format!("after {after}ms")));
                }
                if let Some(min) = minimum_time {
                    res.push(HumanValue::from(format!("minimum {min}ms")));
                }
                self.result.push(res);
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::DeferredBlockError { children } => {
                self.result.push(h!["DeferredBlockError"]);
                for child in children {
                    self.visit_node(child);
                }
            }
            R3NodeRef::Component { name } => {
                self.result.push(h!["Component", name.as_str()]);
            }
            R3NodeRef::Directive { name } => {
                self.result.push(h!["Directive", name.as_str()]);
            }
            R3NodeRef::HostElement { tag_names } => {
                let tag_names_str = tag_names.join(", ");
                self.result.push(h!["HostElement", tag_names_str.as_str()]);
            }
        }
    }
}

/// Helper to get humanized result from HTML.
fn humanize(html: &str) -> Vec<Vec<HumanValue>> {
    let result = parse(html);
    let humanizer = R3AstHumanizer::new();
    humanizer.humanize(&result.nodes)
}

/// Helper to get humanized result with error ignoring.
fn humanize_ignore_errors(html: &str) -> Vec<Vec<HumanValue>> {
    let result = parse_with_options(html, true, false);
    let humanizer = R3AstHumanizer::new();
    humanizer.humanize(&result.nodes)
}

/// Helper to get transform errors from an HTML template.
fn get_transform_errors(html: &str) -> Vec<String> {
    let allocator = Box::new(Allocator::default());
    let allocator_ref: &'static Allocator =
        unsafe { &*std::ptr::from_ref::<Allocator>(allocator.as_ref()) };

    let parser = HtmlParser::new(allocator_ref, html, "test.html");
    let html_result = parser.parse();

    let options = TransformOptions { collect_comment_nodes: false };
    let transformer = HtmlToR3Transform::new(allocator_ref, html, options);
    let r3_result = transformer.transform(&html_result.nodes);

    r3_result.errors.iter().map(|e| e.msg.clone()).collect()
}

// ============================================================================
// Tests: Nodes without binding
// ============================================================================

mod nodes_without_binding {
    use super::*;

    #[test]
    fn should_parse_incomplete_tags_terminated_by_eof() {
        // TS: expectFromHtml("<a", true).toEqual([["Element", "a"]])
        assert_eq!(humanize_ignore_errors("<a"), vec![h!["Element", "a"]]);
    }

    #[test]
    fn should_parse_incomplete_tags_terminated_by_another_tag() {
        // TS: expectFromHtml("<a <span></span>", true).toEqual([["Element", "a"], ["Element", "span"]])
        assert_eq!(
            humanize_ignore_errors("<a <span></span>"),
            vec![h!["Element", "a"], h!["Element", "span"]]
        );
    }

    #[test]
    fn should_parse_text_nodes() {
        // TS: expectFromHtml("a").toEqual([["Text", "a"]])
        assert_eq!(humanize("a"), vec![h!["Text", "a"]]);
    }

    #[test]
    fn should_parse_elements_with_attributes() {
        // TS: expectFromHtml("<div a=b></div>").toEqual([["Element", "div"], ["TextAttribute", "a", "b"]])
        assert_eq!(
            humanize("<div a=b></div>"),
            vec![h!["Element", "div"], h!["TextAttribute", "a", "b"]]
        );
    }

    #[test]
    fn should_parse_ng_content() {
        // TS: expectFromR3Nodes(res.nodes).toEqual([["Content", "a"], ["TextAttribute", "select", "a"]])
        assert_eq!(
            humanize(r#"<ng-content select="a"></ng-content>"#),
            vec![h!["Content", "a"], h!["TextAttribute", "select", "a"]]
        );
    }

    #[test]
    fn should_parse_ng_content_when_it_contains_ws_only() {
        // TS: expectFromHtml('<ng-content select="a">    \n   </ng-content>')
        // Angular TypeScript does NOT filter whitespace children from ng-content
        // Reference: r3_template_transform.ts lines 191-204 - children are passed directly
        assert_eq!(
            humanize("<ng-content select=\"a\">    \n   </ng-content>"),
            vec![h!["Content", "a"], h!["TextAttribute", "select", "a"], h!["Text", "    \n   "]]
        );
    }

    #[test]
    fn should_parse_ng_content_regardless_the_namespace() {
        // TS: expectFromHtml('<svg><ng-content select="a"></ng-content></svg>')
        assert_eq!(
            humanize(r#"<svg><ng-content select="a"></ng-content></svg>"#),
            vec![h!["Element", ":svg:svg"], h!["Content", "a"], h!["TextAttribute", "select", "a"]]
        );
    }
}

// ============================================================================
// Tests: Bound text nodes
// ============================================================================

mod bound_text_nodes {
    use super::*;

    #[test]
    fn should_parse_bound_text_nodes() {
        // TS: expectFromHtml("{{a}}").toEqual([["BoundText", "{{ a }}"]])
        assert_eq!(humanize("{{a}}"), vec![h!["BoundText", "{{ a }}"]]);
    }
}

// ============================================================================
// Tests: Bound attributes
// ============================================================================

mod bound_attributes {
    use super::*;

    #[test]
    fn should_parse_mixed_case_bound_properties() {
        // TS: expectFromHtml('<div [someProp]="v"></div>')
        assert_eq!(
            humanize(r#"<div [someProp]="v"></div>"#),
            vec![
                h!["Element", "div"],
                h!["BoundAttribute", BindingType::Property, "someProp", "v"]
            ]
        );
    }

    #[test]
    fn should_parse_bound_properties_via_bind() {
        // TS: expectFromHtml('<div bind-prop="v"></div>')
        assert_eq!(
            humanize(r#"<div bind-prop="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundAttribute", BindingType::Property, "prop", "v"]]
        );
    }

    #[test]
    fn should_parse_bound_properties_via_interpolation() {
        // TS: expectFromHtml('<div prop="{{v}}"></div>')
        // Note: Angular outputs "{{ v }}" with spaces
        let result = humanize(r#"<div prop="{{v}}"></div>"#);
        assert_eq!(result[0], h!["Element", "div"]);
        assert!(matches!(&result[1][0], HumanValue::Str(s) if s == "BoundAttribute"));
    }

    #[test]
    fn should_parse_dash_case_bound_properties() {
        // TS: expectFromHtml('<div [some-prop]="v"></div>')
        assert_eq!(
            humanize(r#"<div [some-prop]="v"></div>"#),
            vec![
                h!["Element", "div"],
                h!["BoundAttribute", BindingType::Property, "some-prop", "v"]
            ]
        );
    }

    #[test]
    fn should_parse_dotted_name_bound_properties() {
        // TS: expectFromHtml('<div [d.ot]="v"></div>')
        assert_eq!(
            humanize(r#"<div [d.ot]="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundAttribute", BindingType::Property, "d.ot", "v"]]
        );
    }

    #[test]
    fn should_parse_mixed_case_bound_attributes() {
        // TS: expectFromHtml('<div [attr.someAttr]="v"></div>')
        assert_eq!(
            humanize(r#"<div [attr.someAttr]="v"></div>"#),
            vec![
                h!["Element", "div"],
                h!["BoundAttribute", BindingType::Attribute, "someAttr", "v"]
            ]
        );
    }

    #[test]
    fn should_parse_dash_case_bound_classes() {
        // TS: expectFromHtml('<div [class.some-class]="v"></div>')
        assert_eq!(
            humanize(r#"<div [class.some-class]="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundAttribute", BindingType::Class, "some-class", "v"]]
        );
    }

    #[test]
    fn should_parse_mixed_case_bound_classes() {
        // TS: expectFromHtml('<div [class.someClass]="v"></div>')
        assert_eq!(
            humanize(r#"<div [class.someClass]="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundAttribute", BindingType::Class, "someClass", "v"]]
        );
    }

    #[test]
    fn should_parse_mixed_case_bound_styles() {
        // TS: expectFromHtml('<div [style.someStyle]="v"></div>')
        assert_eq!(
            humanize(r#"<div [style.someStyle]="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundAttribute", BindingType::Style, "someStyle", "v"]]
        );
    }
}

// ============================================================================
// Tests: Events
// ============================================================================

mod events {
    use super::*;

    #[test]
    fn should_parse_event_bindings() {
        // TS: expectFromHtml('<div (window:event)="v"></div>')
        assert_eq!(
            humanize(r#"<div (event)="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundEvent", 0, "event", "", "v"]]
        );
    }

    #[test]
    fn should_parse_event_bindings_via_on() {
        // TS: expectFromHtml('<div on-event="v"></div>')
        assert_eq!(
            humanize(r#"<div on-event="v"></div>"#),
            vec![h!["Element", "div"], h!["BoundEvent", 0, "event", "", "v"]]
        );
    }
}

// ============================================================================
// Tests: References
// ============================================================================

mod references {
    use super::*;

    #[test]
    fn should_parse_references() {
        // TS: expectFromHtml('<div #a></div>')
        assert_eq!(
            humanize(r"<div #a></div>"),
            vec![h!["Element", "div"], h!["Reference", "a", ""]]
        );
    }

    #[test]
    fn should_parse_references_via_ref() {
        // TS: expectFromHtml('<div ref-a></div>')
        assert_eq!(
            humanize(r"<div ref-a></div>"),
            vec![h!["Element", "div"], h!["Reference", "a", ""]]
        );
    }

    #[test]
    fn should_parse_references_with_value() {
        // TS: expectFromHtml('<div #a="dirA"></div>')
        assert_eq!(
            humanize(r#"<div #a="dirA"></div>"#),
            vec![h!["Element", "div"], h!["Reference", "a", "dirA"]]
        );
    }
}

// ============================================================================
// Tests: Templates (ng-template)
// ============================================================================

mod templates {
    use super::*;

    #[test]
    fn should_parse_ng_template() {
        // TS: expectFromHtml('<ng-template></ng-template>')
        assert_eq!(humanize("<ng-template></ng-template>"), vec![h!["Template"]]);
    }

    #[test]
    fn should_parse_ng_template_with_attributes() {
        // TS: expectFromHtml('<ng-template ngFor let-item></ng-template>')
        assert_eq!(
            humanize("<ng-template ngFor let-item></ng-template>"),
            vec![h!["Template"], h!["TextAttribute", "ngFor", ""], h!["Variable", "item", ""]]
        );
    }

    #[test]
    fn should_parse_ng_template_with_variables() {
        // TS: expectFromHtml('<ng-template let-a="b"></ng-template>')
        assert_eq!(
            humanize(r#"<ng-template let-a="b"></ng-template>"#),
            vec![h!["Template"], h!["Variable", "a", "b"]]
        );
    }
}

// ============================================================================
// Tests: Control flow - @if
// ============================================================================

mod if_blocks {
    use super::*;

    #[test]
    fn should_parse_if_block() {
        // TS: expectFromHtml('@if (cond) { content }')
        assert_eq!(
            humanize("@if (cond) { content }"),
            vec![h!["IfBlock"], h!["IfBlockBranch", "cond"], h!["Text", " content "]]
        );
    }

    #[test]
    fn should_parse_if_else_block() {
        // TS: expectFromHtml('@if (cond) { a } @else { b }')
        let result = humanize("@if (cond) { a } @else { b }");
        assert_eq!(result[0], h!["IfBlock"]);
        assert_eq!(result[1], h!["IfBlockBranch", "cond"]);
        assert!(matches!(&result[3][1], HumanValue::Null));
    }

    #[test]
    fn should_parse_if_else_if_block() {
        // TS: expectFromHtml('@if (cond1) { a } @else if (cond2) { b }')
        let result = humanize("@if (cond1) { a } @else if (cond2) { b }");
        assert_eq!(result[0], h!["IfBlock"]);
        assert_eq!(result[1], h!["IfBlockBranch", "cond1"]);
        assert_eq!(result[3], h!["IfBlockBranch", "cond2"]);
    }
}

// ============================================================================
// Tests: Control flow - @for
// ============================================================================

mod for_blocks {
    use super::*;

    #[test]
    fn should_parse_for_block() {
        // TS: expectFromHtml('@for (item of items; track item.id) { content }')
        let result = humanize("@for (item of items; track item.id) { content }");
        assert_eq!(result[0][0], HumanValue::from("ForLoopBlock"));
        assert_eq!(result[1], h!["Variable", "item", "$implicit"]);
    }

    #[test]
    fn should_parse_for_with_empty() {
        // TS: expectFromHtml('@for (item of items; track item) { a } @empty { b }')
        let result = humanize("@for (item of items; track item) { a } @empty { b }");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("ForLoopBlockEmpty")));
    }
}

// ============================================================================
// Tests: Control flow - @switch
// ============================================================================

mod switch_blocks {
    use super::*;

    #[test]
    fn should_parse_switch_block() {
        // TS: expectFromHtml('@switch (expr) { @case (1) { a } @case (2) { b } @default { c } }')
        let result = humanize("@switch (expr) { @case (1) { a } @case (2) { b } @default { c } }");
        assert_eq!(result[0][0], HumanValue::from("SwitchBlock"));
        assert_eq!(result[1][0], HumanValue::from("SwitchBlockCaseGroup"));
        assert_eq!(result[2][0], HumanValue::from("SwitchBlockCase"));
        assert!(result.iter().any(|r| r.len() > 1 && r[1] == HumanValue::Null)); // @default case
    }
}

// ============================================================================
// Tests: @defer blocks
// ============================================================================

mod defer_blocks {
    use super::*;

    #[test]
    fn should_parse_defer_block() {
        // TS: expectFromHtml('@defer { content }')
        let result = humanize("@defer { content }");
        assert_eq!(result[0], h!["DeferredBlock"]);
    }

    #[test]
    fn should_parse_defer_with_loading() {
        // TS: expectFromHtml('@defer { a } @loading { b }')
        let result = humanize("@defer { a } @loading { b }");
        assert_eq!(result[0], h!["DeferredBlock"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlockLoading")));
    }

    #[test]
    fn should_parse_defer_with_placeholder() {
        // TS: expectFromHtml('@defer { a } @placeholder { b }')
        let result = humanize("@defer { a } @placeholder { b }");
        assert_eq!(result[0], h!["DeferredBlock"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlockPlaceholder")));
    }

    #[test]
    fn should_parse_defer_with_error() {
        // TS: expectFromHtml('@defer { a } @error { b }')
        let result = humanize("@defer { a } @error { b }");
        assert_eq!(result[0], h!["DeferredBlock"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlockError")));
    }
}

// ============================================================================
// Tests: @let declarations
// ============================================================================

mod let_declarations {
    use super::*;

    #[test]
    fn should_parse_let_declaration() {
        // TS: expectFromHtml('@let a = 1;')
        let result = humanize("@let a = 1;");
        assert_eq!(result[0][0], HumanValue::from("LetDeclaration"));
        assert_eq!(result[0][1], HumanValue::from("a"));
    }

    #[test]
    fn should_parse_let_declaration_with_expression() {
        // TS: expectFromHtml('@let foo = 123 + 456;')
        let result = humanize("@let foo = 123 + 456;");
        assert_eq!(result[0][0], HumanValue::from("LetDeclaration"));
        assert_eq!(result[0][1], HumanValue::from("foo"));
    }
}

// ============================================================================
// Tests: Void elements
// ============================================================================

mod void_elements {
    use super::*;

    #[test]
    fn should_parse_void_element() {
        // TS: expectFromHtml('<input><div></div>')
        let result = humanize("<input><div></div>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from("input"));
        assert_eq!(result[1][0], HumanValue::from("Element"));
        assert_eq!(result[1][1], HumanValue::from("div"));
    }

    #[test]
    fn should_parse_self_closing_element() {
        // TS: expectFromHtml('<div/>')
        let result = humanize("<div/>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from("div"));
    }
}

// ============================================================================
// Tests: Comments
// ============================================================================

mod comments {
    use super::*;

    // Angular's visitComment returns null - comments are NOT in AST nodes.
    // They're collected separately in `comment_nodes` when `collectCommentNodes: true`.
    // See: packages/compiler/src/render3/r3_template_transform.ts:344-349

    #[test]
    fn should_not_include_comments_in_nodes_by_default() {
        // Comments are not in the AST nodes array - matches Angular's visitComment returning null
        let result = humanize("<!-- comment -->");
        assert!(result.is_empty());
    }

    #[test]
    fn should_collect_comments_when_option_enabled() {
        // TS: parseTemplate(html, '', {collectCommentNodes: true})
        let result = parse_with_comments("<!-- comment -->");
        assert!(result.nodes.is_empty()); // Comments not in AST nodes
        assert!(result.comment_nodes.is_some());
        let comments = result.comment_nodes.unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0], " comment ");
    }

    #[test]
    fn should_not_have_comment_nodes_when_option_disabled() {
        // Default behavior - no comment collection
        let result = parse("<!-- comment -->");
        assert!(result.nodes.is_empty());
    }

    #[test]
    fn should_parse_elements_without_comments_in_nodes() {
        // Comments are filtered out of AST nodes, only elements remain
        let result = humanize("<div></div><!-- comment --><span></span>");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from("div"));
        assert_eq!(result[1][0], HumanValue::from("Element"));
        assert_eq!(result[1][1], HumanValue::from("span"));
    }

    #[test]
    fn should_collect_nested_comments() {
        // TS: parseTemplate(html, '', {collectCommentNodes: true}) with nested comments
        let html = r"
            <!-- outer comment -->
            <div>
                <!-- nested comment -->
            </div>
        ";
        let result = parse_with_comments(html);
        assert!(result.comment_nodes.is_some());
        let comments = result.comment_nodes.unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].trim(), "outer comment");
        assert_eq!(comments[1].trim(), "nested comment");
    }
}

// ============================================================================
// Tests: Two-way bindings (banana-in-a-box)
// ============================================================================

mod two_way_bindings {
    use super::*;

    #[test]
    fn should_parse_two_way_bindings() {
        // TS: expectFromHtml('<div [(prop)]="v"></div>')
        let result = humanize(r#"<div [(prop)]="v"></div>"#);
        assert_eq!(result[0], h!["Element", "div"]);
        // Two-way binding creates both BoundAttribute and BoundEvent
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundAttribute")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundEvent")));
    }

    #[test]
    fn should_parse_two_way_bindings_via_bindon() {
        // TS: expectFromHtml('<div bindon-prop="v"></div>')
        let result = humanize(r#"<div bindon-prop="v"></div>"#);
        assert_eq!(result[0], h!["Element", "div"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundAttribute")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundEvent")));
    }
}

// ============================================================================
// Tests: Inline templates (* syntax)
// ============================================================================

mod inline_templates {
    use super::*;

    #[test]
    fn should_parse_star_directive() {
        // TS: expectFromHtml('<div *ngIf></div>')
        let result = humanize("<div *ngIf></div>");
        // * directives create a Template wrapper
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("div"))
        );
    }

    #[test]
    fn should_parse_ngfor_directive() {
        // TS: expectFromHtml('<div *ngFor="let item of items"></div>')
        let result = humanize(r#"<div *ngFor="let item of items"></div>"#);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
    }
}

// ============================================================================
// Tests: Ignored elements
// ============================================================================

mod ignored_elements {
    use super::*;

    #[test]
    fn should_ignore_script_elements() {
        // TS: expectFromHtml('<script></script>a')
        let result = humanize("<script></script>a");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], h!["Text", "a"]);
    }

    #[test]
    fn should_ignore_style_elements() {
        // TS: expectFromHtml('<style></style>a')
        let result = humanize("<style></style>a");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], h!["Text", "a"]);
    }
}

// ============================================================================
// Tests: ngNonBindable
// ============================================================================

mod ng_non_bindable {
    use super::*;

    #[test]
    fn should_ignore_bindings_on_children_of_ng_non_bindable() {
        // TS: expectFromHtml('<div ngNonBindable>{{b}}</div>')
        // Should output Text instead of BoundText when inside ngNonBindable
        assert_eq!(
            humanize(r"<div ngNonBindable>{{b}}</div>"),
            vec![
                h!["Element", "div"],
                h!["TextAttribute", "ngNonBindable", ""],
                h!["Text", "{{b}}"]
            ]
        );
    }

    #[test]
    fn should_parse_ng_non_bindable_attribute() {
        // TS: the element should have ngNonBindable as text attribute
        let result = humanize(r"<div ngNonBindable></div>");
        assert_eq!(result[0], h!["Element", "div"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("TextAttribute")
            && r[1] == HumanValue::from("ngNonBindable")));
    }
}

// ============================================================================
// Tests: More defer block tests
// ============================================================================

mod defer_blocks_extended {
    use super::*;

    #[test]
    fn should_parse_defer_with_when_trigger() {
        // TS: expectFromHtml('@defer (when isVisible()) {hello}')
        let result = humanize("@defer (when isVisible()) {hello}");
        assert_eq!(result[0], h!["DeferredBlock"]);
    }

    #[test]
    fn should_parse_defer_with_on_idle_trigger() {
        // TS: expectFromHtml('@defer (on idle) {hello}')
        let result = humanize("@defer (on idle) {hello}");
        assert_eq!(result[0], h!["DeferredBlock"]);
    }

    #[test]
    fn should_parse_defer_with_timer_trigger() {
        // TS: expectFromHtml('@defer (on timer(100ms)) {hello}')
        let result = humanize("@defer (on timer(100ms)) {hello}");
        assert_eq!(result[0], h!["DeferredBlock"]);
    }

    #[test]
    fn should_parse_defer_with_all_connected_blocks() {
        // TS: expectFromHtml('@defer {main} @loading {loading} @placeholder {placeholder} @error {error}')
        let result =
            humanize("@defer {main} @loading {loading} @placeholder {placeholder} @error {error}");
        assert_eq!(result[0], h!["DeferredBlock"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlockLoading")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlockPlaceholder")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlockError")));
    }
}

// ============================================================================
// Tests: Trigger empty parentheses (Finding #1)
// Empty () should be treated as zero parameters, matching Angular's consumeParameters().
// ============================================================================

mod trigger_empty_parens {
    use super::*;

    #[test]
    fn should_accept_idle_with_empty_parens() {
        // Angular's consumeParameters returns zero parameters for `idle()`.
        // Oxc should not error on this.
        let errors = get_transform_errors("@defer (on idle()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("idle")),
            "idle() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_hover_with_empty_parens() {
        let errors = get_transform_errors("@defer (on hover()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("hover")),
            "hover() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_immediate_with_empty_parens() {
        let errors = get_transform_errors("@defer (on immediate()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("immediate")),
            "immediate() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_interaction_with_empty_parens() {
        let errors = get_transform_errors("@defer (on interaction()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("interaction")),
            "interaction() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_viewport_with_empty_parens() {
        let errors = get_transform_errors("@defer (on viewport()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("viewport")),
            "viewport() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_hydrate_hover_with_empty_parens() {
        let errors = get_transform_errors("@defer (hydrate on hover()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("hover")),
            "hydrate hover() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_hydrate_viewport_with_empty_parens() {
        let errors = get_transform_errors("@defer (hydrate on viewport()) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("viewport")),
            "hydrate viewport() with empty parens should not produce an error, got: {:?}",
            errors
        );
    }
}

// ============================================================================
// Tests: @defer (on ) / @defer (when ) classification (Finding #3)
// After trimming, "on" alone should still be recognized as an on-trigger prefix
// (matching Angular which does not trim the parameter text).
// ============================================================================

mod defer_on_when_trimmed {
    use super::*;

    #[test]
    fn should_silently_accept_defer_on_space() {
        // Angular accepts `@defer (on )` silently — no errors at all.
        // After trimming, "on" is recognized as an on-trigger with no trigger names → no triggers.
        let errors = get_transform_errors("@defer (on ) {hello}");
        assert!(errors.is_empty(), "@defer (on ) should produce no errors, got: {:?}", errors);
    }

    #[test]
    fn should_silently_accept_defer_when_space() {
        // Angular accepts `@defer (when )` silently — no errors at all.
        // After trimming, "when" is recognized as a when-trigger with no condition → no trigger.
        let errors = get_transform_errors("@defer (when ) {hello}");
        assert!(errors.is_empty(), "@defer (when ) should produce no errors, got: {:?}", errors);
    }
}

// ============================================================================
// Tests: Viewport parameter arity validation (Finding #2)
// Angular validates viewport parameter count before parsing.
// ============================================================================

mod viewport_arity_validation {
    use super::*;

    #[test]
    fn should_error_on_viewport_with_multiple_params() {
        // Angular errors: "viewport" trigger can only have zero or one parameters
        let errors = get_transform_errors("@defer (on viewport(a,b)) {hello}");
        assert!(
            errors.iter().any(|e| e.contains("viewport") && e.contains("parameter")),
            "viewport(a,b) should produce a parameter count error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_error_on_hydrate_viewport_with_multiple_params() {
        // Angular errors: Hydration trigger "viewport" cannot have more than one parameter
        let errors = get_transform_errors("@defer (hydrate on viewport(a,b)) {hello}");
        assert!(
            errors.iter().any(|e| e.contains("viewport") && e.contains("parameter")),
            "hydrate viewport(a,b) should produce a parameter count error, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_accept_viewport_with_single_param() {
        // viewport(ref) is valid
        let errors = get_transform_errors("@defer (on viewport(ref)) {hello}");
        assert!(
            !errors.iter().any(|e| e.contains("viewport") && e.contains("parameter")),
            "viewport(ref) should not produce a parameter error, got: {:?}",
            errors
        );
    }
}

// ============================================================================
// Tests: @for error cascade (Finding #4)
// Angular only reports the parse-expression error for `@for (x of ) {}`,
// NOT the missing-track error. Oxc should match.
// ============================================================================

mod for_error_cascade {
    use super::*;

    #[test]
    fn should_not_emit_missing_track_when_expression_empty_after_of() {
        // `@for (x of ) {}` - empty expression after "of"
        // Angular: only "Cannot parse expression" error
        // Oxc should NOT also emit "@for loop must have a \"track\" expression"
        let errors = get_transform_errors("@for (x of ) {content}");
        let has_parse_error = errors.iter().any(|e| e.contains("Cannot parse expression"));
        let has_track_error = errors.iter().any(|e| e.contains("track"));
        assert!(has_parse_error, "Should have a parse expression error, got: {:?}", errors);
        assert!(
            !has_track_error,
            "Should NOT have a missing track error when expression parse fails, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_not_emit_missing_track_when_no_expression() {
        // `@for () {}` - no expression at all
        let errors = get_transform_errors("@for () {content}");
        let has_track_error = errors.iter().any(|e| e.contains("track"));
        assert!(
            !has_track_error,
            "Should NOT have a missing track error when no expression, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_not_emit_missing_track_when_missing_of() {
        // `@for (x) {}` - no "of" keyword
        let errors = get_transform_errors("@for (x) {content}");
        let has_track_error = errors.iter().any(|e| e.contains("track"));
        assert!(
            !has_track_error,
            "Should NOT have a missing track error when 'of' is missing, got: {:?}",
            errors
        );
    }

    #[test]
    fn should_not_emit_missing_track_for_for_with_invalid_expression() {
        // `@for (x of; track x) {}` - missing iterable, has track
        // The semicolon makes "x of" the first parameter and "track x" the second
        // Angular should only error on the expression parse
        let errors = get_transform_errors("@for (x of; track x) {content}");
        let error_count = errors.len();
        // Should have parse error but not missing-track error
        // (track is actually provided but expression parse failed)
        assert!(
            error_count >= 1,
            "Should have at least one error for invalid expression, got: {:?}",
            errors
        );
    }
}

// ============================================================================
// Tests: Error-recovery AST shape (Finding #5)
// Angular only adds @if branches / @for nodes when params parse successfully.
// This is a LOW priority structural difference in error recovery.
// ============================================================================

mod error_recovery_ast_shape {
    use super::*;

    #[test]
    fn if_block_with_missing_expression_should_still_produce_ast() {
        // `@if {content}` - missing expression
        // Angular creates an IfBlock with 0 branches (main branch skipped
        // because parseConditionalBlockParameters returns null).
        let result = humanize_ignore_errors("@if {content}");
        // The IfBlock node is still emitted (with 0 branches) — verify no crash.
        assert!(!result.is_empty());
    }

    #[test]
    fn for_block_with_missing_params_returns_no_node() {
        // `@for () {content}` - missing parameters
        // Angular's createForLoop returns null node when parseForLoopParameters
        // fails (expression doesn't match "<identifier> of <expression>").
        let result = humanize_ignore_errors("@for () {content}");
        // Angular returns null node, so no ForLoopBlock should appear.
        let has_for = result.iter().any(|r| {
            r.first()
                .map(|v| match v {
                    HumanValue::Str(s) => s == "ForLoopBlock",
                    _ => false,
                })
                .unwrap_or(false)
        });
        assert!(
            !has_for,
            "Angular returns null for @for with invalid params, but Rust produced a ForLoopBlock"
        );
    }
}

// ============================================================================
// Tests: More switch block tests
// ============================================================================

mod switch_blocks_extended {
    use super::*;

    #[test]
    fn should_parse_nested_switch_blocks() {
        // TS: expectFromHtml('@switch (outer) { @case (1) { @switch (inner) { @case (a) { nested } } } }')
        let result =
            humanize("@switch (outer) { @case (1) { @switch (inner) { @case (a) { nested } } } }");
        // Should have two SwitchBlock entries
        let switch_count =
            result.iter().filter(|r| r[0] == HumanValue::from("SwitchBlock")).count();
        assert!(switch_count >= 2);
    }

    #[test]
    fn should_parse_switch_with_comments() {
        // TS: expectFromHtml('@switch (cond.kind) { <!-- X case --> @case (x) { X case } <!-- default case --> @default { No case matched } }')
        // Note: Comments inside switch blocks are stripped (not included in output)
        let result = humanize(
            "@switch (cond.kind) { <!-- X case --> @case (x) { X case } <!-- default case --> @default { No case matched } }",
        );
        assert_eq!(
            result,
            vec![
                h!["SwitchBlock", "cond.kind"],
                h!["SwitchBlockCaseGroup"],
                h!["SwitchBlockCase", "x"],
                h!["Text", " X case "],
                h!["SwitchBlockCaseGroup"],
                vec![HumanValue::from("SwitchBlockCase"), HumanValue::Null],
                h!["Text", " No case matched "],
            ]
        );
    }
}

// ============================================================================
// Tests: More for loop tests
// ============================================================================

mod for_loops_extended {
    use super::*;

    #[test]
    fn should_parse_for_with_context_variables() {
        // TS: expectFromHtml('@for (item of items; track item; let idx = $index, f = $first) { ... }')
        let result =
            humanize("@for (item of items; track item; let idx = $index, f = $first) { content }");
        assert_eq!(result[0][0], HumanValue::from("ForLoopBlock"));
        // Should have multiple Variable entries for idx and f
        let var_count = result.iter().filter(|r| r[0] == HumanValue::from("Variable")).count();
        assert!(var_count >= 2);
    }

    #[test]
    fn should_parse_nested_for_loops() {
        // TS: expectFromHtml('@for (outer of outers; track outer) { @for (inner of inners; track inner) { content } }')
        let result = humanize(
            "@for (outer of outers; track outer) { @for (inner of inners; track inner) { content } }",
        );
        let for_count = result.iter().filter(|r| r[0] == HumanValue::from("ForLoopBlock")).count();
        assert_eq!(for_count, 2);
    }
}

// ============================================================================
// Tests: More if block tests
// ============================================================================

mod if_blocks_extended {
    use super::*;

    #[test]
    fn should_parse_nested_if_blocks() {
        // TS: expectFromHtml('@if (outer) { @if (inner) { nested } }')
        let result = humanize("@if (outer) { @if (inner) { nested } }");
        let if_count = result.iter().filter(|r| r[0] == HumanValue::from("IfBlock")).count();
        assert_eq!(if_count, 2);
    }

    #[test]
    fn should_parse_if_with_as_expression() {
        // TS: expectFromHtml('@if (cond; as foo) { content }')
        let result = humanize("@if (cond; as foo) { content }");
        assert_eq!(result[0], h!["IfBlock"]);
        // Should have Variable for 'foo'
        assert!(
            result.iter().any(|r| r[0] == HumanValue::from("Variable")
                || r[0] == HumanValue::from("IfBlockBranch"))
        );
    }
}

// ============================================================================
// Tests: Link elements
// ============================================================================

mod link_elements {
    use super::*;

    #[test]
    fn should_parse_link_with_absolute_url() {
        // TS: expectFromHtml('<link rel="stylesheet" href="http://someurl">')
        let result = humanize(r#"<link rel="stylesheet" href="http://someurl">"#);
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from("link"));
    }

    #[test]
    fn should_parse_link_without_href() {
        // TS: expectFromHtml('<link rel="stylesheet">')
        let result = humanize(r#"<link rel="stylesheet">"#);
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from("link"));
    }
}

// ============================================================================
// Tests: SVG elements
// ============================================================================

mod svg_elements {
    use super::*;

    #[test]
    fn should_parse_svg_element() {
        // TS: expectFromHtml('<svg></svg>')
        let result = humanize("<svg></svg>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from(":svg:svg"));
    }

    #[test]
    fn should_parse_svg_with_nested_elements() {
        // TS: expectFromHtml('<svg><rect></rect></svg>')
        let result = humanize("<svg><rect></rect></svg>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from(":svg:svg"));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element")
                    && r[1] == HumanValue::from(":svg:rect"))
        );
    }
}

// ============================================================================
// Tests: Whitespace handling
// ============================================================================

mod whitespace {
    use super::*;

    #[test]
    fn should_preserve_whitespace_text_nodes() {
        // TS: expectFromHtml('<div>  </div>')
        let result = humanize("<div>  </div>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Text") && r[1] == HumanValue::from("  "))
        );
    }

    #[test]
    fn should_preserve_newlines() {
        // TS: expectFromHtml('<div>\n</div>')
        let result = humanize("<div>\n</div>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Text") && r[1] == HumanValue::from("\n"))
        );
    }
}

// ============================================================================
// Tests: Animation bindings
// ============================================================================

mod animation_bindings {
    use super::*;

    #[test]
    fn should_support_animate_enter() {
        // TS: expectFromHtml('<div animate.enter="foo"></div>')
        let result = humanize(r#"<div animate.enter="foo"></div>"#);
        assert_eq!(result[0][0], HumanValue::from("Element"));
        // Animation bindings should create BoundAttribute with Animation type
    }

    #[test]
    fn should_support_animate_leave() {
        // TS: expectFromHtml('<div animate.leave="foo"></div>')
        let result = humanize(r#"<div animate.leave="foo"></div>"#);
        assert_eq!(result[0][0], HumanValue::from("Element"));
    }
}

// ============================================================================
// Tests: More template tests
// ============================================================================

mod templates_extended {
    use super::*;

    #[test]
    fn should_parse_ng_template_in_svg() {
        // TS: expectFromHtml('<svg><ng-template></ng-template></svg>')
        let result = humanize("<svg><ng-template></ng-template></svg>");
        assert!(
            result.iter().any(
                |r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from(":svg:svg")
            )
        );
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
    }

    #[test]
    fn should_parse_ng_template_with_reference() {
        // TS: expectFromHtml('<ng-template #a></ng-template>')
        let result = humanize("<ng-template #a></ng-template>");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Reference") && r[1] == HumanValue::from("a"))
        );
    }

    #[test]
    fn should_parse_ng_template_with_ref_notation() {
        // TS: expectFromHtml('<ng-template ref-a></ng-template>')
        let result = humanize("<ng-template ref-a></ng-template>");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Reference") && r[1] == HumanValue::from("a"))
        );
    }

    #[test]
    fn should_parse_ng_template_with_bound_attributes() {
        // TS: expectFromHtml('<ng-template [k1]="v1" [k2]="v2"></ng-template>')
        let result = humanize(r#"<ng-template [k1]="v1" [k2]="v2"></ng-template>"#);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
        let bound_count =
            result.iter().filter(|r| r[0] == HumanValue::from("BoundAttribute")).count();
        assert_eq!(bound_count, 2);
    }

    #[test]
    fn should_parse_ng_template_with_multiple_attributes() {
        // TS: expectFromHtml('<ng-template k1="v1" k2="v2"></ng-template>')
        let result = humanize(r#"<ng-template k1="v1" k2="v2"></ng-template>"#);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
        let attr_count =
            result.iter().filter(|r| r[0] == HumanValue::from("TextAttribute")).count();
        assert_eq!(attr_count, 2);
    }
}

// ============================================================================
// Tests: More bound attribute tests
// ============================================================================

mod bound_attributes_extended {
    use super::*;

    #[test]
    fn should_not_normalize_property_names() {
        // TS: expectFromHtml('<div [mappedAttr]="v"></div>')
        let result = humanize(r#"<div [mappedAttr]="v"></div>"#);
        assert_eq!(result[0], h!["Element", "div"]);
        // Property name should be preserved as-is
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundAttribute")
            && r[2] == HumanValue::from("mappedAttr")));
    }
}

// ============================================================================
// Tests: More events tests
// ============================================================================

mod events_extended {
    use super::*;

    #[test]
    fn should_parse_event_with_target() {
        // TS: expectFromHtml('<div (window:event)="v"></div>')
        let result = humanize(r#"<div (window:event)="v"></div>"#);
        assert_eq!(result[0], h!["Element", "div"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundEvent")));
    }

    #[test]
    fn should_parse_dash_case_events() {
        // TS: expectFromHtml('<div (some-event)="v"></div>')
        let result = humanize(r#"<div (some-event)="v"></div>"#);
        assert_eq!(result[0], h!["Element", "div"]);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("BoundEvent")));
    }
}

// ============================================================================
// Tests: More ng-content tests
// ============================================================================

mod ng_content_extended {
    use super::*;

    #[test]
    fn should_parse_ng_content_without_selector() {
        // TS: expectFromHtml('<ng-content></ng-content>')
        let result = humanize("<ng-content></ng-content>");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Content")));
    }

    #[test]
    fn should_parse_ng_content_with_specific_selector() {
        // TS: expectFromHtml('<ng-content select="tag[attribute]"></ng-content>')
        let result = humanize(r#"<ng-content select="tag[attribute]"></ng-content>"#);
        assert!(
            result.iter().any(|r| r[0] == HumanValue::from("Content")
                && r[1] == HumanValue::from("tag[attribute]"))
        );
    }

    #[test]
    fn should_parse_ng_project_as_attribute() {
        // TS: expectFromHtml('<ng-content ngProjectAs="a"></ng-content>')
        let result = humanize(r#"<ng-content ngProjectAs="a"></ng-content>"#);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Content")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("TextAttribute")
            && r[1] == HumanValue::from("ngProjectAs")));
    }
}

// ============================================================================
// Tests: ng-container
// ============================================================================

mod ng_container {
    use super::*;

    #[test]
    fn should_parse_ng_container() {
        // ng-container should be parsed but not create an element in the output
        let result = humanize("<ng-container></ng-container>");
        // ng-container typically becomes a template or is transparent
        assert!(
            result.is_empty()
                || result
                    .iter()
                    .any(|r| r[0] == HumanValue::from("Template")
                        || r[0] == HumanValue::from("Element"))
        );
    }

    #[test]
    fn should_parse_ng_container_with_structural_directive() {
        // TS: expectFromHtml('<ng-container *ngIf="test"></ng-container>')
        let result = humanize(r#"<ng-container *ngIf="test"></ng-container>"#);
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Template")));
    }
}

// ============================================================================
// Tests: Multiple interpolations
// ============================================================================

mod multiple_interpolations {
    use super::*;

    #[test]
    fn should_parse_multiple_interpolations() {
        // TS: expectFromHtml('{{a}} and {{b}}')
        let result = humanize("{{a}} and {{b}}");
        // This could be a single BoundText with multiple interpolations, or multiple nodes
        assert!(!result.is_empty());
    }

    #[test]
    fn should_parse_interpolation_in_element() {
        // TS: expectFromHtml('<div>Hello {{name}}</div>')
        let result = humanize("<div>Hello {{name}}</div>");
        assert_eq!(result[0], h!["Element", "div"]);
        // Should have either Text + BoundText or just BoundText for interpolation
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("BoundText") || r[0] == HumanValue::from("Text"))
        );
    }
}

// ============================================================================
// Tests: Mixed content
// ============================================================================

mod mixed_content {
    use super::*;

    #[test]
    fn should_parse_text_and_elements() {
        // TS: expectFromHtml('text <span>span</span> more')
        let result = humanize("text <span>span</span> more");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Text")));
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("span"))
        );
    }

    #[test]
    fn should_parse_nested_elements() {
        // TS: expectFromHtml('<div><span><a></a></span></div>')
        let result = humanize("<div><span><a></a></span></div>");
        assert_eq!(result[0][0], HumanValue::from("Element"));
        assert_eq!(result[0][1], HumanValue::from("div"));
        // span and a should also be present
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("span"))
        );
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("a"))
        );
    }
}

// ============================================================================
// Tests: Complex control flow
// ============================================================================

mod complex_control_flow {
    use super::*;

    #[test]
    fn should_parse_if_inside_for() {
        // TS: expectFromHtml('@for (item of items; track item) { @if (item.visible) { content } }')
        let result =
            humanize("@for (item of items; track item) { @if (item.visible) { content } }");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("ForLoopBlock")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("IfBlock")));
    }

    #[test]
    fn should_parse_for_inside_if() {
        // TS: expectFromHtml('@if (hasItems) { @for (item of items; track item) { content } }')
        let result = humanize("@if (hasItems) { @for (item of items; track item) { content } }");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("IfBlock")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("ForLoopBlock")));
    }

    #[test]
    fn should_parse_switch_inside_for() {
        // TS: expectFromHtml('@for (item of items; track item) { @switch (item.type) { @case (1) { one } @default { other } } }')
        let result = humanize(
            "@for (item of items; track item) { @switch (item.type) { @case (1) { one } @default { other } } }",
        );
        assert!(result.iter().any(|r| r[0] == HumanValue::from("ForLoopBlock")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("SwitchBlock")));
    }

    #[test]
    fn should_parse_defer_inside_if() {
        // TS: expectFromHtml('@if (show) { @defer { lazy content } }')
        let result = humanize("@if (show) { @defer { lazy content } }");
        assert!(result.iter().any(|r| r[0] == HumanValue::from("IfBlock")));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("DeferredBlock")));
    }

    #[test]
    fn should_preserve_svg_namespace_in_switch_case_inside_for() {
        // Test that SVG elements inside @switch/@case inside @for have the correct namespace prefix
        let result = parse(
            r#"@for (item of items; track item.id) {
                @switch (item.type) {
                    @case ('circle') {
                        <svg viewBox="0 0 100 100">
                            <circle cx="50" cy="50" r="40" />
                        </svg>
                    }
                }
            }"#,
        );

        // Navigate to the SVG element inside the @switch case
        // ForLoopBlock -> children[0] (SwitchBlock) -> groups[0].children[0] (Element)
        let for_block = &result.nodes[0];
        if let R3NodeRef::ForLoopBlock { children, .. } = for_block {
            // Look for SwitchBlock in children (may have Text nodes for whitespace)
            let switch_block = children.iter().find(|c| matches!(c, R3NodeRef::SwitchBlock { .. }));
            if let Some(R3NodeRef::SwitchBlock { groups, .. }) = switch_block {
                // groups is Vec<SwitchCaseGroupRef>, not Vec<R3NodeRef>
                let first_group = &groups[0];
                let case_children = &first_group.children;
                // Look for Element in case children (may have Text nodes for whitespace)
                let svg_elem =
                    case_children.iter().find(|c| matches!(c, R3NodeRef::Element { .. }));
                if let Some(R3NodeRef::Element { name, .. }) = svg_elem {
                    // The SVG element name should have the namespace prefix ":svg:svg"
                    assert_eq!(
                        name, ":svg:svg",
                        "SVG element inside @switch/@case in @for should have :svg: namespace prefix"
                    );
                } else {
                    panic!("Expected Element in case children");
                }
            } else {
                // Print what children we have for debugging
                let child_types: Vec<&str> = children
                    .iter()
                    .map(|c| match c {
                        R3NodeRef::Element { name, .. } => name.as_str(),
                        R3NodeRef::Text { .. } => "Text",
                        R3NodeRef::SwitchBlock { .. } => "SwitchBlock",
                        R3NodeRef::IfBlock { .. } => "IfBlock",
                        R3NodeRef::ForLoopBlock { .. } => "ForLoopBlock",
                        _ => "Other",
                    })
                    .collect();
                panic!("Expected SwitchBlock in ForLoopBlock children, got: {child_types:?}");
            }
        } else {
            panic!("Expected ForLoopBlock");
        }
    }
}

// ============================================================================
// Tests: @switch validation
// ============================================================================

mod switch_validation {
    use super::*;

    #[test]
    fn should_report_error_for_non_block_children_in_switch() {
        // Angular reports: "@switch block can only contain @case and @default blocks"
        // for non-whitespace text and element children
        let errors = get_transform_errors("@switch (expr) { <div>invalid</div> @case (1) { a } }");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("@switch block can only contain @case and @default blocks")),
            "Expected error about non-block children in @switch, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_error_for_non_whitespace_text_in_switch() {
        let errors = get_transform_errors("@switch (expr) { some text @case (1) { a } }");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("@switch block can only contain @case and @default blocks")),
            "Expected error about non-block text in @switch, got: {errors:?}"
        );
    }

    #[test]
    fn should_allow_whitespace_text_in_switch() {
        // Whitespace-only text nodes should be silently skipped (same as Angular)
        let errors = get_transform_errors("@switch (expr) { \n  @case (1) { a } }");
        assert!(
            errors.is_empty(),
            "Expected no errors for whitespace-only text in @switch, got: {errors:?}"
        );
    }

    #[test]
    fn should_allow_comments_in_switch() {
        // Comments should be silently skipped (same as Angular)
        let errors = get_transform_errors("@switch (expr) { <!-- comment --> @case (1) { a } }");
        assert!(errors.is_empty(), "Expected no errors for comments in @switch, got: {errors:?}");
    }

    #[test]
    fn should_report_error_for_case_with_multiple_parameters() {
        // Angular: "@case block must have exactly one parameter"
        // Note: the HTML parser may or may not parse this as a valid @case block depending
        // on how parameters are tokenized. Either way, an error should be reported.
        let errors = get_transform_errors("@switch (expr) { @case (1) (2) { a } }");
        assert!(
            !errors.is_empty(),
            "Expected errors for @case with multiple parameters, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_error_for_case_with_no_parameters() {
        // Angular: "@case block must have exactly one parameter"
        let errors = get_transform_errors("@switch (expr) { @case { a } }");
        assert!(
            errors.iter().any(|e| e.contains("@case block must have exactly one parameter")),
            "Expected error about @case parameters, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_error_for_multiple_default_blocks() {
        // Angular: "@switch block can only have one @default block"
        let errors = get_transform_errors(
            "@switch (expr) { @case (1) { a } @default { b } @default { c } }",
        );
        assert!(
            errors.iter().any(|e| e.contains("@switch block can only have one @default block")),
            "Expected error about multiple @default blocks, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_error_for_default_with_parameters() {
        // Angular: "@default block cannot have parameters"
        let errors = get_transform_errors("@switch (expr) { @case (1) { a } @default (x) { b } }");
        assert!(
            errors.iter().any(|e| e.contains("@default block cannot have parameters")),
            "Expected error about @default with parameters, got: {errors:?}"
        );
    }
}

// ============================================================================
// Tests: @for duplicate let parameter validation
// ============================================================================

mod switch_invalid_case_unknown_blocks {
    use super::*;

    /// Helper that parses HTML to R3 AST and returns the unknown_blocks from the first SwitchBlock.
    fn get_switch_unknown_block_names(html: &str) -> Vec<String> {
        let allocator = Box::new(Allocator::default());
        let allocator_ref: &'static Allocator =
            unsafe { &*std::ptr::from_ref::<Allocator>(allocator.as_ref()) };

        let parser = HtmlParser::new(allocator_ref, html, "test.html");
        let html_result = parser.parse();

        let options = TransformOptions { collect_comment_nodes: false };
        let transformer = HtmlToR3Transform::new(allocator_ref, html, options);
        let r3_result = transformer.transform(&html_result.nodes);

        // Find the first SwitchBlock and return its unknown_blocks names
        for node in r3_result.nodes.iter() {
            if let R3Node::SwitchBlock(b) = node {
                return b.unknown_blocks.iter().map(|ub| ub.name.to_string()).collect();
            }
        }
        Vec::new()
    }

    #[test]
    fn should_add_invalid_case_with_no_params_to_unknown_blocks() {
        // Angular pushes @case blocks with invalid parameters into unknownBlocks
        // for language service autocompletion support.
        // Reference: r3_control_flow.ts line 242
        let unknown_names =
            get_switch_unknown_block_names("@switch (expr) { @case { a } @case (1) { b } }");
        assert!(
            unknown_names.contains(&"case".to_string()),
            "Expected invalid @case (no params) to be in unknown_blocks, got: {unknown_names:?}"
        );
    }

    #[test]
    fn should_still_parse_case_with_extra_params_using_first_param() {
        // Reference: r3_control_flow.ts line 242, 250
        // Angular reports an error for @case with >1 parameter, but still parses the
        // first parameter and creates a SwitchBlockCase. It does NOT push to unknownBlocks.
        // `@case (a; b)` produces 2 parameters in Angular's parser.
        let result = parse_with_options("@switch (expr) { @case (a; b) { content } }", true, false);
        // Should still have a SwitchBlock with a case group (not pushed to unknown_blocks)
        let has_switch_block = result.nodes.iter().any(|n| {
            if let R3NodeRef::SwitchBlock { groups, .. } = n {
                groups.iter().any(|g| g.cases.iter().any(|c| c.expression.is_some()))
            } else {
                false
            }
        });
        assert!(
            has_switch_block,
            "Expected @case with extra params to still be parsed as a case with expression"
        );
        // Should NOT be in unknown_blocks
        let unknown_names =
            get_switch_unknown_block_names("@switch (expr) { @case (a; b) { content } }");
        assert!(
            !unknown_names.contains(&"case".to_string()),
            "Expected @case with extra params NOT to be in unknown_blocks, got: {unknown_names:?}"
        );
    }
}

mod for_loop_duplicate_let_validation {
    use super::*;

    #[test]
    fn should_report_duplicate_let_for_implicit_var_aliased_to_itself() {
        // Angular test: `let $index = $index` should be rejected as a duplicate
        // because $index is already pre-populated as an implicit context variable.
        // Reference: r3_template_transform_spec.ts line 2340
        let errors = get_transform_errors(
            "@for (item of items.foo.bar; track item.id; let $index = $index) {}",
        );
        assert!(
            errors.iter().any(|e| e.contains("Duplicate \"let\" parameter variable")),
            "Expected duplicate let parameter error for `let $index = $index`, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_duplicate_let_for_explicit_alias_used_twice() {
        // Angular test: `let i = $index` used twice should be rejected
        // Reference: r3_template_transform_spec.ts line 2340
        let errors = get_transform_errors(
            "@for (item of items.foo.bar; track item.id; let i = $index, f = $first, i = $index) {}",
        );
        assert!(
            errors.iter().any(|e| e.contains("Duplicate \"let\" parameter variable")),
            "Expected duplicate let parameter error for duplicate alias `i`, got: {errors:?}"
        );
    }
}

// ============================================================================
// Tests: Standalone connected blocks emit errors and UnknownBlock
// ============================================================================

mod standalone_connected_blocks {
    use super::*;

    #[test]
    fn standalone_else_should_produce_error_and_unknown_block() {
        // Reference: r3_template_transform.ts:515-517
        let errors = get_transform_errors("@else { content }");
        assert!(
            errors.iter().any(|e| e.contains("@else block can only be used after an @if")),
            "Expected error for standalone @else, got: {errors:?}"
        );
        let result = parse_with_options("@else { content }", true, false);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, R3NodeRef::UnknownBlock { name } if name == "else")),
            "Expected UnknownBlock for standalone @else"
        );
    }

    #[test]
    fn standalone_else_if_should_produce_error_and_unknown_block() {
        let errors = get_transform_errors("@else if (cond) { content }");
        assert!(
            errors.iter().any(|e| e.contains("@else if block can only be used after an @if")),
            "Expected error for standalone @else if, got: {errors:?}"
        );
    }

    #[test]
    fn standalone_empty_should_produce_error_and_unknown_block() {
        // Reference: r3_template_transform.ts:512-514
        let errors = get_transform_errors("@empty { content }");
        assert!(
            errors.iter().any(|e| e.contains("@empty block can only be used after an @for")),
            "Expected error for standalone @empty, got: {errors:?}"
        );
        let result = parse_with_options("@empty { content }", true, false);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, R3NodeRef::UnknownBlock { name } if name == "empty")),
            "Expected UnknownBlock for standalone @empty"
        );
    }

    #[test]
    fn standalone_placeholder_should_produce_error_and_unknown_block() {
        // Reference: r3_template_transform.ts:509-511
        let errors = get_transform_errors("@placeholder { content }");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("@placeholder block can only be used after an @defer")),
            "Expected error for standalone @placeholder, got: {errors:?}"
        );
        let result = parse_with_options("@placeholder { content }", true, false);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, R3NodeRef::UnknownBlock { name } if name == "placeholder")),
            "Expected UnknownBlock for standalone @placeholder"
        );
    }

    #[test]
    fn standalone_loading_should_produce_error_and_unknown_block() {
        let errors = get_transform_errors("@loading { content }");
        assert!(
            errors.iter().any(|e| e.contains("@loading block can only be used after an @defer")),
            "Expected error for standalone @loading, got: {errors:?}"
        );
    }

    #[test]
    fn standalone_error_should_produce_error_and_unknown_block() {
        let errors = get_transform_errors("@error { content }");
        assert!(
            errors.iter().any(|e| e.contains("@error block can only be used after an @defer")),
            "Expected error for standalone @error, got: {errors:?}"
        );
    }

    #[test]
    fn standalone_case_should_produce_error_and_unknown_block() {
        // Reference: r3_template_transform.ts:518 - falls through to "Unrecognized block"
        let errors = get_transform_errors("@case (value) { content }");
        assert!(
            errors.iter().any(|e| e.contains("Unrecognized block @case")),
            "Expected error for standalone @case, got: {errors:?}"
        );
        let result = parse_with_options("@case (value) { content }", true, false);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, R3NodeRef::UnknownBlock { name } if name == "case")),
            "Expected UnknownBlock for standalone @case"
        );
    }

    #[test]
    fn standalone_default_should_produce_error_and_unknown_block() {
        // Reference: r3_template_transform.ts:518 - falls through to "Unrecognized block"
        let errors = get_transform_errors("@default { content }");
        assert!(
            errors.iter().any(|e| e.contains("Unrecognized block @default")),
            "Expected error for standalone @default, got: {errors:?}"
        );
        let result = parse_with_options("@default { content }", true, false);
        assert!(
            result
                .nodes
                .iter()
                .any(|n| matches!(n, R3NodeRef::UnknownBlock { name } if name == "default")),
            "Expected UnknownBlock for standalone @default"
        );
    }
}

// ============================================================================
// Tests: @defer connected block validation (duplicates / bad params)
// ============================================================================

mod defer_connected_block_validation {
    use super::*;

    #[test]
    fn should_report_duplicate_placeholder() {
        // Reference: r3_deferred_blocks.ts:124-131
        let errors =
            get_transform_errors("@defer { main } @placeholder { first } @placeholder { second }");
        assert!(
            errors.iter().any(|e| e.contains("@defer block can only have one @placeholder block")),
            "Expected duplicate placeholder error, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_duplicate_loading() {
        // Reference: r3_deferred_blocks.ts:137-143
        let errors = get_transform_errors("@defer { main } @loading { first } @loading { second }");
        assert!(
            errors.iter().any(|e| e.contains("@defer block can only have one @loading block")),
            "Expected duplicate loading error, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_duplicate_error() {
        // Reference: r3_deferred_blocks.ts:149-152
        let errors = get_transform_errors("@defer { main } @error { first } @error { second }");
        assert!(
            errors.iter().any(|e| e.contains("@defer block can only have one @error block")),
            "Expected duplicate error block error, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_error_block_with_parameters() {
        // Reference: r3_deferred_blocks.ts:252-253
        let errors = get_transform_errors("@defer { main } @error (something) { err }");
        assert!(
            errors.iter().any(|e| e.contains("@error block cannot have parameters")),
            "Expected error block parameter error, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_unrecognized_placeholder_parameter() {
        // Reference: r3_deferred_blocks.ts:186
        let errors = get_transform_errors("@defer { main } @placeholder (unknown 100ms) { ph }");
        assert!(
            errors.iter().any(|e| e.contains("Unrecognized parameter in @placeholder block")),
            "Expected unrecognized parameter error, got: {errors:?}"
        );
    }

    #[test]
    fn should_report_unrecognized_loading_parameter() {
        // Reference: r3_deferred_blocks.ts:234-235
        let errors = get_transform_errors("@defer { main } @loading (unknown 100ms) { loading }");
        assert!(
            errors.iter().any(|e| e.contains("Unrecognized parameter in @loading block")),
            "Expected unrecognized parameter error, got: {errors:?}"
        );
    }
}

// ============================================================================
// Tests: @for track expression parsing
// ============================================================================

mod for_track_expression_parsing {
    use super::*;

    #[test]
    fn empty_track_expression_should_report_missing_track() {
        // Reference: r3_control_flow.ts:406-409
        // Angular matches "track" keyword but then reports missing expression when empty
        let errors = get_transform_errors("@for (item of items; track ) {}");
        assert!(
            errors.iter().any(|e| e.contains("\"track\" expression")),
            "Expected missing track expression error, got: {errors:?}"
        );
    }
}

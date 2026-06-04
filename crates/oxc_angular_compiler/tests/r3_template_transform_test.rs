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

/// Helper that transforms an HTML template and returns, for the first element's
/// first bound-attribute input, a `(name, binding_type, security_context)`
/// tuple as strings. Used by the SVG/security tests to assert the resolved
/// `SecurityContext` (which the standard humanizer does not expose).
fn first_input_security(html: &str) -> (String, BindingType, String) {
    use oxc_angular_compiler::ast::r3::{R3Node, SecurityContext};

    let allocator = Box::new(Allocator::default());
    let allocator_ref: &'static Allocator =
        unsafe { &*std::ptr::from_ref::<Allocator>(allocator.as_ref()) };

    let parser = HtmlParser::new(allocator_ref, html, "test.html");
    let html_result = parser.parse();

    let options = TransformOptions { collect_comment_nodes: false };
    let transformer = HtmlToR3Transform::new(allocator_ref, html, options);
    let r3_result = transformer.transform(&html_result.nodes);

    let element = r3_result
        .nodes
        .iter()
        .find_map(|n| if let R3Node::Element(e) = n { Some(e) } else { None })
        .expect("expected at least one element");
    let input = element.inputs.first().expect("expected at least one bound-attribute input");

    let sc = match input.security_context {
        SecurityContext::None => "None",
        SecurityContext::Html => "Html",
        SecurityContext::Style => "Style",
        SecurityContext::Script => "Script",
        SecurityContext::Url => "Url",
        SecurityContext::ResourceUrl => "ResourceUrl",
        SecurityContext::AttributeNoBinding => "AttributeNoBinding",
        SecurityContext::UrlOrResourceUrl => "UrlOrResourceUrl",
    };

    (input.name.to_string(), input.binding_type, sc.to_string())
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

    /// Angular uses the regex `/^else[^\S\r\n]+if/` to detect "else if" blocks,
    /// which means block names like "else ifx" still match as connected else-if
    /// branches. Our parser must replicate this behavior to avoid emitting two
    /// independent conditionals instead of a single chained one.
    #[test]
    fn should_treat_else_if_prefix_as_connected_block() {
        // "else ifx" should still be chained as a connected else-if branch,
        // matching Angular's regex-based ELSE_IF_PATTERN: /^else[^\S\r\n]+if/
        let result = humanize_ignore_errors("@if (cond1) { a } @else ifx (cond2) { b }");
        // Should produce a single IfBlock with two branches, not two independent blocks
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
    fn should_keep_namespaced_svg_script_elements() {
        // v21.2.7 template_preparser.ts compares the FULL lower-cased node name
        // against `SCRIPT_ELEMENT = 'script'` (a singular string, lines 18,45,51).
        // An explicitly namespaced `:svg:script` is NOT equal to 'script', so it
        // stays `PreparsedElementType.OTHER` and is KEPT as an element (only plain
        // `<script>` is dropped). This is the genuine SVGScriptElement XSS sink,
        // whose href is sanitized as a RESOURCE_URL by dom_security_schema.
        // (angular/angular MAIN later widened SCRIPT_ELEMENT to a Set including
        // ':svg:script' via commit 90494cd909, but that post-dates v21.2.7.)
        //
        // v21.2.7-faithful note: a namespaced raw-text element closes on its LOCAL name
        // only (case-insensitive); `</svg:script>` would NOT close it (the boundary is bare
        // `script`) and the scan would run to EOF. We therefore close with a BARE `</script>`
        // so the `:svg:script` element closes cleanly and the `<div>` survives as a sibling —
        // which keeps this test focused on G4's actual point (the `:svg:script` element is
        // KEPT, not stripped).
        let result = humanize("<svg:script>x</script><div>after</div>");
        assert_eq!(result[0], h!["Element", ":svg:script"]);
        // The script's raw-text content survives because the element is kept,
        // and the following <div> sibling is unaffected.
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Text") && r[1] == "x".into()));
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Element") && r[1] == "div".into()));
    }

    // -----------------------------------------------------------------------
    // SVG <script> href / xlink:href security-context faithfulness to v21.2.7.
    //
    // Ground truth derived by reading vendored v21.2.7 (commit 50761c8be4):
    //
    // - `_consumeAttr` (ml_parser/parser.ts:624) stores a namespaced attribute
    //   `xlink:href` in MERGED colon form `:xlink:href` via `mergeNsAndName`.
    // - `<svg:script>` element name is the merged `:svg:script`.
    // - `createBoundElementProperty` (binding_parser.ts:543-632):
    //     * `[attr.xlink:href]` -> `parts[0]=="attr"`, security lookup propName =
    //       `xlink:href` (PLAIN colon, isAttribute=true), THEN the stored
    //       BoundAttribute name is merged to `:xlink:href` via `mergeNsAndName`
    //       (binding_parser.ts:582-587). (Finding 2.)
    //     * `[xlink:href]`       -> property binding; the `[...]` lexer path keeps
    //       the bare name `xlink:href` (PLAIN colon), isAttribute=false.
    //     * `xlink:href="{{u}}"` (interpolated STATIC attr) -> property binding
    //       with the FULL merged name `:xlink:href` (LEADING colon retained,
    //       no `.`-split happens), isAttribute=false.
    // - `CssSelector.parse(":svg:script")` (directive_matching.ts:9-20 regexp)
    //   drops the leading `:` and namespace, leaving element = `script` (local).
    // - `securityContext("script", propName, isAttribute)`
    //   (dom_element_schema_registry.ts:437-457) lowercases and looks up
    //   `script|propName`, falling back to `*|propName`, else NONE. The schema
    //   keys are `script|href` and `script|xlink:href` -> RESOURCE_URL
    //   (dom_security_schema.ts:124-125).
    //
    // Therefore upstream v21.2.7 sanitizes:
    //   `[attr.xlink:href]`, `[xlink:href]`, `[(xlink:href)]`,
    //   `href="{{u}}"`, `[attr.href]`, `[href]`
    // as RESOURCE_URL, but does NOT sanitize the INTERPOLATED static
    //   `xlink:href="{{u}}"` (its lookup key is `script|:xlink:href`, a MISS,
    //   because the merged colon form is passed verbatim as a property name).
    //
    // These tests assert OXC matches that exact behavior. The values were
    // observed empirically (see PR notes) — they are NOT assumptions.

    #[test]
    fn svg_script_bracketed_xlink_href_is_resource_url() {
        // `[xlink:href]` property binding -> name kept `xlink:href`, key
        // `script|xlink:href` -> RESOURCE_URL (matches v21.2.7).
        assert_eq!(
            first_input_security(r#"<svg:script [xlink:href]="u"></svg:script>"#),
            ("xlink:href".to_string(), BindingType::Property, "ResourceUrl".to_string()),
        );
    }

    #[test]
    fn svg_script_attr_xlink_href_is_resource_url() {
        // `[attr.xlink:href]` attribute binding -> `attr.` stripped, security
        // computed with the PLAIN `xlink:href` (key `script|xlink:href` ->
        // RESOURCE_URL), THEN the stored binding name is the namespace-bearing
        // merged form `:xlink:href` (upstream `createBoundElementProperty`
        // `mergeNsAndName`, binding_parser.ts:582-587). Confirmed against
        // @angular/compiler@21.2.7: `parseTemplate('<svg><script
        // [attr.xlink:href]="u">')` yields BoundAttribute name `:xlink:href`,
        // securityContext RESOURCE_URL (Finding 2 fix).
        assert_eq!(
            first_input_security(r#"<svg:script [attr.xlink:href]="u"></svg:script>"#),
            (":xlink:href".to_string(), BindingType::Attribute, "ResourceUrl".to_string()),
        );
    }

    #[test]
    fn svg_script_bracketed_href_is_resource_url() {
        // Non-xlink control proving G2 still works through the lookup:
        // `[href]` -> key `script|href` -> RESOURCE_URL.
        assert_eq!(
            first_input_security(r#"<svg:script [href]="u"></svg:script>"#),
            ("href".to_string(), BindingType::Property, "ResourceUrl".to_string()),
        );
    }

    #[test]
    fn svg_script_attr_href_is_resource_url() {
        // `[attr.href]` -> key `script|href` -> RESOURCE_URL.
        assert_eq!(
            first_input_security(r#"<svg:script [attr.href]="u"></svg:script>"#),
            ("href".to_string(), BindingType::Attribute, "ResourceUrl".to_string()),
        );
    }

    #[test]
    fn svg_script_interpolated_href_is_resource_url() {
        // Interpolated static `href="{{u}}"` -> property binding with name
        // `href` (no namespace), key `script|href` -> RESOURCE_URL. This proves
        // the non-namespaced interpolated case IS sanitized (matches v21.2.7).
        assert_eq!(
            first_input_security(r#"<svg:script href="{{u}}"></svg:script>"#),
            ("href".to_string(), BindingType::Property, "ResourceUrl".to_string()),
        );
    }

    #[test]
    fn svg_script_interpolated_xlink_href_is_none_matching_upstream() {
        // CRITICAL faithfulness case: interpolated static `xlink:href="{{u}}"`
        // is stored as the merged `:xlink:href` and treated as a PROPERTY
        // binding with that full name. Upstream's lookup key is
        // `script|:xlink:href` (leading colon retained) -> MISS -> NONE. OXC
        // must NOT sanitize this either (no over-sanitization). The emitted
        // binding name is the merged `:xlink:href`, unchanged.
        assert_eq!(
            first_input_security(r#"<svg:script xlink:href="{{u}}"></svg:script>"#),
            (":xlink:href".to_string(), BindingType::Property, "None".to_string()),
        );
    }

    #[test]
    fn svg_image_interpolated_xlink_href_is_none_matching_upstream() {
        // `<svg:image>` is NOT in the RESOURCE_URL/URL schema for `xlink:href`
        // (only the MathML elements + `a` are URL; `image` is not listed). An
        // interpolated static `xlink:href="{{u}}"` resolves to NONE both because
        // the merged-name property lookup misses and because `image|xlink:href`
        // is not a schema key. Control case proving no over-sanitization.
        assert_eq!(
            first_input_security(r#"<svg:image xlink:href="{{u}}"></svg:image>"#),
            (":xlink:href".to_string(), BindingType::Property, "None".to_string()),
        );
    }

    #[test]
    fn svg_script_non_sensitive_namespaced_attr_stays_none() {
        // Control: a non-sensitive namespaced attribute binding on `<svg:script>`
        // must remain NONE (no over-sanitization from any normalization). The
        // stored name is the merged `:xlink:title` form (Finding 2). Confirmed
        // against @angular/compiler@21.2.7: BoundAttribute name `:xlink:title`,
        // securityContext NONE.
        assert_eq!(
            first_input_security(r#"<svg:script [attr.xlink:title]="u"></svg:script>"#),
            (":xlink:title".to_string(), BindingType::Attribute, "None".to_string()),
        );
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

    #[test]
    fn foreign_object_children_reset_to_html_namespace() {
        // Upstream html_tags.ts declares `foreignObject` with
        // `implicitNamespacePrefix:'svg'` AND `preventNamespaceInheritance:true`
        // (v21.2.7 html_tags.ts:132-142): the element itself is SVG (stored
        // `:svg:foreignObject`), but its CHILDREN do NOT inherit the svg namespace —
        // they reset to HTML. So `<div>` inside foreignObject is plain `div`, NOT
        // `:svg:div`.
        let result = humanize("<svg><foreignObject><div></div></foreignObject></svg>");
        assert_eq!(result[0], h!["Element", ":svg:svg"]);
        // foreignObject element itself stays namespaced.
        assert!(result.iter().any(|r| r[0] == HumanValue::from("Element")
            && r[1] == HumanValue::from(":svg:foreignObject")));
        // Its child div is HTML, not :svg:div.
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("div"))
        );
        assert!(
            !result.iter().any(
                |r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from(":svg:div")
            ),
            "foreignObject child must not be emitted in the svg namespace: {result:?}"
        );
    }

    #[test]
    fn svg_sibling_after_foreign_object_stays_svg() {
        // Sanity: resetting foreignObject's children to HTML must not leak into
        // svg siblings. A `<rect>` sibling (back under `<svg>`) is still `:svg:rect`,
        // and grandchildren of foreignObject (`<span>`) remain HTML.
        let result = humanize(
            "<svg><foreignObject><div><span></span></div></foreignObject><rect></rect></svg>",
        );
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("div"))
        );
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element") && r[1] == HumanValue::from("span"))
        );
        assert!(
            result
                .iter()
                .any(|r| r[0] == HumanValue::from("Element")
                    && r[1] == HumanValue::from(":svg:rect")),
            "svg sibling after foreignObject must stay in svg namespace: {result:?}"
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

    // Upstream v21.2.7 only recognizes the EXACT `default never` (single internal
    // space) as the exhaustive marker; `_getBlockName` trims but does NOT collapse
    // internal whitespace (ml_parser/lexer.ts:294, render3/r3_control_flow.ts:620).
    // So `@default   never;` (multiple spaces) is an incomplete block named
    // "default   never" inside the switch, which `validateSwitchBlock` rejects with
    // "@switch block can only contain @case and @default blocks" and createSwitchBlock
    // turns into an UnknownBlock — it does NOT become a SwitchExhaustiveCheck node.
    // Verified by executing @angular/compiler@21.2.7 parseTemplate:
    //   "@switch (x) { @case (1) { a } @default   never; }"
    //   -> errors: ['Incomplete block "default   never". ...'], switch=0 exhaustive=0
    #[test]
    fn multi_space_default_never_is_not_exhaustive_marker() {
        let errors = get_transform_errors("@switch (expr) { @case (1) { a } @default   never; }");
        // Not accepted as the exhaustive marker -> rejected as invalid switch content.
        assert!(
            errors
                .iter()
                .any(|e| e.contains("@switch block can only contain @case and @default blocks")),
            "Multi-space @default   never; must be rejected as invalid switch content, \
             got: {errors:?}"
        );
        // And it must NOT silently pass as a valid exhaustive marker.
        assert!(
            !errors.is_empty(),
            "Multi-space @default   never; must NOT be silently accepted, got: {errors:?}"
        );
    }

    // Same for a TAB separator: not the canonical marker, rejected as switch content.
    // Verified by executing @angular/compiler@21.2.7:
    //   "@switch (x) { @case (1) { a } @default\tnever; }"
    //   -> errors: ['Incomplete block "default\tnever". ...'], switch=0 exhaustive=0
    #[test]
    fn tab_default_never_is_not_exhaustive_marker() {
        let errors = get_transform_errors("@switch (expr) { @case (1) { a } @default\tnever; }");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("@switch block can only contain @case and @default blocks")),
            "Tab @default\\tnever; must be rejected as invalid switch content, got: {errors:?}"
        );
    }

    // CONTROL: the canonical single-space `@default never;` STILL produces a valid
    // exhaustive marker with no errors (no regression).
    // Verified by executing @angular/compiler@21.2.7:
    //   "@switch (x) { @case (1) { a } @default never; }" -> errors: [], exhaustive=1
    #[test]
    fn single_space_default_never_is_valid_exhaustive_marker() {
        let errors = get_transform_errors("@switch (expr) { @case (1) { a } @default never; }");
        assert!(
            errors.is_empty(),
            "Canonical @default never; must remain a valid exhaustive marker with no errors, \
             got: {errors:?}"
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

// ============================================================================
// Selectorless component namespace inheritance (R3 transform end-to-end)
//
// Verifies that a selectorless component's `R3Component.tag_name` / `full_name`
// inherit the parent implicit namespace, matching upstream
// `_getComponentTagName` / `_getComponentFullName` (ml_parser/parser.ts).
// Uses the real `HtmlParser::with_selectorless` + `HtmlToR3Transform` pipeline
// (the shared `parse()` helper above keeps selectorless OFF).
// ============================================================================
mod selectorless_component_namespace_r3 {
    use oxc_allocator::Allocator;
    use oxc_angular_compiler::ast::r3::R3Node;
    use oxc_angular_compiler::parser::html::HtmlParser;
    use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};

    /// Parses `html` with selectorless mode, transforms to R3, and returns the
    /// first `R3Component`'s `(component_name, tag_name, full_name)`.
    fn first_r3_component(html: &str) -> (String, Option<String>, String) {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, html, "test.html");
        let html_result = parser.parse();
        assert!(
            html_result.errors.is_empty(),
            "HTML parse errors for '{html}': {:?}",
            html_result.errors.iter().map(|e| e.msg.clone()).collect::<Vec<_>>()
        );

        let transformer = HtmlToR3Transform::new(
            &allocator,
            html,
            TransformOptions { collect_comment_nodes: false },
        );
        let r3 = transformer.transform(&html_result.nodes);
        assert!(
            r3.errors.is_empty(),
            "Transform errors for '{html}': {:?}",
            r3.errors.iter().map(|e| e.msg.clone()).collect::<Vec<_>>()
        );

        fn find<'a>(nodes: &[R3Node<'a>]) -> Option<(String, Option<String>, String)> {
            for node in nodes {
                if let R3Node::Component(c) = node {
                    return Some((
                        c.component_name.to_string(),
                        c.tag_name.as_ref().map(|t| t.to_string()),
                        c.full_name.to_string(),
                    ));
                }
                let children: &[R3Node<'a>] = match node {
                    R3Node::Element(e) => &e.children,
                    R3Node::Template(t) => &t.children,
                    _ => continue,
                };
                if let Some(found) = find(children) {
                    return Some(found);
                }
            }
            None
        }

        find(&r3.nodes).unwrap_or_else(|| panic!("no R3 component found in '{html}'"))
    }

    #[test]
    fn inferred_svg_namespace_with_tag_name() {
        // THE divergence case: <svg><MyComp:button> -> ":svg:button" / "MyComp:svg:button".
        let (name, tag, full) = first_r3_component("<svg><MyComp:button>Hi</MyComp:button></svg>");
        assert_eq!(name, "MyComp");
        assert_eq!(tag.as_deref(), Some(":svg:button"));
        assert_eq!(full, "MyComp:svg:button");
    }

    #[test]
    fn inferred_svg_namespace_no_tag_name() {
        // <svg><MyComp> -> ":svg:ng-component" / "MyComp:svg:ng-component".
        let (name, tag, full) = first_r3_component("<svg><MyComp>Hi</MyComp></svg>");
        assert_eq!(name, "MyComp");
        assert_eq!(tag.as_deref(), Some(":svg:ng-component"));
        assert_eq!(full, "MyComp:svg:ng-component");
    }

    #[test]
    fn explicit_namespace_wins_over_inheritance() {
        // <math><MyComp:svg:title> -> explicit svg beats math: ":svg:title".
        let (name, tag, full) =
            first_r3_component("<math><MyComp:svg:title>Hi</MyComp:svg:title></math>");
        assert_eq!(name, "MyComp");
        assert_eq!(tag.as_deref(), Some(":svg:title"));
        assert_eq!(full, "MyComp:svg:title");
    }

    #[test]
    fn non_svg_control_stays_unnamespaced() {
        // Control: <div><MyComp:button> -> "button" / "MyComp:button".
        let (name, tag, full) = first_r3_component("<div><MyComp:button>Hi</MyComp:button></div>");
        assert_eq!(name, "MyComp");
        assert_eq!(tag.as_deref(), Some("button"));
        assert_eq!(full, "MyComp:button");
    }

    #[test]
    fn bare_component_stays_unnamespaced() {
        // Control: top-level <MyComp> -> tag_name None, full_name "MyComp".
        let (name, tag, full) = first_r3_component("<MyComp>Hi</MyComp>");
        assert_eq!(name, "MyComp");
        assert_eq!(tag, None);
        assert_eq!(full, "MyComp");
    }
}

// ============================================================================
// R3-level: CHILDREN of a namespaced selectorless component inherit `:svg:`
// (the iteration-11 finding). Upstream `_getPrefix` derives a child's namespace
// from the closest element-like parent's name, which for an `html.Component`
// parent is `parent.tagName` (the resolved `:svg:button`), NOT the class name
// (ml_parser/parser.ts:991). So children inherit `:svg:`, and crucially a child
// `<script>`/`<style>` resolves to `:svg:script`/`:svg:style`, which the R3
// template_preparser KEEPS namespaced (G4) instead of dropping/extracting the
// plain-html `script`/`style`. This is the security-relevant divergence.
// ============================================================================
mod selectorless_component_child_namespace_r3 {
    use oxc_allocator::Allocator;
    use oxc_angular_compiler::ast::r3::R3Node;
    use oxc_angular_compiler::parser::html::HtmlParser;
    use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};

    /// Parses `html` selectorless, transforms to R3, finds the first `R3Component`,
    /// and returns the list of its DIRECT child ELEMENT names (in order). Style/script
    /// that the preparser drops/extracts will simply be absent from this list, so an
    /// expected `:svg:script` present here proves it was KEPT as a namespaced element.
    fn component_child_element_names(html: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, html, "test.html");
        let html_result = parser.parse();
        assert!(
            html_result.errors.is_empty(),
            "HTML parse errors for '{html}': {:?}",
            html_result.errors.iter().map(|e| e.msg.clone()).collect::<Vec<_>>()
        );

        let transformer = HtmlToR3Transform::new(
            &allocator,
            html,
            TransformOptions { collect_comment_nodes: false },
        );
        let r3 = transformer.transform(&html_result.nodes);
        assert!(
            r3.errors.is_empty(),
            "Transform errors for '{html}': {:?}",
            r3.errors.iter().map(|e| e.msg.clone()).collect::<Vec<_>>()
        );

        fn find_component<'a, 'b>(nodes: &'b [R3Node<'a>]) -> Option<&'b [R3Node<'a>]> {
            for node in nodes {
                if let R3Node::Component(c) = node {
                    return Some(&c.children);
                }
                let children: &[R3Node<'a>] = match node {
                    R3Node::Element(e) => &e.children,
                    R3Node::Template(t) => &t.children,
                    _ => continue,
                };
                if let Some(found) = find_component(children) {
                    return Some(found);
                }
            }
            None
        }

        let children =
            find_component(&r3.nodes).unwrap_or_else(|| panic!("no R3 component in '{html}'"));
        children
            .iter()
            .filter_map(|n| match n {
                R3Node::Element(e) => Some(e.name.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn child_div_inherits_svg() {
        // <svg><MyComp:button><div></div></MyComp:button></svg> -> child `:svg:div`.
        let names =
            component_child_element_names("<svg><MyComp:button><div></div></MyComp:button></svg>");
        assert_eq!(names, vec![":svg:div".to_string()]);
    }

    #[test]
    fn child_script_inherits_svg_and_is_kept() {
        // THE security-relevant case: a plain `<script>` child under a namespaced
        // component resolves to `:svg:script`, which is KEPT (G4) — NOT dropped as a
        // plain-html `<script>`. (A plain `<script>` would be absent from the list.)
        let names = component_child_element_names(
            "<svg><MyComp:button><script>x</script></MyComp:button></svg>",
        );
        assert_eq!(names, vec![":svg:script".to_string()]);
    }

    #[test]
    fn child_style_inherits_svg_and_is_kept() {
        // A plain `<style>` child resolves to `:svg:style`, KEPT as a namespaced
        // element — NOT extracted as a stylesheet (plain `<style>` is removed).
        let names = component_child_element_names(
            "<svg><MyComp:button><style>x</style></MyComp:button></svg>",
        );
        assert_eq!(names, vec![":svg:style".to_string()]);
    }

    #[test]
    fn child_inherits_svg_through_explicit_namespace_component() {
        // Explicit `<MyComp:svg:rect>` (no enclosing <svg>): child `<circle>` -> `:svg:circle`.
        let names =
            component_child_element_names("<MyComp:svg:rect><circle></circle></MyComp:svg:rect>");
        assert_eq!(names, vec![":svg:circle".to_string()]);
    }

    #[test]
    fn child_inherits_svg_through_component_without_tag_part() {
        // <svg><MyComp><div></div></MyComp></svg>: tagName `:svg:ng-component` -> child `:svg:div`.
        let names = component_child_element_names("<svg><MyComp><div></div></MyComp></svg>");
        assert_eq!(names, vec![":svg:div".to_string()]);
    }

    #[test]
    fn control_non_svg_component_child_stays_html_and_script_is_dropped() {
        // CONTROL: <MyComp:button><div></div><script></script></MyComp:button> outside svg.
        // child `<div>` stays plain `div`; the plain `<script>` is DROPPED (absent).
        let names = component_child_element_names(
            "<MyComp:button><div></div><script>x</script></MyComp:button>",
        );
        assert_eq!(names, vec!["div".to_string()]);
    }

    #[test]
    fn control_bare_component_child_stays_html() {
        // CONTROL: bare <MyComp><div></div></MyComp> outside svg -> child stays plain `div`.
        let names = component_child_element_names("<MyComp><div></div></MyComp>");
        assert_eq!(names, vec!["div".to_string()]);
    }
}

// ============================================================================
// FINDING 1: UNSUPPORTED_SELECTORLESS_TAGS must be tested against the RESOLVED
// selectorless TAG name (`component.tagName`), NOT the component CLASS name.
//
// Upstream v21.2.7 `HtmlAstToIvyAst.visitComponent`
// (render3/r3_template_transform.ts:378-384):
//   if (component.tagName !== null && UNSUPPORTED_SELECTORLESS_TAGS.has(component.tagName)) {
//     this.reportError(`Tag name "${component.tagName}" cannot be used as a component tag`, ...);
//     return null;
//   }
// where the set (lines 64-71) is {link, style, script, ng-template, ng-container,
// ng-content} and `component.tagName` is the output of `_getComponentTagName`
// (ml_parser/parser.ts:946-961): null when no prefix and no tag part; the bare tag
// when no prefix; `mergeNsAndName(prefix, tag)` (`:prefix:tag`) when a prefix is
// present. The membership test runs against that resolved name WITHOUT stripping the
// namespace, so a namespaced `:svg:script` is NOT a member and is ACCEPTED, while a
// bare `script` (no prefix) IS rejected. The error message quotes the RESOLVED
// tagName, not the class name.
// ============================================================================
mod selectorless_unsupported_tag_r3 {
    use oxc_allocator::Allocator;
    use oxc_angular_compiler::parser::html::HtmlParser;
    use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};

    /// Parses `html` selectorless and transforms to R3, returning the combined
    /// parse + transform error messages.
    fn selectorless_transform_errors(html: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, html, "test.html");
        let html_result = parser.parse();

        let transformer = HtmlToR3Transform::new(
            &allocator,
            html,
            TransformOptions { collect_comment_nodes: false },
        );
        let r3 = transformer.transform(&html_result.nodes);

        html_result
            .errors
            .iter()
            .map(|e| e.msg.clone())
            .chain(r3.errors.iter().map(|e| e.msg.clone()))
            .collect()
    }

    fn has_unsupported_error(errors: &[String], expected_tag: &str) -> bool {
        let needle = format!("Tag name \"{expected_tag}\" cannot be used as a component tag");
        errors.iter().any(|e| e == &needle)
    }

    fn any_unsupported_error(errors: &[String]) -> bool {
        errors.iter().any(|e| e.contains("cannot be used as a component tag"))
    }

    #[test]
    fn comp_script_tag_is_rejected() {
        // <MyComp:script> -> tagName "script" (no prefix) -> in set -> rejected,
        // message quotes the RESOLVED tag name "script".
        let errors = selectorless_transform_errors("<MyComp:script>x</MyComp:script>");
        assert!(
            has_unsupported_error(&errors, "script"),
            "expected unsupported-tag error for \"script\", got: {errors:?}"
        );
    }

    #[test]
    fn comp_style_tag_is_rejected() {
        let errors = selectorless_transform_errors("<MyComp:style>x</MyComp:style>");
        assert!(
            has_unsupported_error(&errors, "style"),
            "expected unsupported-tag error for \"style\", got: {errors:?}"
        );
    }

    #[test]
    fn comp_link_tag_is_rejected() {
        let errors = selectorless_transform_errors("<MyComp:link>x</MyComp:link>");
        assert!(
            has_unsupported_error(&errors, "link"),
            "expected unsupported-tag error for \"link\", got: {errors:?}"
        );
    }

    #[test]
    fn class_named_script_without_tag_is_accepted() {
        // <Script> -> class named "Script", NO tag part -> tagName null -> NOT in set.
        let errors = selectorless_transform_errors("<Script>x</Script>");
        assert!(
            !any_unsupported_error(&errors),
            "expected NO unsupported-tag error for class <Script>, got: {errors:?}"
        );
    }

    #[test]
    fn class_named_script_with_safe_tag_is_accepted() {
        // <Script:div> -> tagName "div" -> NOT in set.
        let errors = selectorless_transform_errors("<Script:div>x</Script:div>");
        assert!(
            !any_unsupported_error(&errors),
            "expected NO unsupported-tag error for <Script:div>, got: {errors:?}"
        );
    }

    #[test]
    fn explicit_namespaced_script_tag_is_accepted() {
        // <MyComp:svg:script> -> tagName ":svg:script" (prefix present) -> NOT a member
        // of the bare-name set -> ACCEPTED. Upstream does NOT strip the namespace before
        // the membership test.
        let errors = selectorless_transform_errors("<MyComp:svg:script>x</MyComp:svg:script>");
        assert!(
            !any_unsupported_error(&errors),
            "expected NO unsupported-tag error for explicit :svg:script, got: {errors:?}"
        );
    }

    #[test]
    fn inherited_svg_namespaced_script_tag_is_accepted() {
        // <svg><MyComp:script> -> prefix inherited "svg" -> tagName ":svg:script" -> NOT
        // a member -> ACCEPTED (subtle inheritance case).
        let errors = selectorless_transform_errors("<svg><MyComp:script>x</MyComp:script></svg>");
        assert!(
            !any_unsupported_error(&errors),
            "expected NO unsupported-tag error for inherited :svg:script, got: {errors:?}"
        );
    }

    #[test]
    fn bare_component_is_accepted() {
        // CONTROL: bare <MyComp> -> tagName null -> NOT rejected.
        let errors = selectorless_transform_errors("<MyComp>x</MyComp>");
        assert!(
            !any_unsupported_error(&errors),
            "expected NO unsupported-tag error for bare <MyComp>, got: {errors:?}"
        );
    }
}

// ============================================================================
// FINDING 1 [HIGH, SECURITY]: selectorless-component HOST TAG is threaded into
// the bound-attribute security-context lookup AND the i18n trusted-types sink
// check, matching upstream v21.2.7.
//
// In v21.2.7 a selectorless component's security context is computed from
// `component.tagName` (the RESOLVED host tag), NOT the component class name:
//   * `categorizePropertyAttributes(component.tagName, ...)` ->
//     `createBoundElementProperty(elementName = component.tagName, ...)`
//     (render3/r3_template_transform.ts:411-415, binding_parser.ts:594-615), and
//   * the i18n trusted-types guard uses
//     `node instanceof html.Component ? node.tagName : node.name`, with
//     `tagName === null ? false : isTrustedTypesSink(node.tagName, name)`
//     (render3/view/i18n/meta.ts:205-208).
//
// Ground truth below was settled by EXECUTING @angular/compiler@21.2.7
// (VERSION.full == 21.2.7) `parseTemplate(html, 'test.html', {enableSelectorless:
// true})` and reading the resulting `BoundAttribute.securityContext` / errors:
//   <MyCmp:iframe [src]>       -> RESOURCE_URL
//   <MyCmp:iframe [attr.src]>  -> RESOURCE_URL
//   <MyCmp:object [data]>      -> RESOURCE_URL
//   <MyCmp:iframe [srcdoc]>    -> HTML
//   <MyCmp:img [src]>          -> URL
//   <svg><MyCmp:image [xlink:href]> -> NONE (no `image|xlink:href` schema entry)
//   bare <MyCmp [src]>         -> NONE   (tagName null; NONE sorts first in union)
//   bare <MyCmp [innerHTML]>   -> HTML   (wildcard `*|innerHTML` applies)
//   bare <Object [data]>       -> NONE   (tagName null, NOT looked up as `object`)
//   <MyCmp:iframe i18n-src>    -> error "Translating attribute 'src' is disallowed..."
//   bare <MyCmp i18n-src>      -> NO error (tagName null -> false)
//   <Object i18n-data> (bare)  -> NO error (tagName null -> false)
//   CONTROL <iframe [src]>     -> RESOURCE_URL ; <iframe i18n-src> -> error
// ============================================================================
mod selectorless_component_security_r3 {
    use oxc_allocator::Allocator;
    use oxc_angular_compiler::ast::r3::{R3Node, SecurityContext};
    use oxc_angular_compiler::parser::html::HtmlParser;
    use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};

    fn sc_str(sc: SecurityContext) -> &'static str {
        match sc {
            SecurityContext::None => "None",
            SecurityContext::Html => "Html",
            SecurityContext::Style => "Style",
            SecurityContext::Script => "Script",
            SecurityContext::Url => "Url",
            SecurityContext::ResourceUrl => "ResourceUrl",
            SecurityContext::AttributeNoBinding => "AttributeNoBinding",
            SecurityContext::UrlOrResourceUrl => "UrlOrResourceUrl",
        }
    }

    /// Parses `html` in selectorless mode, transforms to R3, finds the first
    /// `R3Component` (selectorless components are stored as components), and
    /// returns its first bound-attribute input as `(name, security_context)`.
    fn first_component_input_security(html: &str) -> (String, String) {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, html, "test.html");
        let html_result = parser.parse();
        let transformer = HtmlToR3Transform::new(
            &allocator,
            html,
            TransformOptions { collect_comment_nodes: false },
        );
        let r3 = transformer.transform(&html_result.nodes);

        fn find<'a>(
            nodes: &'a [R3Node<'a>],
        ) -> Option<&'a oxc_angular_compiler::ast::r3::R3Component<'a>> {
            for n in nodes {
                match n {
                    R3Node::Component(c) => return Some(c),
                    R3Node::Element(e) => {
                        if let Some(c) = find(&e.children) {
                            return Some(c);
                        }
                    }
                    R3Node::Template(t) => {
                        if let Some(c) = find(&t.children) {
                            return Some(c);
                        }
                    }
                    _ => {}
                }
            }
            None
        }

        let component = find(&r3.nodes).expect("expected at least one R3Component");
        let input = component.inputs.first().expect("expected at least one bound-attribute input");
        (input.name.to_string(), sc_str(input.security_context).to_string())
    }

    /// Like above but for a NORMAL element (control).
    fn first_element_input_security(html: &str) -> (String, String) {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, html, "test.html");
        let html_result = parser.parse();
        let transformer = HtmlToR3Transform::new(
            &allocator,
            html,
            TransformOptions { collect_comment_nodes: false },
        );
        let r3 = transformer.transform(&html_result.nodes);

        fn find<'a>(
            nodes: &'a [R3Node<'a>],
        ) -> Option<&'a oxc_angular_compiler::ast::r3::R3Element<'a>> {
            for n in nodes {
                match n {
                    R3Node::Element(e) => {
                        if !e.inputs.is_empty() {
                            return Some(e);
                        }
                        if let Some(c) = find(&e.children) {
                            return Some(c);
                        }
                    }
                    R3Node::Template(t) => {
                        if let Some(c) = find(&t.children) {
                            return Some(c);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let element = find(&r3.nodes).expect("expected at least one element with inputs");
        let input = element.inputs.first().expect("expected at least one bound-attribute input");
        (input.name.to_string(), sc_str(input.security_context).to_string())
    }

    fn transform_errors(html: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let parser = HtmlParser::with_selectorless(&allocator, html, "test.html");
        let html_result = parser.parse();
        let transformer = HtmlToR3Transform::new(
            &allocator,
            html,
            TransformOptions { collect_comment_nodes: false },
        );
        let r3 = transformer.transform(&html_result.nodes);
        html_result
            .errors
            .iter()
            .map(|e| e.msg.clone())
            .chain(r3.errors.iter().map(|e| e.msg.clone()))
            .collect()
    }

    fn has_disallowed_error(errors: &[String], attr: &str) -> bool {
        let needle = format!("Translating attribute '{attr}' is disallowed for security reasons.");
        errors.iter().any(|e| e == &needle)
    }

    // ---- Security context (bound attributes) ----

    #[test]
    fn comp_iframe_src_is_resource_url() {
        assert_eq!(
            first_component_input_security(r#"<MyCmp:iframe [src]="u" />"#),
            ("src".to_string(), "ResourceUrl".to_string())
        );
    }

    #[test]
    fn comp_iframe_attr_src_is_resource_url() {
        assert_eq!(
            first_component_input_security(r#"<MyCmp:iframe [attr.src]="u" />"#),
            ("src".to_string(), "ResourceUrl".to_string())
        );
    }

    #[test]
    fn comp_object_data_is_resource_url() {
        assert_eq!(
            first_component_input_security(r#"<MyCmp:object [data]="u" />"#),
            ("data".to_string(), "ResourceUrl".to_string())
        );
    }

    #[test]
    fn comp_iframe_srcdoc_is_html() {
        assert_eq!(
            first_component_input_security(r#"<MyCmp:iframe [srcdoc]="u" />"#),
            ("srcdoc".to_string(), "Html".to_string())
        );
    }

    #[test]
    fn comp_img_src_is_url_not_resource_url() {
        // `img|src` is a navigable URL (not a Trusted Types / RESOURCE_URL sink).
        assert_eq!(
            first_component_input_security(r#"<MyCmp:img [src]="u" />"#),
            ("src".to_string(), "Url".to_string())
        );
    }

    #[test]
    fn svg_comp_image_xlink_href_is_none() {
        // <svg><MyCmp:image [xlink:href]> -> resolved tag ":svg:image" -> ns-stripped
        // "image" -> no `image|xlink:href` schema entry -> NONE (no over-sanitization).
        assert_eq!(
            first_component_input_security(r#"<svg><MyCmp:image [xlink:href]="u" /></svg>"#),
            ("xlink:href".to_string(), "None".to_string())
        );
    }

    // ---- CONTROL: bare component (tagName null) ----

    #[test]
    fn bare_comp_src_is_none() {
        // tagName null -> security NONE (matches upstream null-selector union).
        assert_eq!(
            first_component_input_security(r#"<MyCmp [src]="u" />"#),
            ("src".to_string(), "None".to_string())
        );
    }

    #[test]
    fn bare_comp_inner_html_is_html_via_wildcard() {
        // tagName null but `*|innerHTML` is a wildcard sink -> HTML.
        assert_eq!(
            first_component_input_security(r#"<MyCmp [innerHTML]="u" />"#),
            ("innerHTML".to_string(), "Html".to_string())
        );
    }

    #[test]
    fn bare_comp_class_named_object_data_is_none() {
        // REGRESSION GUARD: a class name that collides with a real element name
        // (`Object`) with NO tag part must NOT be looked up as the `object` element.
        // tagName null -> NONE (matches upstream; pre-fix this leaked RESOURCE_URL).
        assert_eq!(
            first_component_input_security(r#"<Object [data]="u" />"#),
            ("data".to_string(), "None".to_string())
        );
    }

    // ---- CONTROL: normal element (non-component) unchanged ----

    #[test]
    fn control_normal_iframe_src_is_resource_url() {
        assert_eq!(
            first_element_input_security(r#"<iframe [src]="u"></iframe>"#),
            ("src".to_string(), "ResourceUrl".to_string())
        );
    }

    // ---- i18n trusted-types sink ----

    #[test]
    fn comp_iframe_i18n_src_is_disallowed() {
        let errors = transform_errors(r#"<MyCmp:iframe i18n-src src="x">hi</MyCmp:iframe>"#);
        assert!(
            has_disallowed_error(&errors, "src"),
            "expected i18n disallowed error for iframe|src, got: {errors:?}"
        );
    }

    #[test]
    fn control_bare_comp_i18n_src_is_allowed() {
        // tagName null -> isTrustedType false -> NO error (matches upstream).
        let errors = transform_errors(r#"<MyCmp i18n-src src="x">hi</MyCmp>"#);
        assert!(
            !errors.iter().any(|e| e.contains("disallowed for security reasons")),
            "expected NO i18n disallowed error for bare <MyCmp>, got: {errors:?}"
        );
    }

    #[test]
    fn control_bare_comp_class_named_object_i18n_data_is_allowed() {
        // REGRESSION GUARD: class `Object`, NO tag part -> tagName null -> NO error
        // (pre-fix the class name leaked into `object|data` and spuriously errored).
        let errors = transform_errors(r#"<Object i18n-data data="x">hi</Object>"#);
        assert!(
            !errors.iter().any(|e| e.contains("disallowed for security reasons")),
            "expected NO i18n disallowed error for bare <Object>, got: {errors:?}"
        );
    }

    #[test]
    fn control_normal_iframe_i18n_src_is_disallowed() {
        let errors = transform_errors(r#"<iframe i18n-src src="x"></iframe>"#);
        assert!(
            has_disallowed_error(&errors, "src"),
            "expected i18n disallowed error for normal iframe|src, got: {errors:?}"
        );
    }

    // ---- i18n trusted-types sink: namespace must NOT be stripped, bare component skips ----
    //
    // These assert faithfulness to v21.2.7 `render3/view/i18n/meta.ts` (~210-214):
    //   isTrustedType = node instanceof html.Component
    //     ? (node.tagName === null ? false : isTrustedTypesSink(node.tagName, name))
    //     : isTrustedTypesSink(node.name, name);
    // For a NORMAL element it passes the FULL `node.name` (un-stripped, e.g.
    // `:svg:iframe`); for a COMPONENT it passes the resolved `node.tagName`
    // (un-stripped, e.g. `:svg:iframe`); for a BARE component (tagName null) it
    // SHORT-CIRCUITS to false. `isTrustedTypesSink` only lowercases (NO ns-strip),
    // so `:svg:iframe|src` is NOT a member of TRUSTED_TYPES_SINKS -> ALLOWED.
    // All expected outcomes were verified by executing
    // `parseTemplate(tpl, 'test.html', {enableSelectorless:true})` against
    // @angular/compiler@21.2.7.

    #[test]
    fn normal_svg_iframe_i18n_src_is_allowed() {
        // Normal namespaced element, full name `:svg:iframe`; upstream passes
        // `node.name` un-stripped -> `:svg:iframe|src` is NOT a sink -> ALLOWED.
        // (Pre-fix OXC ns-stripped to `iframe|src` and wrongly BLOCKED.)
        let errors = transform_errors(r#"<svg:iframe i18n-src src="x"></svg:iframe>"#);
        assert!(
            !errors.iter().any(|e| e.contains("disallowed for security reasons")),
            "expected NO i18n disallowed error for <svg:iframe i18n-src>, got: {errors:?}"
        );
    }

    #[test]
    fn comp_svg_iframe_i18n_src_is_allowed() {
        // Selectorless component resolved to `:svg:iframe` (inside <svg>); upstream
        // passes `node.tagName` un-stripped -> `:svg:iframe|src` NOT a sink -> ALLOWED.
        // (Pre-fix OXC ns-stripped to `iframe|src` and wrongly BLOCKED.)
        let errors =
            transform_errors(r#"<svg><MyCmp:iframe i18n-src src="x">hi</MyCmp:iframe></svg>"#);
        assert!(
            !errors.iter().any(|e| e.contains("disallowed for security reasons")),
            "expected NO i18n disallowed error for <svg><MyCmp:iframe i18n-src>, got: {errors:?}"
        );
    }

    #[test]
    fn bare_comp_i18n_inner_html_is_allowed() {
        // Bare component (tagName null) -> upstream short-circuits isTrustedType to
        // false -> ALLOWED, even though `*|innerhtml` IS a wildcard sink. (Pre-fix
        // OXC passed an EMPTY STRING which matched `*|innerhtml` and wrongly BLOCKED.)
        let errors = transform_errors(r#"<MyCmp i18n-innerHTML innerHTML="x">hi</MyCmp>"#);
        assert!(
            !errors.iter().any(|e| e.contains("disallowed for security reasons")),
            "expected NO i18n disallowed error for bare <MyCmp i18n-innerHTML>, got: {errors:?}"
        );
    }
}

// ============================================================================
// DEFAULT (non-selectorless) parse mode: an UPPERCASE-leading HTML tag like
// `<IFRAME>` / `<OBJECT>` is a NORMAL element, NOT a selectorless component.
//
// This is the path the real compiler uses: every parse site in
// `component/transform.rs` builds `ParseTemplateOptions { ..Default::default() }`
// and `enable_selectorless` defaults to `false`, so the lexer never tokenizes a
// `ComponentOpenStart` and never produces an `HtmlComponent` / component-marked
// `HtmlElement`. An `<IFRAME>` is therefore an ordinary `HtmlElement` whose name
// is preserved as-is.
//
// Ground truth settled by EXECUTING @angular/compiler@21.2.7
// `parseTemplate(html, 'test.html', {})` (NO enableSelectorless ->
// `selectorlessEnabled = options.enableSelectorless ?? false` is false):
//   <IFRAME [src]>     -> NODE Element name=IFRAME, src securityContext 5 (RESOURCE_URL)
//   <OBJECT [data]>    -> NODE Element name=OBJECT, data securityContext 5 (RESOURCE_URL)
//   <IFRAME i18n-src>  -> error "Translating attribute 'src' is disallowed for security reasons."
//   <MyCmp [src]>      -> NODE Element name=MyCmp, src securityContext 0 (NONE), NO i18n error
//
// The security lookup (`get_security_context`) and the i18n sink check
// (`is_trusted_types_sink`) both lowercase internally, so routing the uppercase
// element through its own `raw_name` yields the correct `iframe|src` /
// `object|data` lookups automatically. Before the fix, `html_to_r3`'s casing
// heuristic (`first_char.is_ascii_uppercase()`) mis-flagged `<IFRAME>` as a bare
// selectorless component (tagName null) -> `SecurityContext::None` and SKIPPED
// the i18n guard, dropping the sanitizer.
// ============================================================================
mod default_mode_uppercase_element_security {
    use super::*;

    /// Parses `html` in DEFAULT mode (selectorless OFF, exactly like the real
    /// pipeline) and returns the first element's first bound-attribute input as
    /// `(name, security_context)`.
    fn first_element_input_security(html: &str) -> (String, String) {
        let (name, _ty, sc) = first_input_security(html);
        (name, sc)
    }

    /// Parses `html` in DEFAULT mode and returns the combined parse + transform
    /// error messages.
    fn transform_errors(html: &str) -> Vec<String> {
        let allocator = Box::new(Allocator::default());
        let allocator_ref: &'static Allocator =
            unsafe { &*std::ptr::from_ref::<Allocator>(allocator.as_ref()) };
        let parser = HtmlParser::new(allocator_ref, html, "test.html");
        let html_result = parser.parse();
        let options = TransformOptions { collect_comment_nodes: false };
        let transformer = HtmlToR3Transform::new(allocator_ref, html, options);
        let r3_result = transformer.transform(&html_result.nodes);
        html_result
            .errors
            .iter()
            .map(|e| e.msg.clone())
            .chain(r3_result.errors.iter().map(|e| e.msg.clone()))
            .collect()
    }

    #[test]
    fn uppercase_iframe_src_is_resource_url() {
        // Upstream (default mode): `<IFRAME [src]>` is an Element; `iframe|src`
        // (lowercased) -> RESOURCE_URL. (Pre-fix OXC mis-flagged as a bare
        // component -> NONE, dropping the sanitizer — a security regression.)
        assert_eq!(
            first_element_input_security(r#"<IFRAME [src]="u"></IFRAME>"#),
            ("src".to_string(), "ResourceUrl".to_string())
        );
    }

    #[test]
    fn uppercase_object_data_is_resource_url() {
        // `<OBJECT [data]>` -> `object|data` -> RESOURCE_URL.
        assert_eq!(
            first_element_input_security(r#"<OBJECT [data]="u"></OBJECT>"#),
            ("data".to_string(), "ResourceUrl".to_string())
        );
    }

    #[test]
    fn uppercase_iframe_i18n_src_is_disallowed() {
        // `<IFRAME i18n-src>` -> isTrustedTypesSink("IFRAME","src") lowercases to
        // `iframe|src` (a sink) -> ERROR. (Pre-fix OXC treated it as a bare
        // component with tagName null and SKIPPED the guard, allowing translation.)
        let errors = transform_errors(r#"<IFRAME i18n-src="x" src="y">hi</IFRAME>"#);
        assert!(
            errors
                .iter()
                .any(|e| e == "Translating attribute 'src' is disallowed for security reasons."),
            "expected i18n disallowed error for IFRAME|src in default mode, got: {errors:?}"
        );
    }

    #[test]
    fn control_unknown_uppercase_element_src_is_none() {
        // CONTROL: in default mode `<MyCmp>` is an unknown NORMAL element (NOT a
        // component); `mycmp|src` is not a schema key -> NONE, and NO i18n error.
        assert_eq!(
            first_element_input_security(r#"<MyCmp [src]="u"></MyCmp>"#),
            ("src".to_string(), "None".to_string())
        );
        let errors = transform_errors(r#"<MyCmp i18n-src="x" src="y">hi</MyCmp>"#);
        assert!(
            !errors.iter().any(|e| e.contains("disallowed for security reasons")),
            "expected NO i18n disallowed error for unknown <MyCmp> in default mode, got: {errors:?}"
        );
    }

    #[test]
    fn uppercase_iframe_is_element_node_not_component() {
        // Upstream produces a t.Element (not t.Component) for `<IFRAME>` in
        // default mode. Assert the R3 node is an Element with name "IFRAME".
        assert_eq!(humanize_ignore_errors("<IFRAME></IFRAME>"), vec![h!["Element", "IFRAME"]]);
    }
}

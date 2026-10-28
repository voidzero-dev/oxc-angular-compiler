//! ngtsc's implicit signal `debugName` transform
//! (`ngtsc/transform/src/implicit_signal_debug_name_transform.ts`).
//!
//! ngtsc always adds `signalMetadataTransform` to its `before` emit
//! transforms, in full and partial compilation alike, and it runs on every
//! emitted file, with or without an Angular decorator. It names the signal
//! created by a `const x = signal(0)`, a `x = signal(0)` class field or a
//! `this.x = signal(0)` statement after its binding, for dev mode only:
//!
//! ```text
//! signal(0, /* @ts-ignore */ ...(ngDevMode ? [{ debugName: "x" }] : /* istanbul ignore next */ []))
//! input(0, { ...(ngDevMode ? { debugName: "x" } : /* istanbul ignore next */ {}), alias: 'a' })
//! ```
//!
//! The arguments are edited in the source text, so the call keeps the rest
//! of its formatting.

use std::borrow::Cow;
use std::cell::RefCell;

use oxc_ast::ast::{
    AccessorProperty, Argument, AssignmentOperator, AssignmentTarget, CallExpression, ChainElement,
    Comment, Expression, ExpressionStatement, IdentifierReference, ImportDeclarationSpecifier,
    ObjectPropertyKind, Program, PropertyDefinition, PropertyKey, PropertyKind, Statement,
    VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashSet;

use crate::optimizer::Edit;
use crate::output::emitter::escape_string;

/// ngtsc's `signalFunctions`: each function and the `@angular/*` package it
/// must be imported from.
const SIGNAL_FUNCTIONS: [(&str, &str); 13] = [
    ("signal", "core"),
    ("computed", "core"),
    ("linkedSignal", "core"),
    ("input", "core"),
    ("model", "core"),
    ("viewChild", "core"),
    ("viewChildren", "core"),
    ("contentChild", "core"),
    ("contentChildren", "core"),
    ("effect", "core"),
    ("afterRenderEffect", "core"),
    ("resource", "core"),
    ("httpResource", "common"),
];

/// The `debugName` edits for every signal the file creates, in source order.
///
/// The transform's checks (`isVariableDeclarationCase` and friends) match
/// the callee by its local name only, then `expressionIsUsingAngularImportedSymbol`
/// asks the type checker for that name's declaration. Here the semantic
/// model resolves the callee to a named import, so a shadowing local or a
/// differently named alias (`computed as c`) is skipped like upstream.
///
/// `jit_signal_inputs` are the `input()` fields ngtsc's JIT
/// `signalInputsTransform` rebuilds first, under a synthesized name: there
/// `node.name.getText()` throws and the field is left as is.
pub fn collect_signal_debug_names(
    program: &Program<'_>,
    source: &str,
    scoping: &Scoping,
    jit_signal_inputs: &FxHashSet<Span>,
) -> SignalDebugNames {
    let imports = signal_function_imports(program);
    // Nothing in the file can match: skip the walk.
    if imports.is_empty() {
        return SignalDebugNames::default();
    }
    let mut visitor = SignalDebugNameVisitor {
        source,
        comments: &program.comments,
        scoping,
        imports,
        jit_signal_inputs,
        names: SignalDebugNames::default(),
    };
    visitor.visit_program(program);
    visitor.names
}

/// The `debugName` edits of a file, each with the declaration it names.
///
/// Upstream edits the nodes, so every place that prints a node again (the
/// decorator arguments in `ɵcmp` and `ɵsetClassMetadata`, a JIT
/// `__decorate` call) prints the named call. Here the edits apply to the
/// source text: a slice printed elsewhere takes the edits of the
/// declarations inside it ([`source_text`]), and the file keeps the rest
/// ([`SignalDebugNames::into_file_edits`]).
#[derive(Default)]
pub struct SignalDebugNames {
    /// The span of the declaration (variable declarator, `a.b = call()`
    /// statement or field) and the edit to its call.
    edits: Vec<(Span, Edit)>,
}

impl SignalDebugNames {
    /// The edits of the declarations inside `span`, relative to its start.
    fn edits_within(&self, span: Span) -> Vec<Edit> {
        self.edits
            .iter()
            .filter(|(owner, _)| span.start <= owner.start && owner.end <= span.end)
            .map(|(_, edit)| Edit {
                start: edit.start - span.start,
                end: edit.end - span.start,
                ..edit.clone()
            })
            .collect()
    }

    /// The edits the file itself takes: those in text another edit removes
    /// or replaces (a decorator, a hoisted statement) are left out, since
    /// that text is printed again from [`source_text`] where it is kept.
    pub fn into_file_edits(self, edits: &[Edit]) -> Vec<Edit> {
        self.edits
            .into_iter()
            .map(|(_, edit)| edit)
            .filter(|edit| {
                let is_removed = |other: &Edit| {
                    let removes_text = other.start < other.end;
                    removes_text && other.start < edit.end && edit.start < other.end
                };
                !edits.iter().any(is_removed)
            })
            .collect()
    }
}

thread_local! {
    /// The `debugName`s of the file being compiled on this thread, keyed by
    /// its source text's address and length (see [`FileDebugNames`]).
    static FILE_DEBUG_NAMES: RefCell<Option<((usize, usize), SignalDebugNames)>> =
        const { RefCell::new(None) };
}

/// Tells the compiled file's text from any other text the compiler slices.
fn source_key(source: &str) -> (usize, usize) {
    (source.as_ptr() as usize, source.len())
}

/// Makes a file's `debugName`s visible to [`source_text`] and
/// [`source_edits`] while the file compiles. Decorator metadata reaches the
/// source text through many layers (`make_raw_source` among them), and only
/// slices of this very text take the edits.
pub struct FileDebugNames {
    previous: Option<((usize, usize), SignalDebugNames)>,
}

impl FileDebugNames {
    pub fn enter(source: &str, names: SignalDebugNames) -> Self {
        let previous =
            FILE_DEBUG_NAMES.with(|file| file.replace(Some((source_key(source), names))));
        Self { previous }
    }

    /// Ends the scope and hands back the `debugName`s for the file's own edits.
    pub fn finish(self) -> SignalDebugNames {
        let current = FILE_DEBUG_NAMES.with(|file| file.borrow_mut().take());
        // Restores the enclosing file's scope, if any.
        drop(self);
        current.map(|(_, names)| names).unwrap_or_default()
    }
}

impl Drop for FileDebugNames {
    fn drop(&mut self) {
        let previous = self.previous.take();
        FILE_DEBUG_NAMES.with(|file| *file.borrow_mut() = previous);
    }
}

/// `source[span]` for printing elsewhere, with the `debugName`s of the
/// declarations inside it.
pub fn source_text(source: &str, span: Span) -> Cow<'_, str> {
    let text = &source[span.start as usize..span.end as usize];
    let edits = source_edits(source, span);
    if edits.is_empty() {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(crate::optimizer::apply_edits(text, edits))
    }
}

/// The `debugName` edits inside `span` of `source`, relative to its start,
/// for a caller that edits the slice itself.
pub fn source_edits(source: &str, span: Span) -> Vec<Edit> {
    FILE_DEBUG_NAMES.with(|file| match &*file.borrow() {
        Some((key, names)) if *key == source_key(source) => names.edits_within(span),
        _ => Vec::new(),
    })
}

/// The bindings `expressionIsUsingAngularImportedSymbol` accepts: a named
/// import (`import { signal } from ...`) whose local name is one of
/// [`SIGNAL_FUNCTIONS`], from that function's package or one of its entry
/// points (`@angular/core/rxjs-interop`, `@angular/common/http`). Upstream
/// looks the package up by the local name, not the imported one.
fn signal_function_imports(program: &Program<'_>) -> FxHashSet<SymbolId> {
    let mut imports = FxHashSet::default();
    for stmt in &program.body {
        let Statement::ImportDeclaration(import) = stmt else { continue };
        let Some(module) = import.source.value.as_str() else { continue };
        let Some(package) = module.strip_prefix("@angular/") else { continue };
        let Some(specifiers) = &import.specifiers else { continue };
        for specifier in specifiers {
            let ImportDeclarationSpecifier::ImportSpecifier(specifier) = specifier else {
                continue;
            };
            let Some(&(_, owner)) =
                SIGNAL_FUNCTIONS.iter().find(|(name, _)| specifier.local.name == *name)
            else {
                continue;
            };
            let from_owner = package
                .strip_prefix(owner)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
            if from_owner && let Some(symbol_id) = specifier.local.symbol_id.get() {
                imports.insert(symbol_id);
            }
        }
    }
    imports
}

/// `signalMetadataTransform`'s visitor: a variable declaration, a
/// `a.b = call()` statement or a property declaration whose initializer
/// calls a signal function by name is transformed (or left alone when the
/// name isn't Angular's import) and its children are never visited. Any
/// other node is walked into.
struct SignalDebugNameVisitor<'s> {
    source: &'s str,
    comments: &'s [Comment],
    scoping: &'s Scoping,
    imports: FxHashSet<SymbolId>,
    jit_signal_inputs: &'s FxHashSet<Span>,
    names: SignalDebugNames,
}

impl<'a> Visit<'a> for SignalDebugNameVisitor<'_> {
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        // `isVariableDeclarationCase`
        let Some(call) = signal_function_call(it.init.as_ref()) else {
            walk::walk_variable_declarator(self, it);
            return;
        };
        // `node.name.getText()`
        let name = &self.source[it.id.span().start as usize..it.id.span().end as usize];
        self.transform(it.span, call, name);
    }

    fn visit_expression_statement(&mut self, it: &ExpressionStatement<'a>) {
        // `isPropertyAssignmentCase`: `<expr>.name = call()` as a statement.
        // The target drops its parentheses here, but `(this.x) = call()` has
        // a parenthesized left side upstream, which doesn't match.
        if let Expression::AssignmentExpression(assignment) = &it.expression
            && assignment.operator == AssignmentOperator::Assign
            && assignment.span.start == assignment.left.span().start
            && let Some(call) = signal_function_call(Some(&assignment.right))
        {
            // `node.expression.left.name.text`
            let name = match &assignment.left {
                AssignmentTarget::StaticMemberExpression(member) => {
                    Some(member.property.name.to_string())
                }
                AssignmentTarget::PrivateFieldExpression(field) => {
                    Some(format!("#{}", field.field.name))
                }
                _ => None,
            };
            if let Some(name) = name {
                self.transform(it.span, call, &name);
                return;
            }
        }
        walk::walk_expression_statement(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        // `isPropertyDeclarationCase`
        let Some(call) = signal_function_call(it.value.as_ref()) else {
            walk::walk_property_definition(self, it);
            return;
        };
        if self.jit_signal_inputs.contains(&it.span) {
            return;
        }
        let name = property_name_text(self.source, self.comments, &it.key, it.computed);
        self.transform(it.span, call, name);
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        // An `accessor` field is a property declaration upstream too.
        let Some(call) = signal_function_call(it.value.as_ref()) else {
            walk::walk_accessor_property(self, it);
            return;
        };
        let name = property_name_text(self.source, self.comments, &it.key, it.computed);
        self.transform(it.span, call, name);
    }
}

impl SignalDebugNameVisitor<'_> {
    /// `transformVariableDeclaration` / `transformPropertyAssignment` /
    /// `transformPropertyDeclaration`: only a call to Angular's own function
    /// gets the `debugName`.
    fn transform(&mut self, declaration: Span, call: &CallExpression<'_>, debug_name: &str) {
        let Some(callee) = signal_function_reference(call) else { return };
        let is_angular_import = callee
            .reference_id
            .get()
            .and_then(|reference_id| self.scoping.get_reference(reference_id).symbol_id())
            .is_some_and(|symbol_id| self.imports.contains(&symbol_id));
        if !is_angular_import {
            return;
        }
        if let Some(edit) = insert_debug_name_into_call_expression(self.source, call, debug_name) {
            self.names.edits.push((declaration, edit));
        }
    }
}

/// `node.name.getText()` of a property declaration: the name as written,
/// with the quotes of a string key and the brackets of a computed one (and
/// any comments inside them).
fn property_name_text<'s>(
    source: &'s str,
    comments: &[Comment],
    key: &PropertyKey<'_>,
    computed: bool,
) -> &'s str {
    let span = key.span();
    let (mut start, mut end) = (span.start as usize, span.end as usize);
    if computed {
        let bytes = source.as_bytes();
        let open = skip_trivia_backward(source, comments, start);
        let close = skip_trivia_forward(source, comments, end);
        if open > 0 && bytes[open - 1] == b'[' && bytes.get(close) == Some(&b']') {
            start = open - 1;
            end = close + 1;
        }
    }
    &source[start..end]
}

/// The offset before the whitespace and comments that end at `offset`.
fn skip_trivia_backward(source: &str, comments: &[Comment], mut offset: usize) -> usize {
    loop {
        offset = source[..offset].trim_end().len();
        match comments.binary_search_by_key(&offset, |comment| comment.span.end as usize) {
            Ok(index) => offset = comments[index].span.start as usize,
            Err(_) => return offset,
        }
    }
}

/// The offset after the whitespace and comments that start at `offset`.
fn skip_trivia_forward(source: &str, comments: &[Comment], mut offset: usize) -> usize {
    loop {
        offset = source.len() - source[offset..].trim_start().len();
        match comments.binary_search_by_key(&offset, |comment| comment.span.start as usize) {
            Ok(index) => offset = comments[index].span.end as usize,
            Err(_) => return offset,
        }
    }
}

/// The initializer when it calls a signal function by name: `isSignalFunction`
/// on the callee, or on the object of a `input.required` style callee.
fn signal_function_call<'b, 'a>(
    init: Option<&'b Expression<'a>>,
) -> Option<&'b CallExpression<'a>> {
    let call = match init? {
        Expression::CallExpression(call) => call,
        // `signal?.(0)` is a call expression upstream too.
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::CallExpression(call) => call,
            _ => return None,
        },
        _ => return None,
    };
    signal_function_reference(call).map(|_| call.as_ref())
}

/// The identifier naming the signal function: `signal` in `signal(...)` and
/// `input` in `input.required(...)`.
fn signal_function_reference<'b, 'a>(
    call: &'b CallExpression<'a>,
) -> Option<&'b IdentifierReference<'a>> {
    let expression = match &call.callee {
        Expression::StaticMemberExpression(member) => &member.object,
        callee => callee,
    };
    let Expression::Identifier(id) = expression else { return None };
    SIGNAL_FUNCTIONS.iter().any(|(name, _)| id.name == *name).then_some(id.as_ref())
}

/// `insertDebugNameIntoCallExpression`: the edit that passes `debugName` to
/// the call, or `None` when it is left as is.
///
/// The options object sits after the initial value, except for a call with
/// no arguments, `input.required` / `model.required`, and the functions that
/// take only an options object (`resource`, `linkedSignal({...})`). An
/// options object literal gets the `debugName` spread as its first property;
/// a missing one is appended as a spread argument. Anything else there, or an
/// options object that sets `debugName` itself, is left alone.
fn insert_debug_name_into_call_expression(
    source: &str,
    call: &CallExpression<'_>,
    debug_name: &str,
) -> Option<Edit> {
    let is_required_input = is_required_input_function(&call.callee);
    let has_no_args = call.arguments.is_empty();
    let options_first =
        has_no_args || is_signal_with_object_only_definition(source, call) || is_required_input;
    let config_position = usize::from(!options_first);
    let debug_name_property = format!("debugName: {}", escape_string(debug_name, false));

    match call.arguments.get(config_position) {
        Some(Argument::ObjectExpression(object)) => {
            let has_debug_name = object.properties.iter().any(|property| {
                matches!(property, ObjectPropertyKind::ObjectProperty(property)
                    if property.kind == PropertyKind::Init
                        && !property.method
                        && !property.shorthand
                        && !property.computed
                        && matches!(&property.key, PropertyKey::StaticIdentifier(key)
                            if key.name == "debugName"))
            });
            if has_debug_name {
                return None;
            }
            let spread = format!(
                "...{}",
                ng_dev_mode_conditional(&format!("{{ {debug_name_property} }}"), "{}")
            );
            Some(if object.properties.is_empty() {
                Edit::replace(object.span.start, object.span.end, format!("{{ {spread} }}"))
            } else {
                Edit::insert(object.span.start + 1, format!(" {spread},"))
            })
        }
        Some(_) => None,
        None => {
            let spread_args = if has_no_args && !is_required_input {
                format!("[undefined, {{ {debug_name_property} }}]")
            } else {
                format!("[{{ {debug_name_property} }}]")
            };
            let spread =
                format!("/* @ts-ignore */ ...{}", ng_dev_mode_conditional(&spread_args, "[]"));
            Some(match call.arguments.last() {
                Some(last) => Edit::insert(last.span().end, format!(", {spread}")),
                // Just inside the closing parenthesis.
                None => Edit::insert(call.span.end - 1, spread),
            })
        }
    }
}

/// `createNgDevModeConditional`
fn ng_dev_mode_conditional(dev_mode_expression: &str, prod_mode_expression: &str) -> String {
    format!(
        "(ngDevMode ? {dev_mode_expression} : /* istanbul ignore next */ {prod_mode_expression})"
    )
}

/// `isRequiredInputFunction`: `input.required` or `model.required`.
fn is_required_input_function(callee: &Expression<'_>) -> bool {
    let Expression::StaticMemberExpression(member) = callee else { return false };
    let Expression::Identifier(parent) = &member.object else { return false };
    member.property.name == "required" && (parent.name == "input" || parent.name == "model")
}

/// `isSignalWithObjectOnlyDefinition`: `resource(...)`, or `linkedSignal`
/// called with an object first, judged by the callee as written.
fn is_signal_with_object_only_definition(source: &str, call: &CallExpression<'_>) -> bool {
    let callee_text = &source[call.callee.span().start as usize..call.callee.span().end as usize];
    let is_computation_linked_signal = callee_text == "linkedSignal"
        && matches!(call.arguments.first(), Some(Argument::ObjectExpression(_)));
    is_computation_linked_signal || callee_text == "resource"
}

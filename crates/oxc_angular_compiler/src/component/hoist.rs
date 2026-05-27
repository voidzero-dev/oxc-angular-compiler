//! TDZ-safe hoisting of file-scope `const`/`let`/`var` bindings referenced by
//! Angular decorator metadata.
//!
//! When `@Component`, `@Directive`, `@NgModule`, or other Angular decorators
//! reference a top-level binding declared *after* the decorated class, the
//! emitted Ivy definition (e.g. `static ɵcmp = … ɵɵProvidersFeature([{
//! provide: TOKEN, … }])`) evaluates that reference eagerly at
//! class-definition time. Because the declaration is still in the temporal
//! dead zone, this throws `ReferenceError` at module load (issue #287).
//!
//! Angular's official compiler hoists such referenced declarations above the
//! decorated class. This module mirrors that behavior.
//!
//! The implementation is intentionally conservative:
//! * Only top-level `VariableDeclaration` statements are eligible for
//!   hoisting. Function declarations are already JS-hoisted with their
//!   bodies; class declarations are skipped because hoisting them would
//!   clobber other edits the transform pipeline applies to the same span.
//! * Bindings declared *before* the decorated class are never touched.
//! * Identifier collection walks decorator metadata eagerly but stops at
//!   function/arrow bodies and class expression bodies — references that
//!   only fire when a factory or method runs (e.g. `useFactory: () => DEP`)
//!   don't trigger a hoist.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, Class, Declaration, Decorator,
    ExportDefaultDeclarationKind, Expression, ObjectPropertyKind, Program, Statement,
};
use oxc_span::GetSpan;

use crate::optimizer::Edit;

/// One referenced-by-decorator top-level binding scheduled for hoisting.
#[derive(Clone, Copy)]
struct HoistEntry {
    /// Span of the statement to relocate.
    stmt_start: u32,
    stmt_end: u32,
    /// End of the deletion (extends `stmt_end` past trailing newline so the
    /// hoist doesn't leave a stray blank line behind).
    delete_end: u32,
    /// Insertion target — the earliest referencing class's effective start.
    insert_at: u32,
}

/// Build edits that hoist top-level bindings referenced by decorator metadata
/// of any class but declared *after* that class.
///
/// Returns a list of edits the caller appends to the wider edit set. Each
/// hoisted statement becomes a delete-at-original + insert-before-class pair.
/// Insert edits run at `HOIST_INSERT_PRIORITY` so they sort *after* the
/// existing `decls_before_class` insertion at the same offset; since
/// `apply_edits` applies higher-priority edits later — and each later
/// insertion at the same offset pushes earlier text further right — the
/// hoisted statements end up immediately above the class, with any
/// constant-pool declarations from the compiler in between.
pub fn collect_hoist_edits<'a>(program: &Program<'a>, source: &str) -> Vec<Edit> {
    let bindings = collect_top_level_bindings(program, source);
    if bindings.is_empty() {
        return Vec::new();
    }

    // For each top-level decorated class, find the identifiers eagerly
    // referenced in its decorator metadata. Record the earliest such class
    // position per referenced binding so multiple references hoist exactly
    // once, ahead of the first user.
    let mut plan: HashMap<&'a str, HoistEntry> = HashMap::new();

    for stmt in &program.body {
        let Some((class, stmt_start)) = class_of(stmt) else { continue };

        // Skip classes that don't carry any Angular decorator we care about.
        // Walking every class would be safe but wastes work on unrelated code.
        if !has_angular_decorator(class) {
            continue;
        }

        let mut referenced: HashSet<&'a str> = HashSet::new();
        for decorator in &class.decorators {
            collect_decorator_idents(decorator, &mut referenced);
        }

        if referenced.is_empty() {
            continue;
        }

        let class_body_end = class.body.span.end;
        let effective_start = effective_class_start(class, stmt_start);

        for name in referenced {
            let Some(info) = bindings.get(name) else { continue };
            // Only hoist declarations that start AFTER the class body ends.
            // Anything before is already TDZ-safe.
            if info.stmt_start <= class_body_end {
                continue;
            }

            plan.entry(name)
                .and_modify(|existing| {
                    if effective_start < existing.insert_at {
                        existing.insert_at = effective_start;
                    }
                })
                .or_insert(HoistEntry {
                    stmt_start: info.stmt_start,
                    stmt_end: info.stmt_end,
                    delete_end: info.delete_end,
                    insert_at: effective_start,
                });
        }
    }

    if plan.is_empty() {
        return Vec::new();
    }

    // Sort entries by source position so multiple hoists preserve their
    // original relative order in the output.
    let mut entries: Vec<HoistEntry> = plan.into_values().collect();
    entries.sort_by_key(|e| e.stmt_start);

    // We want hoisted text to appear *above* `decls_before_class` (which
    // contains constant-pool decls that may reference the hoisted identifiers).
    // Existing `decls_before_class` runs at priority 0. apply_edits applies
    // lower priority *first* at the same offset, and each later application
    // pushes earlier text further right in the output — so a *higher*
    // priority lands the hoisted text earlier in the result. Pick 5.
    const HOIST_INSERT_PRIORITY: i32 = 5;

    // Group hoisted statements by their target insertion point so that
    // multiple consts headed to the same class are emitted as a single
    // insert edit, with their text concatenated in source order. Emitting
    // them as separate edits would reverse their order, since each insert
    // at the same offset prepends to the prior insert's text.
    let mut emitted_stmts: HashSet<u32> = HashSet::new();
    let mut per_target: HashMap<u32, String> = HashMap::new();
    let mut edits = Vec::new();

    for entry in &entries {
        if !emitted_stmts.insert(entry.stmt_start) {
            continue;
        }

        let text = &source[entry.stmt_start as usize..entry.stmt_end as usize];
        let bucket = per_target.entry(entry.insert_at).or_default();
        bucket.push_str(text);
        bucket.push('\n');

        edits.push(Edit::delete(entry.stmt_start, entry.delete_end));
    }

    for (insert_at, text) in per_target {
        edits.push(Edit::insert(insert_at, text).with_priority(HOIST_INSERT_PRIORITY));
    }

    edits
}

/// Compute the effective start of a class statement, ignoring trailing
/// whitespace but spanning any leading decorators that will remain in the
/// source. We don't have access to the in-progress `decorator_spans_to_remove`
/// list here, so we conservatively use the earliest decorator span — the
/// hoisted text will land before *all* decorators, which is correct regardless
/// of which decorators end up being stripped.
fn effective_class_start(class: &Class<'_>, stmt_start: u32) -> u32 {
    class.decorators.iter().map(|d| d.span.start).min().map_or(stmt_start, |d| d.min(stmt_start))
}

/// Locate the inner class declaration of a top-level statement, returning the
/// effective statement start (including any `export` keyword).
fn class_of<'a, 'src>(stmt: &'src Statement<'a>) -> Option<(&'src Class<'a>, u32)> {
    match stmt {
        Statement::ClassDeclaration(class) => Some((class.as_ref(), class.span.start)),
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                Some((class.as_ref(), export.span.start))
            }
            _ => None,
        },
        Statement::ExportNamedDeclaration(export) => match &export.declaration {
            Some(Declaration::ClassDeclaration(class)) => Some((class.as_ref(), export.span.start)),
            _ => None,
        },
        _ => None,
    }
}

/// Does this class carry any decorator that Angular's compiler emits eager
/// definitions for? We don't try to be precise here — any of the well-known
/// Angular decorators makes the class a candidate.
fn has_angular_decorator(class: &Class<'_>) -> bool {
    class.decorators.iter().any(|d| {
        let callee = match &d.expression {
            Expression::CallExpression(call) => &call.callee,
            expr => expr,
        };
        let name = match callee {
            Expression::Identifier(id) => id.name.as_str(),
            Expression::StaticMemberExpression(member) => member.property.name.as_str(),
            _ => return false,
        };
        matches!(name, "Component" | "Directive" | "Pipe" | "NgModule" | "Injectable")
    })
}

/// Information about a top-level binding declaration's location.
#[derive(Clone, Copy)]
struct BindingInfo {
    stmt_start: u32,
    stmt_end: u32,
    delete_end: u32,
}

/// Walk top-level statements and index every variable binding identifier
/// they declare. Multiple identifiers from a combined declaration
/// (`const A = 1, B = 2;`) share the same statement span — hoisting one
/// hoists the whole statement, which is harmless because the other bindings
/// come along for the ride.
///
/// Only `VariableDeclaration` (const/let/var) and the `export` form of it are
/// considered:
///
/// * `function` declarations are fully hoisted by the JavaScript runtime
///   already (their bodies are available before their textual position), so
///   they never trigger TDZ.
/// * Class declarations are intentionally skipped here because hoisting them
///   would race the rest of the transform pipeline, which inserts static
///   fields and surrounding declarations at the class's original position.
///   Deleting the class's source range would clobber those inserts.
///   Forward-referenced classes are rare in real Angular code and out of
///   scope for this fix.
fn collect_top_level_bindings<'a>(
    program: &Program<'a>,
    source: &str,
) -> HashMap<&'a str, BindingInfo> {
    let bytes = source.as_bytes();
    let mut out: HashMap<&'a str, BindingInfo> = HashMap::new();

    for stmt in &program.body {
        let stmt_span = stmt.span();
        let info = BindingInfo {
            stmt_start: stmt_span.start,
            stmt_end: stmt_span.end,
            delete_end: end_with_trailing_newline(stmt_span.end, bytes),
        };

        let decl = match stmt {
            Statement::VariableDeclaration(decl) => Some(decl.as_ref()),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::VariableDeclaration(decl)) => Some(decl.as_ref()),
                _ => None,
            },
            _ => None,
        };

        let Some(decl) = decl else { continue };
        for declarator in &decl.declarations {
            add_binding_names(&declarator.id, info, &mut out);
        }
    }

    out
}

/// Extract identifier names from a binding pattern. We only handle plain
/// identifier patterns — anything destructured (`const { a } = x;`) is left
/// alone because hoisting destructuring would change observable behavior if
/// the right-hand side has side effects.
fn add_binding_names<'a>(
    pat: &BindingPattern<'a>,
    info: BindingInfo,
    out: &mut HashMap<&'a str, BindingInfo>,
) {
    if let BindingPattern::BindingIdentifier(id) = pat {
        out.insert(id.name.as_str(), info);
    }
}

/// Advance `end` past one trailing line terminator so that deleting the
/// statement also removes its terminating newline, leaving a clean gap.
fn end_with_trailing_newline(end: u32, bytes: &[u8]) -> u32 {
    let mut pos = end as usize;
    while pos < bytes.len() {
        match bytes[pos] {
            b' ' | b'\t' | b'\r' => pos += 1,
            b'\n' => {
                pos += 1;
                break;
            }
            _ => break,
        }
    }
    pos as u32
}

/// Collect identifiers referenced inside the decorator argument expressions.
/// Only the decorator's call arguments (i.e. the metadata object) are walked.
fn collect_decorator_idents<'a>(decorator: &Decorator<'a>, out: &mut HashSet<&'a str>) {
    let Expression::CallExpression(call) = &decorator.expression else {
        return;
    };
    for arg in &call.arguments {
        match arg {
            Argument::SpreadElement(spread) => {
                collect_expr_idents(&spread.argument, out);
            }
            other => {
                if let Some(expr) = argument_to_expression(other) {
                    collect_expr_idents(expr, out);
                }
            }
        }
    }
}

fn argument_to_expression<'a, 'src>(arg: &'src Argument<'a>) -> Option<&'src Expression<'a>> {
    if arg.is_expression() { Some(arg.to_expression()) } else { None }
}

/// Walk an expression collecting every bare identifier reference. Walks
/// through arrays, object literals, spreads, conditionals, calls, etc. Skips:
///
/// * The body of any function/arrow expression — references inside a factory
///   like `useFactory: () => new Service(DEP)` only fire when the factory is
///   invoked at injection time, never at class-definition time.
/// * The body of class expressions for the same lazy-evaluation reason.
/// * Property names that aren't computed — `{ provide: x }` references `x`
///   (the value) but not `provide` (the property name).
/// * Member expression property names — `Foo.BAR` references `Foo`; `BAR` is
///   a property access, not a bare identifier.
/// * TypeScript type annotations and assertions.
fn collect_expr_idents<'a>(expr: &Expression<'a>, out: &mut HashSet<&'a str>) {
    use Expression as E;
    match expr {
        E::Identifier(id) => {
            out.insert(id.name.as_str());
        }
        E::ArrayExpression(arr) => {
            for el in &arr.elements {
                collect_array_element_idents(el, out);
            }
        }
        E::ObjectExpression(obj) => {
            for prop in &obj.properties {
                match prop {
                    ObjectPropertyKind::ObjectProperty(p) => {
                        // Computed keys (e.g. `{ [TOKEN]: 1 }`) reference the
                        // key identifier; static keys don't.
                        if p.computed {
                            if let Some(key_expr) = p.key.as_expression() {
                                collect_expr_idents(key_expr, out);
                            }
                        }
                        collect_expr_idents(&p.value, out);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_expr_idents(&spread.argument, out);
                    }
                }
            }
        }
        E::CallExpression(call) => {
            collect_callee_idents(&call.callee, out);
            for arg in &call.arguments {
                match arg {
                    Argument::SpreadElement(s) => collect_expr_idents(&s.argument, out),
                    other => {
                        if let Some(e) = argument_to_expression(other) {
                            collect_expr_idents(e, out);
                        }
                    }
                }
            }
            // Type arguments may carry identifier references but typed code
            // is erased; they're irrelevant at runtime.
        }
        E::NewExpression(new) => {
            collect_expr_idents(&new.callee, out);
            for arg in &new.arguments {
                match arg {
                    Argument::SpreadElement(s) => collect_expr_idents(&s.argument, out),
                    other => {
                        if let Some(e) = argument_to_expression(other) {
                            collect_expr_idents(e, out);
                        }
                    }
                }
            }
        }
        E::ConditionalExpression(cond) => {
            collect_expr_idents(&cond.test, out);
            collect_expr_idents(&cond.consequent, out);
            collect_expr_idents(&cond.alternate, out);
        }
        E::LogicalExpression(log) => {
            collect_expr_idents(&log.left, out);
            collect_expr_idents(&log.right, out);
        }
        E::BinaryExpression(bin) => {
            collect_expr_idents(&bin.left, out);
            collect_expr_idents(&bin.right, out);
        }
        E::UnaryExpression(un) => {
            collect_expr_idents(&un.argument, out);
        }
        E::SequenceExpression(seq) => {
            for e in &seq.expressions {
                collect_expr_idents(e, out);
            }
        }
        E::ParenthesizedExpression(p) => {
            collect_expr_idents(&p.expression, out);
        }
        E::TemplateLiteral(tpl) => {
            for e in &tpl.expressions {
                collect_expr_idents(e, out);
            }
        }
        E::TaggedTemplateExpression(tagged) => {
            collect_expr_idents(&tagged.tag, out);
            for e in &tagged.quasi.expressions {
                collect_expr_idents(e, out);
            }
        }
        E::StaticMemberExpression(member) => {
            collect_expr_idents(&member.object, out);
        }
        E::ComputedMemberExpression(member) => {
            collect_expr_idents(&member.object, out);
            collect_expr_idents(&member.expression, out);
        }
        E::PrivateFieldExpression(member) => {
            collect_expr_idents(&member.object, out);
        }
        E::AwaitExpression(a) => collect_expr_idents(&a.argument, out),
        E::YieldExpression(y) => {
            if let Some(arg) = &y.argument {
                collect_expr_idents(arg, out);
            }
        }
        E::TSAsExpression(ts) => collect_expr_idents(&ts.expression, out),
        E::TSSatisfiesExpression(ts) => collect_expr_idents(&ts.expression, out),
        E::TSNonNullExpression(ts) => collect_expr_idents(&ts.expression, out),
        E::TSTypeAssertion(ts) => collect_expr_idents(&ts.expression, out),
        E::TSInstantiationExpression(ts) => collect_expr_idents(&ts.expression, out),
        // Class expressions inside metadata are exceedingly rare and their
        // bodies aren't eagerly evaluated; treat them as opaque.
        E::ClassExpression(_) => {}
        // Function and arrow bodies run lazily — references inside don't
        // affect class-init evaluation.
        E::ArrowFunctionExpression(_) | E::FunctionExpression(_) => {}
        // Literals and `this`/`super` carry no identifier references.
        _ => {}
    }
}

fn collect_callee_idents<'a>(callee: &Expression<'a>, out: &mut HashSet<&'a str>) {
    collect_expr_idents(callee, out);
}

fn collect_array_element_idents<'a>(el: &ArrayExpressionElement<'a>, out: &mut HashSet<&'a str>) {
    match el {
        ArrayExpressionElement::SpreadElement(spread) => {
            collect_expr_idents(&spread.argument, out);
        }
        ArrayExpressionElement::Elision(_) => {}
        other => {
            if let Some(expr) = array_element_to_expression(other) {
                collect_expr_idents(expr, out);
            }
        }
    }
}

fn array_element_to_expression<'a, 'src>(
    el: &'src ArrayExpressionElement<'a>,
) -> Option<&'src Expression<'a>> {
    if el.is_expression() { Some(el.to_expression()) } else { None }
}

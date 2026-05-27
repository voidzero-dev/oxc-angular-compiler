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
//! * Hoisting is *transitive*: if a hoisted binding's initializer references
//!   another later-declared top-level binding, that one is hoisted too. The
//!   final emission order is a topological sort of the dependency graph, so
//!   `const PROVIDERS = [{ provide: TOKEN, ... }]` ends up *after*
//!   `const TOKEN = ...` in the hoisted prelude.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, Class, Declaration, Decorator,
    ExportDefaultDeclarationKind, Expression, ObjectPropertyKind, Program, Statement,
};
use oxc_span::GetSpan;

use crate::optimizer::Edit;

/// Per-statement record collected during the initial scan. Multi-declarator
/// statements (`const A = 1, B = 2;`) get a single entry shared by every name
/// they bind; `init_idents` is the union of identifier references across all
/// declarator initializers.
struct StmtInfo<'a> {
    stmt_end: u32,
    /// End of the deletion (extends `stmt_end` past one trailing newline so
    /// the hoist doesn't leave a stray blank line behind).
    delete_end: u32,
    /// Identifier references appearing in any declarator's initializer in
    /// this statement. Used to drive transitive hoisting.
    init_idents: HashSet<&'a str>,
}

/// One statement scheduled for hoisting, keyed by its `stmt_start`. Multiple
/// classes that need the same statement collapse into a single entry whose
/// `insert_at` is the MIN of all referencers' effective starts.
#[derive(Clone, Copy)]
struct PlanEntry {
    stmt_end: u32,
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
    // Step 1: index top-level bindings.
    //   - `binding_to_stmt`: identifier name → containing statement's `start`.
    //   - `stmt_info`: statement start → end/delete bounds and the union of
    //     identifier references across the statement's initializers.
    let (binding_to_stmt, stmt_info) = collect_top_level_bindings(program, source);
    if binding_to_stmt.is_empty() {
        return Vec::new();
    }

    // Step 2: for every Angular-decorated class, BFS through binding
    // initializers starting from the identifiers directly referenced in the
    // decorator metadata. The plan is keyed by `stmt_start` (not name) so
    // multi-declarator statements collapse into a single entry, and the
    // `insert_at` is updated to the MIN across all referencers — that guards
    // against the nondeterministic dedup bug where, with `const A = 1, B = 2;`
    // referenced by two different classes, the surviving entry's `insert_at`
    // depended on HashMap iteration order and could land *after* the earlier
    // class. See PR #302 review.
    let mut plan: HashMap<u32, PlanEntry> = HashMap::new();

    for stmt in &program.body {
        let Some((class, stmt_start_pos)) = class_of(stmt) else { continue };
        if !has_angular_decorator(class) {
            continue;
        }

        let mut direct: HashSet<&'a str> = HashSet::new();
        for decorator in &class.decorators {
            collect_decorator_idents(decorator, &mut direct);
        }
        if direct.is_empty() {
            continue;
        }

        let class_body_end = class.body.span.end;
        let effective_start = effective_class_start(class, stmt_start_pos);

        let mut worklist: Vec<&'a str> = direct.into_iter().collect();
        let mut visited: HashSet<&'a str> = HashSet::new();
        while let Some(name) = worklist.pop() {
            if !visited.insert(name) {
                continue;
            }
            let Some(&stmt_start) = binding_to_stmt.get(name) else { continue };
            let Some(info) = stmt_info.get(&stmt_start) else { continue };
            // Skip bindings declared *before* this class — they're already
            // initialized when the class evaluates.
            if stmt_start <= class_body_end {
                continue;
            }

            plan.entry(stmt_start)
                .and_modify(|p| {
                    if effective_start < p.insert_at {
                        p.insert_at = effective_start;
                    }
                })
                .or_insert(PlanEntry {
                    stmt_end: info.stmt_end,
                    delete_end: info.delete_end,
                    insert_at: effective_start,
                });

            // Transitive hoist: if this binding's initializer references
            // another later-declared binding, that one must move above the
            // class too — otherwise the *hoisted* statement itself TDZ-throws
            // when its initializer runs. Without this, `providers: PROVIDERS`
            // followed by `const PROVIDERS = [{ provide: TOKEN, ... }]; const
            // TOKEN = ...;` moves `PROVIDERS` but leaves `TOKEN` below, so
            // module evaluation now throws inside the hoisted `PROVIDERS`
            // initializer. See PR #302 review.
            for n in &info.init_idents {
                if !visited.contains(n) {
                    worklist.push(n);
                }
            }
        }
    }

    if plan.is_empty() {
        return Vec::new();
    }

    // Step 3: topologically sort the planned statements so dependencies are
    // emitted *before* their dependents in the hoisted prelude. Within a
    // single bucket (same `insert_at`), this guarantees that e.g. `const
    // TOKEN` precedes `const PROVIDERS = [{ provide: TOKEN, ... }]`.
    let order = topological_order(&plan, &binding_to_stmt, &stmt_info);

    // Step 4: emit edits. Group by `insert_at` so multiple statements headed
    // to the same class become a single insert edit whose text is the
    // concatenation in topological order. Emitting them as separate edits at
    // the same offset would invert their order (each insert at the same
    // position prepends to the prior insert's text).
    const HOIST_INSERT_PRIORITY: i32 = 5;
    let mut per_target: HashMap<u32, String> = HashMap::new();
    let mut edits: Vec<Edit> = Vec::new();

    for stmt_start in &order {
        let p = &plan[stmt_start];
        let text = &source[*stmt_start as usize..p.stmt_end as usize];
        let bucket = per_target.entry(p.insert_at).or_default();
        bucket.push_str(text);
        bucket.push('\n');
        edits.push(Edit::delete(*stmt_start, p.delete_end));
    }

    for (insert_at, text) in per_target {
        edits.push(Edit::insert(insert_at, text).with_priority(HOIST_INSERT_PRIORITY));
    }

    edits
}

/// Iterative post-order DFS yielding a topological ordering of planned
/// statements: dependencies first, then dependents. The seed iteration is in
/// ascending `stmt_start` so the result is deterministic. Cycles (which would
/// require ill-formed source where two consts reference each other) are
/// broken silently — they can't produce a valid evaluation order anyway.
fn topological_order(
    plan: &HashMap<u32, PlanEntry>,
    binding_to_stmt: &HashMap<&str, u32>,
    stmt_info: &HashMap<u32, StmtInfo<'_>>,
) -> Vec<u32> {
    let plan_starts: HashSet<u32> = plan.keys().copied().collect();

    // Adjacency list: stmt_start -> stmt_starts it depends on (must come
    // *before* it). Filter to only edges that land inside the plan; deps that
    // resolve outside (declared before the class, or not top-level) are
    // already TDZ-safe.
    let mut deps: HashMap<u32, Vec<u32>> = HashMap::with_capacity(plan_starts.len());
    for &start in &plan_starts {
        let Some(info) = stmt_info.get(&start) else {
            deps.insert(start, Vec::new());
            continue;
        };
        let mut edges: Vec<u32> = info
            .init_idents
            .iter()
            .filter_map(|n| binding_to_stmt.get(n))
            .copied()
            .filter(|s| *s != start && plan_starts.contains(s))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        deps.insert(start, edges);
    }

    let mut all_starts: Vec<u32> = plan_starts.into_iter().collect();
    all_starts.sort_unstable();

    // States: 0 = unvisited, 1 = on stack (visiting), 2 = done.
    let mut state: HashMap<u32, u8> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();

    // Iterative DFS via an explicit stack of (node, child_index). When all of
    // a node's children are processed we move it from "visiting" to "done"
    // and push it onto `order`. Recursion would be simpler but risks stack
    // overflow on pathological inputs.
    for seed in all_starts {
        if matches!(state.get(&seed).copied(), Some(2)) {
            continue;
        }
        let mut stack: Vec<(u32, usize)> = vec![(seed, 0)];
        state.insert(seed, 1);
        while let Some(&(node, idx)) = stack.last() {
            let children = deps.get(&node).map(Vec::as_slice).unwrap_or(&[]);
            if idx < children.len() {
                let child = children[idx];
                stack.last_mut().unwrap().1 += 1;
                match state.get(&child).copied() {
                    Some(2) => {} // already emitted
                    Some(1) => {} // cycle — skip back-edge
                    _ => {
                        state.insert(child, 1);
                        stack.push((child, 0));
                    }
                }
            } else {
                state.insert(node, 2);
                order.push(node);
                stack.pop();
            }
        }
    }

    order
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

/// Walk top-level statements and index every variable binding identifier
/// they declare, returning two complementary maps:
/// * `binding_to_stmt`: identifier name → containing statement's `start`. Used
///   to look up hoist info from an identifier reference.
/// * `stmt_info`: statement `start` → end/delete bounds and the union of
///   identifier references across every declarator's initializer. Used to
///   drive transitive hoisting and the topological sort.
///
/// Only `VariableDeclaration` (const/let/var) and the `export` form of it are
/// considered:
/// * `function` declarations are fully hoisted by the JavaScript runtime
///   already (their bodies are available before their textual position), so
///   they never trigger TDZ.
/// * Class declarations are intentionally skipped because hoisting them would
///   race the rest of the transform pipeline, which inserts static fields and
///   surrounding declarations at the class's original position. Deleting the
///   class's source range would clobber those inserts.
fn collect_top_level_bindings<'a>(
    program: &Program<'a>,
    source: &str,
) -> (HashMap<&'a str, u32>, HashMap<u32, StmtInfo<'a>>) {
    let bytes = source.as_bytes();
    let mut binding_to_stmt: HashMap<&'a str, u32> = HashMap::new();
    let mut stmt_info: HashMap<u32, StmtInfo<'a>> = HashMap::new();

    for stmt in &program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(decl) => Some(decl.as_ref()),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::VariableDeclaration(decl)) => Some(decl.as_ref()),
                _ => None,
            },
            _ => None,
        };
        let Some(decl) = decl else { continue };

        let span = stmt.span();
        let stmt_start = span.start;
        let mut info = StmtInfo {
            stmt_end: span.end,
            delete_end: end_with_trailing_newline(span.end, bytes),
            init_idents: HashSet::new(),
        };

        for declarator in &decl.declarations {
            if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                binding_to_stmt.insert(id.name.as_str(), stmt_start);
            }
            // Destructuring patterns are deliberately ignored — see
            // collect_top_level_bindings docstring above.
            if let Some(init) = &declarator.init {
                collect_expr_idents(init, &mut info.init_idents);
            }
        }
        stmt_info.insert(stmt_start, info);
    }

    (binding_to_stmt, stmt_info)
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
            collect_expr_idents(&call.callee, out);
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

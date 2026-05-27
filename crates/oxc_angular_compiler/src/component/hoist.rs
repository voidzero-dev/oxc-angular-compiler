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
//!
//! Binding resolution is performed via `oxc_semantic`'s symbol table:
//! every identifier reference resolves through its `ReferenceId` to a
//! `SymbolId`, so a nested-scope shadow of a top-level name can't be
//! mistaken for the top-level binding.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, ChainElement, Class, Declaration, Decorator,
    ExportDefaultDeclarationKind, Expression, IdentifierReference, ObjectPropertyKind, Program,
    Statement,
};
use oxc_ast_visit::Visit;
use oxc_semantic::Semantic;
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;

use crate::optimizer::Edit;

/// Per-statement record collected during the initial scan. Multi-declarator
/// statements (`const A = 1, B = 2;`) get a single entry shared by every
/// symbol they bind; `init_symbols` is the union of identifier references
/// (resolved to `SymbolId`) across all declarator initializers.
struct StmtInfo {
    stmt_end: u32,
    /// End of the deletion (extends `stmt_end` past one trailing newline so
    /// the hoist doesn't leave a stray blank line behind).
    delete_end: u32,
    /// Symbols referenced inside any declarator's initializer in this
    /// statement. Used to drive transitive hoisting.
    init_symbols: HashSet<SymbolId>,
    /// Subset of `init_symbols` that appears as a *direct callee* (the
    /// callee of `CallExpression` / `NewExpression`, including the inner
    /// call of an optional `f?.()`) somewhere in the initializer. Used to
    /// seed the "eagerly called" closure: if `f` is in this set and `f` is
    /// a top-level function, the function body's references fire at module
    /// load when this statement evaluates. Symbols referenced but never
    /// called (e.g. `useFactory: f` — Angular's injector invokes `f` lazily)
    /// do NOT belong here.
    init_called_symbols: HashSet<SymbolId>,
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
pub fn collect_hoist_edits<'a>(
    program: &Program<'a>,
    source: &str,
    semantic: &Semantic<'a>,
) -> Vec<Edit> {
    // Step 1: index top-level bindings (keyed by SymbolId).
    //   - `symbol_to_stmt`: binding SymbolId → containing statement's `start`.
    //   - `stmt_info`: statement start → end/delete bounds and the union of
    //     symbol references across the statement's initializers, plus the
    //     subset that appears as a *direct callee* in the initializer.
    //   - `fn_body_symbol_refs`: top-level function SymbolId → set of symbol
    //     references in its body. Top-level function *declarations* are
    //     JS-hoisted so they never need physical hoisting, but if a hoisted
    //     initializer *calls* them (`const PROVIDERS = makeProviders()`), the
    //     function body runs at module load and any later-declared binding it
    //     touches still TDZ-throws. The BFS consults this map to chase
    //     identifiers through function-call boundaries — but only when the
    //     function is actually invoked, not merely referenced as a value.
    //   - `fn_body_called_symbols`: top-level function SymbolId → set of
    //     symbols directly called in its body. Seeds the transitive
    //     "eagerly called" closure.
    let (symbol_to_stmt, stmt_info, fn_body_symbol_refs, fn_body_called_symbols) =
        collect_top_level_bindings(program, source, semantic);
    if symbol_to_stmt.is_empty() && fn_body_symbol_refs.is_empty() {
        return Vec::new();
    }

    // Step 2a: gather per-class decorator-metadata symbols (both the full
    // reference set and the "direct callee" subset). The direct-callee
    // subsets across all classes plus every top-level initializer's
    // direct-callee subset seed the `eagerly_called` closure, expanded
    // through `fn_body_called_symbols` to fixed point.
    //
    // The closure represents "every top-level function whose body runs at
    // module load". In the BFS the function-body-chasing branch fires
    // only for symbols in this set — otherwise a function stored as a
    // value (`useFactory: makeFactory`) would pull its body's references
    // into the hoist plan and introduce a fresh TDZ that didn't exist
    // before. See PR #302 review (Codex).
    let mut classes: Vec<(&Class<'a>, u32, HashSet<SymbolId>)> = Vec::new();
    let mut decorator_called: HashSet<SymbolId> = HashSet::new();
    for stmt in &program.body {
        let Some((class, stmt_start_pos)) = class_of(stmt) else { continue };
        if !has_angular_decorator(class) {
            continue;
        }
        let mut direct: HashSet<SymbolId> = HashSet::new();
        for decorator in &class.decorators {
            collect_decorator_symbols(decorator, semantic, &mut direct, &mut decorator_called);
        }
        if direct.is_empty() {
            continue;
        }
        classes.push((class, stmt_start_pos, direct));
    }

    let eagerly_called =
        compute_eagerly_called(&stmt_info, &decorator_called, &fn_body_called_symbols);

    // Step 2b: for every Angular-decorated class, BFS through binding
    // initializers starting from the symbols directly referenced in the
    // decorator metadata. The plan is keyed by `stmt_start` (not symbol) so
    // multi-declarator statements collapse into a single entry, and the
    // `insert_at` is updated to the MIN across all referencers — that guards
    // against the nondeterministic dedup bug where, with `const A = 1, B = 2;`
    // referenced by two different classes, the surviving entry's `insert_at`
    // depended on HashMap iteration order and could land *after* the earlier
    // class. See PR #302 review.
    let mut plan: HashMap<u32, PlanEntry> = HashMap::new();

    for (class, stmt_start_pos, direct) in classes {
        let class_body_end = class.body.span.end;
        let effective_start = effective_class_start(class, stmt_start_pos);

        let mut worklist: Vec<SymbolId> = direct.into_iter().collect();
        let mut visited: HashSet<SymbolId> = HashSet::new();
        while let Some(symbol) = worklist.pop() {
            if !visited.insert(symbol) {
                continue;
            }
            if let Some(&stmt_start) = symbol_to_stmt.get(&symbol) {
                let Some(info) = stmt_info.get(&stmt_start) else { continue };
                // Skip bindings declared *before* this class — they're
                // already initialized when the class evaluates.
                // `class_body_end` is the exclusive end of the class body
                // (one byte past `}`), so a statement starting at exactly
                // `class_body_end` is the very next byte after the class —
                // declared *after* and still needs hoisting.
                if stmt_start < class_body_end {
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
                // another later-declared binding, that one must move above
                // the class too — otherwise the *hoisted* statement itself
                // TDZ-throws when its initializer runs. Without this,
                // `providers: PROVIDERS` followed by `const PROVIDERS = [{
                // provide: TOKEN, ... }]; const TOKEN = ...;` moves
                // `PROVIDERS` but leaves `TOKEN` below, so module evaluation
                // now throws inside the hoisted `PROVIDERS` initializer.
                // See PR #302 review.
                for &s in &info.init_symbols {
                    if !visited.contains(&s) {
                        worklist.push(s);
                    }
                }
            } else if eagerly_called.contains(&symbol) {
                // The symbol resolves to a top-level function declaration
                // that is *actually called* (transitively) at module load.
                // Don't hoist the function itself (JS already hoists fn
                // decls), but its body's identifier reads fire whenever
                // it runs — and "eagerly_called" guarantees it does run
                // at module load. Chase those references. See PR #302
                // review (Codex).
                if let Some(body_refs) = fn_body_symbol_refs.get(&symbol) {
                    for &s in body_refs {
                        if !visited.contains(&s) {
                            worklist.push(s);
                        }
                    }
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
    let order = topological_order(
        &plan,
        &symbol_to_stmt,
        &stmt_info,
        &fn_body_symbol_refs,
        &eagerly_called,
    );

    // Step 4: emit edits. Group by `insert_at` so multiple statements headed
    // to the same class become a single insert edit whose text is the
    // concatenation in topological order. Emitting them as separate edits at
    // the same offset would invert their order (each insert at the same
    // position prepends to the prior insert's text).
    //
    // `HOIST_INSERT_PRIORITY` (positive) keeps hoisted text *above* the
    // `decls_before_class` insertion at the same offset (which uses default
    // priority 0).
    //
    // `HOIST_DELETE_PRIORITY` (negative) lets a hoist delete that starts at
    // exactly `class.body.span.end` — the byte right after `}`, where a
    // const declared with no whitespace lives — apply *before* the
    // `decls_after_class` insert at the same offset. Without the priority
    // skew, the insert ran first and the delete would then chew into the
    // newly inserted IIFE/metadata text instead of the original const.
    const HOIST_INSERT_PRIORITY: i32 = 5;
    const HOIST_DELETE_PRIORITY: i32 = -1;
    let mut per_target: HashMap<u32, String> = HashMap::new();
    let mut edits: Vec<Edit> = Vec::new();

    for stmt_start in &order {
        let p = &plan[stmt_start];
        let text = &source[*stmt_start as usize..p.stmt_end as usize];
        let bucket = per_target.entry(p.insert_at).or_default();
        bucket.push_str(text);
        bucket.push('\n');
        edits.push(Edit::delete(*stmt_start, p.delete_end).with_priority(HOIST_DELETE_PRIORITY));
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
    symbol_to_stmt: &HashMap<SymbolId, u32>,
    stmt_info: &HashMap<u32, StmtInfo>,
    fn_body_symbol_refs: &HashMap<SymbolId, HashSet<SymbolId>>,
    eagerly_called: &HashSet<SymbolId>,
) -> Vec<u32> {
    let plan_starts: HashSet<u32> = plan.keys().copied().collect();

    // Adjacency list: stmt_start -> stmt_starts it depends on (must come
    // *before* it). Filter to only edges that land inside the plan; deps that
    // resolve outside (declared before the class, or not top-level) are
    // already TDZ-safe.
    //
    // The "effective init symbols" of a planned statement are the transitive
    // closure of its direct `init_symbols` through `fn_body_symbol_refs`,
    // **restricted to functions in `eagerly_called`**. If the initializer
    // calls a function (directly or transitively), the function body's
    // identifier reads count as references that fire when the hoisted
    // statement evaluates. Functions only stored as values are NOT expanded
    // — their bodies don't run at module load. See PR #302 review (Codex).
    let mut deps: HashMap<u32, Vec<u32>> = HashMap::with_capacity(plan_starts.len());
    for &start in &plan_starts {
        let Some(info) = stmt_info.get(&start) else {
            deps.insert(start, Vec::new());
            continue;
        };
        let effective =
            expand_through_functions(&info.init_symbols, fn_body_symbol_refs, eagerly_called);
        let mut edges: Vec<u32> = effective
            .iter()
            .filter_map(|s| symbol_to_stmt.get(s))
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

/// Take a set of symbol references and expand it transitively through
/// `fn_body_symbol_refs`, but only across functions that are in
/// `eagerly_called`. A function only stored as a value (never invoked at
/// module load) doesn't run, so its body's reads must not count toward the
/// hoist plan — chasing them would invent a fresh TDZ. The `seen` set guards
/// against mutual recursion between top-level functions.
fn expand_through_functions(
    seed: &HashSet<SymbolId>,
    fn_body_symbol_refs: &HashMap<SymbolId, HashSet<SymbolId>>,
    eagerly_called: &HashSet<SymbolId>,
) -> HashSet<SymbolId> {
    let mut out: HashSet<SymbolId> = HashSet::new();
    let mut worklist: Vec<SymbolId> = seed.iter().copied().collect();
    let mut seen: HashSet<SymbolId> = HashSet::new();
    while let Some(symbol) = worklist.pop() {
        if !seen.insert(symbol) {
            continue;
        }
        out.insert(symbol);
        if !eagerly_called.contains(&symbol) {
            continue;
        }
        if let Some(body_refs) = fn_body_symbol_refs.get(&symbol) {
            for &s in body_refs {
                if !seen.contains(&s) {
                    worklist.push(s);
                }
            }
        }
    }
    out
}

/// Compute the transitive closure of "top-level functions that actually run
/// at module load". Seeded with every direct callee in either:
/// * a top-level `VariableDeclaration` initializer (`stmt_info[*].init_called_symbols`)
/// * Angular decorator metadata on any class (`decorator_called`).
///
/// Expanded through `fn_body_called_symbols`: if `f` is in the set and `f`
/// directly calls `g`, then `g` is too. Fixed-point — runs until the worklist
/// drains. A function stored as a value (referenced but not called) is NOT
/// added.
///
/// Used by both the BFS (to gate the function-body-chasing branch) and the
/// topological sort (to gate `expand_through_functions`). Without this, a
/// `useFactory: makeFactory` in providers — where Angular invokes
/// `makeFactory` lazily at injection time, NOT at class-definition time —
/// would still pull in `makeFactory`'s body refs and hoist them above the
/// class, sometimes inventing a new TDZ.
fn compute_eagerly_called(
    stmt_info: &HashMap<u32, StmtInfo>,
    decorator_called: &HashSet<SymbolId>,
    fn_body_called_symbols: &HashMap<SymbolId, HashSet<SymbolId>>,
) -> HashSet<SymbolId> {
    let mut out: HashSet<SymbolId> = HashSet::new();
    let mut worklist: Vec<SymbolId> = Vec::new();
    for info in stmt_info.values() {
        for &s in &info.init_called_symbols {
            if out.insert(s) {
                worklist.push(s);
            }
        }
    }
    for &s in decorator_called {
        if out.insert(s) {
            worklist.push(s);
        }
    }
    while let Some(symbol) = worklist.pop() {
        if let Some(calls) = fn_body_called_symbols.get(&symbol) {
            for &s in calls {
                if out.insert(s) {
                    worklist.push(s);
                }
            }
        }
    }
    out
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

/// Cheap pre-check: does `program` contain any top-level class statement
/// carrying one of the Angular decorators recognized by [`has_angular_decorator`]?
///
/// Used by the AOT transform pipeline to skip the `Semantic` build and the
/// full hoist scan for files with no decorated classes (plain TS helpers,
/// type-only modules, services without `@Injectable`, …). This walks
/// `program.body` only and never descends into class bodies or expressions,
/// so it's O(top-level statements) with a tiny per-statement cost.
pub(crate) fn program_has_angular_decorated_class(program: &Program<'_>) -> bool {
    program.body.iter().any(|stmt| match class_of(stmt) {
        Some((class, _)) => has_angular_decorator(class),
        None => false,
    })
}

/// Resolve an `IdentifierReference` to a `SymbolId` via the semantic model.
/// Returns `None` when the reference is unresolved (e.g. globals, imports
/// without a local binding, or undeclared identifiers). The caller silently
/// skips unresolved references — they can't refer to a top-level `const`
/// binding in this module anyway.
fn resolve_symbol(id: &IdentifierReference<'_>, semantic: &Semantic<'_>) -> Option<SymbolId> {
    let reference_id = id.reference_id.get()?;
    semantic.scoping().get_reference(reference_id).symbol_id()
}

/// Walk top-level statements and index every variable binding identifier
/// they declare, returning four complementary maps:
/// * `symbol_to_stmt`: binding `SymbolId` → containing statement's `start`.
/// * `stmt_info`: statement `start` → end/delete bounds and the union of
///   symbol references across every declarator's initializer. Used to drive
///   transitive hoisting and the topological sort.
/// * `fn_body_symbol_refs`: top-level function `SymbolId` → symbols
///   referenced in its body. Used to chase TDZ-relevant identifiers across
///   function-call boundaries.
/// * `fn_body_called_symbols`: top-level function `SymbolId` → symbols of
///   functions/`new` targets directly invoked inside its body. Feeds
///   `compute_eagerly_called` so the BFS only chases bodies that are
///   eagerly reachable from the decorator metadata's call graph.
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
    semantic: &Semantic<'a>,
) -> (
    HashMap<SymbolId, u32>,
    HashMap<u32, StmtInfo>,
    HashMap<SymbolId, HashSet<SymbolId>>,
    HashMap<SymbolId, HashSet<SymbolId>>,
) {
    let bytes = source.as_bytes();
    let mut symbol_to_stmt: HashMap<SymbolId, u32> = HashMap::new();
    let mut stmt_info: HashMap<u32, StmtInfo> = HashMap::new();
    let mut fn_body_symbol_refs: HashMap<SymbolId, HashSet<SymbolId>> = HashMap::new();
    let mut fn_body_called_symbols: HashMap<SymbolId, HashSet<SymbolId>> = HashMap::new();

    for stmt in &program.body {
        let var_decl = match stmt {
            Statement::VariableDeclaration(decl) => Some(decl.as_ref()),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::VariableDeclaration(decl)) => Some(decl.as_ref()),
                _ => None,
            },
            _ => None,
        };
        if let Some(decl) = var_decl {
            let span = stmt.span();
            let stmt_start = span.start;
            let mut info = StmtInfo {
                stmt_end: span.end,
                delete_end: end_with_trailing_newline(span.end, bytes),
                init_symbols: HashSet::new(),
                init_called_symbols: HashSet::new(),
            };

            for declarator in &decl.declarations {
                // Walk the declarator's `BindingPattern` recursively so that
                // destructuring forms (`const { TOKEN } = obj;`, `const [a, b]
                // = arr;`, `const { a: { b } } = obj;`, …) also index every
                // binding identifier they introduce. Without this, decorator
                // metadata referencing such a binding never resolves to its
                // declaring statement and the hoist is skipped. See PR #302
                // Codex review.
                for_each_binding_identifier(&declarator.id, &mut |id| {
                    if let Some(symbol_id) = id.symbol_id.get() {
                        symbol_to_stmt.insert(symbol_id, stmt_start);
                    }
                });
                if let Some(init) = &declarator.init {
                    collect_expr_symbols(
                        init,
                        semantic,
                        &mut info.init_symbols,
                        &mut info.init_called_symbols,
                    );
                }
            }
            stmt_info.insert(stmt_start, info);
            continue;
        }

        // Top-level `function foo() { ... }` (also `export function` /
        // `export default function foo`). Function declarations are
        // JS-hoisted whole-body, so we never *move* them; we only index
        // their body references so the BFS can chase TDZ-relevant
        // identifiers across function-call boundaries.
        let func = match stmt {
            Statement::FunctionDeclaration(f) => Some(f.as_ref()),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::FunctionDeclaration(f)) => Some(f.as_ref()),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(f) => Some(f.as_ref()),
                _ => None,
            },
            _ => None,
        };
        if let Some(func) = func {
            if let (Some(id), Some(body)) = (&func.id, &func.body) {
                let Some(fn_symbol) = id.symbol_id.get() else { continue };
                let mut refs: HashSet<SymbolId> = HashSet::new();
                let mut called: HashSet<SymbolId> = HashSet::new();
                let mut visitor =
                    FunctionBodyIdentVisitor { semantic, out: &mut refs, called: &mut called };
                visitor.visit_function_body(body);
                fn_body_symbol_refs.insert(fn_symbol, refs);
                fn_body_called_symbols.insert(fn_symbol, called);
            }
        }
    }

    (symbol_to_stmt, stmt_info, fn_body_symbol_refs, fn_body_called_symbols)
}

/// Walk a `BindingPattern` and invoke `f` for every nested `BindingIdentifier`
/// it introduces. Handles `BindingIdentifier` (the simple `const x` case),
/// `ObjectPattern` (each `BindingProperty`'s `value`, plus `rest`),
/// `ArrayPattern` (each element `Option<BindingPattern>`, plus `rest`), and
/// `AssignmentPattern` (the `left` pattern of `const { x = 1 } = obj`).
/// Default expressions on `AssignmentPattern` (e.g. `const { x = SOMETHING }
/// = obj`) are nested *inside* the binding pattern but are NOT walked here —
/// a deliberate conservative choice. In the rare case where a default
/// expression references a later-declared top-level binding, that binding
/// will not be transitively hoisted. Decorator metadata almost never uses
/// destructured names with such defaults, so this gap is accepted rather
/// than implemented.
fn for_each_binding_identifier<'a>(
    pat: &BindingPattern<'a>,
    f: &mut impl FnMut(&oxc_ast::ast::BindingIdentifier<'a>),
) {
    match pat {
        BindingPattern::BindingIdentifier(id) => f(id),
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                for_each_binding_identifier(&prop.value, f);
            }
            if let Some(rest) = &obj.rest {
                for_each_binding_identifier(&rest.argument, f);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for el in &arr.elements {
                if let Some(el) = el {
                    for_each_binding_identifier(el, f);
                }
            }
            if let Some(rest) = &arr.rest {
                for_each_binding_identifier(&rest.argument, f);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            for_each_binding_identifier(&assign.left, f);
        }
    }
}

/// AST visitor that collects every `IdentifierReference` reachable from a
/// function body, resolving each to a `SymbolId` via the semantic model, with
/// the same "lazy bodies are opaque" rule the existing expression walker
/// uses: nested function/arrow expressions inside the body don't run when
/// the outer function is called, so their bodies are skipped.
///
/// `called` receives the subset of `out` that appears as a *direct callee*
/// of a `CallExpression` / `NewExpression` (including the inner call of a
/// `f?.()` chain) inside the body. Used to drive the "eagerly called"
/// closure: if function `f` is called at module load, then the symbols
/// `f`'s body directly calls fire too, transitively.
struct FunctionBodyIdentVisitor<'a, 'b> {
    semantic: &'b Semantic<'a>,
    out: &'b mut HashSet<SymbolId>,
    called: &'b mut HashSet<SymbolId>,
}

impl<'a, 'b> Visit<'a> for FunctionBodyIdentVisitor<'a, 'b> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if let Some(symbol) = resolve_symbol(it, self.semantic) {
            self.out.insert(symbol);
        }
    }

    fn visit_call_expression(&mut self, it: &oxc_ast::ast::CallExpression<'a>) {
        record_direct_callee(&it.callee, self.semantic, self.called);
        // Continue default traversal so identifier references inside callee
        // and arguments still feed `self.out`.
        oxc_ast_visit::walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &oxc_ast::ast::NewExpression<'a>) {
        record_direct_callee(&it.callee, self.semantic, self.called);
        oxc_ast_visit::walk::walk_new_expression(self, it);
    }

    // Nested function/arrow expressions only execute when *they* are called,
    // not when the enclosing function is. Don't descend.
    fn visit_function(
        &mut self,
        _it: &oxc_ast::ast::Function<'a>,
        _flags: oxc_syntax::scope::ScopeFlags,
    ) {
    }

    fn visit_arrow_function_expression(&mut self, _it: &oxc_ast::ast::ArrowFunctionExpression<'a>) {
    }

    // Class expressions inside the body define methods that don't run at
    // call time of the outer function. Skip.
    fn visit_class(&mut self, _it: &Class<'a>) {}
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

/// Collect symbols referenced inside the decorator argument expressions.
/// Only the decorator's call arguments (i.e. the metadata object) are walked.
/// `called` receives the subset of `out` that appears as a *direct callee*
/// of a call/new expression — used to drive the "eagerly called" closure.
fn collect_decorator_symbols<'a>(
    decorator: &Decorator<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    let Expression::CallExpression(call) = &decorator.expression else {
        return;
    };
    for arg in &call.arguments {
        match arg {
            Argument::SpreadElement(spread) => {
                collect_expr_symbols(&spread.argument, semantic, out, called);
            }
            other => {
                if let Some(expr) = argument_to_expression(other) {
                    collect_expr_symbols(expr, semantic, out, called);
                }
            }
        }
    }
}

fn argument_to_expression<'a, 'src>(arg: &'src Argument<'a>) -> Option<&'src Expression<'a>> {
    if arg.is_expression() { Some(arg.to_expression()) } else { None }
}

/// Walk an expression collecting every bare identifier reference (resolved
/// to a `SymbolId` via the semantic model). Walks through arrays, object
/// literals, spreads, conditionals, calls, etc. Skips:
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
fn collect_expr_symbols<'a>(
    expr: &Expression<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    use Expression as E;
    match expr {
        E::Identifier(id) => {
            if let Some(symbol) = resolve_symbol(id, semantic) {
                out.insert(symbol);
            }
        }
        E::ArrayExpression(arr) => {
            for el in &arr.elements {
                collect_array_element_symbols(el, semantic, out, called);
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
                                collect_expr_symbols(key_expr, semantic, out, called);
                            }
                        }
                        collect_expr_symbols(&p.value, semantic, out, called);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_expr_symbols(&spread.argument, semantic, out, called);
                    }
                }
            }
        }
        E::CallExpression(call) => {
            record_direct_callee(&call.callee, semantic, called);
            collect_expr_symbols(&call.callee, semantic, out, called);
            for arg in &call.arguments {
                match arg {
                    Argument::SpreadElement(s) => {
                        collect_expr_symbols(&s.argument, semantic, out, called);
                    }
                    other => {
                        if let Some(e) = argument_to_expression(other) {
                            collect_expr_symbols(e, semantic, out, called);
                        }
                    }
                }
            }
            // Type arguments may carry identifier references but typed code
            // is erased; they're irrelevant at runtime.
        }
        E::NewExpression(new) => {
            record_direct_callee(&new.callee, semantic, called);
            collect_expr_symbols(&new.callee, semantic, out, called);
            for arg in &new.arguments {
                match arg {
                    Argument::SpreadElement(s) => {
                        collect_expr_symbols(&s.argument, semantic, out, called);
                    }
                    other => {
                        if let Some(e) = argument_to_expression(other) {
                            collect_expr_symbols(e, semantic, out, called);
                        }
                    }
                }
            }
        }
        E::ConditionalExpression(cond) => {
            collect_expr_symbols(&cond.test, semantic, out, called);
            collect_expr_symbols(&cond.consequent, semantic, out, called);
            collect_expr_symbols(&cond.alternate, semantic, out, called);
        }
        E::LogicalExpression(log) => {
            collect_expr_symbols(&log.left, semantic, out, called);
            collect_expr_symbols(&log.right, semantic, out, called);
        }
        E::BinaryExpression(bin) => {
            collect_expr_symbols(&bin.left, semantic, out, called);
            collect_expr_symbols(&bin.right, semantic, out, called);
        }
        E::UnaryExpression(un) => {
            collect_expr_symbols(&un.argument, semantic, out, called);
        }
        E::SequenceExpression(seq) => {
            for e in &seq.expressions {
                collect_expr_symbols(e, semantic, out, called);
            }
        }
        E::ParenthesizedExpression(p) => {
            collect_expr_symbols(&p.expression, semantic, out, called);
        }
        E::TemplateLiteral(tpl) => {
            for e in &tpl.expressions {
                collect_expr_symbols(e, semantic, out, called);
            }
        }
        E::TaggedTemplateExpression(tagged) => {
            record_direct_callee(&tagged.tag, semantic, called);
            collect_expr_symbols(&tagged.tag, semantic, out, called);
            for e in &tagged.quasi.expressions {
                collect_expr_symbols(e, semantic, out, called);
            }
        }
        E::StaticMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        E::ComputedMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
            collect_expr_symbols(&member.expression, semantic, out, called);
        }
        E::PrivateFieldExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        E::AwaitExpression(a) => collect_expr_symbols(&a.argument, semantic, out, called),
        E::YieldExpression(y) => {
            if let Some(arg) = &y.argument {
                collect_expr_symbols(arg, semantic, out, called);
            }
        }
        E::TSAsExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        E::TSSatisfiesExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        E::TSNonNullExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        E::TSTypeAssertion(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        E::TSInstantiationExpression(ts) => {
            collect_expr_symbols(&ts.expression, semantic, out, called);
        }
        // Optional-chaining expressions (`TOKEN?.id`, `f?.()`). The inner
        // `ChainElement` mirrors a small subset of `Expression`; dispatch
        // each variant to the same logic the matching `Expression` arm
        // uses so identifier references inside the chain are collected.
        E::ChainExpression(chain) => {
            collect_chain_element_symbols(&chain.expression, semantic, out, called);
        }
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

/// If `callee` is a *direct* identifier reference (peeling through
/// parentheses and TS type-only wrappers), record its symbol in `called`.
/// Member callees (`foo.bar()`) and other complex expressions are skipped
/// — only direct callees of `CallExpression`/`NewExpression` count as
/// eager invocations of a top-level function.
fn record_direct_callee<'a>(
    callee: &Expression<'a>,
    semantic: &Semantic<'a>,
    called: &mut HashSet<SymbolId>,
) {
    use Expression as E;
    let mut cur = callee;
    loop {
        match cur {
            E::Identifier(id) => {
                if let Some(symbol) = resolve_symbol(id, semantic) {
                    called.insert(symbol);
                }
                return;
            }
            E::ParenthesizedExpression(p) => cur = &p.expression,
            E::TSAsExpression(ts) => cur = &ts.expression,
            E::TSSatisfiesExpression(ts) => cur = &ts.expression,
            E::TSNonNullExpression(ts) => cur = &ts.expression,
            E::TSTypeAssertion(ts) => cur = &ts.expression,
            E::TSInstantiationExpression(ts) => cur = &ts.expression,
            _ => return,
        }
    }
}

/// Mirror of [`collect_expr_symbols`] for the small set of node kinds that
/// can appear directly inside an `Expression::ChainExpression`. Without this,
/// optional-chaining (`TOKEN?.id`, `f?.()`) would be silently dropped by
/// the catch-all in `collect_expr_symbols` — and decorator metadata
/// referencing the chained binding wouldn't hoist it.
fn collect_chain_element_symbols<'a>(
    el: &ChainElement<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    match el {
        ChainElement::CallExpression(call) => {
            record_direct_callee(&call.callee, semantic, called);
            collect_expr_symbols(&call.callee, semantic, out, called);
            for arg in &call.arguments {
                match arg {
                    Argument::SpreadElement(s) => {
                        collect_expr_symbols(&s.argument, semantic, out, called);
                    }
                    other => {
                        if let Some(e) = argument_to_expression(other) {
                            collect_expr_symbols(e, semantic, out, called);
                        }
                    }
                }
            }
        }
        ChainElement::StaticMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        ChainElement::ComputedMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
            collect_expr_symbols(&member.expression, semantic, out, called);
        }
        ChainElement::PrivateFieldExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        ChainElement::TSNonNullExpression(ts) => {
            collect_expr_symbols(&ts.expression, semantic, out, called);
        }
    }
}

fn collect_array_element_symbols<'a>(
    el: &ArrayExpressionElement<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    match el {
        ArrayExpressionElement::SpreadElement(spread) => {
            collect_expr_symbols(&spread.argument, semantic, out, called);
        }
        ArrayExpressionElement::Elision(_) => {}
        other => {
            if let Some(expr) = array_element_to_expression(other) {
                collect_expr_symbols(expr, semantic, out, called);
            }
        }
    }
}

fn array_element_to_expression<'a, 'src>(
    el: &'src ArrayExpressionElement<'a>,
) -> Option<&'src Expression<'a>> {
    if el.is_expression() { Some(el.to_expression()) } else { None }
}

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
    Argument, ArrayExpressionElement, BindingPattern, ChainElement, Class, ClassElement,
    Declaration, Decorator, ExportDefaultDeclarationKind, Expression, FormalParameters,
    IdentifierReference, MethodDefinitionKind, ObjectPropertyKind, Program, Statement,
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

    // Reverse view of `symbol_to_stmt` ∩ `fn_body_symbol_refs` keyed by
    // `stmt_start`. A statement like `const make = () => TestComponent;`
    // declares a fn-valued binding `make` whose body refs are indexed in
    // `fn_body_symbol_refs[make]`, but the statement's plain `init_symbols`
    // is empty (arrow bodies are lazy in `collect_expr_symbols`). When the
    // binding's symbol enters some class's `eagerly_called` (because a
    // decorator invokes `make()`), the arrow body fires at module load and
    // its reads become TDZ-relevant to that statement.
    //
    // Computed once here so the BFS safe-skip guard, the cascade un-planning
    // pass, and the topological sort can all consult the same map.
    let mut stmt_fn_valued_bindings: HashMap<u32, Vec<SymbolId>> = HashMap::new();
    for (&sym, &stmt_start) in &symbol_to_stmt {
        if fn_body_symbol_refs.contains_key(&sym) {
            stmt_fn_valued_bindings.entry(stmt_start).or_default().push(sym);
        }
    }

    // Index every top-level class declaration by its binding `SymbolId` →
    // the class's `span.start`. Used by the BFS to refuse hoisting any
    // statement whose initializer references a class that lives at-or-after
    // the protect site — see the safe-skip guard near `plan.entry(...)`.
    let top_level_class_positions = collect_top_level_class_positionss(program);

    // Step 2a: gather per-class decorator-metadata symbols (both the full
    // reference set and the "direct callee" subset). Each class gets its
    // OWN `decorator_called` set; it seeds a *per-class* `eagerly_called`
    // closure computed inside the BFS loop below.
    //
    // Why per-class (not global): the `eagerly_called` closure represents
    // "every top-level function whose body runs at module load *because of
    // this class's evaluation*". If `function foo() { return TOKEN; }` is
    // called by `const X = foo()` elsewhere in the module but only
    // referenced as a *value* in this class's metadata
    // (`useFactory: foo`), foo's body does NOT fire when this class
    // evaluates — and chasing TOKEN would invent a new TDZ on the class
    // (when `TOKEN = TestComponent`). A global `eagerly_called` (seeded
    // from every module-init call site) over-reaches across classes.
    let mut classes: Vec<(&Class<'a>, u32, HashSet<SymbolId>, HashSet<SymbolId>)> = Vec::new();
    for stmt in &program.body {
        let Some((class, stmt_start_pos)) = class_of(stmt) else { continue };
        if !has_angular_decorator(class) {
            continue;
        }
        let mut direct: HashSet<SymbolId> = HashSet::new();
        let mut decorator_called: HashSet<SymbolId> = HashSet::new();
        for decorator in &class.decorators {
            collect_decorator_symbols(decorator, semantic, &mut direct, &mut decorator_called);
        }
        if direct.is_empty() {
            continue;
        }
        classes.push((class, stmt_start_pos, direct, decorator_called));
    }

    // Step 2b: for every Angular-decorated class, BFS through binding
    // initializers starting from the symbols directly referenced in the
    // decorator metadata. The plan is keyed by `stmt_start` (not symbol) so
    // multi-declarator statements collapse into a single entry, and the
    // `insert_at` is updated to the MIN across all referencers — that guards
    // against the nondeterministic dedup bug where, with `const A = 1, B = 2;`
    // referenced by two different classes, the surviving entry's `insert_at`
    // depended on HashMap iteration order and could land *after* the earlier
    // class.
    let mut plan: HashMap<u32, PlanEntry> = HashMap::new();
    // Union of per-class `eagerly_called` sets for all classes that
    // contributed to the plan. The topological sort's edge expansion
    // (`expand_through_functions`) must see every function whose body
    // could fire at module load *for some class in the plan*, so that
    // dependency edges between planned statements are computed against
    // the same eager-evaluation set used to plan them.
    let mut combined_eagerly_called: HashSet<SymbolId> = HashSet::new();

    for (class, stmt_start_pos, direct, decorator_called) in classes {
        let class_body_end = class.body.span.end;
        let effective_start = effective_class_start(class, stmt_start_pos);

        // Per-class `eagerly_called`, seeded only from THIS class's
        // decorator metadata direct-callees and closed through
        // `fn_body_called_symbols`. As the BFS visits new binding
        // statements, we splice each statement's `init_called_symbols`
        // into the set and re-close — so a hoisted binding whose
        // initializer calls `g()` makes `g` (and everything `g`
        // transitively calls) eagerly evaluated for the chase.
        let mut eagerly_called: HashSet<SymbolId> = HashSet::new();
        let mut call_worklist: Vec<SymbolId> = Vec::new();
        for &s in &decorator_called {
            if eagerly_called.insert(s) {
                call_worklist.push(s);
            }
        }
        close_eagerly_called(&mut eagerly_called, &mut call_worklist, &fn_body_called_symbols);

        let mut worklist: Vec<SymbolId> = direct.into_iter().collect();
        let mut visited: HashSet<SymbolId> = HashSet::new();
        // Track function symbols whose bodies we've already chased so we
        // can belatedly chase them if they become eagerly_called *after*
        // the BFS has already popped them.
        let mut chased_fn_bodies: HashSet<SymbolId> = HashSet::new();
        // Functions popped before they became eagerly_called — their body
        // refs need to be re-pushed when they do.
        let mut deferred_fns: HashSet<SymbolId> = HashSet::new();
        while let Some(symbol) = worklist.pop() {
            if !visited.insert(symbol) {
                continue;
            }
            if let Some(&stmt_start) = symbol_to_stmt.get(&symbol) {
                let Some(info) = stmt_info.get(&stmt_start) else { continue };

                // Body chase for eagerly-called fn-valued bindings runs
                // BEFORE the pre-class early-continue. Even when the binding
                // itself is declared *before* the class (so its binding is
                // initialized when the class evaluates), the function body
                // it stores still fires when the decorator calls it, and
                // that body's later-declared reads are TDZ-relevant. Without
                // this, `const make = () => [{ provide: TOKEN }]` followed
                // by `@Component({providers: make()}) class C {}` and a
                // later `const TOKEN` would skip the chase entirely because
                // `make`'s stmt_start < class_body_end. Mirrors the post-
                // plan body-chase below; deferred otherwise so a later eager-
                // set promotion belatedly chases via the `now_eager` sweep.
                if fn_body_symbol_refs.contains_key(&symbol) {
                    if eagerly_called.contains(&symbol) {
                        if chased_fn_bodies.insert(symbol)
                            && let Some(body_refs) = fn_body_symbol_refs.get(&symbol)
                        {
                            for &s in body_refs {
                                if !visited.contains(&s) {
                                    worklist.push(s);
                                }
                            }
                        }
                    } else {
                        deferred_fns.insert(symbol);
                    }
                }

                // Skip bindings declared *before* this class — they're
                // already initialized when the class evaluates.
                // `class_body_end` is the exclusive end of the class body
                // (one byte past `}`), so a statement starting at exactly
                // `class_body_end` is the very next byte after the class —
                // declared *after* and still needs hoisting.
                if stmt_start < class_body_end {
                    continue;
                }

                // Safe-skip guard: if hoisting this statement would put any
                // of its initializer's references to a top-level class
                // ahead of that class's declaration, don't hoist. The
                // user's existing TDZ on the directly-referenced binding
                // (e.g. `TOKEN`) is *not* fixed here — but at least we
                // don't *introduce* a new TDZ on the class.
                //
                // Concretely guards against the multi-declarator case
                // `const TOKEN = 'tok', BACKREF = TestComponent;` where
                // hoisting the whole statement above `class TestComponent`
                // would leave `BACKREF = TestComponent` reading a not-yet-
                // declared class. The conservative alternative — splitting
                // the statement into per-declarator emissions — is out of
                // scope; this safe-skip is the minimal "no regressions"
                // defense.
                //
                // The guard ALSO refuses to hoist when the initializer
                // eagerly calls a function whose body reads a later class.
                // Without this transitive check, `var TOKEN = make()`
                // with `function make() { return TestComponent; }` would
                // be hoisted above `class TestComponent` — the hoisted
                // initializer then invokes `make()` which reads
                // `TestComponent` before its class binding is initialized,
                // throwing `ReferenceError: Cannot access 'TestComponent'
                // before initialization`. We close `info.init_called_symbols`
                // under `fn_body_called_symbols` (same shape as the BFS's
                // per-class `eagerly_called` set) and consult each
                // function's `fn_body_symbol_refs` for class refs.
                //
                // The check uses `>=`: a class declared at exactly
                // `effective_start` is itself the class we're protecting
                // — definitely blocking.
                let mut stmt_called: HashSet<SymbolId> =
                    info.init_called_symbols.iter().copied().collect();
                // Fold in any fn-valued binding declared by this statement
                // whose binding symbol is in THIS class's `eagerly_called`
                // set. When a decorator (or a chain of hoisted initializers)
                // invokes such a binding (`const make = () => …` called as
                // `make()`), the arrow/function body fires at module load
                // — its body refs are TDZ-relevant exactly like body refs of
                // a top-level function declaration the initializer calls.
                // Without this, `const make = () => TestComponent;` invoked
                // by `providers: make()` would slip past the guard because
                // the initializer's plain `init_symbols` / `init_called_symbols`
                // are empty (arrow bodies are lazy in `collect_expr_symbols`).
                if let Some(fn_syms) = stmt_fn_valued_bindings.get(&stmt_start) {
                    for &fn_sym in fn_syms {
                        if eagerly_called.contains(&fn_sym) {
                            stmt_called.insert(fn_sym);
                        }
                    }
                }
                let mut stmt_call_wl: Vec<SymbolId> = stmt_called.iter().copied().collect();
                close_eagerly_called(&mut stmt_called, &mut stmt_call_wl, &fn_body_called_symbols);

                let stmt_references_later_class = info.init_symbols.iter().any(|s| {
                    top_level_class_positions.get(s).is_some_and(|&pos| pos >= effective_start)
                }) || stmt_called.iter().any(|f| {
                    fn_body_symbol_refs.get(f).is_some_and(|refs| {
                        refs.iter().any(|s| {
                            top_level_class_positions
                                .get(s)
                                .is_some_and(|&pos| pos >= effective_start)
                        })
                    })
                });
                if stmt_references_later_class {
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

                // The hoisted statement's initializer also runs at module
                // load. Any function it calls (directly or transitively
                // through `fn_body_called_symbols`) joins the eagerly-
                // called set, so its body refs are chased too. Belatedly
                // chase any function we already popped from the worklist
                // *before* it became eagerly_called.
                let mut newly_called: Vec<SymbolId> = Vec::new();
                for &s in &info.init_called_symbols {
                    if eagerly_called.insert(s) {
                        newly_called.push(s);
                    }
                }
                close_eagerly_called(
                    &mut eagerly_called,
                    &mut newly_called,
                    &fn_body_called_symbols,
                );
                // Belated chase: any fn we already saw but skipped because
                // it wasn't eagerly_called at the time. Re-push its body
                // refs onto the worklist.
                let now_eager: Vec<SymbolId> =
                    deferred_fns.iter().copied().filter(|s| eagerly_called.contains(s)).collect();
                for s in now_eager {
                    deferred_fns.remove(&s);
                    if chased_fn_bodies.insert(s) {
                        if let Some(body_refs) = fn_body_symbol_refs.get(&s) {
                            for &r in body_refs {
                                if !visited.contains(&r) {
                                    worklist.push(r);
                                }
                            }
                        }
                    }
                }

                // Transitive hoist: if this binding's initializer references
                // another later-declared binding, that one must move above
                // the class too — otherwise the *hoisted* statement itself
                // TDZ-throws when its initializer runs. Without this,
                // `providers: PROVIDERS` followed by `const PROVIDERS = [{
                // provide: TOKEN, ... }]; const TOKEN = ...;` moves
                // `PROVIDERS` but leaves `TOKEN` below, so module evaluation
                // now throws inside the hoisted `PROVIDERS` initializer.
                for &s in &info.init_symbols {
                    if !visited.contains(&s) {
                        worklist.push(s);
                    }
                }
            } else if eagerly_called.contains(&symbol) {
                // The symbol resolves to a top-level function declaration
                // that is *actually called* (transitively) at module load
                // *for this class*. Don't hoist the function itself (JS
                // already hoists fn decls), but its body's identifier
                // reads fire whenever it runs. Chase those references.
                if chased_fn_bodies.insert(symbol) {
                    if let Some(body_refs) = fn_body_symbol_refs.get(&symbol) {
                        for &s in body_refs {
                            if !visited.contains(&s) {
                                worklist.push(s);
                            }
                        }
                    }
                }
            } else if fn_body_symbol_refs.contains_key(&symbol) {
                // Top-level function not (yet) in eagerly_called for this
                // class. Defer — if a later visit promotes it (because some
                // planned binding's initializer calls it), we'll belatedly
                // chase its body.
                deferred_fns.insert(symbol);
            }
        }
        // Fold this class's eagerly_called into the combined set used by
        // the topological sort below.
        for s in eagerly_called {
            combined_eagerly_called.insert(s);
        }
    }

    if plan.is_empty() {
        return Vec::new();
    }

    // Step 2c: cascade un-planning. The per-class BFS plans a statement S
    // when S's *immediate* `init_symbols` / closed `init_called_symbols`
    // pass the safe-skip guard. But a transitive dependency reached only
    // by chasing through `fn_body_symbol_refs` may itself get guard-skipped
    // when the BFS later pops it — leaving S planned with a missing dep.
    //
    // Example:
    //
    //   class TestComponent { ... }              // ← class C
    //   var TOKEN = make();                       // ← S: passes guard
    //   function make() { return BACKREF; }       // ← make's body
    //   const BACKREF = TestComponent;            // ← D: guard SKIPS
    //
    // S's immediate `init_called_symbols = {make}`. `make`'s body refs are
    // {BACKREF}, which is *not* a class — guard passes, S is planned. Then
    // BFS visits `make`, chases body → pushes BACKREF onto worklist. Pop
    // BACKREF: its `init_symbols = {TestComponent}` and `TestComponent`'s
    // position is `>= effective_start` → guard SKIPS BACKREF. But S
    // stayed planned. At runtime, hoisted `var TOKEN = make()` calls
    // `make()`, which reads not-yet-initialized `BACKREF`, which reads
    // not-yet-initialized `TestComponent`. TDZ.
    //
    // Fix (Approach B from the review): after BFS, compute each planned
    // statement's *full* dependency closure (through eagerly-called fn
    // bodies via `combined_eagerly_called`). If any closure symbol resolves
    // to a top-level binding whose own statement is NOT in the plan AND
    // would have needed hoisting (its position is at-or-after S's
    // `insert_at`), drop S. Iterate to fixed point so the un-plan can
    // cascade: dropping S may itself orphan another planned T that
    // depended only on S.
    //
    // Function-symbol deps (e.g. `make` itself) are NOT flagged here
    // because `symbol_to_stmt.get(&make)` returns None — top-level
    // function declarations are JS-hoisted, not handled by the variable
    // planner. The chase-through-fn-bodies in `expand_through_functions`
    // bridges to the variable bindings they read.
    loop {
        let plan_starts_snapshot: HashSet<u32> = plan.keys().copied().collect();
        let mut to_remove: Vec<u32> = Vec::new();
        for (&start, entry) in &plan {
            let Some(info) = stmt_info.get(&start) else { continue };
            // Use a *per-S* eager-call set —
            // the closure of THIS statement's `init_called_symbols` under
            // `fn_body_called_symbols` — instead of the global
            // `combined_eagerly_called`. The global union over-expands: if
            // class B eagerly calls `make` and class A only references
            // `makeRef = make` as a *value*, the global set pulls A's
            // closure through `make`'s body refs as if A called `make`,
            // which can drop A's safe hoist. The per-S set matches the
            // shape used by the safe-skip guard's `stmt_called` so the
            // cascade un-planning reasons against the same eager-evaluation
            // set the planner used.
            let mut stmt_called: HashSet<SymbolId> =
                info.init_called_symbols.iter().copied().collect();
            // Mirror the BFS safe-skip guard: fold in fn-valued bindings
            // this statement declares whose binding symbol is in
            // `combined_eagerly_called` (no per-class scoping here — the
            // cascade runs after the BFS unions per-class eager sets). Then
            // seed the closure with those binding symbols so
            // `expand_through_functions` descends into their arrow/function
            // bodies. Without this, `const make = () => BACKREF;` planned
            // for a class that calls `make()` would not see its body's
            // dependency on `BACKREF` (whose own statement was guard-
            // skipped), so the cascade would fail to drop `make` and the
            // hoisted `make()` would TDZ on `BACKREF`.
            let mut seed: HashSet<SymbolId> = info.init_symbols.clone();
            if let Some(fn_syms) = stmt_fn_valued_bindings.get(&start) {
                for &fn_sym in fn_syms {
                    if combined_eagerly_called.contains(&fn_sym) {
                        stmt_called.insert(fn_sym);
                        seed.insert(fn_sym);
                    }
                }
            }
            let mut stmt_call_wl: Vec<SymbolId> = stmt_called.iter().copied().collect();
            close_eagerly_called(&mut stmt_called, &mut stmt_call_wl, &fn_body_called_symbols);
            let closure = expand_through_functions(&seed, &fn_body_symbol_refs, &stmt_called);
            for s in &closure {
                // Function symbols and unresolved refs have no
                // `symbol_to_stmt` entry — they can't be a variable
                // binding the planner would have moved. Skip.
                let Some(&dep_start) = symbol_to_stmt.get(s) else { continue };
                // Self-references (multi-declarator stmt referencing its own
                // sibling) don't count.
                if dep_start == start {
                    continue;
                }
                // Dep is in the plan — only safe if its `insert_at` is at
                // or before S's `insert_at`:
                // two planned statements can target *different* `insert_at`
                // positions (one per decorated class). If S targets
                // `insert_at = pos_C1` and its dep `D` is planned only for
                // class C2 with `D.insert_at = pos_C2 > pos_C1`, hoisted S
                // runs before hoisted D at runtime — fresh TDZ on D. The
                // snapshot-membership check we used before missed this; we
                // must consult the dep's *current* `insert_at` and unplan
                // S when the ordering is wrong.
                if plan_starts_snapshot.contains(&dep_start) {
                    if let Some(dep_entry) = plan.get(&dep_start)
                        && dep_entry.insert_at <= entry.insert_at
                    {
                        continue;
                    }
                    // Fall through — dep's `insert_at` is *after* S's,
                    // treat the dep as unsafe and unplan S. (Lowering the
                    // dep's `insert_at` instead is the alternative; we
                    // pick "drop S" for simplicity — the user's existing
                    // TDZ on the dep persists, but we don't introduce a
                    // fresh class TDZ via partial hoisting.)
                    to_remove.push(start);
                    break;
                }
                // Dep is declared *before* the class we're hoisting in
                // front of — already initialized when S evaluates.
                if dep_start <= entry.insert_at {
                    continue;
                }
                // Dep would have to be hoisted but isn't planned — S is
                // unsafe. Flag and stop scanning this S.
                to_remove.push(start);
                break;
            }
        }
        if to_remove.is_empty() {
            break;
        }
        for s in to_remove {
            plan.remove(&s);
        }
    }

    if plan.is_empty() {
        return Vec::new();
    }

    // Step 3: topologically sort the planned statements so dependencies are
    // emitted *before* their dependents in the hoisted prelude. Within a
    // single bucket (same `insert_at`), this guarantees that e.g. `const
    // TOKEN` precedes `const PROVIDERS = [{ provide: TOKEN, ... }]`.
    //
    // Precompute `stmt_eager_sets`: per-S closure of
    // `info.init_called_symbols` under `fn_body_called_symbols`. This is
    // the same shape the cascade un-planning loop computes for its
    // `stmt_called` set — passing it (instead of the global
    // `combined_eagerly_called`) into `topological_order` makes the two
    // passes reason against the same eager-evaluation set. The global
    // union can over-expand: a function `make` eagerly called only by
    // class B leaks into class A's `makeRef = make` closure when
    // computing topo edges, forming a spurious edge that may invert
    // ordering or trigger the cycle-break.
    //
    // `stmt_fn_valued_bindings` (computed once near the top of this
    // function) is consulted here too — see the doc comment on
    // `topological_order` for why each statement's fn-valued binding
    // symbols seed the topo edges.
    let mut stmt_eager_sets: HashMap<u32, HashSet<SymbolId>> = HashMap::with_capacity(plan.len());
    for &start in plan.keys() {
        let Some(info) = stmt_info.get(&start) else {
            stmt_eager_sets.insert(start, HashSet::new());
            continue;
        };
        let mut stmt_called: HashSet<SymbolId> = info.init_called_symbols.iter().copied().collect();
        let mut stmt_call_wl: Vec<SymbolId> = stmt_called.iter().copied().collect();
        close_eagerly_called(&mut stmt_called, &mut stmt_call_wl, &fn_body_called_symbols);
        // Function-valued bindings declared by this statement are part
        // of THIS statement's eager-call surface when their initializer
        // is invoked at module load (i.e. the binding's symbol is in
        // some class's `eagerly_called`). Including them here lets
        // `expand_through_functions` chase through the arrow/function
        // body's refs to find dependency edges to other planned
        // statements — without this the binding's body refs are invisible
        // to the topo sort, mirroring the same issue Bug 1 fixed in the
        // BFS pop branch.
        if let Some(fn_syms) = stmt_fn_valued_bindings.get(&start) {
            for &fn_sym in fn_syms {
                if combined_eagerly_called.contains(&fn_sym) {
                    stmt_called.insert(fn_sym);
                }
            }
        }
        stmt_eager_sets.insert(start, stmt_called);
    }

    let order = topological_order(
        &plan,
        &symbol_to_stmt,
        &stmt_info,
        &fn_body_symbol_refs,
        &stmt_eager_sets,
        &stmt_fn_valued_bindings,
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
///
/// `stmt_eager_sets` is the per-planned-statement closure of
/// `init_called_symbols` under `fn_body_called_symbols`, matching the
/// shape the cascade un-planning loop uses. Passing per-S sets instead of
/// the global `combined_eagerly_called` keeps the cascade and topo
/// passes reasoning against the same eager-evaluation surface.
///
/// `stmt_fn_valued_bindings` maps each planned `stmt_start` to the
/// function-valued binding symbols it declares (e.g. `make` for
/// `const make = () => TOKEN;`). Their `fn_body_symbol_refs` entries are
/// chased to surface body-ref dependencies that are invisible to the
/// statement's plain `init_symbols`.
fn topological_order(
    plan: &HashMap<u32, PlanEntry>,
    symbol_to_stmt: &HashMap<SymbolId, u32>,
    stmt_info: &HashMap<u32, StmtInfo>,
    fn_body_symbol_refs: &HashMap<SymbolId, HashSet<SymbolId>>,
    stmt_eager_sets: &HashMap<u32, HashSet<SymbolId>>,
    stmt_fn_valued_bindings: &HashMap<u32, Vec<SymbolId>>,
) -> Vec<u32> {
    let plan_starts: HashSet<u32> = plan.keys().copied().collect();
    let empty_eager: HashSet<SymbolId> = HashSet::new();

    // Adjacency list: stmt_start -> stmt_starts it depends on (must come
    // *before* it). Filter to only edges that land inside the plan; deps that
    // resolve outside (declared before the class, or not top-level) are
    // already TDZ-safe.
    //
    // The "effective init symbols" of a planned statement are the transitive
    // closure of its direct `init_symbols` through `fn_body_symbol_refs`,
    // **restricted to functions in this statement's per-S eager-call set**.
    // If the initializer calls a function (directly or transitively), the
    // function body's identifier reads count as references that fire when
    // the hoisted statement evaluates. Functions only stored as values are
    // NOT expanded — their bodies don't run at module load.
    //
    // Function-valued binding symbols this statement declares (e.g. `make`
    // in `const make = () => TOKEN;`) are added to the seed so the
    // expansion descends into their arrow/function bodies — those body
    // refs are dependencies of THIS statement at runtime, but invisible
    // to plain `init_symbols` because arrow bodies are lazy.
    let mut deps: HashMap<u32, Vec<u32>> = HashMap::with_capacity(plan_starts.len());
    for &start in &plan_starts {
        let Some(info) = stmt_info.get(&start) else {
            deps.insert(start, Vec::new());
            continue;
        };
        let eager = stmt_eager_sets.get(&start).unwrap_or(&empty_eager);
        let mut seed: HashSet<SymbolId> = info.init_symbols.clone();
        if let Some(fn_syms) = stmt_fn_valued_bindings.get(&start) {
            for &fn_sym in fn_syms {
                seed.insert(fn_sym);
            }
        }
        let effective = expand_through_functions(&seed, fn_body_symbol_refs, eager);
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

/// Close the `eagerly_called` set under `fn_body_called_symbols`: pop each
/// symbol from `worklist`, for every function it directly calls, insert
/// into `eagerly_called` and (if newly inserted) push onto the worklist.
/// Runs until the worklist drains.
///
/// Used by the per-class BFS in [`collect_hoist_edits`]. The caller seeds
/// `eagerly_called` and `worklist` with that class's `decorator_called`
/// (plus, on incremental updates, the `init_called_symbols` of newly
/// planned bindings); we extend the closure to fixed point. A function
/// stored as a value (referenced but not called) is NOT added — that's
/// what prevents `useFactory: makeFactory` from invoking `makeFactory`'s
/// body refs at class-init time.
///
/// Per-class scoping: the seed is THIS class's call graph only. A function
/// invoked elsewhere in the module but only referenced as a value in this
/// class's metadata does not enter this class's set.
fn close_eagerly_called(
    eagerly_called: &mut HashSet<SymbolId>,
    worklist: &mut Vec<SymbolId>,
    fn_body_called_symbols: &HashMap<SymbolId, HashSet<SymbolId>>,
) {
    while let Some(symbol) = worklist.pop() {
        if let Some(calls) = fn_body_called_symbols.get(&symbol) {
            for &s in calls {
                if eagerly_called.insert(s) {
                    worklist.push(s);
                }
            }
        }
    }
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

/// Index every top-level class declaration by its binding `SymbolId` →
/// the class's `span.start`. Covers plain `ClassDeclaration`,
/// `export class …`, and `export default class …` (only the named form —
/// anonymous default-exported classes have no `id`).
///
/// Used by the BFS safe-skip guard in [`collect_hoist_edits`] to refuse
/// hoisting a statement whose initializer references a class declared
/// at-or-after the protect site, which would introduce a new TDZ on the
/// class itself.
fn collect_top_level_class_positionss(program: &Program<'_>) -> HashMap<SymbolId, u32> {
    let mut out: HashMap<SymbolId, u32> = HashMap::new();
    for stmt in &program.body {
        let Some((class, _)) = class_of(stmt) else { continue };
        let Some(id) = &class.id else { continue };
        let Some(symbol) = id.symbol_id.get() else { continue };
        out.insert(symbol, class.span.start);
    }
    out
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
///   `close_eagerly_called` so each class's BFS only chases bodies that
///   are eagerly reachable from that class's decorator-metadata call
///   graph.
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
                // Per-declarator: if the declarator's `id` is a plain
                // `BindingIdentifier` AND its `init` is *directly* an
                // arrow/function expression (after peeling parens / TS
                // wrappers), index that binding symbol as if it were a
                // function declaration — populate `fn_body_symbol_refs` /
                // `fn_body_called_symbols` so the BFS safe-skip guard can
                // chase eager calls through it. Handles both single- and
                // multi-declarator cases (`const make = () => DEP;` and
                // `const make = () => DEP, other = 0;`). Destructured /
                // patterned bindings are skipped — the function-value shape
                // only appears with a plain identifier binding in practice.
                // Run this BEFORE collecting `init_symbols` so the indexing
                // happens before the normal binding/init flow.
                if let (BindingPattern::BindingIdentifier(id), Some(init)) =
                    (&declarator.id, &declarator.init)
                    && let Some(fn_symbol) = id.symbol_id.get()
                {
                    index_fn_valued_binding(
                        init,
                        fn_symbol,
                        semantic,
                        &mut fn_body_symbol_refs,
                        &mut fn_body_called_symbols,
                    );
                }

                // Walk the declarator's `BindingPattern` recursively so that
                // destructuring forms (`const { TOKEN } = obj;`, `const [a, b]
                // = arr;`, `const { a: { b } } = obj;`, …) also index every
                // binding identifier they introduce. Without this, decorator
                // metadata referencing such a binding never resolves to its
                // declaring statement and the hoist is skipped.
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
                // Destructuring defaults (`const { x = FALLBACK } = obj`)
                // fire at evaluation time of THIS statement whenever the
                // matching property is missing/undefined. They read the
                // default expression's identifiers eagerly, so any later-
                // declared top-level binding referenced by a default is
                // TDZ-relevant exactly like the `init` itself. Walk every
                // nested `AssignmentPattern::right` in the binding pattern
                // and feed its refs into the statement's eager sets.
                for_each_pattern_default(&declarator.id, &mut |expr| {
                    collect_expr_symbols(
                        expr,
                        semantic,
                        &mut info.init_symbols,
                        &mut info.init_called_symbols,
                    );
                });
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
                // Parameter defaults (`function f(x = TOKEN)`) evaluate at
                // call time, before the body runs. If this function is
                // eagerly called from a hoisted initializer, any later-
                // declared binding read by a default is just as TDZ-relevant
                // as a body ref. Walk the `FormalParameter::initializer`
                // directly, plus any nested `AssignmentPattern::right` inside
                // destructured params (`function f({ a = X } = {})`).
                walk_param_defaults(&func.params, semantic, &mut refs, &mut called);
                fn_body_symbol_refs.insert(fn_symbol, refs);
                fn_body_called_symbols.insert(fn_symbol, called);
            }
            continue;
        }

        // Top-level class declarations. We don't *move* classes (they're
        // gated separately and never added to `symbol_to_stmt`), but we
        // index the eager parts of each class so the BFS can chase
        // TDZ-relevant identifiers through `new ClassName()` callers in
        // hoisted initializers. Mirrors the function-declaration arm above:
        // `fn_body_symbol_refs[class_symbol]` receives every identifier read
        // at class-definition time (super_class, computed keys, static
        // initializers, static blocks) AND `new`-time (constructor body +
        // params + instance field initializers); `fn_body_called_symbols`
        // receives direct callees seen in those parts.
        //
        // The over-counting (definition-time eager merged with new-time
        // eager) is intentional. The BFS only ever uses these maps to
        // *block* hoisting that would introduce a fresh TDZ — never to
        // greenlight one — so extending the body-ref set can only
        // over-block, never under-block.
        if let Some((class, _)) = class_of(stmt) {
            if let Some(id) = &class.id
                && let Some(class_symbol) = id.symbol_id.get()
            {
                let mut refs: HashSet<SymbolId> = HashSet::new();
                let mut called: HashSet<SymbolId> = HashSet::new();
                walk_class_eager_parts(
                    class,
                    /* include_constructor */ true,
                    semantic,
                    &mut refs,
                    &mut called,
                );
                fn_body_symbol_refs.insert(class_symbol, refs);
                fn_body_called_symbols.insert(class_symbol, called);
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
/// = obj`) are NOT visited by this helper — it only enumerates binding
/// identifiers. Those defaults ARE chased separately by
/// [`for_each_pattern_default`], which callers run at every site where a
/// pattern's defaults evaluate eagerly: at declarator sites in
/// `collect_top_level_bindings`, and at IIFE / parameter-default call sites
/// in [`walk_param_defaults`] / [`walk_iife_callee_body`]. Keeping the two
/// concerns split lets each call site decide whether the defaults are
/// TDZ-relevant for that context.
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

/// Walk a `BindingPattern` and invoke `f` for every default-value
/// `Expression` it carries — i.e. the `right` of every nested
/// `AssignmentPattern`. Used to chase TDZ-relevant identifier reads inside
/// parameter destructuring defaults like `function f({ a = X } = {})`,
/// where the inner `a = X` is an `AssignmentPattern` whose `right` is the
/// `X` default expression.
///
/// `FormalParameter`'s top-level default (`function f(x = TOKEN)`) lives on
/// `FormalParameter::initializer`, NOT inside an `AssignmentPattern`, so
/// callers walk that separately and use this helper to cover the *nested*
/// pattern-default case only.
fn for_each_pattern_default<'a, 'src>(
    pat: &'src BindingPattern<'a>,
    f: &mut impl FnMut(&'src Expression<'a>),
) {
    match pat {
        BindingPattern::BindingIdentifier(_) => {}
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                for_each_pattern_default(&prop.value, f);
            }
            if let Some(rest) = &obj.rest {
                for_each_pattern_default(&rest.argument, f);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for el in &arr.elements {
                if let Some(el) = el {
                    for_each_pattern_default(el, f);
                }
            }
            if let Some(rest) = &arr.rest {
                for_each_pattern_default(&rest.argument, f);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            f(&assign.right);
            for_each_pattern_default(&assign.left, f);
        }
    }
}

/// Walk a class's eager parts and feed identifier refs / direct callees
/// into `out` / `called`. "Eager parts" depend on `include_constructor`:
///
/// * Class-definition-time eager (always walked): `super_class` expression,
///   computed keys on any member, static field/accessor initializers,
///   static blocks.
/// * `new`-time eager (walked when `include_constructor` is true):
///   constructor body + constructor parameter defaults, instance field /
///   accessor initializers (these run inside the synthesized constructor).
///
/// For a top-level *class declaration* indexed as if it were a "function"
/// in `fn_body_symbol_refs`, we want `include_constructor = true` —
/// `new ClassName()` triggers both definition-time AND constructor-time
/// eager evaluations. Over-counting the definition-time parts is fine; it
/// only over-blocks hoisting, never under-blocks.
///
/// For a *class expression* embedded inside an eagerly-evaluated decorator
/// argument, the class expression
/// itself is being defined inline — so only the class-definition-time
/// eager parts fire here. Instance methods/fields/constructor bodies are
/// lazy until someone calls `new` on the class, which the metadata can't
/// see. Use `include_constructor = false`.
///
/// Member decorators and the class's own decorators are skipped — decorator
/// factory invocation is special and out of scope.
fn walk_class_eager_parts<'a>(
    class: &Class<'a>,
    include_constructor: bool,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    // `super_class` evaluates at class-definition time, before the body
    // executes. Always walk it.
    if let Some(super_expr) = &class.super_class {
        collect_expr_symbols(super_expr, semantic, out, called);
    }
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                // Computed keys fire at class-definition time regardless of
                // method kind.
                if method.computed
                    && let Some(key_expr) = method.key.as_expression()
                {
                    collect_expr_symbols(key_expr, semantic, out, called);
                }
                // Constructor body + parameter defaults fire at `new`-time.
                if include_constructor && method.kind == MethodDefinitionKind::Constructor {
                    if let Some(body) = &method.value.body {
                        let mut visitor = FunctionBodyIdentVisitor { semantic, out, called };
                        visitor.visit_function_body(body);
                    }
                    walk_param_defaults(&method.value.params, semantic, out, called);
                }
                // Non-constructor instance method bodies are lazy; static
                // method bodies are also lazy (they're properties on the
                // class object, executed when called). Skip both.
            }
            ClassElement::PropertyDefinition(prop) => {
                if prop.computed
                    && let Some(key_expr) = prop.key.as_expression()
                {
                    collect_expr_symbols(key_expr, semantic, out, called);
                }
                // Static field initializers fire at class-definition time.
                // Instance field initializers fire at `new`-time inside the
                // synthesized constructor.
                if let Some(value) = &prop.value {
                    if prop.r#static {
                        collect_expr_symbols(value, semantic, out, called);
                    } else if include_constructor {
                        collect_expr_symbols(value, semantic, out, called);
                    }
                }
            }
            ClassElement::AccessorProperty(accessor) => {
                if accessor.computed
                    && let Some(key_expr) = accessor.key.as_expression()
                {
                    collect_expr_symbols(key_expr, semantic, out, called);
                }
                if let Some(value) = &accessor.value {
                    if accessor.r#static {
                        collect_expr_symbols(value, semantic, out, called);
                    } else if include_constructor {
                        collect_expr_symbols(value, semantic, out, called);
                    }
                }
            }
            ClassElement::StaticBlock(block) => {
                // `static { … }` body runs once at class-definition time.
                // Walk it like an eagerly-evaluated function body.
                let mut visitor = FunctionBodyIdentVisitor { semantic, out, called };
                for stmt in &block.body {
                    visitor.visit_statement(stmt);
                }
            }
            // `TSIndexSignature` is type-only, erased at runtime.
            ClassElement::TSIndexSignature(_) => {}
        }
    }
}

/// Walk every parameter default expression of a function/arrow's
/// `FormalParameters` and feed the refs / direct callees into the same
/// `out` / `called` sets the body visitor populates. Defaults are
/// evaluated at call time before the body runs, so for an eagerly-called
/// function they're as relevant as body refs.
///
/// Two default shapes are covered:
/// * `param.initializer` — the top-level default for a `FormalParameter`
///   (e.g. the `= TOKEN` in `function f(token = TOKEN)`).
/// * `AssignmentPattern.right` nested anywhere inside the parameter's
///   `BindingPattern` (e.g. the inner `= X` in
///   `function f({ a = X } = {})`).
fn walk_param_defaults<'a>(
    params: &FormalParameters<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    for param in &params.items {
        if let Some(init) = &param.initializer {
            collect_expr_symbols(init, semantic, out, called);
        }
        for_each_pattern_default(&param.pattern, &mut |expr| {
            collect_expr_symbols(expr, semantic, out, called);
        });
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
        record_indirect_callee(&it.callee, self.semantic, self.called);
        record_bind_callee(&it.callee, self.semantic, self.called);
        // IIFE detection mirrors the `collect_expr_symbols` arm: when the
        // callee is `(() => ...)` / `(function() { ... })`, the body runs
        // eagerly at this call site, so its identifier reads contribute to
        // the eager-evaluation set. Without this, `visit_arrow_function`
        // / `visit_function` (intentional no-ops below) would silently drop
        // the IIFE body inside an eagerly-called function — TDZ regression.
        if walk_iife_callee_body(&it.callee, self.semantic, self.out, self.called) {
            // Body handled; only the arguments still need to flow into
            // `self.out` / `self.called`.
            for arg in &it.arguments {
                self.visit_argument(arg);
            }
            return;
        }
        // Continue default traversal so identifier references inside callee
        // and arguments still feed `self.out`.
        oxc_ast_visit::walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &oxc_ast::ast::NewExpression<'a>) {
        record_direct_callee(&it.callee, self.semantic, self.called);
        record_indirect_callee(&it.callee, self.semantic, self.called);
        record_bind_callee(&it.callee, self.semantic, self.called);
        // Symmetric IIFE handling for `new (function() { ... })()`.
        if walk_iife_callee_body(&it.callee, self.semantic, self.out, self.called) {
            for arg in &it.arguments {
                self.visit_argument(arg);
            }
            return;
        }
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

    // Class expressions inside an eagerly-called function body evaluate
    // their eager parts (`super_class`, computed keys, static field /
    // accessor initializers, static blocks) at call time of the outer
    // function. The instance / static method bodies and instance field
    // initializers are lazy until something `new`s the class — but if the
    // surrounding code does call `new` here, the constructor body and
    // parameter defaults fire too. Over-counting only over-blocks (it never
    // under-blocks), so include the constructor to stay conservative.
    fn visit_class(&mut self, it: &Class<'a>) {
        walk_class_eager_parts(
            it,
            /* include_constructor */ true,
            self.semantic,
            self.out,
            self.called,
        );
    }

    fn visit_tagged_template_expression(
        &mut self,
        it: &oxc_ast::ast::TaggedTemplateExpression<'a>,
    ) {
        // Mirror `visit_call_expression`: a tagged template `tag`...`` invokes
        // `tag`, so its direct/indirect/bind callee shapes contribute to
        // `called` just like a `CallExpression`. Without this override, the
        // default walk reaches `tag` via `visit_identifier_reference` (which
        // only feeds `out`), so the tag's body never gets chased through
        // `eagerly_called`.
        record_direct_callee(&it.tag, self.semantic, self.called);
        record_indirect_callee(&it.tag, self.semantic, self.called);
        record_bind_callee(&it.tag, self.semantic, self.called);
        oxc_ast_visit::walk::walk_tagged_template_expression(self, it);
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
            record_indirect_callee(&call.callee, semantic, called);
            record_bind_callee(&call.callee, semantic, called);
            // IIFE detection: `(() => ...)()` or `(function() { ... })()` —
            // the function body runs *eagerly* at this call site, so its
            // identifier reads contribute to the eager-evaluation set. The
            // default `ArrowFunctionExpression` / `FunctionExpression`
            // arms below treat bodies as lazy; for IIFEs we walk the body
            // explicitly via `FunctionBodyIdentVisitor` instead.
            if !walk_iife_callee_body(&call.callee, semantic, out, called) {
                collect_expr_symbols(&call.callee, semantic, out, called);
            }
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
            record_indirect_callee(&new.callee, semantic, called);
            record_bind_callee(&new.callee, semantic, called);
            // Symmetric IIFE handling for `new (function() { ... })()` —
            // exceedingly rare but covered for consistency.
            if !walk_iife_callee_body(&new.callee, semantic, out, called) {
                collect_expr_symbols(&new.callee, semantic, out, called);
            }
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
            // Mirror the call/new arms: a tagged template invokes the tag
            // function eagerly, so direct, member-call (`fn.call`, `fn.apply`),
            // and `fn.bind(...)` shapes must all enter `called`. Without the
            // indirect/bind helpers here, `make.bind(null)\`...\`` in decorator
            // metadata would record `make` as a value reference but never chase
            // its body.
            record_direct_callee(&tagged.tag, semantic, called);
            record_indirect_callee(&tagged.tag, semantic, called);
            record_bind_callee(&tagged.tag, semantic, called);
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
        // `(x = TOKEN)` — both sides carry refs that fire at evaluation
        // time. The `right` is a regular expression; the `left` is an
        // `AssignmentTarget` (bare identifier, member, or pattern-shaped)
        // walked via the dedicated helper. Without this, decorator metadata
        // shaped `providers: [(cached = TOKEN)]` silently dropped `TOKEN`.
        E::AssignmentExpression(assign) => {
            collect_expr_symbols(&assign.right, semantic, out, called);
            collect_assignment_target_symbols(&assign.left, semantic, out, called);
        }
        // `x++`, `--y[k]`, etc. The `argument` is a `SimpleAssignmentTarget`
        // — bare identifiers and member expressions, never patterns.
        E::UpdateExpression(update) => {
            collect_simple_assignment_target_symbols(&update.argument, semantic, out, called);
        }
        // Class expressions inside an eagerly-evaluated context. Several
        // parts of a class expression fire at class-definition time and
        // are TDZ-relevant: the `super_class` expression, computed keys
        // on any member, static field / accessor initializers, and static
        // blocks. Instance methods, instance fields, and the constructor
        // body are lazy until someone calls `new` on the class — and the
        // metadata can't see that call, so they stay opaque.
        //
        // Member decorators and the class expression's own decorators are
        // skipped here.
        E::ClassExpression(class_expr) => {
            walk_class_eager_parts(
                class_expr.as_ref(),
                /* include_constructor */ false,
                semantic,
                out,
                called,
            );
        }
        // Function and arrow bodies run lazily — references inside don't
        // affect class-init evaluation.
        E::ArrowFunctionExpression(_) | E::FunctionExpression(_) => {}
        // Remaining variants carry no identifier references we can resolve
        // to a top-level binding: literals (string/number/boolean/null/regex/
        // big-int/template no-substitution), `this`, `Super`, `MetaProperty`
        // (`import.meta` / `new.target`), `ImportExpression` (dynamic import
        // takes a string literal in practice), and `JSX*` / `V8IntrinsicExpression`
        // which aren't valid in TS source we transform.
        _ => {}
    }
}

/// Walk an `AssignmentTarget` (the `left` of an `AssignmentExpression`,
/// or a nested element inside an array/object pattern target) and feed
/// every identifier reference into `out` / `called`. Member arms mirror
/// the corresponding `Expression::*MemberExpression` arms in
/// [`collect_expr_symbols`]; pattern arms recurse through their nested
/// targets and defaults so e.g. `({ x = TOKEN } = obj)` chases `TOKEN`.
fn collect_assignment_target_symbols<'a>(
    target: &oxc_ast::ast::AssignmentTarget<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    use oxc_ast::ast::AssignmentTarget as T;
    match target {
        T::AssignmentTargetIdentifier(id) => {
            if let Some(symbol) = resolve_symbol(id, semantic) {
                out.insert(symbol);
            }
        }
        T::ComputedMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
            collect_expr_symbols(&member.expression, semantic, out, called);
        }
        T::StaticMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        T::PrivateFieldExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        T::TSAsExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::TSSatisfiesExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::TSNonNullExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::TSTypeAssertion(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::ArrayAssignmentTarget(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_assignment_target_maybe_default_symbols(el, semantic, out, called);
            }
            if let Some(rest) = &arr.rest {
                collect_assignment_target_symbols(&rest.target, semantic, out, called);
            }
        }
        T::ObjectAssignmentTarget(obj) => {
            for prop in &obj.properties {
                collect_assignment_target_property_symbols(prop, semantic, out, called);
            }
            if let Some(rest) = &obj.rest {
                collect_assignment_target_symbols(&rest.target, semantic, out, called);
            }
        }
    }
}

/// `SimpleAssignmentTarget` is the subset of `AssignmentTarget` allowed
/// as the `argument` of `++`/`--`. Same shape minus the pattern variants.
fn collect_simple_assignment_target_symbols<'a>(
    target: &oxc_ast::ast::SimpleAssignmentTarget<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    use oxc_ast::ast::SimpleAssignmentTarget as T;
    match target {
        T::AssignmentTargetIdentifier(id) => {
            if let Some(symbol) = resolve_symbol(id, semantic) {
                out.insert(symbol);
            }
        }
        T::ComputedMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
            collect_expr_symbols(&member.expression, semantic, out, called);
        }
        T::StaticMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        T::PrivateFieldExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        T::TSAsExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::TSSatisfiesExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::TSNonNullExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        T::TSTypeAssertion(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
    }
}

/// Helper for array-pattern element / object-pattern property values:
/// either a plain `AssignmentTarget` or an `AssignmentTargetWithDefault`
/// (`[a = X]`, `{ p: a = X }`) whose `init` default evaluates at
/// destructuring time.
fn collect_assignment_target_maybe_default_symbols<'a>(
    el: &oxc_ast::ast::AssignmentTargetMaybeDefault<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    use oxc_ast::ast::AssignmentTargetMaybeDefault as D;
    match el {
        D::AssignmentTargetWithDefault(with_default) => {
            collect_assignment_target_symbols(&with_default.binding, semantic, out, called);
            collect_expr_symbols(&with_default.init, semantic, out, called);
        }
        // The remaining variants inherit from `AssignmentTarget`. The
        // `AssignmentTarget` variants are matched implicitly by the parent
        // enum's `inherit_variants!` macro; cast back through the helper.
        D::AssignmentTargetIdentifier(id) => {
            if let Some(symbol) = resolve_symbol(id, semantic) {
                out.insert(symbol);
            }
        }
        D::ComputedMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
            collect_expr_symbols(&member.expression, semantic, out, called);
        }
        D::StaticMemberExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        D::PrivateFieldExpression(member) => {
            collect_expr_symbols(&member.object, semantic, out, called);
        }
        D::TSAsExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        D::TSSatisfiesExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        D::TSNonNullExpression(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        D::TSTypeAssertion(ts) => collect_expr_symbols(&ts.expression, semantic, out, called),
        D::ArrayAssignmentTarget(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_assignment_target_maybe_default_symbols(el, semantic, out, called);
            }
            if let Some(rest) = &arr.rest {
                collect_assignment_target_symbols(&rest.target, semantic, out, called);
            }
        }
        D::ObjectAssignmentTarget(obj) => {
            for prop in &obj.properties {
                collect_assignment_target_property_symbols(prop, semantic, out, called);
            }
            if let Some(rest) = &obj.rest {
                collect_assignment_target_symbols(&rest.target, semantic, out, called);
            }
        }
    }
}

/// `({ foo } = obj)` shorthand vs. `({ foo: bar } = obj)` long form.
/// The shorthand carries an optional `init` default; the long form carries
/// a key (possibly computed — `{ [TOKEN]: x }`) and a `binding` that's a
/// nested target with optional default.
fn collect_assignment_target_property_symbols<'a>(
    prop: &oxc_ast::ast::AssignmentTargetProperty<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) {
    use oxc_ast::ast::AssignmentTargetProperty as P;
    match prop {
        P::AssignmentTargetPropertyIdentifier(ident) => {
            if let Some(symbol) = resolve_symbol(&ident.binding, semantic) {
                out.insert(symbol);
            }
            if let Some(init) = &ident.init {
                collect_expr_symbols(init, semantic, out, called);
            }
        }
        P::AssignmentTargetPropertyProperty(prop) => {
            if prop.computed
                && let Some(key_expr) = prop.name.as_expression()
            {
                collect_expr_symbols(key_expr, semantic, out, called);
            }
            collect_assignment_target_maybe_default_symbols(&prop.binding, semantic, out, called);
        }
    }
}

/// If `callee` resolves to one or more *direct* identifier references
/// (peeling through parentheses and TS type-only wrappers, and descending
/// into the branches of conditional / logical / sequence callees), record
/// each symbol in `called`. Member callees (`foo.bar()`) and other complex
/// expressions are skipped — only direct callees of
/// `CallExpression`/`NewExpression` count as eager invocations of a
/// top-level function.
///
/// Conditional (`(cond ? a : b)()`), logical (`(a || b)()`), and sequence
/// (`(x, y, z)()`) callees are first-class shapes: either branch of the
/// conditional/logical may end up invoked, and the last expression in a
/// sequence is the result whose callee is invoked. The worklist below
/// pushes both branches of `?:` / `||`/`&&`/`??` and the LAST expression of
/// a sequence, with a `seen` guard so cycles or shared subtrees in the AST
/// don't loop forever (in practice each `Expression` node is unique, but
/// guarding by raw pointer is cheap insurance against quadratic blow-up on
/// pathological inputs).
fn record_direct_callee<'a>(
    callee: &Expression<'a>,
    semantic: &Semantic<'a>,
    called: &mut HashSet<SymbolId>,
) {
    use Expression as E;
    let mut worklist: Vec<&Expression<'a>> = vec![callee];
    let mut seen: HashSet<*const Expression<'a>> = HashSet::new();
    while let Some(mut cur) = worklist.pop() {
        loop {
            let key = cur as *const Expression<'a>;
            if !seen.insert(key) {
                break;
            }
            match cur {
                E::Identifier(id) => {
                    if let Some(symbol) = resolve_symbol(id, semantic) {
                        called.insert(symbol);
                    }
                    break;
                }
                E::ParenthesizedExpression(p) => cur = &p.expression,
                E::TSAsExpression(ts) => cur = &ts.expression,
                E::TSSatisfiesExpression(ts) => cur = &ts.expression,
                E::TSNonNullExpression(ts) => cur = &ts.expression,
                E::TSTypeAssertion(ts) => cur = &ts.expression,
                E::TSInstantiationExpression(ts) => cur = &ts.expression,
                E::ConditionalExpression(cond) => {
                    worklist.push(&cond.consequent);
                    worklist.push(&cond.alternate);
                    break;
                }
                E::LogicalExpression(log) => {
                    worklist.push(&log.left);
                    worklist.push(&log.right);
                    break;
                }
                E::SequenceExpression(seq) => {
                    // Only the last expression's value becomes the callee.
                    if let Some(last) = seq.expressions.last() {
                        worklist.push(last);
                    }
                    break;
                }
                _ => break,
            }
        }
    }
}

/// Recognize a small set of *indirect* call shapes whose immediate effect
/// is to invoke a top-level function:
///
/// * `fn.call(...)` — `Function.prototype.call`
/// * `fn.apply(...)` — `Function.prototype.apply`
///
/// In both cases the static member's `object` must be a *direct identifier*
/// (`fn`) — we resolve through the semantic model and record the symbol
/// in `called`. Anything more nested (`obj.fn.call(...)`,
/// `getFn().call(...)`) is out of scope and falls through.
///
/// The shape `fn.bind(...)()` is handled at the call site by inspecting
/// the *outer* call's callee: if it's a `CallExpression` whose own callee
/// is `Identifier.bind`, the inner identifier is the bound function and
/// will eventually invoke at the outer call site. See [`record_bind_callee`].
///
/// Used alongside [`record_direct_callee`] at every call/new site so the
/// guard's `init_called_symbols` reflects the actual eager-invocation set.
fn record_indirect_callee<'a>(
    callee: &Expression<'a>,
    semantic: &Semantic<'a>,
    called: &mut HashSet<SymbolId>,
) {
    use Expression as E;
    let mut cur = callee;
    let member = loop {
        match cur {
            E::StaticMemberExpression(member) => break member,
            E::ParenthesizedExpression(p) => cur = &p.expression,
            E::TSAsExpression(ts) => cur = &ts.expression,
            E::TSSatisfiesExpression(ts) => cur = &ts.expression,
            E::TSNonNullExpression(ts) => cur = &ts.expression,
            E::TSTypeAssertion(ts) => cur = &ts.expression,
            E::TSInstantiationExpression(ts) => cur = &ts.expression,
            _ => return,
        }
    };
    let prop = member.property.name.as_str();
    if prop != "call" && prop != "apply" {
        return;
    }
    let E::Identifier(id) = &member.object else { return };
    if let Some(symbol) = resolve_symbol(id, semantic) {
        called.insert(symbol);
    }
}

/// Handle the `fn.bind(...)()` shape. Called from the call site of the
/// *outer* `CallExpression` — its `callee` is the inner `fn.bind(...)`
/// `CallExpression`. If the inner call's callee is `Identifier.bind`
/// (a `StaticMemberExpression` whose `object` is a direct identifier and
/// `property` is `"bind"`), record the identifier's symbol in `called`.
/// Only one level of bind is covered; nested `fn.bind(a).bind(b)()` falls
/// through.
fn record_bind_callee<'a>(
    outer_callee: &Expression<'a>,
    semantic: &Semantic<'a>,
    called: &mut HashSet<SymbolId>,
) {
    use Expression as E;
    let mut cur = outer_callee;
    let inner_call = loop {
        match cur {
            E::CallExpression(call) => break call,
            E::ParenthesizedExpression(p) => cur = &p.expression,
            E::TSAsExpression(ts) => cur = &ts.expression,
            E::TSSatisfiesExpression(ts) => cur = &ts.expression,
            E::TSNonNullExpression(ts) => cur = &ts.expression,
            E::TSTypeAssertion(ts) => cur = &ts.expression,
            E::TSInstantiationExpression(ts) => cur = &ts.expression,
            _ => return,
        }
    };
    let mut bind_callee = &inner_call.callee;
    let member = loop {
        match bind_callee {
            E::StaticMemberExpression(member) => break member,
            E::ParenthesizedExpression(p) => bind_callee = &p.expression,
            E::TSAsExpression(ts) => bind_callee = &ts.expression,
            E::TSSatisfiesExpression(ts) => bind_callee = &ts.expression,
            E::TSNonNullExpression(ts) => bind_callee = &ts.expression,
            E::TSTypeAssertion(ts) => bind_callee = &ts.expression,
            E::TSInstantiationExpression(ts) => bind_callee = &ts.expression,
            _ => return,
        }
    };
    if member.property.name.as_str() != "bind" {
        return;
    }
    let E::Identifier(id) = &member.object else { return };
    if let Some(symbol) = resolve_symbol(id, semantic) {
        called.insert(symbol);
    }
}

/// If `init` is *directly* an `ArrowFunctionExpression` or
/// `FunctionExpression` (after peeling parens / TS wrappers), index the
/// binding `fn_symbol` as if it were a function declaration: record body
/// identifier refs into `fn_body_symbol_refs[fn_symbol]`, direct callees
/// into `fn_body_called_symbols[fn_symbol]`, and walk parameter defaults
/// into both. Returns `true` when indexing happened.
///
/// This makes `const make = () => DEP` visible to the BFS safe-skip guard
/// the same way `function make() { return DEP; }` is.
fn index_fn_valued_binding<'a>(
    init: &Expression<'a>,
    fn_symbol: SymbolId,
    semantic: &Semantic<'a>,
    fn_body_symbol_refs: &mut HashMap<SymbolId, HashSet<SymbolId>>,
    fn_body_called_symbols: &mut HashMap<SymbolId, HashSet<SymbolId>>,
) -> bool {
    use Expression as E;
    let mut cur = init;
    loop {
        match cur {
            E::ArrowFunctionExpression(arrow) => {
                let mut refs: HashSet<SymbolId> = HashSet::new();
                let mut called: HashSet<SymbolId> = HashSet::new();
                let mut visitor =
                    FunctionBodyIdentVisitor { semantic, out: &mut refs, called: &mut called };
                visitor.visit_function_body(&arrow.body);
                walk_param_defaults(&arrow.params, semantic, &mut refs, &mut called);
                fn_body_symbol_refs.insert(fn_symbol, refs);
                fn_body_called_symbols.insert(fn_symbol, called);
                return true;
            }
            E::FunctionExpression(func) => {
                let Some(body) = &func.body else { return false };
                let mut refs: HashSet<SymbolId> = HashSet::new();
                let mut called: HashSet<SymbolId> = HashSet::new();
                let mut visitor =
                    FunctionBodyIdentVisitor { semantic, out: &mut refs, called: &mut called };
                visitor.visit_function_body(body);
                walk_param_defaults(&func.params, semantic, &mut refs, &mut called);
                fn_body_symbol_refs.insert(fn_symbol, refs);
                fn_body_called_symbols.insert(fn_symbol, called);
                return true;
            }
            E::ParenthesizedExpression(p) => cur = &p.expression,
            E::TSAsExpression(ts) => cur = &ts.expression,
            E::TSSatisfiesExpression(ts) => cur = &ts.expression,
            E::TSNonNullExpression(ts) => cur = &ts.expression,
            E::TSTypeAssertion(ts) => cur = &ts.expression,
            E::TSInstantiationExpression(ts) => cur = &ts.expression,
            _ => return false,
        }
    }
}

/// If `callee` is the function expression of an IIFE
/// (`(() => …)()` or `(function() {…})()`, after peeling parens and TS
/// wrappers), walk its body eagerly via `FunctionBodyIdentVisitor` and
/// return `true`. The IIFE body runs at the call site, so its identifier
/// reads contribute to the eager-evaluation set — unlike a function stored
/// as a value, where the lazy-bodies rule in [`collect_expr_symbols`] is
/// correct.
///
/// Returns `false` when the callee is not a function/arrow expression; the
/// caller then falls through to the normal `collect_expr_symbols` descent
/// (which is a no-op for these node kinds anyway, but still correct).
fn walk_iife_callee_body<'a>(
    callee: &Expression<'a>,
    semantic: &Semantic<'a>,
    out: &mut HashSet<SymbolId>,
    called: &mut HashSet<SymbolId>,
) -> bool {
    use Expression as E;
    let mut cur = callee;
    loop {
        match cur {
            E::ArrowFunctionExpression(arrow) => {
                let mut visitor = FunctionBodyIdentVisitor { semantic, out, called };
                visitor.visit_function_body(&arrow.body);
                // Parameter defaults evaluate at IIFE invocation time, before
                // the body runs — symmetric with top-level function decls
                // in `collect_top_level_bindings`.
                walk_param_defaults(&arrow.params, semantic, out, called);
                return true;
            }
            E::FunctionExpression(func) => {
                if let Some(body) = &func.body {
                    let mut visitor = FunctionBodyIdentVisitor { semantic, out, called };
                    visitor.visit_function_body(body);
                }
                walk_param_defaults(&func.params, semantic, out, called);
                return true;
            }
            E::ParenthesizedExpression(p) => cur = &p.expression,
            E::TSAsExpression(ts) => cur = &ts.expression,
            E::TSSatisfiesExpression(ts) => cur = &ts.expression,
            E::TSNonNullExpression(ts) => cur = &ts.expression,
            E::TSTypeAssertion(ts) => cur = &ts.expression,
            E::TSInstantiationExpression(ts) => cur = &ts.expression,
            _ => return false,
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
            record_indirect_callee(&call.callee, semantic, called);
            record_bind_callee(&call.callee, semantic, called);
            if !walk_iife_callee_body(&call.callee, semantic, out, called) {
                collect_expr_symbols(&call.callee, semantic, out, called);
            }
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

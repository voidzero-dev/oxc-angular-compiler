//! Build the dependency-resolver function for `@defer` blocks.
//!
//! In Angular's local compilation mode the compiler emits a single
//! per-component deferrable-dependencies function — a no-arg arrow that
//! returns an array of dynamic `import()` calls resolving each lazy
//! dependency. The runtime invokes it lazily, the first time a defer block
//! triggers, so the deferred chunks can be code-split by the bundler.
//!
//! Example shape:
//!
//! ```js
//! () => [
//!   import('./lazy').then(m => m.LazyCmp),
//!   import('./widget').then(m => m.HeavyWidget),
//!   import('./default-export').then(m => m.default),
//! ]
//! ```
//!
//! This module produces that expression from the component's
//! `@Component.deferredImports` array (mirrored as
//! [`ComponentMetadata::deferred_imports`]) combined with the file-level
//! import map. It is consumed by the main compile path in
//! [`super::transform::compile_component_full`].
//!
//! ## `deferredImports` and implicitly deferred `imports`
//!
//! Angular's full compilation mode also derives a deferrable set from
//! `@Component.imports` by matching template selectors to imported
//! directives/pipes/components (see `resolveAllDeferredDependencies` in
//! `packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts`).
//! That requires cross-file metadata — selectors live on the imported class,
//! not on the import declaration. In local compilation Angular falls back to
//! the `deferredImports` field as the only source of deferrable symbols.
//! OXC does the same unless cross-file analysis can read the imported
//! class's selector: then [`implicitly_deferred_imports`] picks the
//! standalone components and directives the template matches only inside
//! `@defer` blocks, and they join `deferredImports` (#313). Each `@defer`
//! block then gets its own dependency function, as in ngc's full compilation
//! ([`defer_block_dependencies`], [`compile_defer_block_resolver`]).

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Class, Declaration, ExportDefaultDeclarationKind, Expression, ObjectPropertyKind, Program,
    Statement,
};

use crate::ast::expression::AngularExpression;
use crate::ast::expression::BindingType;
use crate::ast::r3::{
    R3BoundAttribute, R3BoundEvent, R3BoundText, R3Component, R3DeferredBlock, R3Directive,
    R3Element, R3ForLoopBlock, R3Icu, R3IcuPlaceholder, R3IfBlockBranch, R3LetDeclaration, R3Node,
    R3SwitchBlock, R3SwitchBlockCase, R3Template, R3TemplateAttr, R3TextAttribute, R3Visitor,
    visit_all,
};
use crate::class_metadata::{
    R3DeferPerComponentDependency, compile_component_metadata_async_resolver,
    compile_deferred_dependency_import,
};
use crate::directive::StringConsts;
use crate::i18n::serializer::is_i18n_attribute;
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, LiteralArrayExpr, OutputExpression, ReadPropExpr,
    ReadVarExpr, WrappedNodeExpr,
};
use crate::parser::html::split_ns_name;
use crate::pipeline::selector::CssSelector;
use oxc_str::Ident;

use super::metadata::{ComponentMetadata, ImportRef};
use super::transform::ImportMap;

/// Returns `true` if any `@defer` block exists anywhere in the template.
///
/// We need this so the compiler only generates a deferrable-dependencies
/// function when it would actually be wired into a `ɵɵdefer` call — otherwise
/// the resolver would be dead code.
pub fn template_has_defer_block<'a>(nodes: &[R3Node<'a>]) -> bool {
    struct DeferDetector {
        found: bool,
    }

    impl<'a> R3Visitor<'a> for DeferDetector {
        fn visit_deferred_block(&mut self, _block: &R3DeferredBlock<'a>) {
            // No need to recurse: a single hit is enough.
            self.found = true;
        }
    }

    let mut visitor = DeferDetector { found: false };
    visit_all(&mut visitor, nodes);
    visitor.found
}

/// Source spans of every `@defer` block, in the order Angular's binder lists
/// them (`getDeferBlocks()`): a block comes before the blocks nested in it, and
/// its main content before its `@placeholder`, `@loading` and `@error`.
pub fn defer_block_spans(nodes: &[R3Node<'_>]) -> std::vec::Vec<oxc_span::Span> {
    struct DeferCollector {
        spans: std::vec::Vec<oxc_span::Span>,
    }

    impl<'a> R3Visitor<'a> for DeferCollector {
        fn visit_deferred_block(&mut self, block: &R3DeferredBlock<'a>) {
            self.spans.push(block.source_span);
            visit_all(self, &block.children);
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
    }

    let mut visitor = DeferCollector { spans: std::vec::Vec::new() };
    visit_all(&mut visitor, nodes);
    visitor.spans
}

/// Build the list of `R3DeferPerComponentDependency` describing the
/// `@Component.deferredImports` entries, used to emit `setClassMetadataAsync`
/// and the per-component `ɵɵdefer` resolver.
///
/// Each entry carries two names that serve distinct roles in the emitted
/// output:
///
/// - `param_name` — the local binding from the source file (e.g. `Heavy` for
///   `import { HeavyWidget as Heavy }`, `LazyCmp` for
///   `import LazyCmp from './lazy'`). Used as the parameter name on the
///   `setClassMetadataAsync` callback so the wrapped decorator metadata
///   literal's identifier references shadow the outer static import,
///   allowing bundlers to drop the eager declaration.
/// - `export_name` — the name under which the symbol is exported from its
///   source module (e.g. `HeavyWidget` or `default`). Used as the property
///   read in the dynamic-import resolver chain (`m.<export_name>`). For
///   default imports the resolver substitutes `m.default` based on
///   `is_default_import`, so the value here is informational in that case.
///
/// Angular collapses both into a single `symbolName` field set to the
/// exported name; that leaves the static `import { Foo as Bar }` pinned for
/// aliased deferrable imports because the callback parameter (`Foo`) doesn't
/// match the metadata body's reference (`Bar`). Splitting the fields here is
/// a deliberate improvement on Angular's emission.
///
/// Entries for unresolved symbols, namespace imports, and type-only imports
/// are skipped — they have no concrete runtime value to lazy-load and the
/// resolver would silently misfire.
pub(super) fn build_defer_per_component_deps<'a>(
    allocator: &'a Allocator,
    deferred_imports: &[Ident<'a>],
    import_map: &ImportMap<'a>,
) -> oxc_allocator::Vec<'a, R3DeferPerComponentDependency<'a>> {
    let mut deps = oxc_allocator::Vec::new_in(&allocator);
    for local_name in deferred_imports {
        let Some(info) = import_map.get(local_name) else { continue };
        if !info.is_named_import || info.is_type_only {
            continue;
        }
        let is_default_import = info.imported_name.as_deref() == Some("default");
        // `export_name` is `info.imported_name` when present (aliased named
        // imports or default imports), otherwise the local name (which
        // equals the exported name for plain `import { Foo }`).
        let export_name = info.imported_name.clone().unwrap_or_else(|| local_name.clone());
        deps.push(R3DeferPerComponentDependency {
            param_name: local_name.clone(),
            export_name,
            import_path: info.source_module.clone(),
            is_default_import,
        });
    }
    deps
}

/// Build the deferrable-dependencies resolver expression from the component's
/// `@Component.deferredImports` array.
///
/// For each local identifier in `metadata.deferred_imports` that resolves to
/// an entry in `import_map`, emit one
/// `import(<path>).then(m => m.<exportedName>)` entry. Returns `None` when the
/// component has no `deferredImports` declared (no resolver to wire) — in
/// that case callers leave `ɵɵdefer`'s resolver argument as `null`, matching
/// Angular's behavior for a defer block with no explicit deferrable
/// dependencies.
///
/// A deferred entry is silently skipped (with no compiler error) when:
/// - the symbol isn't in the file's import map (it might be a local binding;
///   Angular would have flagged this earlier, but we treat it as a no-op),
/// - the import is a namespace import (`import * as ns`) — there is no
///   single `m.X` to point at,
/// - the import is type-only (erased at runtime).
///
/// Unlike eager `imports`, we do NOT filter by source module: Angular's
/// `compileDeferResolverFunction` emits a dynamic `import(<path>)` for every
/// entry the user lists, including bare specifiers like `@angular/material`.
/// Filtering would silently drop intentional opt-ins.
///
/// Shares the emission code path with `setClassMetadataAsync` via
/// [`compile_component_metadata_async_resolver`] so both call sites stay in
/// lockstep — the resolver wired into `ɵɵdefer(...)` is byte-equivalent to
/// the one wired into `ɵsetClassMetadataAsync(...)`.
pub fn build_defer_resolver_expression<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    import_map: &ImportMap<'a>,
) -> Option<OutputExpression<'a>> {
    if metadata.deferred_imports.is_empty() {
        return None;
    }

    let deps = build_defer_per_component_deps(allocator, &metadata.deferred_imports, import_map);
    if deps.is_empty() {
        return None;
    }

    Some(compile_component_metadata_async_resolver(allocator, &deps))
}

/// The [`ClassUse`] of the top-level class `local` declares in `program`,
/// exported or not, when it is a standalone `@Component`, `@Directive` or
/// `@Pipe` whose `selector` (or `name`) and `standalone` are literals: an
/// `imports` entry that names a class of the file being compiled. `consts`
/// are the program's.
pub(super) fn local_standalone_class_selector<'a>(
    program: &'a Program<'a>,
    consts: &StringConsts<'a>,
    local: &str,
    implicit_standalone: bool,
) -> Option<ClassUse> {
    let is_local = |class: &Class| class.id.as_ref().is_some_and(|id| id.name == local);
    let class = program.body.iter().find_map(|stmt| match stmt {
        Statement::ClassDeclaration(class) if is_local(class) => Some(&**class),
        Statement::ExportDeclaration(decl) => match &decl.declaration {
            Declaration::ClassDeclaration(class) if is_local(class) => Some(&**class),
            _ => None,
        },
        Statement::ExportDefaultDeclaration(decl) => match &decl.declaration {
            ExportDefaultDeclarationKind::ClassDeclaration(class) if is_local(class) => {
                Some(&**class)
            }
            _ => None,
        },
        _ => None,
    })?;
    decorated_class_selector(class, consts, implicit_standalone)
}

/// How a template uses a standalone class an `imports` entry names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassUse {
    /// A component or directive, matched by its selector.
    Selector(String),
    /// A pipe, by its name.
    Pipe(String),
}

/// The [`ClassUse`] of `class` when it is a standalone `@Component` or
/// `@Directive` whose `selector` and `standalone` are literals, or a
/// standalone `@Pipe` whose `name` and `standalone` are.
pub(super) fn decorated_class_selector<'a>(
    class: &Class<'a>,
    consts: &StringConsts<'a>,
    implicit_standalone: bool,
) -> Option<ClassUse> {
    let (decorator, name_key) =
        [("Component", "selector"), ("Directive", "selector"), ("Pipe", "name")].iter().find_map(
            |(name, key)| {
                crate::directive::find_angular_class_decorator(&class.decorators, name, consts)
                    .map(|decorator| (decorator, *key))
            },
        )?;
    let Expression::CallExpression(call) = &decorator.expression else { return None };
    let Some(Expression::ObjectExpression(object)) =
        call.arguments.first().and_then(|arg| arg.as_expression())
    else {
        return None;
    };

    let mut selector = None;
    let mut standalone = implicit_standalone;
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
        // A computed key can override `selector` or `standalone`.
        let key = property.key.static_name()?;
        if key == name_key {
            let Expression::StringLiteral(value) = &property.value else { return None };
            selector = Some(value.value.as_str()?.to_string());
        } else if key == "standalone" {
            let Expression::BooleanLiteral(value) = &property.value else { return None };
            standalone = value.value;
        }
    }
    let selector = selector.filter(|selector| standalone && !selector.trim().is_empty())?;
    Some(if name_key == "name" { ClassUse::Pipe(selector) } else { ClassUse::Selector(selector) })
}

/// What a template matches against an `imports` class to tell whether it
/// uses it.
pub(super) enum CandidateUse {
    /// The parsed selector of a component or directive.
    Selectors(std::vec::Vec<CssSelector>),
    /// The name of a pipe.
    Pipe(String),
}

impl CandidateUse {
    pub(super) fn of(class_use: &ClassUse) -> Self {
        match class_use {
            ClassUse::Selector(selector) => Self::Selectors(CssSelector::parse(selector)),
            ClassUse::Pipe(name) => Self::Pipe(name.clone()),
        }
    }
}

/// An `imports` class `@defer` may lazy-load, by the name it is known by.
pub(super) type Candidate<'a> = (Ident<'a>, CandidateUse);

/// The `candidates` matched only inside the main content of `@defer` blocks
/// (`resolveDeferBlocks` in ngc's component `handler.ts`). `@placeholder`,
/// `@loading` and `@error` count as eager. Selectorless templates return none.
pub(super) fn implicitly_deferred_imports<'a>(
    nodes: &[R3Node<'a>],
    candidates: &[Candidate<'a>],
) -> std::vec::Vec<Ident<'a>> {
    let mut matcher = CandidateMatcher::new(candidates);
    visit_all(&mut matcher, nodes);
    if matcher.selectorless {
        return std::vec::Vec::new();
    }
    candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| matcher.deferred[*index] && !matcher.eager[*index])
        .map(|(_, (name, _))| *name)
        .collect()
}

/// The `candidates` each `@defer` block uses, with the block's span, in
/// [`defer_block_spans`] order. ngc binds the main content of each block on
/// its own (`resolveDeferBlocks` in its component `handler.ts`) and keeps
/// what that content uses eagerly: a candidate matched only inside a nested
/// `@defer` belongs to the nested block, while the nested block's
/// `@placeholder`, `@loading` and `@error` belong to the outer one. Each list
/// keeps the order of `candidates`. `None` for a selectorless template.
pub(super) fn defer_block_dependencies<'a>(
    nodes: &[R3Node<'a>],
    candidates: &[Candidate<'a>],
) -> Option<std::vec::Vec<(oxc_span::Span, std::vec::Vec<Ident<'a>>)>> {
    struct BlockCollector<'c, 'a> {
        candidates: &'c [Candidate<'a>],
        blocks: std::vec::Vec<(oxc_span::Span, std::vec::Vec<Ident<'a>>)>,
    }

    impl<'a> R3Visitor<'a> for BlockCollector<'_, 'a> {
        fn visit_deferred_block(&mut self, block: &R3DeferredBlock<'a>) {
            let mut matcher = CandidateMatcher::new(self.candidates);
            visit_all(&mut matcher, &block.children);
            let mut dependencies = std::vec::Vec::new();
            for (index, (name, _)) in self.candidates.iter().enumerate() {
                if matcher.eager[index] && !dependencies.contains(name) {
                    dependencies.push(*name);
                }
            }
            self.blocks.push((block.source_span, dependencies));

            visit_all(self, &block.children);
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
    }

    let mut matcher = CandidateMatcher::new(candidates);
    visit_all(&mut matcher, nodes);
    if matcher.selectorless {
        return None;
    }
    let mut collector = BlockCollector { candidates, blocks: std::vec::Vec::new() };
    visit_all(&mut collector, nodes);
    Some(collector.blocks)
}

/// A `@defer` block's dependency function, for `DeferBlockDepsEmitMode`
/// `PerBlock` (`compileDeferResolverFunction` in Angular's
/// `render3/view/compiler.ts`): a dynamic `import()` for each of the
/// `dependencies` that `deferred` lists, and a plain reference to the others.
/// `None` when the block has no dependencies (`compileDeferBlocks` in ngc's
/// component `handler.ts`).
///
/// ngc's plain reference is the `WrappedNodeExpr` its `LocalIdentifierStrategy`
/// emits (`Reference.getIdentityIn`): the name of a class declared in the
/// file, which is the same node for every component and stays a plain read
/// here, or for an imported class the identifier it was read from, found in
/// `import_refs`. Wrapped nodes are equal only when they are the same node,
/// so the constant pool shares a function that reads an imported class
/// within this component only, unless the components name one `const` array.
pub(super) fn compile_defer_block_resolver<'a>(
    allocator: &'a Allocator,
    dependencies: &[Ident<'a>],
    deferred: &[Ident<'a>],
    import_refs: &[ImportRef<'a>],
    import_map: &ImportMap<'a>,
) -> Option<OutputExpression<'a>> {
    if dependencies.is_empty() {
        return None;
    }
    let mut entries = oxc_allocator::Vec::new_in(&allocator);
    for name in dependencies {
        let deferrable = if deferred.contains(name) {
            build_defer_per_component_deps(allocator, std::slice::from_ref(name), import_map)
                .first()
                .map(|dep| compile_deferred_dependency_import(allocator, dep))
        } else {
            None
        };
        let import = import_refs.iter().find(|import| import.key == *name);
        let read_var = |name: Ident<'a>| {
            OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                ReadVarExpr { name, source_span: None },
                &allocator,
            ))
        };
        entries.push(deferrable.unwrap_or_else(|| match import {
            // ngc emits `ns.Cmp` through an import of its own (`i1.Cmp`).
            Some(&ImportRef { namespace: Some(namespace), name, .. }) => {
                OutputExpression::ReadProp(oxc_allocator::Box::new_in(
                    ReadPropExpr {
                        receiver: oxc_allocator::Box::new_in(read_var(namespace), &allocator),
                        name,
                        optional: false,
                        source_span: None,
                    },
                    &allocator,
                ))
            }
            Some(import) if import_map.contains_key(name) => {
                OutputExpression::WrappedNode(oxc_allocator::Box::new_in(
                    WrappedNodeExpr { node_id: *name, source_span: Some(import.span) },
                    &allocator,
                ))
            }
            _ => read_var(*name),
        }));
    }
    let array = OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        &allocator,
    ));
    Some(OutputExpression::ArrowFunction(oxc_allocator::Box::new_in(
        ArrowFunctionExpr {
            params: oxc_allocator::Vec::new_in(&allocator),
            body: ArrowFunctionBody::Expression(oxc_allocator::Box::new_in(array, &allocator)),
            source_span: None,
        },
        &allocator,
    )))
}

/// Which `candidates` a template matches inside the main content of `@defer`
/// blocks and which it matches anywhere else, as t2_binder's
/// `DirectiveBinder` tells eagerly used directives apart.
struct CandidateMatcher<'c, 'a> {
    candidates: &'c [Candidate<'a>],
    in_defer: bool,
    deferred: std::vec::Vec<bool>,
    eager: std::vec::Vec<bool>,
    selectorless: bool,
}

impl<'c, 'a> CandidateMatcher<'c, 'a> {
    fn new(candidates: &'c [Candidate<'a>]) -> Self {
        Self {
            candidates,
            in_defer: false,
            deferred: vec![false; candidates.len()],
            eager: vec![false; candidates.len()],
            selectorless: false,
        }
    }

    fn record(&mut self, element: &CssSelector) {
        for (index, (_, class_use)) in self.candidates.iter().enumerate() {
            if let CandidateUse::Selectors(selectors) = class_use
                && selectors.iter().any(|selector| selector.matches(element))
            {
                self.mark(index);
            }
        }
    }

    /// Records the pipes `expr` uses (`visitPipe` in t2_binder's
    /// `TemplateBinder`).
    fn record_pipes(&mut self, expr: &AngularExpression<'_>) {
        visit_pipe_names(expr, &mut |name| {
            for (index, (_, class_use)) in self.candidates.iter().enumerate() {
                if matches!(class_use, CandidateUse::Pipe(pipe) if pipe == name) {
                    self.mark(index);
                }
            }
        });
    }

    fn mark(&mut self, index: usize) {
        if self.in_defer {
            self.deferred[index] = true;
        } else {
            self.eager[index] = true;
        }
    }
}

/// Calls `f` with the name of each pipe `expr` uses.
fn visit_pipe_names(expr: &AngularExpression<'_>, f: &mut dyn FnMut(&str)) {
    let all = |exprs: &[AngularExpression<'_>], f: &mut dyn FnMut(&str)| {
        for expr in exprs {
            visit_pipe_names(expr, f);
        }
    };
    match expr {
        AngularExpression::BindingPipe(pipe) => {
            f(&pipe.name);
            visit_pipe_names(&pipe.exp, f);
            all(&pipe.args, f);
        }
        AngularExpression::Binary(b) => {
            visit_pipe_names(&b.left, f);
            visit_pipe_names(&b.right, f);
        }
        AngularExpression::Conditional(c) => {
            visit_pipe_names(&c.condition, f);
            visit_pipe_names(&c.true_exp, f);
            visit_pipe_names(&c.false_exp, f);
        }
        AngularExpression::PropertyRead(p) => visit_pipe_names(&p.receiver, f),
        AngularExpression::SafePropertyRead(p) => visit_pipe_names(&p.receiver, f),
        AngularExpression::KeyedRead(k) => {
            visit_pipe_names(&k.receiver, f);
            visit_pipe_names(&k.key, f);
        }
        AngularExpression::SafeKeyedRead(k) => {
            visit_pipe_names(&k.receiver, f);
            visit_pipe_names(&k.key, f);
        }
        AngularExpression::Call(c) => {
            visit_pipe_names(&c.receiver, f);
            all(&c.args, f);
        }
        AngularExpression::SafeCall(c) => {
            visit_pipe_names(&c.receiver, f);
            all(&c.args, f);
        }
        AngularExpression::PrefixNot(p) => visit_pipe_names(&p.expression, f),
        AngularExpression::Unary(u) => visit_pipe_names(&u.expr, f),
        AngularExpression::TypeofExpression(t) => visit_pipe_names(&t.expression, f),
        AngularExpression::VoidExpression(v) => visit_pipe_names(&v.expression, f),
        AngularExpression::NonNullAssert(n) => visit_pipe_names(&n.expression, f),
        AngularExpression::ParenthesizedExpression(p) => visit_pipe_names(&p.expression, f),
        AngularExpression::SpreadElement(s) => visit_pipe_names(&s.expression, f),
        AngularExpression::ArrowFunction(a) => visit_pipe_names(&a.body, f),
        AngularExpression::LiteralArray(a) => all(&a.expressions, f),
        AngularExpression::LiteralMap(m) => all(&m.values, f),
        AngularExpression::Chain(c) => all(&c.expressions, f),
        AngularExpression::Interpolation(i) => all(&i.expressions, f),
        AngularExpression::TaggedTemplateLiteral(t) => {
            visit_pipe_names(&t.tag, f);
            all(&t.template.expressions, f);
        }
        AngularExpression::TemplateLiteral(t) => all(&t.expressions, f),
        AngularExpression::Empty(_)
        | AngularExpression::ImplicitReceiver(_)
        | AngularExpression::ThisReceiver(_)
        | AngularExpression::LiteralPrimitive(_)
        | AngularExpression::RegularExpressionLiteral(_) => {}
    }
}

impl<'a> R3Visitor<'a> for CandidateMatcher<'_, 'a> {
    fn visit_element(&mut self, element: &R3Element<'a>) {
        if !element.directives.is_empty() {
            self.selectorless = true;
        }
        self.record(&element_selector(
            &element.name,
            &element.attributes,
            &element.inputs,
            &element.outputs,
        ));
        self.visit_element_children(element);
    }

    fn visit_template(&mut self, template: &R3Template<'a>) {
        if !template.directives.is_empty() {
            self.selectorless = true;
        }
        self.record(&template_selector(template));
        for attr in &template.template_attrs {
            if let R3TemplateAttr::Bound(attr) = attr {
                self.record_pipes(&attr.value);
            }
        }
        self.visit_template_children(template);
    }

    fn visit_bound_text(&mut self, text: &R3BoundText<'a>) {
        self.record_pipes(&text.value);
    }

    fn visit_bound_attribute(&mut self, attr: &R3BoundAttribute<'a>) {
        self.record_pipes(&attr.value);
    }

    fn visit_bound_event(&mut self, event: &R3BoundEvent<'a>) {
        self.record_pipes(&event.handler);
    }

    fn visit_icu(&mut self, icu: &R3Icu<'a>) {
        for (_, var) in &icu.vars {
            self.record_pipes(&var.value);
        }
        for (_, placeholder) in &icu.placeholders {
            if let R3IcuPlaceholder::BoundText(text) = placeholder {
                self.record_pipes(&text.value);
            }
        }
    }

    fn visit_if_block_branch(&mut self, branch: &R3IfBlockBranch<'a>) {
        if let Some(expression) = &branch.expression {
            self.record_pipes(expression);
        }
        visit_all(self, &branch.children);
    }

    fn visit_for_loop_block(&mut self, block: &R3ForLoopBlock<'a>) {
        self.record_pipes(&block.expression.ast);
        self.record_pipes(&block.track_by.ast);
        visit_all(self, &block.children);
        if let Some(empty) = &block.empty {
            self.visit_for_loop_block_empty(empty);
        }
    }

    fn visit_switch_block(&mut self, block: &R3SwitchBlock<'a>) {
        self.record_pipes(&block.expression);
        for group in &block.groups {
            self.visit_switch_block_case_group(group);
        }
    }

    fn visit_switch_block_case(&mut self, case: &R3SwitchBlockCase<'a>) {
        if let Some(expression) = &case.expression {
            self.record_pipes(expression);
        }
    }

    fn visit_let_declaration(&mut self, decl: &R3LetDeclaration<'a>) {
        self.record_pipes(&decl.value);
    }

    fn visit_deferred_block(&mut self, block: &R3DeferredBlock<'a>) {
        // Its `when` conditions belong to the enclosing template.
        for triggers in [&block.triggers, &block.prefetch_triggers, &block.hydrate_triggers] {
            if let Some(when) = &triggers.when {
                self.record_pipes(&when.value);
            }
        }
        let outer = self.in_defer;
        self.in_defer = true;
        for child in &block.children {
            child.visit(self);
        }
        // `@placeholder`, `@loading` and `@error` render before the chunk
        // loads, so they are eager unless an outer `@defer` holds them
        // (`visitDeferredBlock` in t2_binder's `DirectiveBinder`).
        self.in_defer = outer;
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

    fn visit_component(&mut self, _component: &R3Component<'a>) {
        self.selectorless = true;
    }

    fn visit_directive(&mut self, _directive: &R3Directive<'a>) {
        self.selectorless = true;
    }
}

/// Whether any element or template anywhere in the template (`@defer` blocks
/// included) matches one of `selectors`, as ngc's `R3TargetBinder` matches a
/// scope's directives. A selectorless component or directive counts as a
/// match, since nothing here can tell what it resolves to.
pub(super) fn template_may_match_selectors(
    nodes: &[R3Node<'_>],
    selectors: &[CssSelector],
) -> bool {
    struct Matcher<'s> {
        selectors: &'s [CssSelector],
        matched: bool,
    }

    impl Matcher<'_> {
        fn record(&mut self, element: &CssSelector) {
            self.matched |= self.selectors.iter().any(|selector| selector.matches(element));
        }
    }

    impl<'a> R3Visitor<'a> for Matcher<'_> {
        fn visit_element(&mut self, element: &R3Element<'a>) {
            self.matched |= !element.directives.is_empty();
            self.record(&element_selector(
                &element.name,
                &element.attributes,
                &element.inputs,
                &element.outputs,
            ));
            self.visit_element_children(element);
        }

        fn visit_template(&mut self, template: &R3Template<'a>) {
            self.matched |= !template.directives.is_empty();
            self.record(&template_selector(template));
            self.visit_template_children(template);
        }

        fn visit_component(&mut self, _component: &R3Component<'a>) {
            self.matched = true;
        }

        fn visit_directive(&mut self, _directive: &R3Directive<'a>) {
            self.matched = true;
        }
    }

    let mut matcher = Matcher { selectors, matched: false };
    visit_all(&mut matcher, nodes);
    matcher.matched
}

/// An element as Angular matches directives against it
/// (`createCssSelectorFromNode` and `getAttrsForDirectiveMatching` in
/// `packages/compiler/src/render3/view/util.ts`): its static attributes with
/// their values, the names of its property and two-way bindings and of its
/// outputs, and its `class` attribute as classes. A binding replaces the value
/// of a static attribute with the same name, and namespace prefixes
/// (`:svg:`, `:xlink:`) are dropped.
fn element_selector(
    name: &str,
    attributes: &[R3TextAttribute<'_>],
    inputs: &[R3BoundAttribute<'_>],
    outputs: &[R3BoundEvent<'_>],
) -> CssSelector {
    let mut attrs = std::vec::Vec::new();
    for attribute in attributes {
        if !is_i18n_attribute(&attribute.name) {
            set_attribute(&mut attrs, &attribute.name, &attribute.value);
        }
    }
    for input in inputs {
        if matches!(input.binding_type, BindingType::Property | BindingType::TwoWay) {
            set_attribute(&mut attrs, &input.name, "");
        }
    }
    for output in outputs {
        set_attribute(&mut attrs, &output.name, "");
    }

    let mut selector = CssSelector::new();
    selector.set_element(split_ns_name(name).1);
    for (name, value) in attrs {
        selector.add_attribute(split_ns_name(name).1, Some(value));
        if name.eq_ignore_ascii_case("class") {
            for class in value.split_whitespace() {
                selector.add_class_name(class);
            }
        }
    }
    selector
}

/// Sets `name` to `value` in `attrs`, a name/value map kept in insertion order
/// like the object `getAttrsForDirectiveMatching` builds.
fn set_attribute<'s>(attrs: &mut std::vec::Vec<(&'s str, &'s str)>, name: &'s str, value: &'s str) {
    match attrs.iter_mut().find(|(existing, _)| *existing == name) {
        Some(entry) => entry.1 = value,
        None => attrs.push((name, value)),
    }
}

/// A template as Angular matches directives against it: an `<ng-template>`
/// like an element, a structural directive's template (`*ngIf`) by the names
/// of its template attributes.
fn template_selector(template: &R3Template<'_>) -> CssSelector {
    match &template.tag_name {
        Some(tag_name) if tag_name.as_str() == "ng-template" => element_selector(
            "ng-template",
            &template.attributes,
            &template.inputs,
            &template.outputs,
        ),
        _ => {
            let mut selector = CssSelector::new();
            selector.set_element("ng-template");
            for attribute in &template.template_attrs {
                let name = match attribute {
                    R3TemplateAttr::Bound(bound) => &bound.name,
                    R3TemplateAttr::Text(text) => &text.name,
                };
                selector.add_attribute(split_ns_name(name).1, None);
            }
            selector
        }
    }
}

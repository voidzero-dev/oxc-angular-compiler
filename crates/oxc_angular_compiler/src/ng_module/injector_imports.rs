//! Which `imports` and `exports` entries of an NgModule go into its `ɵinj`,
//! and how they are written.
//!
//! ngtsc's `NgModuleDecoratorHandler.resolve` (compiler-cli
//! `ngtsc/annotations/ng_module/src/handler.ts`) builds the injector's
//! `imports` in full and partial compilation from what each reference
//! resolves to. Only local compilation adds the lists as written, which is
//! what Oxc did. Each top-level `imports` entry (an array element, a spread
//! by its argument, or the whole value) is flattened into classes, `const`
//! lists and `forwardRef` read through:
//! - an entry holding a `ModuleWithProviders` is written as is;
//! - directives and pipes are left out, and so is a standalone component
//!   that brings in no providers (`ExportedProviderStatusResolver
//!   .mayExportProviders`, `ngtsc/metadata/src/providers.ts`);
//! - when every class stays, the entry is written as is (a spread by its
//!   argument: `...LIST` gives `LIST`), else each class that stays is
//!   written by itself (`i1.X`, or the name this file has for it).
//!
//! `exports` adds each class that is not a directive or pipe of the module's
//! scope, each by itself.
//!
//! Oxc does the same when it can read every class an entry names: from this
//! file alone ([`InjectorEntries::local_plans`]), or with cross-file analysis
//! (`CrossFileAnalyzer::injector_plans`). Anything unknown stays as written.
//! An entry also stays as written when a class it keeps is read through a
//! `forwardRef`: written by itself, a class declared later would be read
//! before it exists (ngtsc's output throws there).

use std::borrow::Cow;

use oxc_allocator::{Allocator, Box, Vec as OxcVec};
use oxc_ast::ast::{
    ArrayExpressionElement, Class, Declaration, Decorator, ExportDefaultDeclarationKind,
    Expression, ObjectPropertyKind, Program, PropertyKey, Statement, TSTypeName,
};
use oxc_str::Ident;
use rustc_hash::FxHashSet;

use super::decorator::{NgModuleMetadata, find_ng_module_decorator};
use super::module_with_providers::{MwpModule, static_method_ng_module, top_level_const_init};
use crate::component::ImportedName;
use crate::directive::{StringConsts, angular_core_identifier, find_angular_class_decorator};
use crate::output::ast::{LiteralArrayExpr, OutputExpression, ReadPropExpr, ReadVarExpr};
use crate::util::is_metadata_property;

/// How deep `const` lists and imports are followed in this file.
const MAX_DEPTH: u16 = 64;

/// A top-level entry of `imports`, `exports` or `declarations`: an array
/// element, the argument of a spread, or the whole value (`imports: LIST`).
#[derive(Debug, Clone, Copy)]
pub struct InjectorEntry<'s> {
    pub expr: &'s Expression<'s>,
    pub is_spread: bool,
}

/// What `ɵinj` does with a top-level entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryPlan {
    /// Keep it as written.
    Keep,
    /// Leave it out.
    Drop,
    /// Write a spread as its argument: `...LIST` gives `LIST`.
    Unwrap,
    /// Write the classes it keeps, each by itself.
    Replace(Vec<ImportedName>),
}

/// Per top-level entry of `imports` and of `exports`, what `ɵinj` does with
/// it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InjectorPlans {
    pub imports: Vec<EntryPlan>,
    pub exports: Vec<EntryPlan>,
}

impl InjectorPlans {
    /// Take what `other` knows: its plan wins, unless it keeps the entry as
    /// written or this one leaves the entry out.
    #[cfg_attr(not(feature = "cross_file_elision"), allow(dead_code))]
    pub(crate) fn merge(&mut self, other: &Self) {
        for (mine, theirs) in
            [(&mut self.imports, &other.imports), (&mut self.exports, &other.exports)]
        {
            for (plan, other_plan) in mine.iter_mut().zip(theirs) {
                if *plan != EntryPlan::Drop && *other_plan != EntryPlan::Keep {
                    *plan = other_plan.clone();
                }
            }
        }
    }
}

/// A class a top-level entry names, with lists flattened.
#[derive(Debug, Clone)]
pub struct ResolvedRef {
    /// Whether `ɵinj` keeps it; `None` when that isn't known.
    pub kept: Option<bool>,
    /// How the compiled file writes it, when it can.
    pub name: Option<ImportedName>,
    /// Reached through a `ModuleWithProviders`.
    pub is_mwp: bool,
    /// Reached through a `forwardRef`.
    pub forward: bool,
}

/// The plan of an `imports` entry naming `refs` (`None`: not known).
pub fn imports_plan(refs: Option<&[ResolvedRef]>, is_spread: bool) -> EntryPlan {
    let Some(refs) = refs else { return EntryPlan::Keep };
    if refs.iter().any(|r| r.kept.is_none()) {
        return EntryPlan::Keep;
    }
    // ngtsc writes the expression as is, without the spread.
    if refs.iter().any(|r| r.is_mwp) || refs.iter().all(|r| r.kept == Some(true)) {
        return if is_spread { EntryPlan::Unwrap } else { EntryPlan::Keep };
    }
    replace(refs)
}

/// The plan of an `exports` entry naming `refs` (`None`: not known). ngtsc
/// writes each class that stays by itself.
pub fn exports_plan(refs: Option<&[ResolvedRef]>) -> EntryPlan {
    let Some(refs) = refs else { return EntryPlan::Keep };
    // A `ModuleWithProviders` is a type error here (ngtsc gives its NgModule
    // when the list is cast): keep it, with its providers.
    if refs.iter().any(|r| r.kept.is_none() || r.is_mwp) {
        return EntryPlan::Keep;
    }
    replace(refs)
}

/// Each class of `refs` that stays, by itself.
fn replace(refs: &[ResolvedRef]) -> EntryPlan {
    let kept: Vec<&ResolvedRef> = refs.iter().filter(|r| r.kept == Some(true)).collect();
    if kept.is_empty() {
        return EntryPlan::Drop;
    }
    if kept.iter().any(|r| r.forward) {
        return EntryPlan::Keep;
    }
    kept.iter()
        .map(|r| r.name.clone())
        .collect::<Option<Vec<_>>>()
        .map_or(EntryPlan::Keep, EntryPlan::Replace)
}

/// One of the `imports` / `exports` / `declarations` lists, split the way
/// ngtsc's `analyze` splits `imports`: each array element (a spread by its
/// argument), or the whole value when it is not an array literal.
#[derive(Debug, Default)]
struct List<'s> {
    /// `None` for an array hole.
    entries: Vec<Option<InjectorEntry<'s>>>,
    is_array: bool,
}

/// The top-level entries of an `@NgModule`'s lists.
#[derive(Debug)]
pub struct InjectorEntries<'s> {
    imports: List<'s>,
    exports: List<'s>,
    declarations: List<'s>,
}

/// The entries of the Angular `@NgModule` decorator of `class`, or `None`
/// when it has none or its argument is not an object literal.
pub fn injector_entries<'s>(
    class: &'s Class<'s>,
    consts: &StringConsts<'_>,
) -> Option<InjectorEntries<'s>> {
    let decorator = find_ng_module_decorator(&class.decorators, Some(consts))?;
    let Expression::CallExpression(call) = &decorator.expression else { return None };
    let Expression::ObjectExpression(object) = call.arguments.first()?.as_expression()? else {
        return None;
    };
    let mut entries = InjectorEntries {
        imports: List::default(),
        exports: List::default(),
        declarations: List::default(),
    };
    // Like `extract_ng_module_metadata_in`, a later property wins.
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else { continue };
        if !is_metadata_property(property) {
            continue;
        }
        let key = match &property.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(lit) => lit.value.as_str().unwrap_or_default(),
            _ => continue,
        };
        let list = match key {
            "imports" => &mut entries.imports,
            "exports" => &mut entries.exports,
            "declarations" => &mut entries.declarations,
            _ => continue,
        };
        *list = read_list(&property.value);
    }
    Some(entries)
}

fn read_list<'s>(value: &'s Expression<'s>) -> List<'s> {
    let Expression::ArrayExpression(array) = value else {
        return List {
            entries: vec![Some(InjectorEntry { expr: value, is_spread: false })],
            is_array: false,
        };
    };
    let entries = array
        .elements
        .iter()
        .map(|element| match element {
            ArrayExpressionElement::SpreadElement(spread) => {
                Some(InjectorEntry { expr: &spread.argument, is_spread: true })
            }
            ArrayExpressionElement::Elision(_) => None,
            _ => Some(InjectorEntry { expr: element.to_expression(), is_spread: false }),
        })
        .collect();
    List { entries, is_array: true }
}

/// The class or list an entry names as written: `A`, `ns.A`, or either one
/// in a `forwardRef` arrow imported from `@angular/core`.
fn entry_name<'s>(
    expr: &'s Expression<'s>,
    consts: &StringConsts<'_>,
) -> Option<(Option<&'s str>, &'s str)> {
    match expr {
        Expression::Identifier(id) => Some((None, id.name.as_str())),
        Expression::StaticMemberExpression(member) => match &member.object {
            Expression::Identifier(ns) => {
                Some((Some(ns.name.as_str()), member.property.name.as_str()))
            }
            _ => None,
        },
        _ => entry_name(forward_ref_target(expr, consts)?, consts),
    }
}

/// What the `forwardRef(() => x)` call `expr` returns, with `forwardRef`
/// imported from `@angular/core`: another function may return anything.
fn forward_ref_target<'s>(
    expr: &'s Expression<'s>,
    consts: &StringConsts<'_>,
) -> Option<&'s Expression<'s>> {
    let Expression::CallExpression(call) = expr else { return None };
    let Expression::Identifier(callee) = call.callee.get_inner_expression() else { return None };
    angular_core_identifier(&callee.name, Some(consts), &["forwardRef"])?;
    match call.arguments.first()?.as_expression()?.get_inner_expression() {
        Expression::ArrowFunctionExpression(arrow) => arrow.get_expression(),
        Expression::FunctionExpression(function) => {
            match function.body.as_ref()?.statements.as_slice() {
                [Statement::ReturnStatement(ret)] => ret.argument.as_ref(),
                _ => None,
            }
        }
        _ => None,
    }
}

/// What a class of this file is to an NgModule's lists, by its Angular
/// decorator.
#[derive(Clone, Copy)]
enum LocalKind<'s> {
    DirectiveOrPipe,
    Component(&'s Decorator<'s>),
    NgModule(&'s Decorator<'s>),
}

/// The kind of `class`, when it has exactly one Angular `@Directive`,
/// `@Pipe`, `@Component` or `@NgModule` decorator.
fn local_kind<'s>(class: &'s Class<'s>, consts: &StringConsts<'_>) -> Option<LocalKind<'s>> {
    let find = |kind: &str| find_angular_class_decorator(&class.decorators, kind, consts);
    match (find("Directive"), find("Pipe"), find("Component"), find("NgModule")) {
        (Some(_), None, None, None) | (None, Some(_), None, None) => {
            Some(LocalKind::DirectiveOrPipe)
        }
        (None, None, Some(decorator), None) => Some(LocalKind::Component(decorator)),
        (None, None, None, Some(decorator)) => Some(LocalKind::NgModule(decorator)),
        _ => None,
    }
}

/// The fields of the object literal `decorator` is called with (none for
/// no argument), or `None` when a spread or a computed key could set any.
fn decorator_fields<'s>(
    decorator: &'s Decorator<'s>,
) -> Option<Vec<(Cow<'s, str>, &'s Expression<'s>)>> {
    let Expression::CallExpression(call) = &decorator.expression else { return None };
    let mut fields = Vec::new();
    let Some(argument) = call.arguments.first() else { return Some(fields) };
    let Expression::ObjectExpression(object) = argument.as_expression()?.get_inner_expression()
    else {
        return None;
    };
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
        fields.push((property.key.static_name()?.into_cow_str()?, &property.value));
    }
    Some(fields)
}

/// The value of the field `key` (the last one wins).
fn field<'s>(
    fields: &[(Cow<'s, str>, &'s Expression<'s>)],
    key: &str,
) -> Option<&'s Expression<'s>> {
    fields.iter().rev().find(|(name, _)| name == key).map(|(_, value)| *value)
}

/// The top-level class `program` declares as `name`. A name the file
/// imports is never a class it declares.
fn top_level_class<'s>(program: &'s Program<'s>, name: &str) -> Option<&'s Class<'s>> {
    program.body.iter().find_map(|stmt| {
        let class = match stmt {
            Statement::ClassDeclaration(class) => class,
            Statement::ExportDeclaration(decl) => match &decl.declaration {
                Declaration::ClassDeclaration(class) => class,
                _ => return None,
            },
            Statement::ExportDefaultDeclaration(decl) => match &decl.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => class,
                _ => return None,
            },
            _ => return None,
        };
        class.id.as_ref().is_some_and(|id| id.name == name).then_some(&**class)
    })
}

/// A class of this file an entry reaches.
#[derive(Clone, Copy)]
struct LocalRef<'s> {
    class: &'s Class<'s>,
    is_mwp: bool,
    forward: bool,
}

/// The classes and `const` lists of the compiled file. Anything that leaves
/// the file is not known.
struct LocalGraph<'s, 'c, 'k> {
    program: &'s Program<'s>,
    consts: &'c StringConsts<'k>,
}

impl<'s> LocalGraph<'s, '_, '_> {
    /// The classes of this file `expr` names, with `const` lists, a
    /// `forwardRef` and a `ModuleWithProviders` read through. `None` when
    /// any part of it is not a class of this file.
    fn refs(&self, expr: &'s Expression<'s>) -> Option<Vec<LocalRef<'s>>> {
        let mut out = Vec::new();
        self.collect(expr, false, false, &mut Vec::new(), &mut out, 0)?;
        Some(out)
    }

    fn collect(
        &self,
        expr: &'s Expression<'s>,
        is_mwp: bool,
        forward: bool,
        lists: &mut Vec<&'s str>,
        out: &mut Vec<LocalRef<'s>>,
        depth: u16,
    ) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        let depth = depth + 1;
        match expr.get_inner_expression() {
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    let element = match element {
                        ArrayExpressionElement::SpreadElement(spread) => &spread.argument,
                        ArrayExpressionElement::Elision(_) => return None,
                        _ => element.to_expression(),
                    };
                    self.collect(element, is_mwp, forward, lists, out, depth)?;
                }
            }
            Expression::Identifier(id) => {
                let name = id.name.as_str();
                if let Some(class) = top_level_class(self.program, name) {
                    out.push(LocalRef { class, is_mwp, forward });
                } else {
                    // A list that holds itself has no value.
                    if lists.contains(&name) {
                        return None;
                    }
                    let init = top_level_const_init(self.program, name)?;
                    lists.push(name);
                    let found = self.collect(init, is_mwp, forward, lists, out, depth);
                    lists.pop();
                    found?;
                }
            }
            // `{ ngModule: X, providers }`
            Expression::ObjectExpression(object) => {
                let module = object.properties.iter().find_map(|property| match property {
                    ObjectPropertyKind::ObjectProperty(p)
                        if p.key.static_name().is_some_and(|key| key == "ngModule") =>
                    {
                        Some(&p.value)
                    }
                    _ => None,
                })?;
                self.collect(module, true, forward, lists, out, depth)?;
            }
            Expression::CallExpression(call) => {
                if let Some(target) = forward_ref_target(expr.get_inner_expression(), self.consts) {
                    return self.collect(target, is_mwp, true, lists, out, depth);
                }
                // `X.forRoot()`, with `X` a class of this file.
                let Expression::StaticMemberExpression(member) = call.callee.get_inner_expression()
                else {
                    return None;
                };
                let Expression::Identifier(holder) = member.object.get_inner_expression() else {
                    return None;
                };
                let holder = top_level_class(self.program, &holder.name)?;
                match static_method_ng_module(holder, &member.property.name)? {
                    MwpModule::Value(value) => {
                        self.collect(value, true, forward, lists, out, depth)?;
                    }
                    MwpModule::Type(TSTypeName::IdentifierReference(id)) => {
                        let class = top_level_class(self.program, &id.name)?;
                        out.push(LocalRef { class, is_mwp: true, forward });
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
        Some(())
    }

    /// Whether the class may bring in providers, like ngtsc's
    /// `mayExportProviders`: a standalone component or an NgModule that
    /// reaches an NgModule with a `providers` key (any value). A component's
    /// own providers don't count. `None` when a class it reaches can't be
    /// read and none is known to bring in providers.
    fn may_export_providers(&self, class: &'s Class<'s>) -> Option<bool> {
        // ngtsc stops at a class it is already looking at, so from the top
        // this is whether such a module can be reached.
        let mut seen = FxHashSet::default();
        let mut pending = vec![class];
        let mut unknown = false;
        while let Some(class) = pending.pop() {
            if !seen.insert(class.span.start) {
                continue;
            }
            let (decorator, is_component) = match local_kind(class, self.consts) {
                Some(LocalKind::DirectiveOrPipe) => continue,
                Some(LocalKind::Component(decorator)) => (decorator, true),
                Some(LocalKind::NgModule(decorator)) => (decorator, false),
                None => {
                    unknown = true;
                    continue;
                }
            };
            let Some(fields) = decorator_fields(decorator) else {
                unknown = true;
                continue;
            };
            if is_component {
                // A component that is not standalone is an error in
                // `imports`: keep it.
                match field(&fields, "standalone").map(Expression::get_inner_expression) {
                    None => {}
                    Some(Expression::BooleanLiteral(b)) if b.value => {}
                    Some(Expression::BooleanLiteral(_)) => return Some(true),
                    Some(_) => {
                        unknown = true;
                        continue;
                    }
                }
            } else if field(&fields, "providers").is_some() {
                return Some(true);
            }
            let Some(imports) = field(&fields, "imports") else { continue };
            let Some(refs) = self.refs(imports) else {
                unknown = true;
                continue;
            };
            pending.extend(refs.iter().map(|r| r.class));
        }
        (!unknown).then_some(false)
    }

    /// `refs` as `ɵinj` sees them, `kept` telling what stays of a class
    /// whose kind is known.
    fn resolved(
        &self,
        refs: &[LocalRef<'s>],
        kept: impl Fn(&LocalRef<'s>, LocalKind<'s>) -> Option<bool>,
    ) -> Vec<ResolvedRef> {
        refs.iter()
            .map(|r| ResolvedRef {
                kept: local_kind(r.class, self.consts).and_then(|kind| kept(r, kind)),
                name: r.class.id.as_ref().map(|id| ImportedName::Local(id.name.to_string())),
                is_mwp: r.is_mwp,
                forward: r.forward,
            })
            .collect()
    }
}

impl<'s> InjectorEntries<'s> {
    /// The top-level `imports` entries, in order.
    #[cfg_attr(not(feature = "cross_file_elision"), allow(dead_code))]
    pub(crate) fn imports(&self) -> &[Option<InjectorEntry<'s>>] {
        &self.imports.entries
    }

    /// The top-level `exports` entries, in order.
    #[cfg_attr(not(feature = "cross_file_elision"), allow(dead_code))]
    pub(crate) fn exports(&self) -> &[Option<InjectorEntry<'s>>] {
        &self.exports.entries
    }

    /// What this file alone shows `ɵinj` does with each entry (see the
    /// module doc), reading the classes and `const` lists `program`
    /// declares. In `exports`, an entry `declarations` also lists is left
    /// out (only directives, components and pipes can be declared), and so
    /// is a component that `imports` or `declarations` also name, as ngtsc
    /// requires of an exported class. Another component stays, as its
    /// providers may be needed.
    pub(crate) fn local_plans(
        &self,
        program: &'s Program<'s>,
        consts: &StringConsts<'_>,
    ) -> InjectorPlans {
        let graph = LocalGraph { program, consts };
        let refs = |entry: &Option<InjectorEntry<'s>>| graph.refs(entry.as_ref()?.expr);
        let imports = self
            .imports
            .entries
            .iter()
            .map(|entry| {
                let resolved = refs(entry).map(|refs| {
                    graph.resolved(&refs, |r, kind| match kind {
                        LocalKind::DirectiveOrPipe => Some(false),
                        LocalKind::Component(_) => graph.may_export_providers(r.class),
                        LocalKind::NgModule(_) => Some(true),
                    })
                });
                imports_plan(resolved.as_deref(), entry.is_some_and(|entry| entry.is_spread))
            })
            .collect();
        // The classes of the module's compilation scope this file shows.
        let in_scope: FxHashSet<u32> = if self.exports.entries.is_empty() {
            FxHashSet::default()
        } else {
            let lists = self.declarations.entries.iter().chain(&self.imports.entries);
            lists.filter_map(refs).flatten().map(|r| r.class.span.start).collect()
        };
        let name = |entry: &Option<InjectorEntry<'s>>| entry_name(entry.as_ref()?.expr, consts);
        let exports = self
            .exports
            .entries
            .iter()
            .map(|entry| {
                if name(entry).is_some_and(|written| {
                    self.declarations.entries.iter().any(|other| name(other) == Some(written))
                }) {
                    return EntryPlan::Drop;
                }
                let resolved = refs(entry).map(|refs| {
                    graph.resolved(&refs, |r, kind| match kind {
                        LocalKind::DirectiveOrPipe => Some(false),
                        LocalKind::Component(_) => Some(!in_scope.contains(&r.class.span.start)),
                        LocalKind::NgModule(_) => Some(true),
                    })
                });
                exports_plan(resolved.as_deref())
            })
            .collect();
        InjectorPlans { imports, exports }
    }

    /// Apply `plans` to the `ɵinj` imports `metadata` builds. `namespace`
    /// gives the namespace import of a module specifier. A list whose
    /// converted expression doesn't line up with the entries read here is
    /// kept as is.
    pub(crate) fn apply<'a>(
        &self,
        allocator: &'a Allocator,
        metadata: &mut NgModuleMetadata<'a>,
        plans: &InjectorPlans,
        namespace: &mut dyn FnMut(&str) -> Ident<'a>,
    ) {
        apply_list(
            allocator,
            &mut metadata.raw_imports_expr,
            &self.imports,
            &plans.imports,
            namespace,
        );
        apply_list(
            allocator,
            &mut metadata.raw_exports_expr,
            &self.exports,
            &plans.exports,
            namespace,
        );
    }
}

fn apply_list<'a>(
    allocator: &'a Allocator,
    raw: &mut Option<OutputExpression<'a>>,
    list: &List<'_>,
    plans: &[EntryPlan],
    namespace: &mut dyn FnMut(&str) -> Ident<'a>,
) {
    if plans.len() != list.entries.len() || plans.iter().all(|plan| *plan == EntryPlan::Keep) {
        return;
    }
    let mut write = |names: &[ImportedName], out: &mut OxcVec<'a, OutputExpression<'a>>| {
        for name in names {
            out.push(name_expr(allocator, name, namespace));
        }
    };
    match raw {
        Some(OutputExpression::LiteralArray(array))
            if list.is_array && array.entries.len() == plans.len() =>
        {
            let entries = std::mem::replace(&mut array.entries, OxcVec::new_in(&allocator));
            for (entry, plan) in entries.into_iter().zip(plans) {
                match plan {
                    EntryPlan::Keep => array.entries.push(entry),
                    EntryPlan::Drop => {}
                    EntryPlan::Unwrap => match entry {
                        OutputExpression::SpreadElement(spread) => {
                            array.entries.push(spread.unbox().expr.unbox());
                        }
                        entry => array.entries.push(entry),
                    },
                    EntryPlan::Replace(names) => write(names, &mut array.entries),
                }
            }
        }
        // `imports: LIST`, written as its classes.
        Some(expr) if !list.is_array && !matches!(expr, OutputExpression::LiteralArray(_)) => {
            let mut entries = OxcVec::new_in(&allocator);
            match &plans[0] {
                EntryPlan::Drop => {}
                EntryPlan::Replace(names) => write(names, &mut entries),
                EntryPlan::Keep | EntryPlan::Unwrap => return,
            }
            *expr = OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                &allocator,
            ));
        }
        _ => {}
    }
}

/// `Name`, or `iN.Name` for a class of another module.
fn name_expr<'a>(
    allocator: &'a Allocator,
    name: &ImportedName,
    namespace: &mut dyn FnMut(&str) -> Ident<'a>,
) -> OutputExpression<'a> {
    let var = |name: Ident<'a>| {
        OutputExpression::ReadVar(Box::new_in(ReadVarExpr { name, source_span: None }, &allocator))
    };
    match name {
        ImportedName::Local(name) => var(Ident::from(allocator.alloc_str(name))),
        ImportedName::Module { specifier, name } => OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(var(namespace(specifier)), &allocator),
                name: Ident::from(allocator.alloc_str(name)),
                optional: false,
                source_span: None,
            },
            &allocator,
        )),
    }
}

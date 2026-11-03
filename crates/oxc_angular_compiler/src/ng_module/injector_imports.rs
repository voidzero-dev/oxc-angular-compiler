//! Which `imports` and `exports` entries of an NgModule go into its `ɵinj`.
//!
//! ngtsc's `NgModuleDecoratorHandler.resolve` (compiler-cli
//! `ngtsc/annotations/ng_module/src/handler.ts`) builds the injector's
//! `imports` in full and partial compilation from what each reference
//! resolves to: it leaves out every directive and pipe of `imports`, and
//! every directive, component and pipe of `exports` (`isNgModule`). Only
//! local compilation adds the lists as written, which is what Oxc did.
//!
//! The runtime's `walkProviderTree` skips anything that is neither an
//! NgModule nor a standalone component, so leaving those entries out never
//! changes which providers are registered. Here an entry is left out only
//! when it is known to be one of them; anything unknown stays.
//!
//! Two sources say what an entry is:
//! - this file alone ([`InjectorEntries::local_drops`]): an `exports` entry
//!   that `declarations` also lists, and a class of this file with an
//!   Angular `@Directive`, `@Pipe` or `@Component` decorator;
//! - cross-file analysis, when it is on, which reads imported classes and
//!   `const` lists too (`CrossFileAnalyzer::injector_drops`).
//!
//! Left out on purpose: ngtsc also drops a standalone component from
//! `imports` when it brings in no providers (`mayExportProviders`), and
//! rewrites a list that is only partly dropped into the references it keeps.
//! Oxc keeps such entries whole.

use oxc_allocator::{Allocator, Box, Vec as OxcVec};
use oxc_ast::ast::{
    ArrayExpressionElement, Class, Declaration, ExportDefaultDeclarationKind, Expression,
    ObjectPropertyKind, Program, PropertyKey, Statement,
};

use super::decorator::{NgModuleMetadata, find_ng_module_decorator};
use crate::directive::{StringConsts, find_angular_class_decorator};
use crate::output::ast::{LiteralArrayExpr, OutputExpression};
use crate::util::is_metadata_property;

/// A top-level entry of `imports`, `exports` or `declarations` that names a
/// class or a `const` list: `A`, `ns.A`, `forwardRef(() => A)`, `...LIST`,
/// or the whole value (`imports: LIST`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectorEntry<'s> {
    pub namespace: Option<&'s str>,
    pub name: &'s str,
}

/// Per top-level entry of `imports` and of `exports`, whether `ɵinj` leaves
/// it out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InjectorDrops {
    pub imports: Vec<bool>,
    pub exports: Vec<bool>,
}

impl InjectorDrops {
    /// Leave out what `other` leaves out too.
    #[cfg_attr(not(feature = "cross_file_elision"), allow(dead_code))]
    pub(crate) fn merge(&mut self, other: &Self) {
        for (mine, theirs) in
            [(&mut self.imports, &other.imports), (&mut self.exports, &other.exports)]
        {
            for (drop, other_drop) in mine.iter_mut().zip(theirs) {
                *drop |= *other_drop;
            }
        }
    }
}

/// One of the `imports` / `exports` / `declarations` lists, split the way
/// ngtsc's `analyze` splits `imports`: each array element (a spread by its
/// argument), or the whole value when it is not an array literal.
#[derive(Debug, Default)]
struct List<'s> {
    /// `None` for an entry that names no class or list (a call, a literal...).
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
    class: &'s Class<'_>,
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
        *list = read_list(&property.value, consts);
    }
    Some(entries)
}

fn read_list<'s>(value: &'s Expression<'_>, consts: &StringConsts<'_>) -> List<'s> {
    let Expression::ArrayExpression(array) = value else {
        return List { entries: vec![entry_ref(value, consts)], is_array: false };
    };
    let entries = array
        .elements
        .iter()
        .map(|element| match element {
            ArrayExpressionElement::SpreadElement(spread) => entry_ref(&spread.argument, consts),
            ArrayExpressionElement::Elision(_) => None,
            _ => entry_ref(element.to_expression(), consts),
        })
        .collect();
    List { entries, is_array: true }
}

/// The class or list `expr` names: `A`, `ns.A`, or either one in a
/// `forwardRef` arrow imported from `@angular/core` (the evaluator resolves
/// it to the class, as ngtsc's `forwardRefResolver` does).
fn entry_ref<'s>(expr: &'s Expression<'_>, consts: &StringConsts<'_>) -> Option<InjectorEntry<'s>> {
    match expr {
        Expression::Identifier(id) => {
            Some(InjectorEntry { namespace: None, name: id.name.as_str() })
        }
        Expression::StaticMemberExpression(member) => match &member.object {
            Expression::Identifier(ns) => Some(InjectorEntry {
                namespace: Some(ns.name.as_str()),
                name: member.property.name.as_str(),
            }),
            _ => None,
        },
        Expression::CallExpression(call) => {
            let Expression::Identifier(callee) = &call.callee else { return None };
            crate::directive::angular_core_identifier(&callee.name, Some(consts), &["forwardRef"])?;
            let Some(Expression::ArrowFunctionExpression(arrow)) =
                call.arguments.first().and_then(|arg| arg.as_expression())
            else {
                return None;
            };
            match arrow.get_expression()? {
                inner @ (Expression::Identifier(_) | Expression::StaticMemberExpression(_)) => {
                    entry_ref(inner, consts)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// What a class of this file is to an NgModule's lists, by its Angular
/// decorator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalKind {
    DirectiveOrPipe,
    Component,
}

/// The kind of the top-level class `program` declares as `name`, when it has
/// exactly one Angular `@Directive`, `@Pipe` or `@Component` decorator and
/// no `@NgModule` one. A name the file imports is never a class it declares.
fn local_kind(program: &Program<'_>, consts: &StringConsts<'_>, name: &str) -> Option<LocalKind> {
    let class = program.body.iter().find_map(|stmt| {
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
        class.id.as_ref().is_some_and(|id| id.name == name).then_some(class)
    })?;
    let has = |kind: &str| find_angular_class_decorator(&class.decorators, kind, consts).is_some();
    if has("NgModule") {
        return None;
    }
    match (has("Directive"), has("Pipe"), has("Component")) {
        (true, false, false) | (false, true, false) => Some(LocalKind::DirectiveOrPipe),
        (false, false, true) => Some(LocalKind::Component),
        _ => None,
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

    /// What this file alone shows `ɵinj` leaves out:
    /// - an `exports` entry `declarations` also lists: only directives,
    ///   components and pipes can be declared;
    /// - an entry naming a class of this file with a `@Directive` or `@Pipe`
    ///   decorator;
    /// - an `exports` entry naming a class of this file with a `@Component`
    ///   decorator that `imports` (where it stays) or `declarations` also
    ///   lists, as ngtsc requires of an exported class.
    pub(crate) fn local_drops(
        &self,
        program: &Program<'_>,
        consts: &StringConsts<'_>,
    ) -> InjectorDrops {
        let listed_in = |list: &List<'s>, entry: &InjectorEntry<'s>| {
            list.entries.iter().flatten().any(|other| other == entry)
        };
        let kind = |entry: &InjectorEntry<'s>| match entry.namespace {
            None => local_kind(program, consts, entry.name),
            Some(_) => None,
        };
        let imports = self
            .imports
            .entries
            .iter()
            .map(|entry| {
                entry.is_some_and(|entry| kind(&entry) == Some(LocalKind::DirectiveOrPipe))
            })
            .collect();
        let exports = self
            .exports
            .entries
            .iter()
            .map(|entry| {
                entry.is_some_and(|entry| {
                    listed_in(&self.declarations, &entry)
                        || match kind(&entry) {
                            Some(LocalKind::DirectiveOrPipe) => true,
                            Some(LocalKind::Component) => listed_in(&self.imports, &entry),
                            None => false,
                        }
                })
            })
            .collect();
        InjectorDrops { imports, exports }
    }

    /// Leave the entries `drops` marks out of the `ɵinj` imports `metadata`
    /// builds. A list whose converted expression doesn't line up with the
    /// entries read here is kept as is.
    pub(crate) fn apply<'a>(
        &self,
        allocator: &'a Allocator,
        metadata: &mut NgModuleMetadata<'a>,
        drops: &InjectorDrops,
    ) {
        apply_list(allocator, &mut metadata.raw_imports_expr, &self.imports, &drops.imports);
        apply_list(allocator, &mut metadata.raw_exports_expr, &self.exports, &drops.exports);
    }
}

fn apply_list<'a>(
    allocator: &'a Allocator,
    raw: &mut Option<OutputExpression<'a>>,
    list: &List<'_>,
    drops: &[bool],
) {
    if drops.len() != list.entries.len() || !drops.contains(&true) {
        return;
    }
    match raw {
        Some(OutputExpression::LiteralArray(array))
            if list.is_array && array.entries.len() == drops.len() =>
        {
            let mut drop = drops.iter();
            array.entries.retain(|_| drop.next() == Some(&false));
        }
        // `imports: LIST`, all of whose classes are left out.
        Some(expr) if !list.is_array && !matches!(expr, OutputExpression::LiteralArray(_)) => {
            *expr = OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: OxcVec::new_in(&allocator), source_span: None },
                &allocator,
            ));
        }
        _ => {}
    }
}

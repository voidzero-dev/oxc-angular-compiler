//! A single-file model of ngtsc's partial evaluator.
//!
//! Decorator metadata such as `inputs:`, `outputs:` and `queries:` is read by
//! ngtsc through its partial evaluator
//! (packages/compiler-cli/src/ngtsc/partial_evaluator), and its diagnostics
//! describe values the way that evaluator resolves them. This module reproduces
//! the parts of it that can be answered from the current file, so the compiled
//! metadata and the diagnostics match ngtsc word for word. Imported bindings
//! can't be seen into and are treated as opaque references; anything computed
//! from one stays a reference to that import (see [`Value::is_import`]).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionBody, ArrowFunctionExpression,
    BinaryExpression, BindingPattern, BlockStatement, CallExpression, CatchClause, ChainElement,
    Class, ClassElement, ComputedMemberExpression, ConditionalExpression, Declaration,
    ExportDefaultDeclarationKind, Expression, ForInStatement, ForOfStatement, ForStatement,
    ForStatementInit, ForStatementLeft, FormalParameters, Function, FunctionBody,
    IdentifierReference, ImportDeclarationSpecifier, LogicalExpression, MethodDefinition,
    MethodDefinitionKind, ModuleExportName, ObjectExpression, ObjectPropertyKind, Program,
    PropertyKey, PropertyKind, Statement, StaticBlock, StaticMemberExpression, Super,
    SwitchStatement, TSCallSignatureDeclaration, TSConditionalType,
    TSConstructSignatureDeclaration, TSConstructorType, TSEnumDeclaration, TSEnumMemberName,
    TSFunctionType, TSInferType, TSInterfaceDeclaration, TSLiteral, TSMappedType,
    TSMethodSignature, TSModuleReference, TSNamespaceDeclaration, TSNamespaceDeclarationBody,
    TSQualifiedName, TSTupleElement, TSType, TSTypeAliasDeclaration, TSTypeName,
    TSTypeOperatorOperator, TSTypeParameterDeclaration, TSTypeParameterInstantiation,
    TSTypeQueryExprName, TSTypeReference, TemplateLiteral, ThisExpression, UnaryExpression,
    VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::{GetSpan, Span};
use oxc_syntax::operator::{BinaryOperator, LogicalOperator, UnaryOperator};
use oxc_syntax::scope::ScopeFlags;

use crate::output::emitter::format_number_like_js;

/// Everything declared at the top level of a file that the evaluator can resolve.
#[derive(Default)]
pub(crate) struct FileScope<'a> {
    /// `const`/`let`/`var` bindings. The first declaration of a name wins, as
    /// it's TypeScript's value declaration (`var X = ['a']; var X = ['b'];`).
    variables: HashMap<&'a str, Variable<'a>>,
    /// Function declarations: the implementation (the first declaration when
    /// there's none) and how many body-less declarations (overloads,
    /// `declare function`) come with it.
    functions: HashMap<&'a str, (&'a Function<'a>, usize)>,
    /// The first declaration of each function (its first overload, if any),
    /// which is the one ngtsc checks as an input transform.
    first_functions: HashMap<&'a str, &'a Function<'a>>,
    enums: HashMap<&'a str, &'a TSEnumDeclaration<'a>>,
    classes: HashMap<&'a str, &'a Class<'a>>,
    imports: HashMap<&'a str, Import<'a>>,
    /// Names exported from the file (`export ...`, `export { ... }` and
    /// `export default name`).
    exported: HashSet<&'a str>,
    /// Interfaces, type aliases, classes and enums declared in the file, and
    /// whether the first declaration of each is itself marked `export`.
    types: HashMap<&'a str, bool>,
    /// Namespaces (`namespace NS {}`, `declare namespace NS {}`, `module NS {}`),
    /// with every declaration of each (TypeScript merges them).
    namespaces: HashMap<&'a str, std::vec::Vec<&'a TSNamespaceDeclaration<'a>>>,
    /// Every declaration of each enum (TypeScript merges them).
    enum_declarations: HashMap<&'a str, std::vec::Vec<&'a TSEnumDeclaration<'a>>>,
    /// Import-equals aliases (`import A = NS.T`, `export import A = NS`,
    /// `import A = require('m')`): what each stands for.
    aliases: HashMap<&'a str, &'a TSModuleReference<'a>>,
    /// Where each top-level function, class and variable is first declared, as
    /// ngtsc's diagnostics point at it: the whole statement (with `export`) for
    /// a function or class, the declarator (`x: T`) for a variable.
    declaration_spans: HashMap<&'a str, Span>,
    /// Namespaces and `import x = ...` aliases: declared in the file, but not
    /// read by the evaluator (their values are dynamic).
    unread: HashSet<&'a str>,
    /// The bodies of the file's namespaces (`namespace NS { ... }`, nested
    /// ones included), which ngtsc reads through `typeof NS.X`.
    blocks: std::vec::Vec<NamespaceBlock<'a>>,
    /// The id of each top-level namespace (see [`NamespaceBlock::namespace`]).
    namespace_ids: HashMap<&'a str, usize>,
    /// The id of the namespace exported as `name` from namespace `n`, by `(n, name)`.
    member_namespaces: HashMap<(usize, &'a str), usize>,
    /// The block each function, class and static method declared in a
    /// namespace is written in, by the address of its node.
    declared_in: HashMap<usize, usize>,
    /// The name of each namespace, by id.
    namespace_names: std::vec::Vec<&'a str>,
    /// The ids of the instantiated namespaces: those that declare a value
    /// (a variable, function, class or enum, or an instantiated namespace),
    /// which makes the namespace a value too. TypeScript gives the others no
    /// value (`M` in `namespace M { type T = 1; }` isn't one).
    instantiated: HashSet<usize>,
}

/// The body of a namespace declaration (`namespace NS { ... }`).
#[derive(Default)]
struct NamespaceBlock<'a> {
    /// The namespace it belongs to. TypeScript merges namespaces declared with
    /// the same name in the same place (`namespace NS {}` twice, or
    /// `namespace A.B {}` and `namespace A { export namespace B {} }`), so
    /// their bodies share the id.
    namespace: usize,
    /// The block it's written in, or `None` at the top level.
    parent: Option<usize>,
    /// Its declarations, collected like the top level's (with `export`ed
    /// names in `exported`).
    scope: FileScope<'a>,
    /// The namespaces declared directly in it, with their ids.
    namespaces: HashMap<&'a str, usize>,
    /// The namespace's name, qualified from the top level (`A.B`).
    name: String,
    span: Span,
    /// The statement that declares it (with `export`), where ngtsc's
    /// diagnostics point.
    statement: Span,
}

/// What `typeof A.B.X` names, for `A.B` a namespace the file declares.
enum Qualified<'a> {
    /// The value `X` declared (and exported) in a namespace block.
    Declared(usize, &'a str),
    /// The (instantiated) namespace `X`, and its id.
    Namespace(&'a str, usize),
    /// Nothing TypeScript can resolve: `X` isn't exported, or doesn't exist.
    Missing,
}

/// Whether an enum `e` declares the member `member`, however its name is
/// written (`A`, `'A'`, `['A']` or `` [`A`] ``).
fn enum_declares_member(e: &TSEnumDeclaration<'_>, member: &str) -> bool {
    e.body.members.iter().any(|m| match &m.id {
        TSEnumMemberName::Identifier(id) => id.name == member,
        TSEnumMemberName::String(s) | TSEnumMemberName::ComputedString(s) => s.value == member,
        TSEnumMemberName::ComputedTemplateString(t) => {
            t.single_quasi().is_some_and(|q| q == member)
        }
    })
}

/// Whether the namespace `ns` declares the enum `path` (`["M", "E"]` for
/// `M.E`, with `M` a namespace in `ns`) with the member `member`.
fn namespace_declares_enum_member(
    ns: &TSNamespaceDeclaration<'_>,
    path: &[&str],
    member: &str,
) -> bool {
    let statements = match &ns.body {
        // `namespace A.B {}`: `B` is the only member of `A`.
        TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
            return match path {
                [name, rest @ ..] if !rest.is_empty() && inner.id.name == *name => {
                    namespace_declares_enum_member(inner, rest, member)
                }
                _ => false,
            };
        }
        TSNamespaceDeclarationBody::TSModuleBlock(block) => &block.body,
    };
    statements.iter().any(|statement| {
        let decl = match statement {
            Statement::ExportDeclaration(export) => Some(&export.declaration),
            _ => statement.as_declaration(),
        };
        match (decl, path) {
            (Some(Declaration::TSEnumDeclaration(e)), [name]) => {
                e.id.name == *name && enum_declares_member(e, member)
            }
            (Some(Declaration::TSNamespaceDeclaration(inner)), [name, rest @ ..])
                if !rest.is_empty() && inner.id.name == *name =>
            {
                namespace_declares_enum_member(inner.as_ref(), rest, member)
            }
            _ => false,
        }
    })
}

/// What an import-equals alias stands for (see [`FileScope::alias`]).
pub(crate) enum AliasTarget<'a> {
    /// `import A = require('m')`: the module `m`.
    Module(&'a str),
    /// `import A = NS.T`: the entity's parts, head first (`["NS", "T"]`).
    Entity(std::vec::Vec<&'a str>),
}

impl<'a> AliasTarget<'a> {
    fn of(reference: &'a TSModuleReference<'a>) -> Option<Self> {
        fn parts<'a>(name: &'a TSTypeName<'a>, out: &mut std::vec::Vec<&'a str>) -> Option<()> {
            match name {
                TSTypeName::IdentifierReference(id) => out.push(id.name.as_str()),
                TSTypeName::QualifiedName(q) => {
                    parts(&q.left, out)?;
                    out.push(q.right.name.as_str());
                }
                TSTypeName::ThisExpression(_) => return None,
            }
            Some(())
        }
        Some(match reference {
            TSModuleReference::ExternalModuleReference(m) => {
                AliasTarget::Module(m.expression.value.as_str())
            }
            TSModuleReference::IdentifierReference(id) => {
                AliasTarget::Entity(std::vec![id.name.as_str()])
            }
            TSModuleReference::QualifiedName(q) => {
                let mut out = std::vec::Vec::new();
                parts(&q.left, &mut out)?;
                out.push(q.right.name.as_str());
                AliasTarget::Entity(out)
            }
        })
    }
}

/// A declaration directly in a namespace, as far as names resolve through it.
enum NamespaceMember<'a> {
    Namespace(&'a TSNamespaceDeclaration<'a>),
    Alias(&'a TSModuleReference<'a>),
    Other,
}

/// The declarations directly in the namespace `ns`, by name.
fn namespace_members<'a>(
    ns: &'a TSNamespaceDeclaration<'a>,
) -> std::vec::Vec<(&'a str, NamespaceMember<'a>)> {
    let statements = match &ns.body {
        // `namespace A.B {}`: `B` is the only member of `A`.
        TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
            return std::vec![(inner.id.name.as_str(), NamespaceMember::Namespace(inner))];
        }
        TSNamespaceDeclarationBody::TSModuleBlock(block) => &block.body,
    };
    let mut members = std::vec::Vec::new();
    for statement in statements {
        let decl = match statement {
            Statement::ExportDeclaration(export) => &export.declaration,
            _ => match statement.as_declaration() {
                Some(decl) => decl,
                None => continue,
            },
        };
        match decl {
            Declaration::TSNamespaceDeclaration(inner) => {
                members.push((inner.id.name.as_str(), NamespaceMember::Namespace(inner)));
            }
            Declaration::TSImportEqualsDeclaration(alias) => {
                members.push((
                    alias.id.name.as_str(),
                    NamespaceMember::Alias(&alias.module_reference),
                ));
            }
            Declaration::VariableDeclaration(vars) => {
                for var in &vars.declarations {
                    let mut bindings = std::vec::Vec::new();
                    collect_bindings(&var.id, &mut std::vec::Vec::new(), &mut bindings);
                    members
                        .extend(bindings.into_iter().map(|(id, _)| (id, NamespaceMember::Other)));
                }
            }
            Declaration::FunctionDeclaration(f) => {
                members.extend(f.id.as_ref().map(|id| (id.name.as_str(), NamespaceMember::Other)));
            }
            Declaration::ClassDeclaration(c) => {
                members.extend(c.id.as_ref().map(|id| (id.name.as_str(), NamespaceMember::Other)));
            }
            Declaration::TSEnumDeclaration(e) => {
                members.push((e.id.name.as_str(), NamespaceMember::Other));
            }
            Declaration::TSInterfaceDeclaration(i) => {
                members.push((i.id.name.as_str(), NamespaceMember::Other));
            }
            Declaration::TSTypeAliasDeclaration(t) => {
                members.push((t.id.name.as_str(), NamespaceMember::Other));
            }
            _ => {}
        }
    }
    members
}

/// A top-level variable binding.
enum Variable<'a> {
    /// A name bound by a declaration with an initializer, and the path to it
    /// when it's destructured (`const { a: [x] } = init` binds `x` at `a`, `0`).
    Init(&'a Expression<'a>, std::vec::Vec<PathKey<'a>>),
    /// `declare const X: T`: evaluated from its type when that's a literal.
    Declared(Option<&'a TSType<'a>>),
    /// `let x;`: `undefined`.
    Uninitialized,
}

/// A step into a destructured initializer, as ngtsc's `visitBindingElement` reads it.
#[derive(Clone, Copy)]
enum PathKey<'a> {
    Index(usize),
    Key(&'a str),
    /// A key ngtsc doesn't follow (a string literal or computed property name).
    Unknown,
}

/// An import binding: the module it comes from and, unless it's a namespace
/// import, the name it's exported under.
#[derive(Clone, Copy)]
pub(crate) struct Import<'a> {
    pub module: &'a str,
    pub imported: Option<&'a str>,
}

impl<'a> FileScope<'a> {
    pub(crate) fn collect(program: &'a Program<'a>) -> Self {
        let mut scope = Self::default();
        for stmt in &program.body {
            match stmt {
                Statement::ImportDeclaration(import) => {
                    let module = import.source.value.as_str();
                    for spec in import.specifiers.iter().flatten() {
                        let (local, imported) = match spec {
                            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                let imported = match &s.imported {
                                    ModuleExportName::IdentifierName(n) => n.name.as_str(),
                                    ModuleExportName::IdentifierReference(n) => n.name.as_str(),
                                    ModuleExportName::StringLiteral(n) => n.value.as_str(),
                                };
                                (s.local.name.as_str(), Some(imported))
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                                (s.local.name.as_str(), Some("default"))
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                                (s.local.name.as_str(), None)
                            }
                        };
                        scope.imports.insert(local, Import { module, imported });
                    }
                }
                Statement::ExportDeclaration(export) => {
                    scope.declaration(&export.declaration, true, export.span);
                }
                Statement::ExportNamedDeclaration(export) => {
                    for spec in &export.specifiers {
                        if let ModuleExportName::IdentifierReference(local) = &spec.local {
                            scope.exported.insert(local.name.as_str());
                        }
                    }
                }
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        scope.class(class, true, export.span);
                    }
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        scope.function(function, true, export.span);
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => {
                        scope.types.entry(i.id.name.as_str()).or_insert(true);
                    }
                    ExportDefaultDeclarationKind::Identifier(id) => {
                        scope.exported.insert(id.name.as_str());
                    }
                    _ => {}
                },
                _ => {
                    if let Some(decl) = stmt.as_declaration() {
                        scope.declaration(decl, false, decl.span());
                    }
                }
            }
        }
        for stmt in &program.body {
            let (decl, exported) = match stmt {
                Statement::ExportDeclaration(export) => (&export.declaration, true),
                _ => match stmt.as_declaration() {
                    Some(decl) => (decl, false),
                    None => continue,
                },
            };
            match decl {
                Declaration::TSNamespaceDeclaration(ns) => {
                    scope.unread.insert(ns.id.name.as_str());
                    scope.namespace(ns, None, exported, false, stmt.span());
                }
                Declaration::TSImportEqualsDeclaration(alias) => {
                    scope.unread.insert(alias.id.name.as_str());
                }
                _ => {}
            }
        }
        scope.mark_instantiated();
        scope
    }

    /// Collects the namespace `ns`, declared in `parent` (`None`: at the top
    /// level) by `statement`, and in an ambient context (`declare namespace`)
    /// if `ambient`.
    fn namespace(
        &mut self,
        ns: &'a TSNamespaceDeclaration<'a>,
        parent: Option<usize>,
        exported: bool,
        ambient: bool,
        statement: Span,
    ) {
        let name = ns.id.name.as_str();
        // Every declaration in an ambient namespace is exported, with or
        // without `export` (TypeScript's `ExportContext`).
        let ambient = ambient || ns.declare;
        // Top-level namespaces merge by name, exported ones by the namespace
        // they're a member of, and others only within their block.
        let next = self.namespace_names.len();
        let id = match parent {
            None => *self.namespace_ids.entry(name).or_insert(next),
            Some(p) if exported => {
                let key = (self.blocks[p].namespace, name);
                *self.member_namespaces.entry(key).or_insert(next)
            }
            Some(p) => self.blocks[p].namespaces.get(name).copied().unwrap_or(next),
        };
        if id == next {
            self.namespace_names.push(name);
        }
        let qualified = match parent {
            Some(p) => format!("{}.{name}", self.blocks[p].name),
            None => name.to_string(),
        };
        if let Some(p) = parent {
            self.blocks[p].namespaces.insert(name, id);
            if exported {
                self.blocks[p].scope.exported.insert(name);
            }
        }
        let index = self.blocks.len();
        self.blocks.push(NamespaceBlock {
            namespace: id,
            parent,
            name: qualified,
            span: ns.span,
            statement,
            ..NamespaceBlock::default()
        });
        let statements = match &ns.body {
            // `namespace A.B {}`: `B` is an exported member of `A`.
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
                self.namespace(inner, Some(index), true, ambient, inner.span);
                return;
            }
            TSNamespaceDeclarationBody::TSModuleBlock(block) => &block.body,
        };
        let mut scope = FileScope::default();
        let mut nested = std::vec::Vec::new();
        for stmt in statements {
            let (decl, exported, span) = match stmt {
                Statement::ExportDeclaration(export) => (&export.declaration, true, export.span),
                _ => match stmt.as_declaration() {
                    Some(decl) => (decl, false, decl.span()),
                    None => continue,
                },
            };
            scope.declaration(decl, exported, span);
            // Without `export` it isn't statically exported for ngtsc (see
            // `UnexportedType`), but still visible outside the namespace.
            let exported = exported || ambient;
            if exported {
                scope.exported.extend(value_names(decl));
            }
            match decl {
                Declaration::FunctionDeclaration(f) => {
                    self.declared_in.insert(std::ptr::from_ref::<Function>(f) as usize, index);
                }
                Declaration::ClassDeclaration(class) => {
                    self.declared_in.insert(std::ptr::from_ref::<Class>(class) as usize, index);
                    for element in &class.body.body {
                        if let ClassElement::MethodDefinition(m) = element {
                            let method = std::ptr::from_ref::<Function>(&m.value) as usize;
                            self.declared_in.insert(method, index);
                        }
                    }
                }
                Declaration::TSNamespaceDeclaration(inner) => {
                    nested.push((&**inner, exported, span));
                }
                _ => {}
            }
        }
        self.blocks[index].scope = scope;
        for (inner, exported, span) in nested {
            self.namespace(inner, Some(index), exported, ambient, span);
        }
    }

    /// Marks the namespaces that declare a value as instantiated (see
    /// [`Self::instantiated`]), once they're all collected.
    fn mark_instantiated(&mut self) {
        loop {
            let before = self.instantiated.len();
            for b in 0..self.blocks.len() {
                if self.block_has_values(b) {
                    self.instantiated.insert(self.blocks[b].namespace);
                }
            }
            if self.instantiated.len() == before {
                break;
            }
        }
    }

    /// Whether the namespace block `b` declares a value: a variable,
    /// function, class or enum, or an instantiated namespace.
    fn block_has_values(&self, b: usize) -> bool {
        let block = &self.blocks[b];
        let scope = &block.scope;
        !scope.variables.is_empty()
            || !scope.functions.is_empty()
            || !scope.classes.is_empty()
            || !scope.enums.is_empty()
            || block.namespaces.values().any(|id| self.instantiated.contains(id))
    }

    /// A reference to the (instantiated) namespace `name` with the id `id`,
    /// which ngtsc points at the first declaration of it that declares a value.
    fn namespace_reference<'v>(&self, name: &str, id: usize) -> Value<'v> {
        let span = (0..self.blocks.len())
            .find(|&b| self.blocks[b].namespace == id && self.block_has_values(b))
            .map_or(Span::default(), |b| self.blocks[b].statement);
        Value::Reference { name: name.into(), kind: RefKind::Namespace(span) }
    }

    /// The innermost namespace block whose declaration contains `span`.
    fn block_at(&self, span: Span) -> Option<usize> {
        (0..self.blocks.len())
            .filter(|&b| self.blocks[b].span.contains_inclusive(span))
            .min_by_key(|&b| self.blocks[b].span.size())
    }

    /// For a type named `name` in the namespace block `block` (`None`: at the
    /// top level), whether ngtsc's `isStaticallyExported` holds for the
    /// declaration it resolves to (see [`Self::type_is_exported`]): in a
    /// namespace, whether it's marked `export`. `None` for a name the file
    /// doesn't declare as a type.
    fn type_is_exported_in(&self, block: Option<usize>, name: &str) -> Option<bool> {
        let mut current = block;
        while let Some(b) = current {
            let namespace = self.blocks[b].namespace;
            if let Some(exported) = self.blocks[b].scope.types.get(name) {
                return Some(*exported);
            }
            // Exported from another body of the namespace.
            if self
                .blocks
                .iter()
                .any(|o| o.namespace == namespace && o.scope.types.get(name) == Some(&true))
            {
                return Some(true);
            }
            current = self.blocks[b].parent;
        }
        self.type_is_exported(name)
    }

    /// The declarations of the scope (a namespace block or the top level) the
    /// function, static method or class `node` is declared in.
    fn scope_of<T>(&self, node: &T) -> &FileScope<'a> {
        self.declarations(self.block_of(node))
    }

    /// Whether the namespace block `b` itself declares the value `name`: a
    /// variable, function, class, enum, import-equals alias or instantiated
    /// namespace.
    fn block_declares(&self, b: usize, name: &str) -> bool {
        let block = &self.blocks[b];
        block.namespaces.get(name).is_some_and(|id| self.instantiated.contains(id))
            || block.scope.aliases.contains_key(name)
            || block.scope.variables.contains_key(name)
            || block.scope.functions.contains_key(name)
            || block.scope.enums.contains_key(name)
            || block.scope.classes.contains_key(name)
    }

    /// A block of the namespace `namespace` that exports the value or namespace `name`.
    fn exporting_block(&self, namespace: usize, name: &str) -> Option<usize> {
        (0..self.blocks.len()).find(|&b| {
            self.blocks[b].namespace == namespace
                && self.blocks[b].scope.exported.contains(name)
                && self.block_declares(b, name)
        })
    }

    /// The namespace block whose declaration `name` refers to inside block
    /// `from`: the block's own, one its namespace exports from another of its
    /// bodies, or the same for each enclosing block. `None` when the name is
    /// resolved at the top level.
    fn declaring_block(&self, from: usize, name: &str) -> Option<usize> {
        let mut current = Some(from);
        while let Some(b) = current {
            if self.block_declares(b, name) {
                return Some(b);
            }
            if let Some(other) = self.exporting_block(self.blocks[b].namespace, name) {
                return Some(other);
            }
            current = self.blocks[b].parent;
        }
        None
    }

    /// The id of the namespace `name` names in `from` (`None`: the top level),
    /// as the head of a qualified name: a namespace, or an import-equals alias
    /// of one, declared there or in an enclosing block. `None` when `name` is
    /// something else.
    fn namespace_of(&self, from: Option<usize>, name: &str, depth: u16) -> Option<usize> {
        if depth > MAX_ALIASES {
            return None;
        }
        let mut current = from;
        while let Some(b) = current {
            let block = &self.blocks[b];
            if let Some(id) = block.namespaces.get(name) {
                return Some(*id);
            }
            if let Some(reference) = block.scope.aliases.get(name) {
                return self.alias_namespace(Some(b), reference, depth + 1);
            }
            if self.block_declares(b, name) {
                return None;
            }
            if let Some(id) = self.member_namespaces.get(&(block.namespace, name)) {
                return Some(*id);
            }
            if let Some(other) = self.exporting_block(block.namespace, name) {
                let reference = self.blocks[other].scope.aliases.get(name)?;
                return self.alias_namespace(Some(other), reference, depth + 1);
            }
            current = block.parent;
        }
        if let Some(id) = self.namespace_ids.get(name) {
            return Some(*id);
        }
        let reference = self.aliases.get(name)?;
        self.alias_namespace(None, reference, depth + 1)
    }

    /// The id of the namespace an import-equals alias declared in `block`
    /// stands for, if it's one.
    fn alias_namespace(
        &self,
        block: Option<usize>,
        reference: &TSModuleReference<'_>,
        depth: u16,
    ) -> Option<usize> {
        let parts = match reference {
            TSModuleReference::IdentifierReference(id) => std::vec![id.name.as_str()],
            TSModuleReference::QualifiedName(q) => qualified_parts(q)?,
            TSModuleReference::ExternalModuleReference(_) => return None,
        };
        let mut id = self.namespace_of(block, parts[0], depth)?;
        for part in &parts[1..] {
            id = *self.member_namespaces.get(&(id, *part))?;
        }
        Some(id)
    }

    /// The declarations of the block `block`, or of the top level for `None`.
    fn declarations(&self, block: Option<usize>) -> &FileScope<'a> {
        block.map_or(self, |b| &self.blocks[b].scope)
    }

    /// The block a function, static method or class declared in a namespace is
    /// written in, or `None` for one declared at the top level.
    fn block_of<T>(&self, node: &T) -> Option<usize> {
        self.declared_in.get(&(std::ptr::from_ref(node) as usize)).copied()
    }

    /// What the qualified name `parts` (`["A", "B", "X"]` for `A.B.X`),
    /// written in `from`, names when its head is a namespace the file
    /// declares, as TypeScript resolves it: each part after the head is an
    /// exported member of the namespace before it. `None` when the head, or a
    /// part before the last, isn't a namespace.
    fn qualified(&self, from: Option<usize>, parts: &[&'a str]) -> Option<Qualified<'a>> {
        let (head, rest) = parts.split_first()?;
        let (last, middle) = rest.split_last()?;
        let mut namespace = self.namespace_of(from, head, 0)?;
        for part in middle {
            match self.member_namespaces.get(&(namespace, *part)) {
                Some(id) => namespace = *id,
                None if self.exporting_block(namespace, part).is_some() => return None,
                None => return Some(Qualified::Missing),
            }
        }
        let Some(b) = self.exporting_block(namespace, last) else {
            return Some(Qualified::Missing);
        };
        Some(match self.blocks[b].namespaces.get(last) {
            Some(id) => Qualified::Namespace(last, *id),
            None => Qualified::Declared(b, last),
        })
    }

    /// The qualified name of the namespace `expr` is written in, when it uses
    /// that namespace's declarations (or an enclosing one's), which don't
    /// resolve at the top level.
    pub(crate) fn namespace_used_by(&self, expr: &Expression<'a>) -> Option<&str> {
        let block = self.block_at(expr.span())?;
        let frame = Frame::at(Some(block));
        let mut uses = UsesFrame::new(&frame, self);
        uses.visit_expression(expr);
        uses.found.then(|| self.blocks[block].name.as_str())
    }

    fn declaration(&mut self, decl: &'a Declaration<'a>, exported: bool, span: Span) {
        let name = |scope: &mut Self, name: &'a str| {
            if exported {
                scope.exported.insert(name);
            }
        };
        match decl {
            Declaration::VariableDeclaration(vars) => {
                for var in &vars.declarations {
                    let mut bindings = std::vec::Vec::new();
                    collect_bindings(&var.id, &mut std::vec::Vec::new(), &mut bindings);
                    for (id, path) in bindings {
                        let variable = match &var.init {
                            Some(init) => Variable::Init(init, path),
                            None if vars.declare => Variable::Declared(
                                var.type_annotation.as_ref().map(|t| &t.type_annotation),
                            ),
                            None => Variable::Uninitialized,
                        };
                        self.variables.entry(id).or_insert(variable);
                        self.declaration_spans.entry(id).or_insert(var.span);
                        name(self, id);
                    }
                }
            }
            Declaration::FunctionDeclaration(function) => self.function(function, exported, span),
            Declaration::ClassDeclaration(class) => self.class(class, exported, span),
            Declaration::TSEnumDeclaration(e) => {
                let id = e.id.name.as_str();
                self.enums.entry(id).or_insert(e);
                self.enum_declarations.entry(id).or_default().push(e);
                self.types.entry(id).or_insert(exported);
                name(self, id);
            }
            Declaration::TSInterfaceDeclaration(i) => {
                self.types.entry(i.id.name.as_str()).or_insert(exported);
                name(self, i.id.name.as_str());
            }
            Declaration::TSTypeAliasDeclaration(t) => {
                self.types.entry(t.id.name.as_str()).or_insert(exported);
                name(self, t.id.name.as_str());
            }
            Declaration::TSImportEqualsDeclaration(alias) => {
                self.aliases.entry(alias.id.name.as_str()).or_insert(&alias.module_reference);
                name(self, alias.id.name.as_str());
            }
            Declaration::TSNamespaceDeclaration(ns) => {
                self.namespaces.entry(ns.id.name.as_str()).or_default().push(ns);
            }
            _ => {}
        }
    }

    /// Whether the file declares `name` at its top level: a namespace, class,
    /// function, variable, enum, interface or type alias.
    pub(crate) fn declares(&self, name: &str) -> bool {
        self.namespaces.contains_key(name)
            || self.types.contains_key(name)
            || self.classes.contains_key(name)
            || self.enums.contains_key(name)
            || self.functions.contains_key(name)
            || self.variables.contains_key(name)
    }

    /// Whether `path.member` (`["NS", "E"]` and `A` for `NS.E.A`) is a member
    /// of an enum the file declares: at its top level, or in a namespace it
    /// declares (a class or function merged with one included).
    pub(crate) fn is_enum_member(&self, path: &[&str], member: &str) -> bool {
        match path {
            [] => false,
            [name] => self
                .enum_declarations
                .get(name)
                .is_some_and(|decls| decls.iter().any(|e| enum_declares_member(e, member))),
            [head, rest @ ..] => self.namespaces.get(head).is_some_and(|decls| {
                decls.iter().any(|ns| namespace_declares_enum_member(ns, rest, member))
            }),
        }
    }

    /// What `name` stands for when it's a top-level import-equals alias.
    pub(crate) fn alias(&self, name: &str) -> Option<AliasTarget<'a>> {
        AliasTarget::of(self.aliases.get(name)?)
    }

    /// For the qualified name `parts` (head first) whose head is a namespace
    /// the file declares, the first member that's an import-equals alias
    /// declared in the namespace before it (`NS.A` for
    /// `namespace NS { export import A = X; }`): how many parts it covers, and
    /// what they stand for. The alias's target is named from inside its
    /// namespace, so its head resolves there first, then in the enclosing
    /// namespaces, then at the top level: `X` is `["NS", "X"]` when `NS`
    /// declares `X`.
    pub(crate) fn namespace_alias<'p>(&self, parts: &[&'p str]) -> Option<(usize, AliasTarget<'p>)>
    where
        'a: 'p,
    {
        let mut decls = self.namespaces.get(parts.first()?)?.clone();
        // The members of `parts[..=k]`, for each namespace `k` walked through.
        let mut scopes: std::vec::Vec<std::vec::Vec<(&'a str, NamespaceMember<'a>)>> =
            std::vec::Vec::new();
        for (i, name) in parts.iter().enumerate().skip(1) {
            let members: std::vec::Vec<_> =
                decls.iter().flat_map(|ns| namespace_members(ns)).collect();
            let alias = members.iter().find_map(|(n, m)| match m {
                NamespaceMember::Alias(reference) if n == name => Some(*reference),
                _ => None,
            });
            decls = members
                .iter()
                .filter_map(|(n, m)| match m {
                    NamespaceMember::Namespace(ns) if n == name => Some(*ns),
                    _ => None,
                })
                .collect();
            scopes.push(members);
            if let Some(reference) = alias {
                return Some((
                    i + 1,
                    match AliasTarget::of(reference)? {
                        AliasTarget::Module(m) => AliasTarget::Module(m),
                        AliasTarget::Entity(target) => {
                            let head = target[0];
                            let path = match scopes
                                .iter()
                                .rposition(|members| members.iter().any(|(n, _)| *n == head))
                            {
                                Some(k) => parts[..=k].iter().copied().chain(target).collect(),
                                None => target,
                            };
                            AliasTarget::Entity(path)
                        }
                    },
                ));
            }
            if decls.is_empty() {
                return None;
            }
        }
        None
    }

    /// Whether `function` is the top-level function declaration called `name`
    /// (whose name is in scope where the metadata is compiled), rather than a
    /// static method.
    pub(crate) fn is_top_level_function(&self, name: &str, function: &Function<'_>) -> bool {
        self.functions.get(name).is_some_and(|(f, _)| std::ptr::eq(*f, function))
    }

    /// The first declaration of the function or static method `name` whose
    /// implementation is `function`: its first overload, if any. That's
    /// TypeScript's value declaration, the one ngtsc checks as a transform.
    fn first_declaration(&self, name: &str, function: &'a Function<'a>) -> &'a Function<'a> {
        // In the namespace it's declared in, if any.
        let scope = self.scope_of(function);
        if scope.is_top_level_function(name, function) {
            return scope.first_functions.get(name).copied().unwrap_or(function);
        }
        scope.first_static_method(function).map_or(function, |m| &m.value)
    }

    /// The first declaration of the static method whose implementation is
    /// `function`.
    fn first_static_method(&self, function: &Function<'_>) -> Option<&'a MethodDefinition<'a>> {
        self.classes.values().find_map(|class| {
            let methods = || {
                class.body.body.iter().filter_map(|el| match el {
                    ClassElement::MethodDefinition(m) if m.r#static => Some(&**m),
                    _ => None,
                })
            };
            let method = methods().find(|m| std::ptr::eq(&*m.value, function))?;
            let name = method.key.static_name()?;
            methods().find(|m| m.key.static_name().is_some_and(|n| n == name))
        })
    }

    fn function(&mut self, function: &'a Function<'a>, exported: bool, span: Span) {
        let Some(id) = &function.id else { return };
        let id = id.name.as_str();
        if exported {
            self.exported.insert(id);
        }
        self.declaration_spans.entry(id).or_insert(span);
        self.first_functions.entry(id).or_insert(function);
        // Overloads are body-less declarations followed by the implementation.
        let entry = self.functions.entry(id).or_insert((function, 0));
        if function.body.is_none() {
            entry.1 += 1;
        }
        if function.body.is_some() || entry.0.body.is_none() {
            entry.0 = function;
        }
    }

    fn class(&mut self, class: &'a Class<'a>, exported: bool, span: Span) {
        let Some(id) = &class.id else { return };
        let id = id.name.as_str();
        self.classes.insert(id, class);
        self.declaration_spans.entry(id).or_insert(span);
        self.types.entry(id).or_insert(exported);
        if exported {
            self.exported.insert(id);
        }
    }

    /// For a type declared in the file, whether ngtsc's `isStaticallyExported`
    /// holds for the declaration the name resolves to: `None` for a name the
    /// file doesn't declare as a type.
    ///
    /// A name with a value declaration (a class or enum, or a variable or
    /// function merged with the type) resolves to it, and a value is also
    /// exported by `export { X }` or `export default X`. An interface or type
    /// alias has no value declaration: only its own `export` counts.
    fn type_is_exported(&self, name: &str) -> Option<bool> {
        let exported_itself = *self.types.get(name)?;
        let has_value = self.classes.contains_key(name)
            || self.enums.contains_key(name)
            || self.variables.contains_key(name)
            || self.functions.contains_key(name);
        Some(if has_value { self.exported.contains(name) } else { exported_itself })
    }

    pub(crate) fn import(&self, name: &str) -> Option<Import<'a>> {
        self.imports.get(name).copied()
    }

    /// The expression the top-level variable `name` is initialized with: its
    /// initializer or, destructured from an object or array literal, the part
    /// it binds (`transform` in `const { transform } = { transform: f }` is `f`).
    fn initializer(&self, name: &str) -> Option<&'a Expression<'a>> {
        let Variable::Init(init, path) = self.variables.get(name)? else { return None };
        let mut init: &'a Expression<'a> = init;
        for key in path {
            init = match (key, init.without_parentheses()) {
                (PathKey::Key(k), Expression::ObjectExpression(obj)) => {
                    // A spread could replace the property: only a literal one counts.
                    let spread = |p: &ObjectPropertyKind<'_>| {
                        matches!(p, ObjectPropertyKind::SpreadProperty(_))
                    };
                    if obj.properties.iter().any(spread) {
                        return None;
                    }
                    obj.properties.iter().rev().find_map(|p| match p {
                        ObjectPropertyKind::ObjectProperty(p)
                            if !p.computed && p.key.static_name().is_some_and(|n| n == *k) =>
                        {
                            Some(&p.value)
                        }
                        _ => None,
                    })?
                }
                // Up to the element, every one must be a plain expression.
                (PathKey::Index(i), Expression::ArrayExpression(arr)) => {
                    let before = arr.elements.get(..=*i)?;
                    if !before.iter().all(ArrayExpressionElement::is_expression) {
                        return None;
                    }
                    before[*i].as_expression()?
                }
                _ => return None,
            };
        }
        Some(init)
    }

    /// The type parameters of the innermost same-file function or static
    /// method (the functions the evaluator calls) whose body contains `span`.
    fn enclosing_type_parameters(&self, span: Span) -> Option<&'a TSTypeParameterDeclaration<'a>> {
        // Declared at the top level or in a namespace.
        let scopes = || std::iter::once(self).chain(self.blocks.iter().map(|b| &b.scope));
        let methods = scopes().flat_map(|scope| scope.classes.values()).flat_map(|class| {
            class.body.body.iter().filter_map(|el| match el {
                ClassElement::MethodDefinition(m) if m.r#static => Some(&*m.value),
                _ => None,
            })
        });
        scopes()
            .flat_map(|scope| scope.functions.values())
            .map(|(function, _)| *function)
            .chain(methods)
            .filter(|f| f.body.as_ref().is_some_and(|body| body.span.contains_inclusive(span)))
            .min_by_key(|f| f.span.size())
            .and_then(|f| f.type_parameters.as_deref())
    }
}

/// The values a declaration declares (not its types, or a namespace).
fn value_names<'a>(decl: &'a Declaration<'a>) -> std::vec::Vec<&'a str> {
    let mut names = std::vec::Vec::new();
    match decl {
        Declaration::VariableDeclaration(vars) => {
            for var in &vars.declarations {
                let mut bindings = std::vec::Vec::new();
                collect_bindings(&var.id, &mut std::vec::Vec::new(), &mut bindings);
                names.extend(bindings.into_iter().map(|(name, _)| name));
            }
        }
        Declaration::FunctionDeclaration(f) => {
            names.extend(f.id.as_ref().map(|id| id.name.as_str()))
        }
        Declaration::ClassDeclaration(c) => names.extend(c.id.as_ref().map(|id| id.name.as_str())),
        Declaration::TSEnumDeclaration(e) => names.push(e.id.name.as_str()),
        Declaration::TSImportEqualsDeclaration(a) => names.push(a.id.name.as_str()),
        _ => {}
    }
    names
}

/// The names a binding pattern declares, each with its path into the initializer.
fn collect_bindings<'a>(
    pattern: &'a BindingPattern<'a>,
    path: &mut std::vec::Vec<PathKey<'a>>,
    out: &mut std::vec::Vec<(&'a str, std::vec::Vec<PathKey<'a>>)>,
) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.push((id.name.as_str(), path.clone())),
        // A default value isn't evaluated (`const { a = 1 } = {}` is `undefined`).
        BindingPattern::AssignmentPattern(p) => collect_bindings(&p.left, path, out),
        BindingPattern::ObjectPattern(p) => {
            for prop in &p.properties {
                path.push(match &prop.key {
                    PropertyKey::StaticIdentifier(id) if !prop.computed => {
                        PathKey::Key(id.name.as_str())
                    }
                    _ => PathKey::Unknown,
                });
                collect_bindings(&prop.value, path, out);
                path.pop();
            }
            // ngtsc reads `{ ...rest }` as the property named after the binding.
            if let Some(rest) = &p.rest {
                path.push(match &rest.argument {
                    BindingPattern::BindingIdentifier(id) => PathKey::Key(id.name.as_str()),
                    _ => PathKey::Unknown,
                });
                collect_bindings(&rest.argument, path, out);
                path.pop();
            }
        }
        BindingPattern::ArrayPattern(p) => {
            for (i, element) in p.elements.iter().enumerate() {
                if let Some(element) = element {
                    path.push(PathKey::Index(i));
                    collect_bindings(element, path, out);
                    path.pop();
                }
            }
            // ... and `[a, ...rest]` as the element at the rest element's position.
            if let Some(rest) = &p.rest {
                path.push(PathKey::Index(p.elements.len()));
                collect_bindings(&rest.argument, path, out);
                path.pop();
            }
        }
    }
}

/// The parts of a qualified name, head first (`["A", "B", "X"]` for `A.B.X`).
fn qualified_parts_of<'a>(name: &TSTypeName<'a>) -> Option<std::vec::Vec<&'a str>> {
    fn parts<'a>(name: &TSTypeName<'a>, out: &mut std::vec::Vec<&'a str>) -> Option<()> {
        match name {
            TSTypeName::IdentifierReference(id) => out.push(id.name.as_str()),
            TSTypeName::QualifiedName(q) => {
                parts(&q.left, out)?;
                out.push(q.right.name.as_str());
            }
            TSTypeName::ThisExpression(_) => return None,
        }
        Some(())
    }
    let mut out = std::vec::Vec::new();
    parts(name, &mut out)?;
    Some(out)
}

/// [`qualified_parts_of`] for the name in `typeof A.B.X`.
fn qualified_parts<'a>(name: &TSQualifiedName<'a>) -> Option<std::vec::Vec<&'a str>> {
    let mut out = qualified_parts_of(&name.left)?;
    out.push(name.right.name.as_str());
    Some(out)
}

/// A function definition ngtsc's `getDefinitionOfFunction` accepts.
#[derive(Clone, Copy)]
pub(crate) enum FnDef<'a> {
    Function(&'a Function<'a>),
    Arrow(&'a ArrowFunctionExpression<'a>),
}

impl<'a> FnDef<'a> {
    fn is_generic(self) -> bool {
        match self {
            FnDef::Function(f) => f.type_parameters.is_some(),
            FnDef::Arrow(f) => f.type_parameters.is_some(),
        }
    }

    pub(crate) fn params(self) -> &'a FormalParameters<'a> {
        match self {
            FnDef::Function(f) => &f.params,
            FnDef::Arrow(f) => &f.params,
        }
    }

    fn span(self) -> Span {
        match self {
            FnDef::Function(f) => f.span,
            FnDef::Arrow(f) => f.span,
        }
    }

    /// The type annotation of the first parameter (after any `this` parameter,
    /// which oxc keeps separately): `Ok(None)` when there are no parameters,
    /// `Err(())` when the first one has no type.
    pub(crate) fn first_param_type(self) -> Result<Option<&'a TSType<'a>>, ()> {
        let params = self.params();
        let annotation = match (params.items.first(), &params.rest) {
            (Some(param), _) => &param.type_annotation,
            (None, Some(rest)) => &rest.type_annotation,
            (None, None) => return Ok(None),
        };
        annotation.as_ref().map(|t| Some(&t.type_annotation)).ok_or(())
    }
}

/// What a declaration reference resolves to.
#[derive(Clone)]
pub(crate) enum RefKind<'a> {
    /// A function declaration or static method in this file, with the number of
    /// body-less declarations (overloads) that come with it.
    Function(&'a Function<'a>, usize),
    Class(&'a Class<'a>),
    /// An imported binding, `ns.x` through `import * as ns`
    /// (`namespace_member`), or a value computed from one: its value is in
    /// another file.
    Import {
        namespace_member: bool,
    },
    /// An identifier with no declaration in this file that names a standard
    /// ECMAScript global (see [`ES_GLOBALS`]).
    Global,
    /// Any other identifier this file doesn't declare or import: a global
    /// declared outside it, in a lib such as the DOM's (`atob`, `window`) or in
    /// a project `.d.ts`, which ngtsc resolves to that declaration, or a name
    /// declared nowhere, which ngtsc can't resolve. oxc can't tell these apart,
    /// so it's dynamic (see [`Value::is_dynamic`]), except that an input
    /// transform assumes it's a function, as it does an imported one.
    Ambient,
    /// A static getter or setter, or a static property without an
    /// initializer, declared at the span.
    StaticMember(Span),
    /// A namespace that declares a value, whose first declaration that does
    /// is at the span.
    Namespace(Span),
    /// Any other declaration (a `declare`d variable, an enum member, ...).
    Other,
}

/// A value as ngtsc's partial evaluator resolves it.
#[derive(Clone)]
pub(crate) enum Value<'a> {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value<'a>>),
    Object(Vec<Prop<'a>>),
    /// `import * as ns`.
    Module,
    Reference {
        name: String,
        kind: RefKind<'a>,
    },
    /// A member of an enum declared in this file (ngtsc's `EnumValue`): the
    /// enum's name and the member's value.
    Enum {
        name: String,
        value: Box<Value<'a>>,
    },
    /// `array.slice`, `array.concat` or `string.concat`, which ngtsc can call.
    Builtin(Builtin<'a>),
    Dynamic,
    /// An arrow or function expression written directly as a property value;
    /// the only place ngtsc keeps a function expression analyzable.
    Function(FnDef<'a>),
}

/// ngtsc's `KnownFn`s (partial_evaluator/src/builtin.ts), bound to their receiver.
#[derive(Clone)]
pub(crate) enum Builtin<'a> {
    ArraySlice(Vec<Value<'a>>),
    ArrayConcat(Vec<Value<'a>>),
    StringConcat(String),
}

/// An object literal property: its key, value, and the source expression it came from.
#[derive(Clone)]
pub(crate) struct Prop<'a> {
    pub key: String,
    pub value: Value<'a>,
    pub expr: Option<&'a Expression<'a>>,
    /// `expr` as it can be written where the metadata is compiled, if it can
    /// (see [`Evaluator::origin`]): the same expression outside a function,
    /// the argument a parameter was passed, or `None` for an expression that
    /// uses the parameters of the function it's written in.
    pub origin: Option<&'a Expression<'a>>,
}

impl<'a> Value<'a> {
    pub(crate) fn prop(&self, key: &str) -> Option<&Prop<'a>> {
        match self {
            Value::Object(props) => props.iter().rev().find(|p| p.key == key),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Whether this is an imported binding (or a value computed from one), which
    /// can't be evaluated without reading another file.
    pub(crate) fn is_import(&self) -> bool {
        matches!(self, Value::Reference { kind: RefKind::Import { .. }, .. })
    }

    /// Whether ngtsc's value is unknown: dynamic, or a global declared outside
    /// the file, which is only kept as a name so a transform can refer to it.
    pub(crate) fn is_dynamic(&self) -> bool {
        matches!(self, Value::Dynamic | Value::Reference { kind: RefKind::Ambient, .. })
    }

    /// ngtsc's `describeResolvedType`, one level deep like its diagnostics.
    pub(crate) fn describe(&self) -> String {
        self.describe_to(1)
    }

    fn describe_to(&self, depth: u16) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Undefined => "undefined".into(),
            Value::Bool(_) => "boolean".into(),
            Value::Number(_) => "number".into(),
            Value::String(_) => "string".into(),
            Value::Object(_) if depth == 0 => "object".into(),
            Value::Object(props) if props.is_empty() => "{}".into(),
            Value::Object(props) => {
                let mut seen = HashSet::new();
                // Later duplicates win but keep the first key's position, like a Map.
                let entries: std::vec::Vec<String> = props
                    .iter()
                    .filter(|p| seen.insert(p.key.as_str()))
                    .map(|p| {
                        let value = self.prop(&p.key).map_or(&p.value, |p| &p.value);
                        format!("{}: {}", quote_key(&p.key), value.describe_to(depth - 1))
                    })
                    .collect();
                format!("{{ {} }}", entries.join("; "))
            }
            Value::Array(_) if depth == 0 => "Array".into(),
            Value::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|v| v.describe_to(depth - 1))
                    .collect::<std::vec::Vec<_>>()
                    .join(", ")
            ),
            Value::Module => "(module)".into(),
            _ if self.is_dynamic() => "(not statically analyzable)".into(),
            Value::Reference { name, .. } | Value::Enum { name, .. } => name.clone(),
            Value::Builtin(_) => "Function".into(),
            Value::Dynamic | Value::Function(_) => "(not statically analyzable)".into(),
        }
    }

    /// The chained line ngtsc's `createValueHasWrongTypeError` adds after a message.
    pub(crate) fn wrong_type_suffix(&self) -> String {
        match self {
            Value::Function(_) => " Value could not be determined statically.".into(),
            _ if self.is_dynamic() => " Value could not be determined statically.".into(),
            Value::Reference { name, .. } => format!(" Value is a reference to '{name}'."),
            _ => format!(" Value is of type '{}'.", self.describe()),
        }
    }

    /// JavaScript truthiness. Arrays, objects and references are objects.
    fn truthy(&self) -> bool {
        match self {
            Value::Null | Value::Undefined => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    /// A rough size, to charge for copying the value.
    fn weight(&self) -> u32 {
        match self {
            Value::Array(items) => {
                items.iter().fold(1u32, |sum, item| sum.saturating_add(item.weight()))
            }
            Value::Object(props) => {
                props.iter().fold(1u32, |sum, p| sum.saturating_add(p.value.weight()))
            }
            Value::Enum { value, .. } => value.weight().saturating_add(1),
            Value::Builtin(Builtin::ArraySlice(items) | Builtin::ArrayConcat(items)) => {
                items.iter().fold(1u32, |sum, item| sum.saturating_add(item.weight()))
            }
            _ => 1,
        }
    }
}

fn quote_key(key: &str) -> String {
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        key.to_string()
    } else {
        format!("'{}'", key.replace('\'', "\\'"))
    }
}

/// The `declare var` / `declare function` / `declare namespace` globals of
/// TypeScript's ES2022 library (lib.es5.d.ts ... lib.es2022.*.d.ts), which
/// ngtsc resolves to a reference to their declaration. Other globals (the
/// DOM's, a project's `.d.ts`) depend on the program, and are
/// [`RefKind::Ambient`].
const ES_GLOBALS: &[&str] = &[
    "AggregateError",
    "Array",
    "ArrayBuffer",
    "Atomics",
    "BigInt",
    "BigInt64Array",
    "BigUint64Array",
    "Boolean",
    "DataView",
    "Date",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "Error",
    "escape",
    "eval",
    "EvalError",
    "FinalizationRegistry",
    "Float32Array",
    "Float64Array",
    "Function",
    "Infinity",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Intl",
    "isFinite",
    "isNaN",
    "JSON",
    "Map",
    "Math",
    "NaN",
    "Number",
    "Object",
    "parseFloat",
    "parseInt",
    "Promise",
    "Proxy",
    "RangeError",
    "ReferenceError",
    "Reflect",
    "RegExp",
    "Set",
    "SharedArrayBuffer",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "unescape",
    "URIError",
    "WeakMap",
    "WeakRef",
    "WeakSet",
];

/// The globals in [`ES_GLOBALS`] declared as functions (`declare function`),
/// each with a single signature, and the type of its first parameter as
/// TypeScript's `lib.es5.d.ts` declares it; the others are `declare var`s.
const ES_GLOBAL_FUNCTIONS: &[(&str, &str)] = &[
    ("decodeURI", "string"),
    ("decodeURIComponent", "string"),
    ("encodeURI", "string"),
    ("encodeURIComponent", "string | number | boolean"),
    ("escape", "string"),
    ("eval", "string"),
    ("isFinite", "number"),
    ("isNaN", "number"),
    ("parseFloat", "string"),
    ("parseInt", "string"),
    ("unescape", "string"),
];

/// The first parameter type of [`ES_GLOBAL_FUNCTIONS`]'s `name`.
fn es_global_function_param(name: &str) -> Option<&'static str> {
    ES_GLOBAL_FUNCTIONS.iter().find(|(n, _)| *n == name).map(|(_, ty)| *ty)
}

/// Bounds a chain of import-equals aliases (`import A = B; import B = A;`).
const MAX_ALIASES: u16 = 64;

/// Bounds expression nesting (including calls), to keep deeply nested values
/// off the end of the stack; a debug build uses about 2 KiB per level. Chains of
/// consts don't nest (see [`Evaluator::evaluate_dependencies`]).
const MAX_DEPTH: u16 = 500;

/// Bounds the total work of one evaluator: values that grow exponentially
/// (`const B = [...A, ...A]`, recursive calls) become dynamic instead of hanging.
const FUEL: u32 = 1 << 22;

/// Parameter bindings of the function being evaluated, or the members of the
/// enum whose initializers are being evaluated.
#[derive(Default)]
struct Frame<'a> {
    bindings: HashMap<&'a str, Binding<'a>>,
    /// Evaluating the body of a called function, whose parameters, `this` and
    /// `arguments` don't exist where the metadata is compiled.
    in_call: bool,
    /// The namespace block the code being evaluated is written in, whose
    /// names resolve before the top level's (`None`: the top level).
    block: Option<usize>,
}

impl Frame<'_> {
    /// The frame for code written directly in `block`.
    fn at(block: Option<usize>) -> Self {
        Self { block, ..Self::default() }
    }
}

/// A variable or enum the evaluator caches: the namespace block that declares
/// it (`None`: the top level) and its name.
type Slot<'a> = (Option<usize>, &'a str);

/// A name bound in a [`Frame`].
#[derive(Clone)]
struct Binding<'a> {
    value: Value<'a>,
    /// For a parameter, the argument it was passed, when that can be written
    /// where the metadata is compiled (see [`Evaluator::origin`]).
    origin: Option<&'a Expression<'a>>,
    /// For a rest parameter, where each of its elements was written, like
    /// `origin` (see [`Evaluator::element_origins`]).
    elements: Option<std::vec::Vec<Option<&'a Expression<'a>>>>,
}

impl<'a> Binding<'a> {
    fn new(value: Value<'a>, origin: Option<&'a Expression<'a>>) -> Self {
        Self { value, origin, elements: None }
    }
}

/// A property key or index, as ngtsc's `accessHelper` receives it.
#[derive(Clone, Copy)]
enum Key<'k> {
    Str(&'k str),
    Num(f64),
}

pub(crate) struct Evaluator<'s, 'a> {
    /// The file's declarations are only looked at when an identifier is resolved.
    consts: &'s super::StringConsts<'a>,
    /// Variables and enums already evaluated (with their weight), so a chain
    /// of consts that each reference the previous one several times stays
    /// linear. `None` while one is being evaluated, which makes a circular
    /// reference dynamic (ngtsc overflows its stack on those).
    variables: RefCell<HashMap<Slot<'a>, Option<(Value<'a>, u32)>>>,
    fuel: Cell<u32>,
}

impl<'s, 'a> Evaluator<'s, 'a> {
    pub(crate) fn new(consts: &'s super::StringConsts<'a>) -> Self {
        Self { consts, variables: RefCell::default(), fuel: Cell::new(FUEL) }
    }

    pub(crate) fn evaluate(&self, expr: &'a Expression<'a>) -> Value<'a> {
        self.eval(expr, 0, &Frame::default())
    }

    /// `expr` (evaluated in `frame`) as it can be written where the metadata is
    /// compiled. Outside a called function that's `expr` itself. In one, a
    /// parameter stands for the argument it was passed (ngtsc emits the
    /// identifier the argument's reference was first named by, or the argument
    /// itself); any other expression is kept only if it doesn't use the
    /// function's parameters, `this` or `arguments`, which don't exist there.
    ///
    /// The same goes for an expression written in a namespace: it's kept only
    /// if it doesn't use the namespace's declarations, which aren't in scope
    /// at the top level either.
    fn origin(&self, expr: &'a Expression<'a>, frame: &Frame<'a>) -> Option<&'a Expression<'a>> {
        if !frame.in_call && frame.block.is_none() {
            return Some(expr);
        }
        if let Expression::Identifier(id) = expr
            && let Some(binding) = frame.bindings.get(id.name.as_str())
        {
            return binding.origin;
        }
        // `args[1]`, with `args` a parameter: the element's own origin.
        if let Expression::ComputedMemberExpression(m) = expr
            && let Expression::Identifier(id) = &m.object
            && frame.bindings.contains_key(id.name.as_str())
        {
            return match self.eval(&m.expression, 0, frame) {
                Value::Number(i) if i >= 0.0 && i.fract() == 0.0 => self
                    .element_origins(&m.object, frame)
                    .and_then(|elements| elements.get(i as usize).copied().flatten()),
                _ => None,
            };
        }
        let mut uses = UsesFrame::new(frame, self.consts.scope());
        uses.visit_expression(expr);
        (!uses.found).then_some(expr)
    }

    /// Where each element of the array `expr` (evaluated in `frame`) was
    /// written, as [`Self::origin`] gives it, when that's known: for an array
    /// literal, a top-level variable initialized with one, or a parameter
    /// passed one (a rest parameter gets its arguments'). One entry per value
    /// [`Self::spread`] gives for `expr`.
    fn element_origins(
        &self,
        expr: &'a Expression<'a>,
        frame: &Frame<'a>,
    ) -> Option<std::vec::Vec<Option<&'a Expression<'a>>>> {
        self.element_origins_at(expr, frame, 0)
    }

    fn element_origins_at(
        &self,
        expr: &'a Expression<'a>,
        frame: &Frame<'a>,
        depth: u16,
    ) -> Option<std::vec::Vec<Option<&'a Expression<'a>>>> {
        // Bounded like `eval`: cycles (`const A = [...A]`) and arrays that grow
        // exponentially give up instead of overflowing or hanging.
        if depth > MAX_DEPTH || !self.spend(1) {
            return None;
        }
        let depth = depth + 1;
        match expr {
            Expression::ParenthesizedExpression(e) => {
                self.element_origins_at(&e.expression, frame, depth)
            }
            Expression::TSAsExpression(e) => self.element_origins_at(&e.expression, frame, depth),
            Expression::TSNonNullExpression(e) => {
                self.element_origins_at(&e.expression, frame, depth)
            }
            Expression::ArrayExpression(arr) => {
                let mut origins = std::vec::Vec::new();
                for el in &arr.elements {
                    match el {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            let inner = self.element_origins_at(&spread.argument, frame, depth)?;
                            if !self.spend(inner.len() as u32) {
                                return None;
                            }
                            origins.extend(inner);
                        }
                        ArrayExpressionElement::Elision(_) => origins.push(None),
                        _ => origins.push(self.origin(el.to_expression(), frame)),
                    }
                }
                Some(origins)
            }
            Expression::Identifier(id) => {
                let name = id.name.as_str();
                if let Some(binding) = frame.bindings.get(name) {
                    return match &binding.elements {
                        Some(elements) => Some(elements.clone()),
                        // An origin is written where the metadata is compiled.
                        None => self.element_origins_at(binding.origin?, &Frame::default(), depth),
                    };
                }
                let scope = self.consts.scope();
                // A namespace's declarations are written where they don't
                // resolve at the top level.
                if frame.block.and_then(|from| scope.declaring_block(from, name)).is_some() {
                    return None;
                }
                match scope.variables.get(name)? {
                    // Top-level declarations don't see the caller's parameters.
                    Variable::Init(init, path) if path.is_empty() => {
                        self.element_origins_at(init, &Frame::default(), depth)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Use up `amount` of the fuel; `false` once it has run out.
    fn spend(&self, amount: u32) -> bool {
        let left = self.fuel.get();
        self.fuel.set(left.saturating_sub(amount));
        left >= amount
    }

    /// Evaluates `expr`. Each kind of expression has its own function, so this
    /// one's stack frame (paid on every level of nesting) stays small.
    fn eval(&self, expr: &'a Expression<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        if depth > MAX_DEPTH || !self.spend(1) {
            return Value::Dynamic;
        }
        let depth = depth + 1;
        match expr {
            Expression::NullLiteral(_) => Value::Null,
            Expression::BooleanLiteral(b) => Value::Bool(b.value),
            Expression::NumericLiteral(n) => Value::Number(n.value),
            Expression::StringLiteral(s) => Value::String(s.value.to_string()),
            Expression::TemplateLiteral(tpl) => self.template(tpl, depth, frame),
            Expression::Identifier(id) => self.identifier(id.name.as_str(), depth, frame),
            Expression::ParenthesizedExpression(e) => self.eval(&e.expression, depth, frame),
            Expression::TSAsExpression(e) => self.eval(&e.expression, depth, frame),
            Expression::TSNonNullExpression(e) => self.eval(&e.expression, depth, frame),
            Expression::ArrayExpression(arr) => self.array(arr, depth, frame),
            Expression::ObjectExpression(obj) => self.object(obj, depth, frame),
            Expression::StaticMemberExpression(m) => self.static_member_expr(m, depth, frame),
            Expression::ComputedMemberExpression(m) => self.computed_member_expr(m, depth, frame),
            // `a?.b` and `f?.()` evaluate like `a.b` and `f()`.
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.call(call, depth, frame),
                ChainElement::TSNonNullExpression(e) => self.eval(&e.expression, depth, frame),
                ChainElement::StaticMemberExpression(m) => self.static_member_expr(m, depth, frame),
                ChainElement::ComputedMemberExpression(m) => {
                    self.computed_member_expr(m, depth, frame)
                }
                ChainElement::PrivateFieldExpression(_) => Value::Dynamic,
            },
            Expression::CallExpression(call) => self.call(call, depth, frame),
            Expression::ConditionalExpression(c) => self.conditional(c, depth, frame),
            Expression::UnaryExpression(u) => self.unary_expr(u, depth, frame),
            Expression::BinaryExpression(b) => self.binary_expr(b, depth, frame),
            Expression::LogicalExpression(l) => self.logical_expr(l, depth, frame),
            // Everything else is syntax ngtsc doesn't evaluate: `satisfies`,
            // `<T>x`, functions, `typeof`, `new`, ...
            _ => Value::Dynamic,
        }
    }

    #[inline(never)]
    fn template(&self, tpl: &'a TemplateLiteral<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        let mut out = String::new();
        for (i, quasi) in tpl.quasis.iter().enumerate() {
            let Some(cooked) = &quasi.value.cooked else { return Value::Dynamic };
            out.push_str(cooked);
            if let Some(e) = tpl.expressions.get(i) {
                match literal(self.eval(e, depth, frame)) {
                    Value::Dynamic => return Value::Dynamic,
                    value if value.is_import() => return value,
                    value => out.push_str(&to_js_string(&value)),
                }
            }
        }
        Value::String(out)
    }

    #[inline(never)]
    fn array(&self, arr: &'a ArrayExpression<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        let mut items = std::vec::Vec::new();
        for el in &arr.elements {
            match el {
                ArrayExpressionElement::SpreadElement(spread) => {
                    items.extend(self.spread(&spread.argument, depth, frame));
                }
                ArrayExpressionElement::Elision(_) => items.push(Value::Dynamic),
                _ => items.push(self.eval(el.to_expression(), depth, frame)),
            }
        }
        Value::Array(items)
    }

    #[inline(never)]
    fn object(&self, obj: &'a ObjectExpression<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        let mut props = std::vec::Vec::new();
        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    if p.method || !matches!(p.kind, PropertyKind::Init) {
                        return Value::Dynamic;
                    }
                    let Some(key) = self.property_key(&p.key, p.computed, depth, frame) else {
                        return Value::Dynamic;
                    };
                    let value = match (&p.value, function_value(&p.value)) {
                        (_, Some(function)) => function,
                        // `{ transform }` reads a parameter as it is: ngtsc
                        // looks a shorthand up by its declaration, which
                        // doesn't make a function expression opaque the way
                        // naming it does (see `identifier`).
                        (Expression::Identifier(id), None) if p.shorthand => {
                            let name = id.name.as_str();
                            match frame.bindings.get(name) {
                                Some(binding) => binding.value.clone(),
                                None => match self.slot(frame.block, name) {
                                    Some(slot) => self.stored(slot, depth),
                                    // ngtsc never resolves a shorthand to a global
                                    // it can use as a transform.
                                    None => match self.eval(&p.value, depth, frame) {
                                        value if value.is_dynamic() => Value::Dynamic,
                                        value => value,
                                    },
                                },
                            }
                        }
                        (value, None) => self.eval(value, depth, frame),
                    };
                    let mut expr = &p.value;
                    let mut origin = self.origin(&p.value, frame);
                    // A shorthand for a variable stands for its initializer
                    // (what ngtsc emits, or points its error at).
                    if let Expression::Identifier(id) = &p.value
                        && p.shorthand
                        && !frame.bindings.contains_key(id.name.as_str())
                        && let Some((block, name)) = self.slot(frame.block, id.name.as_str())
                        && let Some(init) =
                            self.consts.scope().declarations(block).initializer(name)
                        && matches!(value, Value::Function(_) | Value::Dynamic)
                    {
                        expr = init;
                        origin = self.origin(init, &Frame::at(block));
                    }
                    props.push(Prop { key, value, expr: Some(expr), origin });
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    match self.eval(&spread.argument, depth, frame) {
                        Value::Object(inner) if self.spend(inner.len() as u32) => {
                            props.extend(inner);
                        }
                        value if value.is_import() => return value,
                        _ => return Value::Dynamic,
                    }
                }
            }
        }
        Value::Object(props)
    }

    #[inline(never)]
    fn static_member_expr(
        &self,
        m: &'a StaticMemberExpression<'a>,
        depth: u16,
        frame: &Frame<'a>,
    ) -> Value<'a> {
        let object = self.eval(&m.object, depth, frame);
        self.member(object, Key::Str(m.property.name.as_str()), depth)
    }

    #[inline(never)]
    fn computed_member_expr(
        &self,
        m: &'a ComputedMemberExpression<'a>,
        depth: u16,
        frame: &Frame<'a>,
    ) -> Value<'a> {
        let object = self.eval(&m.object, depth, frame);
        if object.is_dynamic() {
            return Value::Dynamic;
        }
        match self.eval(&m.expression, depth, frame) {
            Value::String(key) => self.member(object, Key::Str(&key), depth),
            Value::Number(n) => self.member(object, Key::Num(n), depth),
            key if key.is_import() => key,
            _ => Value::Dynamic,
        }
    }

    #[inline(never)]
    fn conditional(
        &self,
        c: &'a ConditionalExpression<'a>,
        depth: u16,
        frame: &Frame<'a>,
    ) -> Value<'a> {
        match self.eval(&c.test, depth, frame) {
            test if test.is_dynamic() => Value::Dynamic,
            test if test.is_import() => test,
            test if test.truthy() => self.eval(&c.consequent, depth, frame),
            _ => self.eval(&c.alternate, depth, frame),
        }
    }

    #[inline(never)]
    fn unary_expr(&self, u: &'a UnaryExpression<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        if !matches!(
            u.operator,
            UnaryOperator::UnaryNegation
                | UnaryOperator::UnaryPlus
                | UnaryOperator::LogicalNot
                | UnaryOperator::BitwiseNot
        ) {
            return Value::Dynamic;
        }
        match self.eval(&u.argument, depth, frame) {
            value if value.is_dynamic() => Value::Dynamic,
            value if value.is_import() => value,
            value => unary(u.operator, &value),
        }
    }

    #[inline(never)]
    fn binary_expr(&self, b: &'a BinaryExpression<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        if matches!(b.operator, BinaryOperator::In | BinaryOperator::Instanceof) {
            return Value::Dynamic;
        }
        // Operands must be primitives (an enum member counts as its value).
        let left = literal(self.eval(&b.left, depth, frame));
        let right = literal(self.eval(&b.right, depth, frame));
        match (left, right) {
            (Value::Dynamic, _) | (_, Value::Dynamic) => Value::Dynamic,
            (value, _) | (_, value) if value.is_import() => value,
            (left, right) => binary(b.operator, &left, &right),
        }
    }

    #[inline(never)]
    fn logical_expr(
        &self,
        l: &'a LogicalExpression<'a>,
        depth: u16,
        frame: &Frame<'a>,
    ) -> Value<'a> {
        // `??` isn't one of ngtsc's operators.
        if l.operator == LogicalOperator::Coalesce {
            return Value::Dynamic;
        }
        // ngtsc evaluates both operands, and either being dynamic makes the
        // result dynamic. An import only matters when it's the left operand
        // (which picks the result) or the operand picked: `'a' || NAME` is
        // `'a'` whatever `NAME` is.
        let left = self.eval(&l.left, depth, frame);
        let right = self.eval(&l.right, depth, frame);
        match (left, right) {
            (left, right) if left.is_dynamic() || right.is_dynamic() => Value::Dynamic,
            (left, _) if left.is_import() => left,
            (left, right) => match (l.operator, left.truthy()) {
                (LogicalOperator::And, true) | (LogicalOperator::Or, false) => right,
                _ => left,
            },
        }
    }

    /// The elements `...expr` adds to an array or an argument list.
    fn spread(
        &self,
        expr: &'a Expression<'a>,
        depth: u16,
        frame: &Frame<'a>,
    ) -> std::vec::Vec<Value<'a>> {
        match self.eval(expr, depth, frame) {
            Value::Array(inner) if self.spend(inner.len() as u32) => inner,
            value if value.is_import() => vec![value],
            // ngtsc marks only this element as dynamic, not the whole array.
            _ => vec![Value::Dynamic],
        }
    }

    fn property_key(
        &self,
        key: &'a PropertyKey<'a>,
        computed: bool,
        depth: u16,
        frame: &Frame<'a>,
    ) -> Option<String> {
        if computed {
            // A computed key has to evaluate to a string (`{ [1]: x }` doesn't).
            return match self.eval(key.to_expression(), depth, frame) {
                Value::String(s) => Some(s),
                _ => None,
            };
        }
        match key {
            PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
            PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
            PropertyKey::NumericLiteral(n) => Some(format_number_like_js(n.value)),
            _ => None,
        }
    }

    fn identifier(&self, name: &'a str, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        if let Some(binding) = frame.bindings.get(name) {
            return match &binding.value {
                // A function expression passed as an argument isn't
                // analyzable through the parameter's name (ngtsc wraps it in
                // a dynamic value for that identifier).
                Value::Function(_) => Value::Dynamic,
                value => value.clone(),
            };
        }
        let scope = self.consts.scope();
        // Inside a namespace, its declarations (and its enclosing ones') come first.
        if let Some(block) = frame.block.and_then(|from| scope.declaring_block(from, name)) {
            return self.declared(Some(block), name, depth).unwrap_or(Value::Dynamic);
        }
        if let Some(value) = self.declared(None, name, depth) {
            return value;
        }
        if let Some(import) = scope.imports.get(name) {
            return match import.imported {
                Some(imported) => {
                    let name = if imported == "default" { name } else { imported };
                    Value::Reference {
                        name: name.into(),
                        kind: RefKind::Import { namespace_member: false },
                    }
                }
                None => Value::Module,
            };
        }
        match name {
            "undefined" => Value::Undefined,
            _ if ES_GLOBALS.contains(&name) => {
                Value::Reference { name: name.into(), kind: RefKind::Global }
            }
            // TypeScript declares `globalThis` without a declaration ngtsc
            // can find.
            "globalThis" => Value::Dynamic,
            _ if scope.unread.contains(name) => Value::Dynamic,
            _ => Value::Reference { name: name.into(), kind: RefKind::Ambient },
        }
    }

    /// The value of `name` when `block` (`None`: the top level) declares it
    /// as a variable, enum, function, class or namespace. A namespace is a
    /// reference to it, and an `import x = ...` alias in a namespace is dynamic.
    fn declared(&self, block: Option<usize>, name: &'a str, depth: u16) -> Option<Value<'a>> {
        let file = self.consts.scope();
        let scope = file.declarations(block);
        if scope.variables.contains_key(name) || scope.enums.contains_key(name) {
            return Some(match self.stored((block, name), depth) {
                // A function expression isn't analyzable through the
                // variable's name (ngtsc wraps it in a dynamic value for that
                // identifier).
                Value::Function(_) => Value::Dynamic,
                value => value,
            });
        }
        if let Some((function, overloads)) = scope.functions.get(name) {
            let kind = RefKind::Function(function, *overloads);
            return Some(Value::Reference { name: name.into(), kind });
        }
        if let Some(class) = scope.classes.get(name) {
            return Some(Value::Reference { name: name.into(), kind: RefKind::Class(class) });
        }
        if let Some(reference) = scope.aliases.get(name) {
            return Some(self.alias(block, reference, depth));
        }
        let namespace = match block {
            Some(b) => file.blocks[b]
                .namespaces
                .get(name)
                .or_else(|| file.member_namespaces.get(&(file.blocks[b].namespace, name))),
            None => file.namespace_ids.get(name),
        };
        namespace
            .filter(|id| file.instantiated.contains(id))
            .map(|id| file.namespace_reference(name, *id))
    }

    /// The value of an import-equals alias declared in `block`: what its
    /// entity names (`import f = NS.f` is `NS.f`), resolved where the alias
    /// is written. Another module's (`import m = require('m')`) is dynamic.
    fn alias(
        &self,
        block: Option<usize>,
        reference: &'a TSModuleReference<'a>,
        depth: u16,
    ) -> Value<'a> {
        if depth > MAX_DEPTH || !self.spend(1) {
            return Value::Dynamic;
        }
        let depth = depth + 1;
        let scope = self.consts.scope();
        let parts = match reference {
            // TypeScript resolves `import A = B` only as a namespace.
            TSModuleReference::IdentifierReference(id) => {
                return match scope.namespace_of(block, id.name.as_str(), 0) {
                    Some(ns) if scope.instantiated.contains(&ns) => {
                        scope.namespace_reference(scope.namespace_names[ns], ns)
                    }
                    _ => Value::Dynamic,
                };
            }
            TSModuleReference::QualifiedName(q) => qualified_parts(q),
            TSModuleReference::ExternalModuleReference(_) => None,
        };
        match parts.and_then(|parts| scope.qualified(block, &parts)) {
            Some(Qualified::Declared(b, member)) => {
                self.declared(Some(b), member, depth).unwrap_or(Value::Dynamic)
            }
            Some(Qualified::Namespace(name, id)) => scope.namespace_reference(name, id),
            _ => Value::Dynamic,
        }
    }

    /// A variable's or enum's value, evaluated once.
    fn stored(&self, slot: Slot<'a>, depth: u16) -> Value<'a> {
        if let Some(cached) = self.variables.borrow().get(&slot) {
            return match cached {
                Some((value, weight)) if self.spend(*weight) => value.clone(),
                _ => Value::Dynamic,
            };
        }
        self.evaluate_dependencies(slot, depth);
        self.evaluate_top_level(slot, depth)
    }

    /// Where the variable or enum `name`, mentioned in code written in
    /// `block`, is declared, if it's one.
    fn slot(&self, block: Option<usize>, name: &'a str) -> Option<Slot<'a>> {
        let scope = self.consts.scope();
        let block = block.and_then(|from| scope.declaring_block(from, name));
        let declarations = scope.declarations(block);
        (declarations.variables.contains_key(name) || declarations.enums.contains_key(name))
            .then_some((block, name))
    }

    /// Evaluates the top-level bindings `root`'s initializer mentions (and theirs)
    /// before `root`, deepest first, so a long chain of consts
    /// (`const X2 = [...X1]`) is evaluated one link at a time instead of
    /// recursively through the whole chain.
    fn evaluate_dependencies(&self, root: Slot<'a>, depth: u16) {
        let scope = self.consts.scope();
        let pending = |slot: &Slot<'a>| !self.variables.borrow().contains_key(slot);
        let mut seen = HashSet::from([root]);
        let mut stack = vec![(root, false)];
        while let Some((slot, ready)) = stack.pop() {
            if ready {
                if slot != root && pending(&slot) {
                    self.evaluate_top_level(slot, depth);
                }
                continue;
            }
            stack.push((slot, true));
            let mut mentions = Mentions(std::vec::Vec::new());
            let declarations = scope.declarations(slot.0);
            match (declarations.variables.get(slot.1), declarations.enums.get(slot.1)) {
                (Some(Variable::Init(init, _)), _) => mentions.visit_expression(init),
                (_, Some(e)) => mentions.visit_ts_enum_declaration(e),
                _ => {}
            }
            for dep in mentions.0 {
                if let Some(dep) = self.slot(slot.0, dep)
                    && pending(&dep)
                    && seen.insert(dep)
                {
                    stack.push((dep, false));
                }
            }
        }
    }

    fn evaluate_top_level(&self, slot: Slot<'a>, depth: u16) -> Value<'a> {
        let scope = self.consts.scope().declarations(slot.0);
        self.variables.borrow_mut().insert(slot, None);
        let value = match (scope.variables.get(slot.1), scope.enums.get(slot.1)) {
            (Some(variable), _) => self.variable(slot, variable, depth),
            (_, Some(e)) => self.enumeration(e, depth, slot.0),
            _ => Value::Dynamic,
        };
        let weight = value.weight();
        self.variables.borrow_mut().insert(slot, Some((value.clone(), weight)));
        value
    }

    fn variable(&self, slot: Slot<'a>, variable: &Variable<'a>, depth: u16) -> Value<'a> {
        let (block, name) = slot;
        match variable {
            // Declarations don't see the caller's parameters.
            // A function expression initializer stays analyzable (ngtsc visits
            // it as is); naming the variable makes it opaque (see
            // `identifier`), but a shorthand property doesn't (see `object`).
            Variable::Init(init, path) => {
                let scope = self.consts.scope().declarations(block);
                if let Some(function) = scope.initializer(name).and_then(function_value) {
                    return function;
                }
                let mut value = self.eval(init, depth, &Frame::at(block));
                for key in path {
                    value = match key {
                        PathKey::Index(i) => self.member(value, Key::Num(*i as f64), depth),
                        // Destructuring reads a property as it is, so a
                        // function expression there stays analyzable too.
                        PathKey::Key(k) => match value {
                            Value::Object(props) => props
                                .into_iter()
                                .rev()
                                .find(|p| p.key == *k)
                                .map_or(Value::Undefined, |p| p.value),
                            value => self.member(value, Key::Str(k), depth),
                        },
                        PathKey::Unknown => Value::Dynamic,
                    };
                    if matches!(value, Value::Dynamic) {
                        break;
                    }
                }
                value
            }
            // A literal type is its value; otherwise it's a reference to the variable.
            Variable::Declared(ty) => match ty.map(|ty| self.eval_type(ty, depth, block)) {
                Some(value) if !matches!(value, Value::Dynamic) => value,
                _ => Value::Reference { name: name.into(), kind: RefKind::Other },
            },
            Variable::Uninitialized => Value::Undefined,
        }
    }

    /// ngtsc's `visitEnumDeclaration`: a map of members to enum values. A member
    /// without an initializer is its index (not the previous member plus one).
    fn enumeration(
        &self,
        e: &'a TSEnumDeclaration<'a>,
        depth: u16,
        block: Option<usize>,
    ) -> Value<'a> {
        let name_of = |member: &'a TSEnumMemberName<'a>| match member {
            TSEnumMemberName::Identifier(id) => Some(id.name.as_str()),
            TSEnumMemberName::String(s) | TSEnumMemberName::ComputedString(s) => {
                Some(s.value.as_str())
            }
            TSEnumMemberName::ComputedTemplateString(t) => {
                t.quasis.first().and_then(|q| q.value.cooked.as_ref()).map(oxc_str::Str::as_str)
            }
        };
        // Inside the initializers, a member's name refers to the member.
        let bindings = e
            .body
            .members
            .iter()
            .filter_map(|m| name_of(&m.id))
            .map(|n| {
                let value = Value::Reference { name: n.into(), kind: RefKind::Other };
                (n, Binding::new(value, None))
            })
            .collect();
        let frame = Frame { bindings, in_call: false, block };
        let props = e
            .body
            .members
            .iter()
            .enumerate()
            .filter_map(|(index, member)| {
                let key = name_of(&member.id)?;
                let value = match &member.initializer {
                    Some(init) => self.eval(init, depth, &frame),
                    None => Value::Number(index as f64),
                };
                let value = Value::Enum { name: e.id.name.to_string(), value: Box::new(value) };
                Some(Prop { key: key.to_string(), value, expr: None, origin: None })
            })
            .collect();
        Value::Object(props)
    }

    /// ngtsc's `visitType`, for `declare const X: T` written in `block`
    /// (`None`: at the top level).
    fn eval_type(&self, ty: &'a TSType<'a>, depth: u16, block: Option<usize>) -> Value<'a> {
        if depth > MAX_DEPTH || !self.spend(1) {
            return Value::Dynamic;
        }
        let depth = depth + 1;
        match ty {
            TSType::TSNullKeyword(_) => Value::Null,
            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::BooleanLiteral(b) => Value::Bool(b.value),
                TSLiteral::NumericLiteral(n) => Value::Number(n.value),
                TSLiteral::StringLiteral(s) => Value::String(s.value.to_string()),
                TSLiteral::TemplateLiteral(t) if t.expressions.is_empty() => t
                    .quasis
                    .first()
                    .and_then(|q| q.value.cooked.as_ref())
                    .map_or(Value::Dynamic, |c| Value::String(c.to_string())),
                TSLiteral::UnaryExpression(u) => {
                    match self.eval(&u.argument, depth, &Frame::at(block)) {
                        Value::Dynamic => Value::Dynamic,
                        value => unary(u.operator, &value),
                    }
                }
                _ => Value::Dynamic,
            },
            TSType::TSTupleType(tuple) => Value::Array(
                tuple
                    .element_types
                    .iter()
                    .map(|el| self.eval_tuple_element(el, depth, block))
                    .collect(),
            ),
            TSType::TSNamedTupleMember(member) => {
                self.eval_tuple_element(&member.element_type, depth, block)
            }
            TSType::TSTypeOperatorType(op) if op.operator == TSTypeOperatorOperator::Readonly => {
                self.eval_type(&op.type_annotation, depth, block)
            }
            TSType::TSTypeQuery(query) => match &query.expr_name {
                TSTypeQueryExprName::IdentifierReference(id) => {
                    self.identifier(id.name.as_str(), depth, &Frame::at(block))
                }
                TSTypeQueryExprName::QualifiedName(name) => {
                    // `typeof NS.X`, for a namespace `NS` the file declares,
                    // is the value `X` the namespace exports.
                    let scope = self.consts.scope();
                    match qualified_parts(name).and_then(|parts| scope.qualified(block, &parts)) {
                        Some(Qualified::Declared(b, member)) => {
                            return self.declared(Some(b), member, depth).unwrap_or(Value::Dynamic);
                        }
                        Some(Qualified::Namespace(name, id)) => {
                            return scope.namespace_reference(name, id);
                        }
                        Some(Qualified::Missing) => return Value::Dynamic,
                        None => {}
                    }
                    // Otherwise, `typeof E.A` is a reference to the declaration of `A`.
                    let mut left = &name.left;
                    while let TSTypeName::QualifiedName(q) = left {
                        left = &q.left;
                    }
                    let imported = matches!(left, TSTypeName::IdentifierReference(id)
                        if self.consts.scope().imports.contains_key(id.name.as_str()));
                    let kind = if imported {
                        RefKind::Import { namespace_member: false }
                    } else {
                        RefKind::Other
                    };
                    Value::Reference { name: name.right.name.to_string(), kind }
                }
                _ => Value::Dynamic,
            },
            // A type naming a class is a reference to it.
            TSType::TSTypeReference(reference) => {
                let scope = self.consts.scope();
                let (block, name) = match &reference.type_name {
                    TSTypeName::IdentifierReference(id) => {
                        let name = id.name.as_str();
                        (block.and_then(|from| scope.declaring_block(from, name)), name)
                    }
                    TSTypeName::QualifiedName(q) => {
                        match qualified_parts_of(&reference.type_name)
                            .and_then(|parts| scope.qualified(block, &parts))
                        {
                            Some(Qualified::Declared(b, name)) => (Some(b), name),
                            Some(_) => return Value::Dynamic,
                            None => (None, q.right.name.as_str()),
                        }
                    }
                    TSTypeName::ThisExpression(_) => return Value::Dynamic,
                };
                match scope.declarations(block).classes.get(name) {
                    Some(class) => {
                        Value::Reference { name: name.into(), kind: RefKind::Class(class) }
                    }
                    None => Value::Dynamic,
                }
            }
            _ => Value::Dynamic,
        }
    }

    fn eval_tuple_element(
        &self,
        element: &'a TSTupleElement<'a>,
        depth: u16,
        block: Option<usize>,
    ) -> Value<'a> {
        element.as_ts_type().map_or(Value::Dynamic, |ty| self.eval_type(ty, depth, block))
    }

    /// ngtsc's `visitCallExpression`: builtins, and same-file functions whose
    /// body is a single `return`.
    fn call(&self, call: &'a CallExpression<'a>, depth: u16, frame: &Frame<'a>) -> Value<'a> {
        let (function, overloads) = match self.eval(&call.callee, depth, frame) {
            Value::Builtin(builtin) => {
                let args = self.arguments(call, depth, frame);
                return self.call_builtin(builtin, args);
            }
            Value::Reference { kind: RefKind::Function(function, overloads), .. } => {
                (function, overloads)
            }
            // An imported function runs in another file.
            callee if callee.is_import() => return callee,
            _ => return Value::Dynamic,
        };
        // A body-less first declaration (an overload, `declare function`) can't be evaluated.
        let Some(body) = function.body.as_ref().filter(|_| overloads == 0) else {
            return Value::Dynamic;
        };
        let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
            return Value::Dynamic;
        };
        if !body.directives.is_empty() {
            return Value::Dynamic;
        }
        // Each argument's value, and where it was written.
        let mut args = std::vec::Vec::new();
        let mut origins = std::vec::Vec::new();
        // A function expression passed as an argument (or as a default value)
        // stays analyzable in the parameter it's bound to, as ngtsc binds the
        // value as it is; but not after a spread, or in a rest parameter.
        let mut functions = std::vec::Vec::new();
        let mut spread_seen = false;
        for arg in &call.arguments {
            if let Argument::SpreadElement(spread) = arg {
                spread_seen = true;
                let values = self.spread(&spread.argument, depth, frame);
                let known = self
                    .element_origins(&spread.argument, frame)
                    .filter(|known| known.len() == values.len());
                origins.extend(known.unwrap_or_else(|| vec![None; values.len()]));
                args.extend(values);
            } else {
                let expr = arg.to_expression();
                if !spread_seen {
                    functions.push(function_value(expr));
                }
                args.push(self.eval(expr, depth, frame));
                origins.push(self.origin(expr, frame));
            }
        }
        // ngtsc counts a `this` parameter as the first one.
        let offset = usize::from(function.this_param.is_some());
        // Its body sees the declarations where it's written.
        let block = self.consts.scope().block_of(function);
        let mut scope = Frame { bindings: HashMap::new(), in_call: true, block };
        for (i, param) in function.params.items.iter().enumerate() {
            let binding = match args.get(i + offset) {
                None | Some(Value::Undefined) if param.initializer.is_some() => {
                    param.initializer.as_ref().map(|init| {
                        let value =
                            function_value(init).unwrap_or_else(|| self.eval(init, depth, &scope));
                        Binding::new(value, self.origin(init, &scope))
                    })
                }
                arg => arg.map(|arg| {
                    let function = functions.get(i + offset).cloned().flatten();
                    Binding::new(
                        function.unwrap_or_else(|| arg.clone()),
                        origins.get(i + offset).copied().flatten(),
                    )
                }),
            };
            let binding = binding.unwrap_or(Binding::new(Value::Undefined, None));
            bind(&mut scope, &param.pattern, binding);
        }
        if let Some(rest) = &function.params.rest {
            let start = function.params.items.len() + offset;
            let rest_args = args.get(start..).map_or_else(std::vec::Vec::new, <[_]>::to_vec);
            let elements = origins.get(start..).map_or_else(std::vec::Vec::new, <[_]>::to_vec);
            let binding =
                Binding { value: Value::Array(rest_args), origin: None, elements: Some(elements) };
            bind(&mut scope, &rest.rest.argument, binding);
        }
        ret.argument.as_ref().map_or(Value::Undefined, |e| self.eval(e, depth, &scope))
    }

    fn arguments(
        &self,
        call: &'a CallExpression<'a>,
        depth: u16,
        frame: &Frame<'a>,
    ) -> std::vec::Vec<Value<'a>> {
        let mut args = std::vec::Vec::new();
        for arg in &call.arguments {
            match arg {
                Argument::SpreadElement(spread) => {
                    args.extend(self.spread(&spread.argument, depth, frame));
                }
                _ => args.push(self.eval(arg.to_expression(), depth, frame)),
            }
        }
        args
    }

    fn call_builtin(&self, builtin: Builtin<'a>, args: std::vec::Vec<Value<'a>>) -> Value<'a> {
        match builtin {
            Builtin::ArraySlice(items) if args.is_empty() => Value::Array(items),
            Builtin::ArraySlice(_) => Value::Dynamic,
            Builtin::ArrayConcat(mut items) => {
                for arg in args {
                    match arg {
                        Value::Array(inner) if self.spend(inner.len() as u32) => {
                            items.extend(inner);
                        }
                        Value::Array(_) => return Value::Dynamic,
                        arg => items.push(arg),
                    }
                }
                Value::Array(items)
            }
            Builtin::StringConcat(mut s) => {
                for arg in args {
                    let arg = match arg {
                        Value::Enum { value, .. } => *value,
                        arg => arg,
                    };
                    match arg {
                        Value::Null
                        | Value::Undefined
                        | Value::Bool(_)
                        | Value::Number(_)
                        | Value::String(_) => s.push_str(&to_js_string(&arg)),
                        arg if arg.is_import() => return arg,
                        _ => return Value::Dynamic,
                    }
                }
                Value::String(s)
            }
        }
    }

    /// ngtsc's `accessHelper`: `object.key` / `object[key]`.
    fn member(&self, object: Value<'a>, key: Key<'_>, depth: u16) -> Value<'a> {
        let key_str = || match key {
            Key::Str(s) => s.to_string(),
            Key::Num(n) => format_number_like_js(n),
        };
        match object {
            // A function expression reached through a property access isn't
            // analyzable (`({ f: (v: string) => 1 }).f`).
            Value::Object(props) => {
                let key = key_str();
                match props.into_iter().rev().find(|p| p.key == key) {
                    Some(Prop { value: Value::Function(_), .. }) => Value::Dynamic,
                    Some(p) => p.value,
                    None => Value::Undefined,
                }
            }
            Value::Array(items) => match key {
                Key::Str("length") => Value::Number(items.len() as f64),
                Key::Str("slice") => Value::Builtin(Builtin::ArraySlice(items)),
                Key::Str("concat") => Value::Builtin(Builtin::ArrayConcat(items)),
                // Only an integer indexes an array; `X['0']` doesn't.
                Key::Num(n) if n.fract() == 0.0 => {
                    if n >= 0.0 && n < items.len() as f64 {
                        items.into_iter().nth(n as usize).unwrap_or(Value::Undefined)
                    } else {
                        Value::Undefined
                    }
                }
                _ => Value::Dynamic,
            },
            Value::String(s) if matches!(key, Key::Str("concat")) => {
                Value::Builtin(Builtin::StringConcat(s))
            }
            Value::Module => Value::Reference {
                name: key_str(),
                kind: RefKind::Import { namespace_member: true },
            },
            Value::Reference { kind: RefKind::Class(class), .. } => {
                self.static_member(class, &key_str(), depth)
            }
            // The object is in another file, and so is its member.
            object @ Value::Reference { kind: RefKind::Import { .. }, .. } => object,
            // Including a member of a global (`Math.round`): ngtsc only
            // resolves the global's own declaration.
            _ => Value::Dynamic,
        }
    }

    fn static_member(&self, class: &'a Class<'a>, key: &str, depth: u16) -> Value<'a> {
        let mut overloads = 0;
        let mut method = None;
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(m)
                    if m.r#static && m.key.static_name().is_some_and(|n| n == key) =>
                {
                    if m.kind != MethodDefinitionKind::Method {
                        return Value::Reference {
                            name: key.into(),
                            kind: RefKind::StaticMember(m.span),
                        };
                    }
                    if m.value.body.is_none() {
                        overloads += 1;
                    }
                    if m.value.body.is_some() || method.is_none() {
                        method = Some(&*m.value);
                    }
                }
                ClassElement::PropertyDefinition(p)
                    if method.is_none()
                        && p.r#static
                        && p.key.static_name().is_some_and(|n| n == key) =>
                {
                    return match &p.value {
                        Some(value) => {
                            let block = self.consts.scope().block_of(class);
                            self.eval(value, depth, &Frame::at(block))
                        }
                        None => Value::Reference {
                            name: key.into(),
                            kind: RefKind::StaticMember(p.span),
                        },
                    };
                }
                _ => {}
            }
        }
        match method {
            Some(method) => {
                Value::Reference { name: key.into(), kind: RefKind::Function(method, overloads) }
            }
            None => Value::Undefined,
        }
    }
}

// =============================================================================
// Input transforms
// =============================================================================

/// ngtsc's checks on an input `transform`
/// (`parseDecoratorInputTransformFunction` in
/// packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts), with
/// the node each error points at.
///
/// `position` is the index in the `inputs:` array, `None` for
/// `@Input({ transform })`; `container` is the `inputs:` value or the `@Input`
/// argument, where ngtsc reports a transform that isn't even a reference.
pub(crate) fn transform_error<'a>(
    transform: &Prop<'a>,
    position: Option<usize>,
    input_name: &str,
    class: &Class<'a>,
    scope: &FileScope<'a>,
    container: Span,
) -> Option<(String, Span)> {
    let value = &transform.value;
    let suffix = value.wrong_type_suffix();
    let expr_span = transform.expr.map_or(container, GetSpan::span);
    // ngtsc's `value.node`: where the reference is declared, or the expression
    // that isn't analyzable. For a declaration in another file, the nearest
    // thing in this one is the expression.
    let node = || value_node_span(value, expr_span, scope);
    let conflicting = format!("ngAcceptInputType_{input_name}");
    let clash = || {
        class
            .body
            .body
            .iter()
            .any(|el| {
                el.r#static()
                    && el
                        .property_key()
                        .and_then(PropertyKey::static_name)
                        .is_some_and(|n| n == conflicting)
            })
            .then(|| {
                (
                    format!(
                        "Class cannot have both a transform function on Input {input_name} and a static member called {conflicting}"
                    ),
                    node(),
                )
            })
    };
    let def = match value {
        Value::Function(def) => *def,
        Value::Reference { name, kind: RefKind::Function(function, _) } => {
            FnDef::Function(scope.first_declaration(name, function))
        }
        // An imported function can't be inspected from this file, so whether
        // it's generic or overloaded is unknown; the name clash is checked
        // after those.
        Value::Reference { kind: RefKind::Import { namespace_member: false }, .. } => {
            return clash();
        }
        // ngtsc can't name `ns.f` in the compiled file.
        Value::Reference { kind: RefKind::Import { namespace_member: true }, .. } => {
            return clash().or_else(|| {
                Some((format!("Input transform function could not be referenced{suffix}"), node()))
            });
        }
        // A `declare function` of TypeScript's library: neither generic nor
        // overloaded.
        Value::Reference { name, kind: RefKind::Global }
            if es_global_function_param(name).is_some() =>
        {
            return clash();
        }
        // A global declared outside the file (`atob`) is assumed to be a
        // function, like an imported one.
        Value::Reference { kind: RefKind::Ambient, .. } => return clash(),
        Value::Reference { .. } | Value::Dynamic => {
            return Some((format!("Input transform must be a function{suffix}"), node()));
        }
        _ => {
            let message = match position {
                Some(i) => format!(
                    "Transform of value at position {i} of @Directive.inputs array must be a function{suffix}"
                ),
                None => format!("Input transform must be a function{suffix}"),
            };
            return Some((message, container));
        }
    };
    if def.is_generic() {
        return Some((format!("Input transform function cannot be generic{suffix}"), node()));
    }
    if let Value::Reference { kind: RefKind::Function(_, overloads), .. } = value
        && (*overloads).max(1) > 1
    {
        return Some((
            format!("Input transform function cannot have multiple signatures{suffix}"),
            node(),
        ));
    }
    if let Some(error) = clash() {
        return Some(error);
    }
    match def.first_param_type() {
        Ok(None) => None,
        Err(()) => Some((
            format!("Input transform function first parameter must have a type{suffix}"),
            node(),
        )),
        Ok(Some(_)) if def.params().items.is_empty() => Some((
            format!(
                "Input transform function first parameter cannot be a spread parameter{suffix}"
            ),
            node(),
        )),
        Ok(Some(ty)) => {
            // An arrow written in a member's `@Input` sees the class's type
            // parameters.
            let in_class = matches!(value, Value::Function(_))
                && class.body.span.contains_inclusive(def.span());
            let class_params = class.type_parameters.as_deref().filter(|_| in_class);
            // One written in a function the metadata calls sees that
            // function's type parameters.
            let helper_params = match value {
                Value::Function(_) => scope.enclosing_type_parameters(def.span()),
                _ => None,
            };
            // Its types resolve where it's written, in a namespace or not.
            let block = scope.block_at(def.span());
            let mut check =
                UnexportedType { scope, block, type_params: std::vec::Vec::new(), found: false };
            check.with_params(helper_params, |check| {
                check.with_params(class_params, |check| check.visit_ts_type(ty));
            });
            check.found.then(|| {
                (
                    "Symbol must be exported in order to be used as the type of an Input transform function"
                        .to_string(),
                    ty.span(),
                )
            })
        }
    }
}

/// The node of ngtsc's `value.node` for a transform: a function expression,
/// the declaration a reference resolves to, or else the expression written.
fn value_node_span(value: &Value<'_>, expr_span: Span, scope: &FileScope<'_>) -> Span {
    let declared = |name: &str| scope.declaration_spans.get(name).copied();
    let span = match value {
        Value::Function(def) => Some(def.span()),
        // Declared at the top level or in a namespace.
        Value::Reference { name, kind: RefKind::Function(function, _) } => {
            let scope = scope.scope_of(*function);
            if scope.is_top_level_function(name, function) {
                scope.declaration_spans.get(name.as_str()).copied()
            } else {
                scope.first_static_method(function).map(|m| m.span)
            }
        }
        Value::Reference { kind: RefKind::Class(class), .. } => class
            .id
            .as_ref()
            .and_then(|id| scope.scope_of(*class).declaration_spans.get(id.name.as_str()).copied()),
        Value::Reference {
            kind: RefKind::StaticMember(span) | RefKind::Namespace(span), ..
        } => Some(*span),
        Value::Reference { name, kind: RefKind::Other }
            if matches!(scope.variables.get(name.as_str()), Some(Variable::Declared(_))) =>
        {
            declared(name)
        }
        _ => None,
    };
    span.unwrap_or(expr_span)
}

/// Finds what ngtsc's `assertEmittableInputType` rejects in a transform's
/// parameter type: a type reference whose name is a plain identifier that
/// resolves to a declaration in this file that isn't exported, which can't be
/// written into the `.d.ts`. A type parameter never is. `typeof X` and the
/// left side of `X.Y` aren't type references, so they're never checked.
struct UnexportedType<'s, 'a> {
    scope: &'s FileScope<'a>,
    /// The namespace block the function is written in (`None`: the top level).
    block: Option<usize>,
    /// The type parameters in scope, innermost last.
    type_params: std::vec::Vec<&'a str>,
    found: bool,
}

impl<'a> UnexportedType<'_, 'a> {
    /// Visits `walk` with `params` in scope.
    fn with_params(
        &mut self,
        params: Option<&TSTypeParameterDeclaration<'a>>,
        walk: impl FnOnce(&mut Self),
    ) {
        let len = self.type_params.len();
        self.type_params
            .extend(params.into_iter().flat_map(|d| &d.params).map(|p| p.name.name.as_str()));
        walk(self);
        self.type_params.truncate(len);
    }
}

impl<'a> Visit<'a> for UnexportedType<'_, 'a> {
    fn visit_ts_type_reference(&mut self, reference: &TSTypeReference<'a>) {
        if let TSTypeName::IdentifierReference(id) = &reference.type_name {
            let name = id.name.as_str();
            // Innermost first: a type parameter shadows a declaration.
            let exported = if self.type_params.contains(&name) {
                Some(false)
            } else {
                self.scope.type_is_exported_in(self.block, name)
            };
            self.found |= exported == Some(false);
        }
        if let Some(args) = &reference.type_arguments {
            self.visit_ts_type_parameter_instantiation(args);
        }
    }

    fn visit_ts_mapped_type(&mut self, ty: &TSMappedType<'a>) {
        let len = self.type_params.len();
        self.type_params.push(ty.key.name.as_str());
        walk::walk_ts_mapped_type(self, ty);
        self.type_params.truncate(len);
    }

    fn visit_ts_function_type(&mut self, ty: &TSFunctionType<'a>) {
        self.with_params(ty.type_parameters.as_deref(), |v| walk::walk_ts_function_type(v, ty));
    }

    fn visit_ts_constructor_type(&mut self, ty: &TSConstructorType<'a>) {
        self.with_params(ty.type_parameters.as_deref(), |v| walk::walk_ts_constructor_type(v, ty));
    }

    fn visit_ts_method_signature(&mut self, sig: &TSMethodSignature<'a>) {
        self.with_params(sig.type_parameters.as_deref(), |v| {
            walk::walk_ts_method_signature(v, sig)
        });
    }

    fn visit_ts_call_signature_declaration(&mut self, sig: &TSCallSignatureDeclaration<'a>) {
        self.with_params(sig.type_parameters.as_deref(), |v| {
            walk::walk_ts_call_signature_declaration(v, sig);
        });
    }

    fn visit_ts_construct_signature_declaration(
        &mut self,
        sig: &TSConstructSignatureDeclaration<'a>,
    ) {
        self.with_params(sig.type_parameters.as_deref(), |v| {
            walk::walk_ts_construct_signature_declaration(v, sig);
        });
    }

    /// `infer U` declares `U` for the `extends` clause and the true branch.
    fn visit_ts_conditional_type(&mut self, ty: &TSConditionalType<'a>) {
        self.visit_ts_type(&ty.check_type);
        let mut infers = InferNames(std::vec::Vec::new());
        infers.visit_ts_type(&ty.extends_type);
        let len = self.type_params.len();
        self.type_params.extend(infers.0);
        self.visit_ts_type(&ty.extends_type);
        self.visit_ts_type(&ty.true_type);
        self.type_params.truncate(len);
        self.visit_ts_type(&ty.false_type);
    }
}

/// The type parameters `infer` declares in a conditional type's `extends`
/// clause (not those of a conditional type nested in it).
struct InferNames<'a>(std::vec::Vec<&'a str>);

impl<'a> Visit<'a> for InferNames<'a> {
    fn visit_ts_infer_type(&mut self, ty: &TSInferType<'a>) {
        self.0.push(ty.type_parameter.name.name.as_str());
    }

    fn visit_ts_conditional_type(&mut self, _: &TSConditionalType<'a>) {}
}

/// The identifiers an expression mentions.
struct Mentions<'a>(std::vec::Vec<&'a str>);

impl<'a> Visit<'a> for Mentions<'a> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        self.0.push(id.name.as_str());
    }
}

/// An arrow or function expression written in place, which ngtsc can analyze
/// as an input transform.
fn function_value<'a>(expr: &'a Expression<'a>) -> Option<Value<'a>> {
    match expr {
        Expression::ArrowFunctionExpression(f) => Some(Value::Function(FnDef::Arrow(f))),
        Expression::FunctionExpression(f) => Some(Value::Function(FnDef::Function(f))),
        _ => None,
    }
}

/// Bind a parameter: a destructured parameter's names aren't evaluated.
fn bind<'a>(frame: &mut Frame<'a>, pattern: &'a BindingPattern<'a>, binding: Binding<'a>) {
    if let BindingPattern::BindingIdentifier(id) = pattern {
        frame.bindings.insert(id.name.as_str(), binding);
        return;
    }
    let mut names = std::vec::Vec::new();
    collect_bindings(pattern, &mut std::vec::Vec::new(), &mut names);
    for (name, _) in names {
        frame.bindings.insert(name, Binding::new(Value::Dynamic, None));
    }
}

/// Whether an expression uses a name bound in a called function's frame, or
/// that function's `this` or `arguments`. Names are scoped like JavaScript
/// does: a parameter, variable, function or class the expression declares
/// itself (in a nested function, block, loop or `catch`) hides the frame's,
/// and a nested non-arrow function has its own `this` and `arguments`.
struct UsesFrame<'f, 'a> {
    frame: &'f Frame<'a>,
    /// Resolves the names of the namespace block the frame is in, if any.
    scope: &'f FileScope<'a>,
    found: bool,
    /// The names each scope entered inside the expression declares, innermost last.
    scopes: std::vec::Vec<std::vec::Vec<String>>,
    /// How many functions entered inside the expression have their own
    /// `arguments` (non-arrow functions) ...
    own_arguments: usize,
    /// ... and their own `this` (those, and class bodies).
    own_this: usize,
}

impl<'f, 'a> UsesFrame<'f, 'a> {
    fn new(frame: &'f Frame<'a>, scope: &'f FileScope<'a>) -> Self {
        Self {
            frame,
            scope,
            found: false,
            scopes: std::vec::Vec::new(),
            own_arguments: 0,
            own_this: 0,
        }
    }

    fn declared(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.iter().any(|n| n == name))
    }

    /// Visits `f` with `names` in scope.
    fn scoped(&mut self, names: std::vec::Vec<String>, f: impl FnOnce(&mut Self)) {
        self.scopes.push(names);
        f(self);
        self.scopes.pop();
    }
}

/// The names a binding pattern declares.
fn bound_names(pattern: &BindingPattern<'_>, out: &mut std::vec::Vec<String>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.push(id.name.to_string()),
        BindingPattern::AssignmentPattern(p) => bound_names(&p.left, out),
        BindingPattern::ObjectPattern(p) => {
            for prop in &p.properties {
                bound_names(&prop.value, out);
            }
            if let Some(rest) = &p.rest {
                bound_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(p) => {
            for element in p.elements.iter().flatten() {
                bound_names(element, out);
            }
            if let Some(rest) = &p.rest {
                bound_names(&rest.argument, out);
            }
        }
    }
}

/// The names a function's parameters declare.
fn parameter_names(params: &FormalParameters<'_>, out: &mut std::vec::Vec<String>) {
    for param in &params.items {
        bound_names(&param.pattern, out);
    }
    if let Some(rest) = &params.rest {
        bound_names(&rest.rest.argument, out);
    }
}

/// The names `let`, `const`, `class` and `function` declarations directly in
/// a block declare, for the whole block.
fn lexical_names(statements: &[Statement<'_>], out: &mut std::vec::Vec<String>) {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration(decl) if decl.kind != VariableDeclarationKind::Var => {
                for d in &decl.declarations {
                    bound_names(&d.id, out);
                }
            }
            Statement::FunctionDeclaration(f) => {
                out.extend(f.id.as_ref().map(|id| id.name.to_string()));
            }
            Statement::ClassDeclaration(c) => {
                out.extend(c.id.as_ref().map(|id| id.name.to_string()));
            }
            _ => {}
        }
    }
}

/// The names `var` declarations in a function body declare (in nested blocks
/// too, but not in nested functions or classes), for the whole function.
struct VarNames<'n>(&'n mut std::vec::Vec<String>);

impl<'a> Visit<'a> for VarNames<'_> {
    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        if decl.kind == VariableDeclarationKind::Var {
            for d in &decl.declarations {
                bound_names(&d.id, self.0);
            }
        }
    }

    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}

    fn visit_class(&mut self, _: &Class<'a>) {}
}

/// The names a function body declares: its `var`s and its lexical declarations.
fn body_names(body: &FunctionBody<'_>, out: &mut std::vec::Vec<String>) {
    let mut vars = VarNames(out);
    for statement in &body.statements {
        vars.visit_statement(statement);
    }
    lexical_names(&body.statements, out);
}

impl<'a> Visit<'a> for UsesFrame<'_, 'a> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        let name = id.name.as_str();
        let in_namespace =
            || self.frame.block.and_then(|from| self.scope.declaring_block(from, name)).is_some();
        self.found |= if name == "arguments" && self.frame.in_call {
            self.own_arguments == 0 && !self.declared(name)
        } else {
            (self.frame.bindings.contains_key(name) || in_namespace()) && !self.declared(name)
        };
    }

    fn visit_this_expression(&mut self, _: &ThisExpression) {
        self.found |= self.frame.in_call && self.own_this == 0;
    }

    fn visit_super(&mut self, _: &Super) {
        self.found |= self.frame.in_call && self.own_this == 0;
    }

    // Types are erased from the emitted code, so a name used only in one
    // (`(v: string): typeof name => v`, `v as typeof name`, `id<typeof name>(v)`)
    // doesn't need to be in scope where the transform is emitted. The runtime
    // parts around them (a parameter's default, the expression inside `as`,
    // `satisfies`, `<T>x` and `x!`) are still visited.
    fn visit_ts_type(&mut self, _: &TSType<'a>) {}

    fn visit_ts_type_parameter_declaration(&mut self, _: &TSTypeParameterDeclaration<'a>) {}

    fn visit_ts_type_parameter_instantiation(&mut self, _: &TSTypeParameterInstantiation<'a>) {}

    fn visit_ts_type_alias_declaration(&mut self, _: &TSTypeAliasDeclaration<'a>) {}

    fn visit_ts_interface_declaration(&mut self, _: &TSInterfaceDeclaration<'a>) {}

    fn visit_function(&mut self, f: &Function<'a>, flags: ScopeFlags) {
        let mut names = std::vec::Vec::new();
        // A function expression's own name is in scope in its body; a
        // declaration's is declared in the enclosing block.
        names.extend(f.id.as_ref().map(|id| id.name.to_string()));
        parameter_names(&f.params, &mut names);
        if let Some(body) = &f.body {
            body_names(body, &mut names);
        }
        self.own_arguments += 1;
        self.own_this += 1;
        self.scoped(names, |v| walk::walk_function(v, f, flags));
        self.own_arguments -= 1;
        self.own_this -= 1;
    }

    fn visit_arrow_function_expression(&mut self, f: &ArrowFunctionExpression<'a>) {
        let mut names = std::vec::Vec::new();
        parameter_names(&f.params, &mut names);
        if let ArrowFunctionBody::FunctionBody(body) = &f.body {
            body_names(body, &mut names);
        }
        self.scoped(names, |v| walk::walk_arrow_function_expression(v, f));
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        // A class expression's own name is in scope in its body.
        let names = class.id.iter().map(|id| id.name.to_string()).collect();
        self.scoped(names, |v| {
            if let Some(heritage) = &class.heritage {
                v.visit_expression(&heritage.expression);
            }
            v.own_this += 1;
            v.visit_class_body(&class.body);
            v.own_this -= 1;
        });
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        let mut names = std::vec::Vec::new();
        let mut vars = VarNames(&mut names);
        for statement in &block.body {
            vars.visit_statement(statement);
        }
        lexical_names(&block.body, &mut names);
        self.scoped(names, |v| walk::walk_static_block(v, block));
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        let mut names = std::vec::Vec::new();
        lexical_names(&block.body, &mut names);
        self.scoped(names, |v| walk::walk_block_statement(v, block));
    }

    fn visit_for_statement(&mut self, stmt: &ForStatement<'a>) {
        let mut names = std::vec::Vec::new();
        if let Some(ForStatementInit::VariableDeclaration(decl)) = &stmt.init
            && decl.kind != VariableDeclarationKind::Var
        {
            for d in &decl.declarations {
                bound_names(&d.id, &mut names);
            }
        }
        self.scoped(names, |v| walk::walk_for_statement(v, stmt));
    }

    fn visit_for_in_statement(&mut self, stmt: &ForInStatement<'a>) {
        let names = loop_names(&stmt.left);
        self.scoped(names, |v| walk::walk_for_in_statement(v, stmt));
    }

    fn visit_for_of_statement(&mut self, stmt: &ForOfStatement<'a>) {
        let names = loop_names(&stmt.left);
        self.scoped(names, |v| walk::walk_for_of_statement(v, stmt));
    }

    fn visit_switch_statement(&mut self, stmt: &SwitchStatement<'a>) {
        // The discriminant is outside the cases' block.
        self.visit_expression(&stmt.discriminant);
        let mut names = std::vec::Vec::new();
        for case in &stmt.cases {
            lexical_names(&case.consequent, &mut names);
        }
        self.scoped(names, |v| {
            for case in &stmt.cases {
                v.visit_switch_case(case);
            }
        });
    }

    fn visit_catch_clause(&mut self, clause: &CatchClause<'a>) {
        let mut names = std::vec::Vec::new();
        if let Some(param) = &clause.param {
            bound_names(&param.pattern, &mut names);
        }
        self.scoped(names, |v| walk::walk_catch_clause(v, clause));
    }
}

/// The names a `for (let ... of/in ...)` head declares for the loop.
fn loop_names(left: &ForStatementLeft<'_>) -> std::vec::Vec<String> {
    let mut names = std::vec::Vec::new();
    if let ForStatementLeft::VariableDeclaration(decl) = left
        && decl.kind != VariableDeclarationKind::Var
    {
        for d in &decl.declarations {
            bound_names(&d.id, &mut names);
        }
    }
    names
}

/// ngtsc's `literal()`: an operand of a binary operator or a template literal
/// must be a primitive, and an enum member counts as its value.
fn literal(value: Value<'_>) -> Value<'_> {
    let value = match value {
        Value::Enum { value, .. } => *value,
        value => value,
    };
    match value {
        Value::Dynamic
        | Value::Null
        | Value::Undefined
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_) => value,
        value if value.is_import() => value,
        _ => Value::Dynamic,
    }
}

/// `String(value)` for a primitive.
fn to_js_string(value: &Value<'_>) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Undefined => "undefined".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number_like_js(*n),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null | Value::Undefined => String::new(),
                item => to_js_string(item),
            })
            .collect::<std::vec::Vec<_>>()
            .join(","),
        _ => "[object Object]".into(),
    }
}

/// `Number(value)`. Arrays convert through their string form, like in JavaScript.
fn to_number(value: &Value<'_>) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => *n,
        Value::String(s) => string_to_number(s),
        Value::Array(_) => string_to_number(&to_js_string(value)),
        _ => f64::NAN,
    }
}

/// JavaScript's `StringToNumber`.
fn string_to_number(s: &str) -> f64 {
    let is_js_space = |c: char| {
        matches!(
            c,
            '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
                    | '\u{feff}'
        )
    };
    let s = s.trim_matches(is_js_space);
    if s.is_empty() {
        return 0.0;
    }
    let radix = |digits: &str, radix: u32| {
        if digits.is_empty() {
            return f64::NAN;
        }
        digits
            .chars()
            .try_fold(0.0, |acc: f64, c| {
                c.to_digit(radix).map(|d| acc * f64::from(radix) + f64::from(d))
            })
            .unwrap_or(f64::NAN)
    };
    match s.get(..2) {
        Some("0x" | "0X") => return radix(&s[2..], 16),
        Some("0o" | "0O") => return radix(&s[2..], 8),
        Some("0b" | "0B") => return radix(&s[2..], 2),
        _ => {}
    }
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    if unsigned == "Infinity" {
        return if s.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // Rust also parses `inf`, `nan`, ...: only accept JavaScript's decimal syntax.
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let valid_mantissa = digits(int) && digits(frac) && !(int.is_empty() && frac.is_empty());
    let valid_exponent = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && digits(e)
    });
    if valid_mantissa && valid_exponent { s.parse().unwrap_or(f64::NAN) } else { f64::NAN }
}

/// JavaScript's `ToInt32`.
fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    (n.trunc().rem_euclid(4_294_967_296.0) as u32) as i32
}

fn unary<'a>(operator: UnaryOperator, value: &Value<'a>) -> Value<'a> {
    let value = match value {
        // An enum member is an object at runtime.
        Value::Enum { .. } => &Value::Dynamic,
        value => value,
    };
    let number = || if matches!(value, Value::Dynamic) { f64::NAN } else { to_number(value) };
    match operator {
        UnaryOperator::UnaryNegation => Value::Number(-number()),
        UnaryOperator::UnaryPlus => Value::Number(number()),
        UnaryOperator::LogicalNot => Value::Bool(!value.truthy()),
        UnaryOperator::BitwiseNot => Value::Number(f64::from(!to_int32(number()))),
        _ => Value::Dynamic,
    }
}

/// A binary operator on two primitives.
fn binary<'a>(operator: BinaryOperator, left: &Value<'a>, right: &Value<'a>) -> Value<'a> {
    let (l, r) = (to_number(left), to_number(right));
    let shift = || to_int32(r) as u32 & 31;
    match operator {
        BinaryOperator::Addition => match (left, right) {
            (Value::String(_), _) | (_, Value::String(_)) => {
                Value::String(to_js_string(left) + &to_js_string(right))
            }
            _ => Value::Number(l + r),
        },
        BinaryOperator::Subtraction => Value::Number(l - r),
        BinaryOperator::Multiplication => Value::Number(l * r),
        BinaryOperator::Division => Value::Number(l / r),
        BinaryOperator::Remainder => Value::Number(l % r),
        BinaryOperator::Exponential => {
            Value::Number(if r.is_nan() || (l.abs() == 1.0 && r.is_infinite()) {
                f64::NAN
            } else {
                l.powf(r)
            })
        }
        BinaryOperator::BitwiseAnd => Value::Number(f64::from(to_int32(l) & to_int32(r))),
        BinaryOperator::BitwiseOR => Value::Number(f64::from(to_int32(l) | to_int32(r))),
        BinaryOperator::BitwiseXOR => Value::Number(f64::from(to_int32(l) ^ to_int32(r))),
        BinaryOperator::ShiftLeft => Value::Number(f64::from(to_int32(l).wrapping_shl(shift()))),
        BinaryOperator::ShiftRight => Value::Number(f64::from(to_int32(l) >> shift())),
        BinaryOperator::ShiftRightZeroFill => {
            Value::Number(f64::from((to_int32(l) as u32) >> shift()))
        }
        BinaryOperator::LessThan => Value::Bool(compare(left, right, false)),
        BinaryOperator::GreaterThan => Value::Bool(compare(right, left, false)),
        BinaryOperator::LessEqualThan => Value::Bool(compare(left, right, true)),
        BinaryOperator::GreaterEqualThan => Value::Bool(compare(right, left, true)),
        BinaryOperator::Equality => Value::Bool(loose_equals(left, right)),
        BinaryOperator::Inequality => Value::Bool(!loose_equals(left, right)),
        BinaryOperator::StrictEquality => Value::Bool(strict_equals(left, right)),
        BinaryOperator::StrictInequality => Value::Bool(!strict_equals(left, right)),
        BinaryOperator::In | BinaryOperator::Instanceof => Value::Dynamic,
    }
}

/// `a < b` (or `a <= b`): strings compare by UTF-16 code units, anything else as numbers.
fn compare(a: &Value<'_>, b: &Value<'_>, or_equal: bool) -> bool {
    let ordering = match (a, b) {
        (Value::String(a), Value::String(b)) => Some(a.encode_utf16().cmp(b.encode_utf16())),
        _ => to_number(a).partial_cmp(&to_number(b)),
    };
    ordering.is_some_and(|o| o.is_lt() || (or_equal && o.is_eq()))
}

fn strict_equals(a: &Value<'_>, b: &Value<'_>) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        _ => false,
    }
}

fn loose_equals(a: &Value<'_>, b: &Value<'_>) -> bool {
    match (a, b) {
        (Value::Null | Value::Undefined, Value::Null | Value::Undefined) => true,
        (Value::Null | Value::Undefined, _) | (_, Value::Null | Value::Undefined) => false,
        (Value::String(_), Value::String(_)) => strict_equals(a, b),
        _ => to_number(a) == to_number(b),
    }
}

// =============================================================================
// `.d.ts` input transform types
// =============================================================================

/// An input transform whose first parameter the `.d.ts` can be typed with.
#[derive(Clone, Copy)]
enum Transform<'a> {
    /// A function defined in this file (its first declaration).
    Def(FnDef<'a>),
    /// A function of TypeScript's library: the type of its first parameter.
    Lib(&'static str),
}

/// The `.d.ts` type of each input transform's first parameter, for
/// `static ngAcceptInputType_<name>: T;`.
///
/// Only transforms defined in this file or in TypeScript's library
/// ([`ES_GLOBAL_FUNCTIONS`]) are covered; imported ones can't be inspected and
/// are left out.
pub fn input_transform_types<'a>(
    class: &'a Class<'a>,
    consts: &super::StringConsts<'a>,
    source: &'a str,
) -> HashMap<String, String> {
    let evaluator = Evaluator::new(consts);
    let scope = consts.scope();
    // The transform that ends up compiled for each input: a member `@Input`
    // overrides an `inputs:` entry, like the compiled inputs map.
    let mut transforms: std::vec::Vec<(String, Option<Transform<'a>>)> = std::vec::Vec::new();
    let mut record = |name: String, value: &Value<'a>| {
        let transform = match value {
            Value::Function(def) => Some(Transform::Def(*def)),
            // An overloaded function or static method is typed from its first
            // declaration, as ngtsc does.
            Value::Reference { name, kind: RefKind::Function(function, _) } => {
                Some(Transform::Def(FnDef::Function(scope.first_declaration(name, function))))
            }
            Value::Reference { name, kind: RefKind::Global } => {
                es_global_function_param(name).map(Transform::Lib)
            }
            _ => None,
        };
        match transforms.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = transform,
            None => transforms.push((name, transform)),
        }
    };
    if let Some((Some(config), _)) = super::angular_decorator_config(class)
        && let Some(inputs) = super::decorator::config_property(config, "inputs", consts)
        && let Value::Array(items) = evaluator.evaluate(inputs)
    {
        for item in &items {
            if let (Some(Value::String(name)), Some(transform)) =
                (item.prop("name").map(|p| &p.value), item.prop("transform"))
            {
                record(name.clone(), &transform.value);
            }
        }
    }
    for element in &class.body.body {
        let (key, decorators) = match element {
            ClassElement::PropertyDefinition(p) => (&p.key, &p.decorators),
            ClassElement::AccessorProperty(p) => (&p.key, &p.decorators),
            ClassElement::MethodDefinition(m) => (&m.key, &m.decorators),
            _ => continue,
        };
        let Some(name) = key.static_name() else { continue };
        if let Some(options) =
            super::property_decorators::input_decorator_options(decorators, consts)
            && let Some(transform) = evaluator.evaluate(options).prop("transform")
        {
            record(name.to_string(), &transform.value);
        }
    }

    transforms
        .into_iter()
        .filter_map(|(name, transform)| {
            let def = match transform? {
                Transform::Def(def) => def,
                Transform::Lib(ty) => return Some((name, ty.to_string())),
            };
            let ty = match def.first_param_type().ok()? {
                Some(ty) => {
                    let mut printer =
                        super::dts_type::TypePrinter { scope, source, other_module: false };
                    let ty = printer.print(ty);
                    if printer.other_module {
                        return None;
                    }
                    ty
                }
                None => "unknown".to_string(),
            };
            Some((name, ty))
        })
        .collect()
}

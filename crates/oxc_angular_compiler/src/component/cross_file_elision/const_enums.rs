//! Which imported types are `const enum`s, for the class metadata `type:` of
//! a constructor parameter (#623). ngtsc's full and partial `typeToValue`
//! write `undefined` when the symbol at the end of the type name is a
//! `ConstEnum`, wherever it is declared: a source file, a `.d.ts` file or a
//! package's typings, reached through named imports, namespace imports,
//! re-exports and namespaces (`k.NS.Inner`).
//!
//! Each file read is kept as an [`EnumShape`]: its imports, `const enum`s,
//! namespaces and exports. A type name is followed through them the way
//! TypeScript resolves its symbol. Anything that can't be followed gives
//! `false`, and the caller keeps today's output.

use std::path::Path;
use std::rc::Rc;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, Program, Statement,
    TSNamespaceDeclarationBody,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{CrossFileAnalyzer, Inner, MAX_EXPORT_CHAIN};

/// Names in one scope: a file's top level, or the exported members of the
/// namespaces of one name.
type Scope = FxHashMap<String, Binding>;

/// What a name is bound to in a [`Scope`]. One name can be a `const enum`
/// and a namespace at once: TypeScript merges them.
#[derive(Debug, Default)]
struct Binding {
    /// A `const enum` declares the name.
    const_enum: bool,
    /// The exported members of the namespaces declared under the name,
    /// across all their blocks.
    members: Option<Scope>,
    /// The import that binds the name (top level only).
    import: Option<Import>,
}

#[derive(Debug)]
enum Import {
    /// `import { name as local }`, or `import local` for `"default"`.
    Named { spec: String, name: String },
    /// `import * as local`.
    Namespace { spec: String },
}

#[derive(Debug)]
enum Export {
    /// `export const enum X`, `export { X as Y }`, `export default X`: the
    /// top-level binding it names.
    Local(String),
    /// `export { name as Y } from 'spec'`, `export type` too.
    From { spec: String, name: String },
    /// `export * as Y from 'spec'`.
    Namespace { spec: String },
    /// Anything else (`export default class`, `export import X = ...`).
    Other,
}

/// What [`Inner::export_path`] needs to know about one file.
#[derive(Debug, Default)]
pub(super) struct EnumShape {
    locals: Scope,
    exports: FxHashMap<String, Export>,
    /// The specifiers of `export * from`, in source order.
    stars: Vec<String>,
    /// `export =`: the module's exports can't be read.
    opaque: bool,
}

impl CrossFileAnalyzer {
    /// Whether `path`, read from the module `import_source` (as `from_file`
    /// writes it), ends at a `const enum`. `path[0]` is the name the module
    /// exports (`"default"` for a default import), and the rest walks into
    /// namespaces: `k.NS.Inner` of `import * as k` is `["NS", "Inner"]`.
    /// `false` when unsure. Files read are recorded in
    /// [`Self::take_value_dependencies`].
    pub(crate) fn is_const_enum_path(
        &self,
        import_source: &str,
        path: &[&str],
        from_file: &Path,
    ) -> bool {
        let Some(file) = self.inner.resolve_types(&from_file.to_string_lossy(), import_source)
        else {
            return false;
        };
        self.inner.export_path(&file, path, 0, &mut FxHashSet::default()) == Some(true)
    }
}

impl Inner {
    /// Whether `path` ends at a `const enum`, when the module `file` exports
    /// `path[0]`: `Some(false)` when unsure, `None` when it doesn't export
    /// that name. `seen` holds the `(file, path)` pairs already looked at, so
    /// a cycle or a second way to the same file ends.
    fn export_path(
        &self,
        file: &str,
        path: &[&str],
        depth: u16,
        seen: &mut FxHashSet<(String, String)>,
    ) -> Option<bool> {
        let (name, rest) = path.split_first()?;
        if !seen.insert((file.to_string(), path.join("."))) {
            return None;
        }
        if depth > MAX_EXPORT_CHAIN {
            return Some(false);
        }
        let Some(shape) = self.enum_shape(file) else { return Some(false) };
        if shape.opaque {
            return Some(false);
        }
        match shape.exports.get(*name) {
            Some(Export::Local(local)) => {
                Some(self.local_path(file, &shape, local, rest, depth, seen))
            }
            Some(Export::From { spec, name }) => {
                let path: Vec<&str> =
                    std::iter::once(name.as_str()).chain(rest.iter().copied()).collect();
                Some(self.module_path(file, spec, &path, depth, seen))
            }
            Some(Export::Namespace { spec }) => {
                Some(!rest.is_empty() && self.module_path(file, spec, rest, depth, seen))
            }
            Some(Export::Other) => Some(false),
            // `export *` leaves out `default`.
            None if *name == "default" => None,
            // Only a plain `export *` adds names: `export * as ns` adds one.
            None => shape.stars.iter().find_map(|spec| match self.resolve_types(file, spec) {
                Some(target) => self.export_path(&target, path, depth + 1, seen),
                None => Some(false),
            }),
        }
    }

    /// [`Self::export_path`] in the module `spec` names from `file`, which
    /// has to export `path[0]`.
    fn module_path(
        &self,
        file: &str,
        spec: &str,
        path: &[&str],
        depth: u16,
        seen: &mut FxHashSet<(String, String)>,
    ) -> bool {
        self.resolve_types(file, spec)
            .and_then(|target| self.export_path(&target, path, depth + 1, seen))
            == Some(true)
    }

    /// Whether the top-level binding `local` of `file`, then `rest`, ends at
    /// a `const enum`.
    fn local_path(
        &self,
        file: &str,
        shape: &EnumShape,
        local: &str,
        rest: &[&str],
        depth: u16,
        seen: &mut FxHashSet<(String, String)>,
    ) -> bool {
        let Some(binding) = shape.locals.get(local) else { return false };
        match &binding.import {
            Some(Import::Named { spec, name }) => {
                let path: Vec<&str> =
                    std::iter::once(name.as_str()).chain(rest.iter().copied()).collect();
                self.module_path(file, spec, &path, depth, seen)
            }
            Some(Import::Namespace { spec }) => {
                !rest.is_empty() && self.module_path(file, spec, rest, depth, seen)
            }
            None => binding.ends_at_const_enum(rest),
        }
    }

    /// The [`EnumShape`] of `file`, read on first use. `None` when it can't
    /// be read or parsed.
    fn enum_shape(&self, file: &str) -> Option<Rc<EnumShape>> {
        if let Some(shape) = self.enum_shapes.borrow().get(file) {
            return Some(Rc::clone(shape));
        }
        let source = std::fs::read_to_string(file).ok()?;
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(file).unwrap_or_default();
        let parsed = Parser::new(&allocator, &source, source_type).parse();
        if parsed.fatal_error {
            return None;
        }
        let shape =
            Rc::new(collect_enum_shape(&parsed.program, source_type.is_typescript_definition()));
        self.enum_shapes.borrow_mut().insert(file.to_string(), Rc::clone(&shape));
        Some(shape)
    }
}

impl Binding {
    /// Whether `path`, read from this binding's namespace members, ends at a
    /// `const enum` (this binding itself for an empty path).
    fn ends_at_const_enum(&self, path: &[&str]) -> bool {
        match path.split_first() {
            None => self.const_enum,
            Some((name, rest)) => self
                .members
                .as_ref()
                .and_then(|members| members.get(*name))
                .is_some_and(|member| member.ends_at_const_enum(rest)),
        }
    }

    fn merge_members(&mut self, members: Scope) {
        let mine = self.members.get_or_insert_with(Scope::default);
        for (name, binding) in members {
            mine.entry(name).or_default().merge(binding);
        }
    }

    fn merge(&mut self, other: Binding) {
        self.const_enum |= other.const_enum;
        if let Some(members) = other.members {
            self.merge_members(members);
        }
        if other.import.is_some() {
            self.import = other.import;
        }
    }
}

/// The [`EnumShape`] of `program`. `declaration_file` is whether it is a
/// `.d.ts` file: without `export {}`, `export ... from`, `export *`,
/// `export =` or `export default <expr>`, its top-level declarations are
/// exported even without `export` (TypeScript's `ExportContext`).
fn collect_enum_shape(program: &Program<'_>, declaration_file: bool) -> EnumShape {
    let export_context = declaration_file
        && !program.body.iter().any(|stmt| match stmt {
            Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::TSExportAssignment(_) => true,
            Statement::ExportDefaultDeclaration(export) => export.declaration.is_expression(),
            _ => false,
        });
    let mut shape = EnumShape::default();
    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                let Some(spec) = import.source.value.as_str() else { continue };
                for specifier in import.specifiers.iter().flatten() {
                    let (local, import) = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => (
                            &s.local.name,
                            Import::Named {
                                spec: spec.to_string(),
                                name: s.imported.name().to_string(),
                            },
                        ),
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => (
                            &s.local.name,
                            Import::Named { spec: spec.to_string(), name: "default".to_string() },
                        ),
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                            (&s.local.name, Import::Namespace { spec: spec.to_string() })
                        }
                    };
                    shape.locals.entry(local.to_string()).or_default().import = Some(import);
                }
            }
            Statement::ExportDeclaration(export) => {
                declare(&mut shape, &export.declaration, declaration_file, true);
            }
            Statement::ExportNamedDeclaration(export) => {
                for spec in &export.specifiers {
                    let local = spec.local.name().to_string();
                    add_export(&mut shape, &spec.exported.name(), Export::Local(local));
                }
            }
            Statement::ExportFromDeclaration(export) => {
                let Some(spec) = export.source.value.as_str() else { continue };
                for specifier in &export.specifiers {
                    let name = specifier.local.name().to_string();
                    let how = Export::From { spec: spec.to_string(), name };
                    add_export(&mut shape, &specifier.exported.name(), how);
                }
            }
            Statement::ExportAllDeclaration(export) => {
                let Some(spec) = export.source.value.as_str() else { continue };
                match &export.exported {
                    Some(name) => {
                        let how = Export::Namespace { spec: spec.to_string() };
                        add_export(&mut shape, &name.name(), how);
                    }
                    None => shape.stars.push(spec.to_string()),
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                let how = match &export.declaration {
                    ExportDefaultDeclarationKind::Identifier(id) => {
                        Export::Local(id.name.to_string())
                    }
                    _ => Export::Other,
                };
                add_export(&mut shape, "default", how);
            }
            Statement::TSExportAssignment(_) => shape.opaque = true,
            _ => {
                if let Some(declaration) = stmt.as_declaration() {
                    declare(&mut shape, declaration, declaration_file, export_context);
                }
            }
        }
    }
    shape
}

fn add_export(shape: &mut EnumShape, name: &str, how: Export) {
    shape.exports.entry(name.to_string()).or_insert(how);
}

/// Add a top-level `declaration` to `shape`, and to its exports when
/// `exported`. `ambient` is whether the file is a `.d.ts` file.
fn declare(shape: &mut EnumShape, declaration: &Declaration<'_>, ambient: bool, exported: bool) {
    let mut names: Vec<String> = Vec::new();
    match declaration {
        Declaration::TSEnumDeclaration(e) => {
            names.push(e.id.name.to_string());
            if e.r#const {
                shape.locals.entry(e.id.name.to_string()).or_default().const_enum = true;
            }
        }
        Declaration::TSNamespaceDeclaration(ns) => {
            names.push(ns.id.name.to_string());
            let members = namespace_members(&ns.body, ambient || ns.declare);
            shape.locals.entry(ns.id.name.to_string()).or_default().merge_members(members);
        }
        Declaration::ClassDeclaration(class) => {
            names.extend(class.id.iter().map(|id| id.name.to_string()));
        }
        Declaration::FunctionDeclaration(func) => {
            names.extend(func.id.iter().map(|id| id.name.to_string()));
        }
        Declaration::VariableDeclaration(vars) => {
            for var in &vars.declarations {
                names.extend(var.id.get_binding_identifiers().iter().map(|id| id.name.to_string()));
            }
        }
        Declaration::TSInterfaceDeclaration(d) => names.push(d.id.name.to_string()),
        Declaration::TSTypeAliasDeclaration(d) => names.push(d.id.name.to_string()),
        Declaration::TSImportEqualsDeclaration(d) => {
            // `export import X = ...` isn't followed.
            if exported {
                add_export(shape, &d.id.name, Export::Other);
            }
        }
        Declaration::TSExternalModuleDeclaration(_) | Declaration::TSGlobalDeclaration(_) => {}
    }
    if exported {
        for name in names {
            let how = Export::Local(name.clone());
            add_export(shape, &name, how);
        }
    }
}

/// The members a namespace body exports: in an `ambient` (`declare`d) one,
/// every member.
fn namespace_members(body: &TSNamespaceDeclarationBody<'_>, ambient: bool) -> Scope {
    let mut members = Scope::default();
    match body {
        TSNamespaceDeclarationBody::TSModuleBlock(block) => {
            for stmt in &block.body {
                let declaration = match stmt {
                    Statement::ExportDeclaration(export) => &export.declaration,
                    _ if ambient => match stmt.as_declaration() {
                        Some(declaration) => declaration,
                        None => continue,
                    },
                    _ => continue,
                };
                match declaration {
                    Declaration::TSEnumDeclaration(e) if e.r#const => {
                        members.entry(e.id.name.to_string()).or_default().const_enum = true;
                    }
                    Declaration::TSNamespaceDeclaration(ns) => {
                        let nested = namespace_members(&ns.body, ambient || ns.declare);
                        members.entry(ns.id.name.to_string()).or_default().merge_members(nested);
                    }
                    _ => {}
                }
            }
        }
        // `namespace A.B { ... }`: `B` is a member of `A`.
        TSNamespaceDeclarationBody::TSNamespaceDeclaration(ns) => {
            let nested = namespace_members(&ns.body, ambient || ns.declare);
            members.entry(ns.id.name.to_string()).or_default().merge_members(nested);
        }
    }
    members
}

//! Static NgModule scopes for partial compilation (#655).
//!
//! A partial declaration of a component an NgModule declares carries no
//! `dependencies`: ngtsc lists the module's compilation scope there, which it
//! reads from the whole program. Oxc compiles one file at a time, and a
//! component's file doesn't name the module that declares it. The module's
//! file does, so it calls `ɵɵsetComponentScope` for each component it
//! declares, with the scope read here: the `@NgModule` of a source file, or
//! the `ɵmod` type of a declaration file, like ngtsc's `LocalModuleScopeRegistry`
//! and `DtsMetadataReader`.
//!
//! Anything that can't be read statically gives `None`, and the caller emits
//! what it did before.

use std::cell::{OnceCell, RefCell};
use std::path::Path;
use std::rc::Rc;

use oxc_ast::ast::{
    ArrayExpressionElement, Class, ClassElement, Declaration, ExportDefaultDeclarationKind,
    Expression, ImportDeclarationSpecifier, ObjectPropertyKind, Program, PropertyKey, Statement,
    TSLiteral, TSTupleElement, TSType, TSTypeName,
};
use oxc_span::GetSpan;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{CrossFileAnalyzer, Inner, MAX_EXPORT_CHAIN};
use crate::component::metadata::{ComponentScope, ImportedName, ScopeEntry, ScopeKind};
use crate::directive::{Evaluator, StringConsts, find_angular_class_decorator};

/// A top-level declaration: the file declaring it and its name there
/// (`"default"` for an anonymous `export default class`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Decl {
    file: String,
    name: String,
}

/// A reference to a class as some file writes it: `name`, `namespace.name`,
/// or `name.method(...)`, a static method returning `ModuleWithProviders`.
#[derive(Debug, Clone)]
struct RawRef {
    file: String,
    namespace: Option<String>,
    name: String,
    method: Option<String>,
}

/// A class a reference resolves to, and the non-relative module specifier
/// it was imported through, if any (ngtsc's `bestGuessOwningModule`): the
/// specifier the compiled file imports it from.
#[derive(Debug, Clone)]
struct ClassRef {
    decl: Decl,
    owner: Option<String>,
}

/// What a declaration is to a module scope.
#[derive(Debug)]
enum Declared {
    Directive {
        selector: Option<String>,
        is_component: bool,
    },
    Pipe {
        name: Option<String>,
    },
    NgModule {
        declarations: Vec<RawRef>,
        imports: Vec<RawRef>,
        exports: Vec<RawRef>,
    },
    /// A `const` list of references, spread or named in a module's lists.
    List(Vec<RawRef>),
}

/// A directive or pipe of a scope, before it's named for the compiled file.
#[derive(Debug, Clone)]
struct ScopeItem {
    class: ClassRef,
    kind: ScopeKind,
}

/// How a file exports a name.
#[derive(Debug)]
enum Export {
    /// `export class X`, `export { Y as X }`: the local binding.
    Local(String),
    /// `export { Y as X } from 'spec'`.
    From { spec: String, name: String },
}

/// A top-level binding of a file.
#[derive(Debug)]
enum Local {
    /// A class or variable the file declares.
    Declared,
    /// An import of `imported` from `spec`; `None` for `import * as`.
    Import { spec: String, imported: Option<String> },
}

/// A file's exports and top-level bindings.
#[derive(Debug, Default)]
struct FileExports {
    /// Export names, in source order.
    names: Vec<String>,
    named: FxHashMap<String, Export>,
    /// The specifiers of `export * from`.
    stars: Vec<String>,
    locals: FxHashMap<String, Local>,
    /// The specifiers of its imports, in source order.
    import_specs: Vec<String>,
    /// The specifiers of the imports and re-exports that load a module at
    /// runtime: not those of types only.
    loads: Vec<String>,
}

/// Per NgModule, and the module it was imported through, its exported scope.
type ExportedScopes = FxHashMap<(Decl, Option<String>), Option<Rc<[ScopeItem]>>>;

/// Memoized reads of [`Inner`]'s scope queries.
#[derive(Default)]
pub(super) struct ScopeState {
    /// The analyzer's resolver, reading the `types` of packages.
    resolver: OnceCell<oxc_resolver::Resolver>,
    files: RefCell<FxHashMap<String, Rc<FileExports>>>,
    declared: RefCell<FxHashMap<Decl, Option<Rc<Declared>>>>,
    exported: RefCell<ExportedScopes>,
    /// Modules whose exported scope is being read: a cycle gives `None`.
    visiting: RefCell<FxHashSet<Decl>>,
    /// Per module file, the name it exports each declaration by.
    export_names: RefCell<FxHashMap<String, Rc<FxHashMap<Decl, String>>>>,
    /// The declaration each `(file, name)` is exported as.
    found_exports: RefCell<FxHashMap<(String, String), Option<Decl>>>,
    /// `const` lists being flattened: a list that spreads itself has none.
    flattening: RefCell<FxHashSet<Decl>>,
}

impl CrossFileAnalyzer {
    /// The compilation scope of each component the `@NgModule` class
    /// `class_name` of `file` declares: the directives and pipes the
    /// module's imports export, then its own declarations (ngtsc's
    /// `LocalModuleScopeRegistry`). `source` is the text of `file` being
    /// compiled. `None` when part of it can't be read statically.
    pub(crate) fn ng_module_component_scopes(
        &self,
        file: &Path,
        source: &str,
        class_name: &str,
    ) -> Option<Vec<ComponentScope>> {
        let file = self.inner.set_compiled_source(file, source);
        let module =
            ClassRef { decl: Decl { file: file.clone(), name: class_name.into() }, owner: None };
        let declared = self.inner.declared(&module.decl)?;
        let Declared::NgModule { declarations, imports, .. } = &*declared else { return None };
        let inner = &self.inner;

        let mut scope = Vec::new();
        for (class, declared) in inner.resolve_refs(imports, None, 0)? {
            match &*declared {
                Declared::NgModule { exports, .. } => {
                    scope.extend(inner.exported_scope(&class, exports)?.iter().cloned());
                }
                // A standalone class imported directly.
                _ => scope.push(ScopeItem { kind: scope_kind(&declared)?, class }),
            }
        }
        let mut components = Vec::new();
        for (class, declared) in inner.resolve_refs(declarations, None, 0)? {
            let kind = scope_kind(&declared)?;
            if matches!(kind, ScopeKind::Directive { is_component: true, .. }) {
                components.push(class.clone());
            }
            scope.push(ScopeItem { class, kind });
        }
        let entries = inner.named_entries(dedupe(scope), &file)?;
        let mut scopes = Vec::new();
        for class in &components {
            let component = inner.imported_name(class, &file)?;
            // The call reads the component class when this file runs.
            if inner.loads_back(&file, &component, &class.decl) {
                continue;
            }
            scopes.push(ComponentScope { component, entries: entries.clone() });
        }
        Some(scopes)
    }

    /// For each of the `imports` (`namespace.name` or `name` in `file`) of the
    /// standalone `component`, the directives and pipes it adds when it's an
    /// NgModule, or `None` when it's none or its scope can't be read
    /// statically. A class is listed once: an NgModule leaves out what
    /// another import already is, or an earlier NgModule exports.
    pub(crate) fn standalone_import_scopes(
        &self,
        file: &Path,
        source: &str,
        component: &str,
        imports: &[(Option<&str>, &str)],
    ) -> Vec<Option<Vec<ScopeEntry>>> {
        let file = self.inner.set_compiled_source(file, source);
        // The classes of the file declared after the component, which its
        // definition can't read when it's created.
        let declared_later: FxHashSet<String> = self
            .inner
            .parsed_file(&file)
            .map(|parsed| {
                parsed.with(|_, program, _| {
                    let start = find_class(program, component).map_or(u32::MAX, |c| c.span.start);
                    program
                        .body
                        .iter()
                        .filter(|stmt| stmt.span().start > start)
                        .filter_map(|stmt| match stmt {
                            Statement::ClassDeclaration(class) => class.id.as_ref(),
                            Statement::ExportDeclaration(decl) => match &decl.declaration {
                                Declaration::ClassDeclaration(class) => class.id.as_ref(),
                                _ => None,
                            },
                            Statement::ExportDefaultDeclaration(decl) => match &decl.declaration {
                                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                                    class.id.as_ref()
                                }
                                _ => None,
                            },
                            _ => None,
                        })
                        .map(|id| id.name.to_string())
                        .collect()
                })
            })
            .unwrap_or_default();
        let resolved: Vec<_> = imports
            .iter()
            .map(|(namespace, name)| {
                let raw = RawRef {
                    file: file.clone(),
                    namespace: namespace.map(Into::into),
                    name: (*name).into(),
                    method: None,
                };
                let class = self.inner.resolve_ref(&raw, None, 0)?;
                let declared = self.inner.declared(&class.decl)?;
                Some((class, declared))
            })
            .collect();
        let mut listed: FxHashSet<Decl> =
            resolved.iter().flatten().map(|(class, _)| class.decl.clone()).collect();
        resolved
            .into_iter()
            .map(|import| {
                let (class, declared) = import?;
                let Declared::NgModule { exports, .. } = &*declared else { return None };
                let scope = self.inner.exported_scope(&class, exports)?;
                let added: Vec<_> = scope
                    .iter()
                    .filter(|item| !listed.contains(&item.class.decl))
                    .cloned()
                    .collect();
                let mut entries = self.inner.named_entries(added.clone(), &file)?;
                // The component's definition reads them when it's created.
                for (entry, item) in entries.iter_mut().zip(&added) {
                    entry.is_forward_reference = match &entry.name {
                        ImportedName::Local(name) => declared_later.contains(name),
                        ImportedName::Module { .. } => {
                            self.inner.loads_back(&file, &entry.name, &item.class.decl)
                        }
                    };
                }
                // A pipe the linker can't match without a name.
                if entries.iter().any(|entry| matches!(entry.kind, ScopeKind::Pipe { name: None }))
                {
                    return None;
                }
                listed.extend(added.into_iter().map(|item| item.class.decl));
                Some(entries)
            })
            .collect()
    }
}

/// `items` with each class once, where it first appears.
fn dedupe(items: Vec<ScopeItem>) -> Vec<ScopeItem> {
    let mut seen = FxHashSet::default();
    items.into_iter().filter(|item| seen.insert(item.class.decl.clone())).collect()
}

/// The scope kind of a directive or pipe.
fn scope_kind(declared: &Declared) -> Option<ScopeKind> {
    match declared {
        Declared::Directive { selector, is_component } => {
            Some(ScopeKind::Directive { selector: selector.clone(), is_component: *is_component })
        }
        Declared::Pipe { name } => Some(ScopeKind::Pipe { name: name.clone() }),
        Declared::NgModule { .. } | Declared::List(_) => None,
    }
}

/// `file` as the resolver writes a real path: `/tmp` is a link on macOS,
/// and on Windows without the `\\?\` prefix `canonicalize` adds.
fn real_path(file: &Path) -> String {
    let path = std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .to_string_lossy()
        .into_owned();
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else if let Some(local) = path.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        path
    }
}

/// Whether `file` is in a package rather than the workspace.
fn in_node_modules(file: &str) -> bool {
    Path::new(file).components().any(|part| part.as_os_str() == "node_modules")
}

/// Whether `spec` names a module by path rather than by package.
fn is_relative(spec: &str) -> bool {
    spec.starts_with('.') || Path::new(spec).is_absolute()
}

impl Inner {
    /// Parse `source` as the text of `file`, the file being compiled, in
    /// place of what's on disk. Returns the key it's read by.
    fn set_compiled_source(&self, file: &Path, source: &str) -> String {
        let key = real_path(file);
        let current = self
            .parsed
            .borrow()
            .get(&key)
            .map(|parsed| parsed.with(|_, p, _| p.source_text == source));
        if current != Some(true) {
            let parsed = Rc::new(self.parse_file(&key, source));
            self.parsed.borrow_mut().insert(key.clone(), parsed);
            self.scope.files.borrow_mut().remove(&key);
        }
        key
    }

    /// Resolve `spec` the way TypeScript does from `from_file`: `.d.ts`
    /// files for packages, `.ts` for `./x.js`.
    fn resolve_types(&self, from_file: &str, spec: &str) -> Option<String> {
        let resolver = self.scope.resolver.get_or_init(|| {
            // A package's `types` export, as `tsc` reads it.
            let options = oxc_resolver::ResolveOptions {
                condition_names: vec!["types".into(), "import".into()],
                ..self.resolver.options().clone()
            };
            self.resolver.clone_with_options(options)
        });
        let resolved = resolver.resolve_dts(from_file, spec).ok()?;
        let file = resolved.path().to_string_lossy().into_owned();
        // A package is no file the build watches.
        if !in_node_modules(&file) {
            self.value_dependencies.borrow_mut().insert(file.clone());
        }
        Some(file)
    }

    /// Whether the class `decl`, which `file` refers to as `name`, can be
    /// read before it exists when `file` runs: whether the file it's
    /// imported from or the one that declares it loads `file` back.
    fn loads_back(&self, file: &str, name: &ImportedName, decl: &Decl) -> bool {
        let loaded = match name {
            ImportedName::Local(_) => None,
            ImportedName::Module { specifier, .. } => self.resolve_types(file, specifier),
        };
        loaded.iter().chain([&decl.file]).any(|from| {
            from != file && real_path(Path::new(from)) != file && self.imports_file(from, file)
        })
    }

    /// Whether loading the workspace file `from` loads `target`: whether it
    /// imports or re-exports it, directly or through other workspace files.
    fn imports_file(&self, from: &str, target: &str) -> bool {
        let mut seen = FxHashSet::default();
        let mut pending = vec![from.to_string()];
        while let Some(file) = pending.pop() {
            if in_node_modules(&file) || !seen.insert(file.clone()) {
                continue;
            }
            // A path through a tsconfig alias can be spelled differently
            // (a Windows short name, a link).
            if file == target || real_path(Path::new(&file)) == target {
                return true;
            }
            let Some(exports) = self.file_exports(&file) else { continue };
            for spec in &exports.loads {
                pending.extend(self.resolve_types(&file, spec));
            }
        }
        false
    }

    /// The exports and top-level bindings of `file`.
    fn file_exports(&self, file: &str) -> Option<Rc<FileExports>> {
        if let Some(exports) = self.scope.files.borrow().get(file) {
            return Some(Rc::clone(exports));
        }
        let parsed = self.parsed_file(file)?;
        let exports = Rc::new(parsed.with(|_, program, _| collect_file_exports(program)));
        self.scope.files.borrow_mut().insert(file.to_string(), Rc::clone(&exports));
        Some(exports)
    }

    /// The declaration `file` exports as `name`, following re-exports.
    fn find_export(&self, file: &str, name: &str) -> Option<Decl> {
        let key = (file.to_string(), name.to_string());
        if let Some(found) = self.scope.found_exports.borrow().get(&key) {
            return found.clone();
        }
        let found = self.search_export(file, name, 0, &mut FxHashSet::default());
        self.scope.found_exports.borrow_mut().insert(key, found.clone());
        found
    }

    /// [`Self::find_export`], looking at each `(file, name)` once, as
    /// TypeScript does: an `export *` cycle ends, and no path is followed
    /// twice.
    fn search_export(
        &self,
        file: &str,
        name: &str,
        depth: u16,
        seen: &mut FxHashSet<(String, String)>,
    ) -> Option<Decl> {
        if depth > MAX_EXPORT_CHAIN || !seen.insert((file.to_string(), name.to_string())) {
            return None;
        }
        let exports = self.file_exports(file)?;
        match exports.named.get(name) {
            Some(Export::Local(local)) => match exports.locals.get(local)? {
                Local::Declared => Some(Decl { file: file.to_string(), name: local.clone() }),
                Local::Import { spec, imported: Some(imported) } => {
                    self.search_export(&self.resolve_types(file, spec)?, imported, depth + 1, seen)
                }
                Local::Import { imported: None, .. } => None,
            },
            Some(Export::From { spec, name }) => {
                self.search_export(&self.resolve_types(file, spec)?, name, depth + 1, seen)
            }
            None if name == "default" => None,
            None => exports.stars.iter().find_map(|spec| {
                self.search_export(&self.resolve_types(file, spec)?, name, depth + 1, seen)
            }),
        }
    }

    /// The class `raw` names, reached through the module `owner` names.
    fn resolve_ref(&self, raw: &RawRef, owner: Option<&str>, depth: u16) -> Option<ClassRef> {
        if depth > MAX_EXPORT_CHAIN {
            return None;
        }
        let exports = self.file_exports(&raw.file)?;
        let binding = raw.namespace.as_deref().unwrap_or(&raw.name);
        let (decl, owner) = match (exports.locals.get(binding)?, &raw.namespace) {
            (Local::Declared, None) => {
                (Decl { file: raw.file.clone(), name: raw.name.clone() }, owner.map(Into::into))
            }
            (Local::Import { spec, imported }, namespace) => {
                let name = match (imported, namespace) {
                    (Some(imported), None) => imported.as_str(),
                    (None, Some(_)) => raw.name.as_str(),
                    _ => return None,
                };
                let decl = self.find_export(&self.resolve_types(&raw.file, spec)?, name)?;
                let owner =
                    if is_relative(spec) { owner.map(Into::into) } else { Some(spec.clone()) };
                (decl, owner)
            }
            (Local::Declared, Some(_)) => return None,
        };
        let class = ClassRef { decl, owner };
        match &raw.method {
            None => Some(class),
            Some(method) => {
                let module = self.module_with_providers(&class.decl, method)?;
                self.resolve_ref(&module, class.owner.as_deref(), depth + 1)
            }
        }
    }

    /// The NgModule the static method `method` of the class `decl` returns
    /// a `ModuleWithProviders` of: its return type's argument, or the
    /// `ngModule` of the object it returns.
    fn module_with_providers(&self, decl: &Decl, method: &str) -> Option<RawRef> {
        let parsed = self.parsed_file(&decl.file)?;
        parsed.with(|_, program, _| {
            let class = find_class(program, &decl.name)?;
            let function = class.body.body.iter().find_map(|element| match element {
                ClassElement::MethodDefinition(m)
                    if m.r#static && m.key.static_name().is_some_and(|key| key == method) =>
                {
                    Some(&m.value)
                }
                _ => None,
            })?;
            if let Some(return_type) = &function.return_type
                && let TSType::TSTypeReference(reference) = &return_type.type_annotation
                && type_name_ends_with(&reference.type_name, "ModuleWithProviders")
            {
                let argument = reference.type_arguments.as_ref()?.params.first()?;
                let TSType::TSTypeReference(module) = argument else { return None };
                return type_name_ref(&decl.file, &module.type_name);
            }
            let body = function.body.as_ref()?;
            let returned = body.statements.iter().rev().find_map(|stmt| match stmt {
                Statement::ReturnStatement(ret) => ret.argument.as_ref(),
                _ => None,
            })?;
            let mut refs = Vec::new();
            collect_refs(returned, &decl.file, &mut refs, 0)?;
            match <[RawRef; 1]>::try_from(refs) {
                Ok([module]) => Some(module),
                Err(_) => None,
            }
        })
    }

    /// What the declaration `decl` is to a module scope.
    fn declared(&self, decl: &Decl) -> Option<Rc<Declared>> {
        if let Some(declared) = self.scope.declared.borrow().get(decl) {
            return declared.clone();
        }
        let declared = self.parsed_file(&decl.file).and_then(|parsed| {
            parsed.with(|_, program, consts| read_declared(program, consts, decl)).map(Rc::new)
        });
        self.scope.declared.borrow_mut().insert(decl.clone(), declared.clone());
        declared
    }

    /// The classes `refs` name, with `const` lists flattened.
    fn resolve_refs(
        &self,
        refs: &[RawRef],
        owner: Option<&str>,
        depth: u16,
    ) -> Option<Vec<(ClassRef, Rc<Declared>)>> {
        if depth > MAX_EXPORT_CHAIN {
            return None;
        }
        let mut out = Vec::new();
        for raw in refs {
            let class = self.resolve_ref(raw, owner, 0)?;
            let declared = self.declared(&class.decl)?;
            if let Declared::List(items) = &*declared {
                if !self.scope.flattening.borrow_mut().insert(class.decl.clone()) {
                    return None;
                }
                let items = self.resolve_refs(items, class.owner.as_deref(), depth + 1);
                self.scope.flattening.borrow_mut().remove(&class.decl);
                out.extend(items?);
            } else {
                out.push((class, declared));
            }
        }
        Some(out)
    }

    /// The directives and pipes the NgModule `module` exports, with those of
    /// the modules it exports.
    fn exported_scope(&self, module: &ClassRef, exports: &[RawRef]) -> Option<Rc<[ScopeItem]>> {
        let key = (module.decl.clone(), module.owner.clone());
        if let Some(scope) = self.scope.exported.borrow().get(&key) {
            return scope.clone();
        }
        if !self.scope.visiting.borrow_mut().insert(module.decl.clone()) {
            return None;
        }
        let scope = (|| {
            let mut scope = Vec::new();
            for (class, declared) in self.resolve_refs(exports, module.owner.as_deref(), 0)? {
                match &*declared {
                    Declared::NgModule { exports, .. } => {
                        scope.extend(self.exported_scope(&class, exports)?.iter().cloned());
                    }
                    _ => scope.push(ScopeItem { kind: scope_kind(&declared)?, class }),
                }
            }
            Some(Rc::from(dedupe(scope)))
        })();
        self.scope.visiting.borrow_mut().remove(&module.decl);
        self.scope.exported.borrow_mut().insert(key, scope.clone());
        scope
    }

    /// `items` named for the compiled `file`.
    fn named_entries(&self, items: Vec<ScopeItem>, file: &str) -> Option<Vec<ScopeEntry>> {
        items
            .into_iter()
            .map(|item| {
                Some(ScopeEntry {
                    name: self.imported_name(&item.class, file)?,
                    kind: item.kind,
                    is_forward_reference: false,
                })
            })
            .collect()
    }

    /// How the compiled `file` names `class`: by its name when `file`
    /// declares it, else by its export from the module it was imported
    /// through, or from its own file by a relative path (ngtsc's
    /// `AbsoluteModuleStrategy` and `RelativePathStrategy`).
    fn imported_name(&self, class: &ClassRef, file: &str) -> Option<ImportedName> {
        if class.decl.file == file {
            return (class.decl.name != "default")
                .then(|| ImportedName::Local(class.decl.name.clone()));
        }
        if let Some(specifier) = &class.owner {
            let name = self
                .resolve_types(file, specifier)
                .and_then(|entry| self.export_name(&entry, &class.decl));
            if let Some(name) = name {
                return Some(ImportedName::Module { specifier: specifier.clone(), name });
            }
            // A workspace library's source, by path, when its entry point
            // doesn't export the class.
            if in_node_modules(&class.decl.file) {
                return None;
            }
        }
        let name = self.export_name(&class.decl.file, &class.decl)?;
        // The file's own import of that file, as it's written
        // (`./dep.js`), else a relative path like ngtsc's.
        let imported = self
            .file_exports(file)?
            .import_specs
            .iter()
            .find(|spec| {
                is_relative(spec)
                    && self.resolve_types(file, spec).as_ref() == Some(&class.decl.file)
            })
            .cloned();
        if let Some(specifier) = imported {
            return Some(ImportedName::Module { specifier, name });
        }
        let from_dir = Path::new(file).parent()?;
        let specifier = self.make_relative_path(from_dir, Path::new(&class.decl.file))?;
        // The JavaScript file TypeScript writes for it: `./a.mts` is loaded
        // as `./a.mjs`.
        let specifier = [
            (".d.mts", ".mjs"),
            (".mts", ".mjs"),
            (".d.cts", ".cjs"),
            (".cts", ".cjs"),
            (".d", ""),
        ]
        .iter()
        .find_map(|(ts, js)| specifier.strip_suffix(ts).map(|path| format!("{path}{js}")))
        .unwrap_or(specifier);
        Some(ImportedName::Module { specifier, name })
    }

    /// The name the module `file` exports `decl` by, preferring the
    /// declaration's own name, like ngtsc's `AbsoluteModuleStrategy`.
    fn export_name(&self, file: &str, decl: &Decl) -> Option<String> {
        let cached = self.scope.export_names.borrow().get(file).cloned();
        if let Some(names) = cached {
            return names.get(decl).cloned();
        }
        let mut all = Vec::new();
        self.all_export_names(file, &mut FxHashSet::default(), &mut all, 0);
        let mut names: FxHashMap<Decl, String> = FxHashMap::default();
        for name in all {
            let Some(found) = self.find_export(file, &name) else { continue };
            if found.name == name || !names.contains_key(&found) {
                names.insert(found, name);
            }
        }
        let found = names.get(decl).cloned();
        self.scope.export_names.borrow_mut().insert(file.to_string(), Rc::new(names));
        found
    }

    /// Every name `file` exports, with those of its `export *` modules.
    fn all_export_names(
        &self,
        file: &str,
        visited: &mut FxHashSet<String>,
        out: &mut Vec<String>,
        depth: u16,
    ) {
        if depth > MAX_EXPORT_CHAIN || !visited.insert(file.to_string()) {
            return;
        }
        let Some(exports) = self.file_exports(file) else { return };
        out.extend(exports.names.iter().cloned());
        for spec in &exports.stars {
            if let Some(target) = self.resolve_types(file, spec) {
                let start = out.len();
                self.all_export_names(&target, visited, out, depth + 1);
                // `export *` leaves out `default`.
                let mut index = start;
                while index < out.len() {
                    if out[index] == "default" {
                        out.remove(index);
                    } else {
                        index += 1;
                    }
                }
            }
        }
    }
}

/// The exports and top-level bindings of `program`. Type-only ones are left
/// out: no scope names an interface.
fn collect_file_exports(program: &Program<'_>) -> FileExports {
    let mut exports = FileExports::default();
    let export = |exports: &mut FileExports, name: &str, how: Export| {
        if !exports.named.contains_key(name) {
            exports.names.push(name.to_string());
            exports.named.insert(name.to_string(), how);
        }
    };
    let declare = |exports: &mut FileExports, declaration: &Declaration<'_>| -> Vec<String> {
        let mut names = Vec::new();
        match declaration {
            Declaration::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    names.push(id.name.to_string());
                }
            }
            Declaration::VariableDeclaration(vars) => {
                for var in &vars.declarations {
                    for id in var.id.get_binding_identifiers() {
                        names.push(id.name.to_string());
                    }
                }
            }
            _ => {}
        }
        for name in &names {
            exports.locals.insert(name.clone(), Local::Declared);
        }
        names
    };
    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                let Some(spec) = import.source.value.as_str() else { continue };
                exports.import_specs.push(spec.to_string());
                // `import { type A }` is erased; `import {} from` and
                // `import 'x'` load the module.
                let types_only = import.import_kind.is_type()
                    || import.specifiers.as_ref().is_some_and(|specifiers| {
                        !specifiers.is_empty()
                            && specifiers.iter().all(|specifier| {
                                matches!(specifier, ImportDeclarationSpecifier::ImportSpecifier(s)
                                    if s.import_kind.is_type())
                            })
                    });
                if !types_only {
                    exports.loads.push(spec.to_string());
                }
                for specifier in import.specifiers.iter().flatten() {
                    let (local, imported) = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            (s.local.name.to_string(), Some(s.imported.name().to_string()))
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            (s.local.name.to_string(), Some("default".to_string()))
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                            (s.local.name.to_string(), None)
                        }
                    };
                    exports
                        .locals
                        .insert(local, Local::Import { spec: spec.to_string(), imported });
                }
            }
            Statement::ExportDeclaration(decl) => {
                for name in declare(&mut exports, &decl.declaration) {
                    export(&mut exports, &name, Export::Local(name.clone()));
                }
            }
            Statement::ExportNamedDeclaration(decl) if !decl.export_kind.is_type() => {
                for spec in decl.specifiers.iter().filter(|spec| !spec.export_kind.is_type()) {
                    let local = spec.local.name().to_string();
                    export(&mut exports, &spec.exported.name(), Export::Local(local));
                }
            }
            Statement::ExportFromDeclaration(decl) if !decl.export_kind.is_type() => {
                let Some(source) = decl.source.value.as_str() else { continue };
                if decl.specifiers.is_empty()
                    || decl.specifiers.iter().any(|spec| !spec.export_kind.is_type())
                {
                    exports.loads.push(source.to_string());
                }
                for spec in decl.specifiers.iter().filter(|spec| !spec.export_kind.is_type()) {
                    let how = Export::From {
                        spec: source.to_string(),
                        name: spec.local.name().to_string(),
                    };
                    export(&mut exports, &spec.exported.name(), how);
                }
            }
            Statement::ExportAllDeclaration(decl) if !decl.export_kind.is_type() => {
                if let Some(source) = decl.source.value.as_str() {
                    exports.loads.push(source.to_string());
                    // `export * as ns` names a namespace, which no scope
                    // reads a class through.
                    if decl.exported.is_none() {
                        exports.stars.push(source.to_string());
                    }
                }
            }
            Statement::ExportDefaultDeclaration(decl) => match &decl.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    let name =
                        class.id.as_ref().map_or("default".to_string(), |id| id.name.to_string());
                    exports.locals.insert(name.clone(), Local::Declared);
                    export(&mut exports, "default", Export::Local(name));
                }
                ExportDefaultDeclarationKind::Identifier(id) => {
                    export(&mut exports, "default", Export::Local(id.name.to_string()));
                }
                _ => {}
            },
            _ => {
                if let Some(declaration) = stmt.as_declaration() {
                    declare(&mut exports, declaration);
                }
            }
        }
    }
    exports
}

/// The top-level class `program` declares as `name` (`"default"` for an
/// anonymous `export default class`).
fn find_class<'p, 'a>(program: &'p Program<'a>, name: &str) -> Option<&'p Class<'a>> {
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
        let matches = match &class.id {
            Some(id) => id.name == name,
            None => name == "default",
        };
        matches.then_some(&**class)
    })
}

/// What the declaration `decl` of `program` is to a module scope: a class
/// by its decorator, or by the `ɵcmp`/`ɵdir`/`ɵpipe`/`ɵmod` type a
/// declaration file gives it, or a `const` list of references.
fn read_declared<'a>(
    program: &'a Program<'a>,
    consts: &StringConsts<'a>,
    decl: &Decl,
) -> Option<Declared> {
    if let Some(class) = find_class(program, &decl.name) {
        return read_declaration_type(class, &decl.file)
            .or_else(|| read_decorator(class, consts, &decl.file));
    }
    // `const LIST = [...]`
    program.body.iter().find_map(|stmt| {
        let declaration = match stmt {
            Statement::ExportDeclaration(decl) => &decl.declaration,
            _ => stmt.as_declaration()?,
        };
        let Declaration::VariableDeclaration(vars) = declaration else { return None };
        let var = vars.declarations.iter().find(|var| {
            var.id.get_binding_identifier().is_some_and(|id| id.name == decl.name.as_str())
        })?;
        if !vars.kind.is_const() {
            return None;
        }
        let mut refs = Vec::new();
        collect_refs(var.init.as_ref()?, &decl.file, &mut refs, 0)?;
        Some(Declared::List(refs))
    })
}

/// The scope a declaration file's `static ɵcmp`/`ɵdir`/`ɵpipe`/`ɵmod`
/// type gives `class`.
fn read_declaration_type(class: &Class<'_>, file: &str) -> Option<Declared> {
    class.body.body.iter().find_map(|element| {
        let ClassElement::PropertyDefinition(property) = element else { return None };
        if !property.r#static {
            return None;
        }
        let PropertyKey::StaticIdentifier(key) = &property.key else { return None };
        let TSType::TSTypeReference(reference) =
            &property.type_annotation.as_ref()?.type_annotation
        else {
            return None;
        };
        let arguments = &reference.type_arguments.as_ref()?.params;
        let string_at = |index: usize| match arguments.get(index) {
            Some(TSType::TSLiteralType(literal)) => match &literal.literal {
                TSLiteral::StringLiteral(s) => s.value.as_str().map(Into::into),
                _ => None,
            },
            _ => None,
        };
        match key.name.as_str() {
            "ɵcmp" => Some(Declared::Directive { selector: string_at(1), is_component: true }),
            "ɵdir" => Some(Declared::Directive { selector: string_at(1), is_component: false }),
            "ɵpipe" => Some(Declared::Pipe { name: string_at(1) }),
            "ɵmod" => {
                let list = |index: usize| type_list(arguments.get(index)?, file);
                Some(Declared::NgModule {
                    declarations: list(1)?,
                    imports: list(2)?,
                    exports: list(3)?,
                })
            }
            _ => None,
        }
    })
}

/// The classes a `[typeof A, typeof i1.B]` tuple type names; `never` names
/// none.
fn type_list(ty: &TSType<'_>, file: &str) -> Option<Vec<RawRef>> {
    match ty {
        TSType::TSNeverKeyword(_) => Some(Vec::new()),
        TSType::TSTupleType(tuple) => tuple
            .element_types
            .iter()
            .map(|element| {
                let TSTupleElement::TSTypeQuery(query) = element else { return None };
                type_name_ref(file, query.expr_name.as_ts_type_name()?)
            })
            .collect(),
        _ => None,
    }
}

/// The class a type name refers to: `A`, or `i1.A` of a namespace import.
fn type_name_ref(file: &str, name: &TSTypeName<'_>) -> Option<RawRef> {
    let (namespace, name) = match name {
        TSTypeName::IdentifierReference(id) => (None, id.name.to_string()),
        TSTypeName::QualifiedName(qualified) => match &qualified.left {
            TSTypeName::IdentifierReference(ns) => {
                (Some(ns.name.to_string()), qualified.right.name.to_string())
            }
            _ => return None,
        },
        TSTypeName::ThisExpression(_) => return None,
    };
    Some(RawRef { file: file.into(), namespace, name, method: None })
}

/// Whether a type name is `name` or `ns.name`.
fn type_name_ends_with(type_name: &TSTypeName<'_>, name: &str) -> bool {
    match type_name {
        TSTypeName::IdentifierReference(id) => id.name == name,
        TSTypeName::QualifiedName(qualified) => qualified.right.name == name,
        TSTypeName::ThisExpression(_) => false,
    }
}

/// The scope the Angular decorator of a source `class` gives it.
fn read_decorator<'a>(
    class: &'a Class<'a>,
    consts: &StringConsts<'a>,
    file: &str,
) -> Option<Declared> {
    let (decorator, kind) =
        ["Component", "Directive", "Pipe", "NgModule"].iter().find_map(|kind| {
            find_angular_class_decorator(&class.decorators, kind, consts).map(|d| (d, *kind))
        })?;
    let Expression::CallExpression(call) = &decorator.expression else { return None };
    let object = match call.arguments.first() {
        None => None,
        Some(argument) => match argument.as_expression()?.get_inner_expression() {
            Expression::ObjectExpression(object) => Some(object),
            _ => return None,
        },
    };
    // A spread or a computed key can set any field.
    let mut fields: Vec<(std::borrow::Cow<'_, str>, &Expression<'a>)> = Vec::new();
    for property in object.iter().flat_map(|object| &object.properties) {
        let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
        fields.push((property.key.static_name()?.into_cow_str()?, &property.value));
    }
    let field = |key: &str| fields.iter().rev().find(|(name, _)| name == key).map(|(_, v)| *v);
    let string = |key: &str| {
        field(key).and_then(|value| Evaluator::new(consts).evaluate(value).as_str().map(Into::into))
    };
    match kind {
        "Component" => {
            Some(Declared::Directive { selector: string("selector"), is_component: true })
        }
        "Directive" => {
            Some(Declared::Directive { selector: string("selector"), is_component: false })
        }
        "Pipe" => Some(Declared::Pipe { name: string("name") }),
        _ => {
            let list = |key: &str| -> Option<Vec<RawRef>> {
                let mut refs = Vec::new();
                if let Some(value) = field(key) {
                    collect_refs(value, file, &mut refs, 0)?;
                }
                Some(refs)
            };
            Some(Declared::NgModule {
                declarations: list("declarations")?,
                imports: list("imports")?,
                exports: list("exports")?,
            })
        }
    }
}

/// Collect the references `expr` lists into `out`: a class (`A`, `ns.A`,
/// `forwardRef(() => A)`), a `ModuleWithProviders` (`A.forRoot()`,
/// `{ ngModule: A }`), or an array of those, spread or nested. `None` for
/// anything else.
fn collect_refs(
    expr: &Expression<'_>,
    file: &str,
    out: &mut Vec<RawRef>,
    depth: u16,
) -> Option<()> {
    if depth > MAX_EXPORT_CHAIN {
        return None;
    }
    let raw = |namespace: Option<&str>, name: &str, method: Option<&str>| RawRef {
        file: file.into(),
        namespace: namespace.map(Into::into),
        name: name.into(),
        method: method.map(Into::into),
    };
    match expr.get_inner_expression() {
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                match element {
                    ArrayExpressionElement::SpreadElement(spread) => {
                        collect_refs(&spread.argument, file, out, depth + 1)?;
                    }
                    ArrayExpressionElement::Elision(_) => return None,
                    _ => collect_refs(element.as_expression()?, file, out, depth + 1)?,
                }
            }
        }
        Expression::Identifier(id) => out.push(raw(None, &id.name, None)),
        Expression::StaticMemberExpression(member) => {
            let Expression::Identifier(object) = member.object.get_inner_expression() else {
                return None;
            };
            out.push(raw(Some(&object.name), &member.property.name, None));
        }
        Expression::CallExpression(call) => match call.callee.get_inner_expression() {
            // `forwardRef(() => A)`
            Expression::Identifier(callee) if callee.name == "forwardRef" => {
                let returned = match call.arguments.first()?.as_expression()?.get_inner_expression()
                {
                    Expression::ArrowFunctionExpression(arrow) => arrow.get_expression()?,
                    Expression::FunctionExpression(function) => {
                        match function.body.as_ref()?.statements.as_slice() {
                            [Statement::ReturnStatement(ret)] => ret.argument.as_ref()?,
                            _ => return None,
                        }
                    }
                    _ => return None,
                };
                collect_refs(returned, file, out, depth + 1)?;
            }
            // `A.forRoot()`, `ns.A.forRoot()`
            Expression::StaticMemberExpression(member) => {
                let method = member.property.name.as_str();
                match member.object.get_inner_expression() {
                    Expression::Identifier(class) => out.push(raw(None, &class.name, Some(method))),
                    Expression::StaticMemberExpression(class) => {
                        let Expression::Identifier(ns) = class.object.get_inner_expression() else {
                            return None;
                        };
                        out.push(raw(Some(&ns.name), &class.property.name, Some(method)));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        },
        Expression::ObjectExpression(object) => {
            let module = object.properties.iter().find_map(|property| match property {
                ObjectPropertyKind::ObjectProperty(p)
                    if p.key.static_name().is_some_and(|key| key == "ngModule") =>
                {
                    Some(&p.value)
                }
                _ => None,
            })?;
            collect_refs(module, file, out, depth + 1)?;
        }
        _ => return None,
    }
    Some(())
}

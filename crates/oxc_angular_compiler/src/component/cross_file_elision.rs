//! Cross-file type-only export detection and imported-value resolution using
//! oxc_resolver and oxc_parser.
//!
//! This module resolves import paths to actual files and analyzes their exports:
//!
//! - `is_type_only_import` determines if exports are type-only (interfaces,
//!   type aliases) or have runtime values, improving `ImportElisionAnalyzer`'s
//!   accuracy — compare-test machinery, since bundlers handle elision in
//!   production.
//! - `resolve_export_value` evaluates exported `const` initializers the way
//!   ngtsc's program-wide checker does, feeding the decorator metadata
//!   evaluator (`TransformOptions::resolve_imported_values`). A value that
//!   can't be resolved or read statically resolves to `None`, and the caller
//!   falls back to the opaque import reference — unchanged behavior.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Class, Declaration, ExportDefaultDeclarationKind, Expression, Program,
    Statement, TSNamespaceDeclarationBody,
};
use oxc_parser::Parser;
use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use oxc_span::SourceType;
use rustc_hash::{FxHashMap, FxHashSet};

use super::defer_resolver::{ClassUse, decorated_class_selector};
use crate::directive::{ImportValueResolver, StringConsts};

mod ng_module_scope;

/// `(file, exported_name, local_name, implicit_standalone)` -> how a
/// template uses the class (see `Inner::selector_memo`).
type SelectorMemo = FxHashMap<(String, String, String, bool), Option<ClassUse>>;

/// Result of analyzing a file's exports
#[derive(Debug, Clone)]
pub struct ExportInfo {
    /// Whether this export is type-only (interface, type alias)
    pub is_type_only: bool,
    /// If re-export, the source module and original name
    pub re_export_source: Option<(String, String)>,
    /// The local binding this export's value is defined by: its own name for
    /// `export const X = …`, the local name for `export { Y as X }`, and
    /// `"default"` for `export default <expr>` (evaluated lazily, since
    /// `evaluate_name("default")` can't). `None` for types and re-exports.
    ///
    /// Values are evaluated on demand in [`Inner::eval_export`] rather than
    /// during `analyze_file`: eager evaluation cached `None` for exports that
    /// hit a file already in `analyzing`, making the result depend on which
    /// file happened to be read first.
    pub(crate) value_local: Option<String>,
    /// A top-level `export class` or `export function`: the evaluator reads
    /// it as a reference, which `StaticValue` can't hold, so
    /// [`Inner::export_value_at`] answers `None` without evaluating it.
    pub(crate) never_static: bool,
}

/// Bounds a re-export chain (`export { X } from ...`, `export *`) followed
/// while resolving a value; deeper chains stay opaque.
const MAX_EXPORT_CHAIN: u16 = 64;

/// Cross-file analyzer for detecting type-only exports and reading exported
/// values.
///
/// This analyzer resolves import paths to actual files, parses them, and
/// determines if exports are type-only (interfaces, type aliases) or have
/// runtime values (classes, functions, variables). The state lives in an
/// `Rc<Inner>` so [`CrossFileResolver`]s handed to the metadata evaluator can
/// outlive the borrow of this analyzer; interior mutability lets the
/// evaluator reenter: evaluating a file's exports can require the values of
/// files it imports.
pub struct CrossFileAnalyzer {
    inner: Rc<Inner>,
}

struct Inner {
    resolver: Resolver,
    /// Cache of file path -> export analysis
    cache: RefCell<FxHashMap<String, FxHashMap<String, ExportInfo>>>,
    /// Files currently being analyzed (for circular import detection)
    analyzing: RefCell<FxHashSet<String>>,
    /// Files parsed for `eval_export` and `class_selector`, so each is
    /// parsed once, on first use. `analyze_file` doesn't keep its parse.
    parsed: RefCell<FxHashMap<String, Rc<ParsedFile>>>,
    /// Memoized per-export value evaluations: `(file, local_name)` -> value.
    /// `None` means evaluated-but-opaque.
    value_memo: RefCell<FxHashMap<(String, String), Option<crate::directive::StaticValue>>>,
    /// Memoized standalone class selectors: `(file, exported_name, local_name,
    /// implicit_standalone)` -> selector (see
    /// [`Self::standalone_directive_selector`]).
    selector_memo: RefCell<SelectorMemo>,
    /// Exports whose evaluation is in progress (circular `const` chains —
    /// `a.A = b.B; b.B = a.A` — resolve the cycle edge to opaque, like
    /// `analyzing` does for files).
    evaluating_values: RefCell<FxHashSet<(String, String)>>,
    /// Files read to resolve exported values, so build tools can watch them.
    value_dependencies: RefCell<FxHashSet<String>>,
    /// Handle to this state, for the resolvers of the files in `parsed`
    /// (`parse_file` only sees `&self`).
    this: Weak<Inner>,
    /// Memoized NgModule scope reads (see [`ng_module_scope`]).
    scope: ng_module_scope::ScopeState,
}

/// A file parsed once per analyzer: its program, and the consts the
/// evaluator reads it through, which resolve its imports with a
/// [`ParsedFileResolver`].
struct ParsedFile {
    // `consts` and `program` borrow `allocator`, which also holds the source
    // text. They're declared first so they drop first; the allocator is boxed
    // so it stays put when `ParsedFile` moves.
    consts: StringConsts<'static>,
    program: &'static Program<'static>,
    allocator: Box<Allocator>,
}

impl ParsedFile {
    /// Run `f` on the file's allocator, program and consts. `f` works for
    /// any lifetime, so nothing borrowed from them outlives `self`.
    fn with<R>(
        &self,
        f: impl for<'a> FnOnce(&'a Allocator, &'a Program<'a>, &'a StringConsts<'a>) -> R,
    ) -> R {
        // SAFETY: `program` and `consts` only borrow `self.allocator`, which
        // lives as long as `self`, so they're valid for the borrow of `self`.
        // `StringConsts` is invariant over its lifetime (its caches are
        // `RefCell`s), hence the cast; `f` can only store data borrowed for
        // that same lifetime in them, i.e. from the allocator.
        let consts = unsafe { &*std::ptr::from_ref(&self.consts).cast::<StringConsts<'_>>() };
        f(&self.allocator, self.program, consts)
    }
}

/// The [`ImportValueResolver`] of a file in [`Inner::parsed`]: a
/// [`CrossFileResolver`] holding a `Weak` handle, so the cache doesn't keep
/// the analyzer alive.
struct ParsedFileResolver {
    inner: Weak<Inner>,
    from_file: PathBuf,
}

impl ImportValueResolver for ParsedFileResolver {
    fn resolve(&self, module: &str, name: &str) -> Option<crate::directive::StaticValue> {
        self.inner.upgrade()?.resolve_export_value(module, name, &self.from_file)
    }

    fn exports(
        &self,
        module: &str,
    ) -> Option<Vec<(String, Option<crate::directive::StaticValue>)>> {
        self.inner.upgrade()?.export_values_of(module, &self.from_file)
    }
}

/// An [`ImportValueResolver`] bound to the file whose imports it resolves:
/// `module` specifiers are resolved relative to `from_file`'s directory,
/// exactly where the importing file's compiler would look.
pub(crate) struct CrossFileResolver {
    inner: Rc<Inner>,
    from_file: PathBuf,
}

impl ImportValueResolver for CrossFileResolver {
    fn resolve(&self, module: &str, name: &str) -> Option<crate::directive::StaticValue> {
        self.inner.resolve_export_value(module, name, &self.from_file)
    }

    fn exports(
        &self,
        module: &str,
    ) -> Option<Vec<(String, Option<crate::directive::StaticValue>)>> {
        self.inner.export_values_of(module, &self.from_file)
    }
}

/// The [`ClassUse`] of the class `local` declares in `program` (`"default"`
/// for `export default class`) and exports as `exported`, when it is a
/// standalone `@Component`, `@Directive` or `@Pipe` whose `selector` (or
/// `name`) and `standalone` are literals. `consts` are the program's.
fn standalone_class_selector<'a>(
    program: &'a Program<'a>,
    consts: &StringConsts<'a>,
    exported: &str,
    local: &str,
    implicit_standalone: bool,
) -> Option<ClassUse> {
    // A plain `class Local` must be exported as `exported` by a local
    // `export { Local as exported }`: the export map also records namespace
    // members, which can share the name of a top-level class the file doesn't
    // export (or exports under another name).
    let exported_locally = program.body.iter().any(|stmt| match stmt {
        Statement::ExportNamedDeclaration(decl) if !decl.export_kind.is_type() => {
            decl.specifiers.iter().any(|spec| {
                !spec.export_kind.is_type()
                    && spec.local.name() == local
                    && spec.exported.name() == exported
            })
        }
        _ => false,
    });
    let is_local = |class: &Class| class.id.as_ref().is_some_and(|id| id.name == local);
    let class = program.body.iter().find_map(|stmt| match stmt {
        Statement::ClassDeclaration(class) if exported_locally && is_local(class) => Some(&**class),
        Statement::ExportDeclaration(decl) => match &decl.declaration {
            Declaration::ClassDeclaration(class) if is_local(class) => Some(&**class),
            _ => None,
        },
        Statement::ExportDefaultDeclaration(decl) if local == "default" => {
            match &decl.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(&**class),
                _ => None,
            }
        }
        _ => None,
    })?;
    decorated_class_selector(class, consts, implicit_standalone)
}

impl CrossFileAnalyzer {
    /// Create a new cross-file analyzer.
    ///
    /// # Arguments
    ///
    /// * `base_dir` - The base directory for module resolution
    /// * `tsconfig_path` - Optional path to tsconfig.json for path aliases
    /// * `_values_enabled` - Unused: files are parsed on demand either way. Was: enable exported-value resolution (the
    ///   `resolve_imported_values` transform option)
    pub fn new(_base_dir: &Path, tsconfig_path: Option<&Path>, _values_enabled: bool) -> Self {
        let options = ResolveOptions {
            extensions: vec![".ts".into(), ".tsx".into(), ".js".into(), ".jsx".into()],
            tsconfig: tsconfig_path.map(|p| {
                TsconfigDiscovery::Manual(TsconfigOptions {
                    config_file: p.to_path_buf(),
                    references: TsconfigReferences::Auto,
                })
            }),
            ..Default::default()
        };

        Self {
            inner: Rc::new_cyclic(|this| Inner {
                resolver: Resolver::new(options),
                cache: RefCell::new(FxHashMap::default()),
                analyzing: RefCell::new(FxHashSet::default()),
                parsed: RefCell::new(FxHashMap::default()),
                value_memo: RefCell::new(FxHashMap::default()),
                selector_memo: RefCell::new(FxHashMap::default()),
                evaluating_values: RefCell::new(FxHashSet::default()),
                value_dependencies: RefCell::new(FxHashSet::default()),
                this: this.clone(),
                scope: ng_module_scope::ScopeState::default(),
            }),
        }
    }

    /// Check if an import is type-only by analyzing the source file.
    ///
    /// Returns `true` if the export is definitely type-only (interface, type alias).
    /// Returns `false` if the export has a runtime value or cannot be determined.
    pub fn is_type_only_import(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
    ) -> bool {
        self.inner.is_type_only_import(import_source, import_name, from_file)
    }

    /// Resolve the actual source file path for an import, tracing through barrel exports.
    ///
    /// Returns the relative path from `from_file` to the actual source file where
    /// the export is defined, or `None` for unresolvable/package imports.
    pub fn resolve_import_source_path(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
    ) -> Option<String> {
        self.inner.resolve_import_source_path(import_source, import_name, from_file)
    }

    /// The static value `import_name` is exported with from `import_source`,
    /// as `import_source` is written in `from_file`, following re-export
    /// chains. `None` when unresolvable — the caller keeps the opaque import
    /// reference. Files read are recorded in [`Self::take_value_dependencies`].
    ///
    /// `CrossFileResolver` calls `Inner::resolve_export_value` directly; this
    /// wrapper exists for tests.
    #[cfg(test)]
    pub(crate) fn resolve_export_value(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
    ) -> Option<crate::directive::StaticValue> {
        self.inner.resolve_export_value(import_source, import_name, from_file)
    }

    /// The selector of the standalone `@Component` or `@Directive` class (or
    /// the name of the standalone `@Pipe`) that `import_name` is exported as
    /// from `import_source`, as `import_source` is written in `from_file`,
    /// following named re-exports. Used to tell whether a template uses the
    /// class, for `@defer` (#313). It comes
    /// with the file that declares the class and the class's name there,
    /// which tell two references to one class apart from two classes.
    ///
    /// `None` when the import can't be followed to a class declared in a
    /// source file (packages, `export *`, unresolvable paths), the class isn't
    /// a standalone component or directive, or its `selector` or `standalone`
    /// isn't a literal: the caller keeps the import eager. Files read are
    /// recorded in [`Self::take_value_dependencies`].
    pub(crate) fn standalone_directive_selector(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
        implicit_standalone: bool,
    ) -> Option<(ClassUse, (String, String))> {
        self.inner.standalone_directive_selector(
            import_source,
            import_name,
            from_file,
            implicit_standalone,
        )
    }

    /// The resolver the decorator evaluator reads `from_file`'s imported
    /// bindings through (see `TransformOptions::resolve_imported_values`).
    pub(crate) fn value_resolver(&self, from_file: PathBuf) -> CrossFileResolver {
        CrossFileResolver { inner: Rc::clone(&self.inner), from_file }
    }

    /// The files read to resolve exported values since the last call, for
    /// `TransformResult::dependencies` / watch wiring.
    pub(crate) fn take_value_dependencies(&self) -> FxHashSet<String> {
        self.inner.take_value_dependencies()
    }

    /// Clear the analysis cache.
    pub fn clear_cache(&mut self) {
        self.inner.clear_cache();
    }

    /// Get the number of cached files.
    pub fn cache_size(&self) -> usize {
        self.inner.cache_size()
    }
}

impl Inner {
    /// Check if an import is type-only by analyzing the source file.
    ///
    /// Returns `true` if the export is definitely type-only (interface, type alias).
    /// Returns `false` if the export has a runtime value or cannot be determined.
    ///
    /// # Arguments
    ///
    /// * `import_source` - The import source path (e.g., "./types", "@angular/core")
    /// * `import_name` - The name being imported (e.g., "User", "Component")
    /// * `from_file` - The file containing the import statement
    pub fn is_type_only_import(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
    ) -> bool {
        // Resolve the import path to a file
        let resolved = match from_file.parent() {
            Some(parent) => match self.resolver.resolve(parent, import_source) {
                Ok(resolution) => {
                    let full_path = resolution.full_path();
                    // Skip node_modules - pre-compiled packages don't have interface
                    // declarations visible in their source. Assume value (conservative).
                    if full_path.components().any(|c| c.as_os_str() == "node_modules") {
                        return false;
                    }
                    full_path.to_string_lossy().to_string()
                }
                Err(_) => return false, // Cannot resolve - assume value (conservative)
            },
            None => return false,
        };

        // Ensure the file is analyzed (if not already cached)
        if !self.cache.borrow().contains_key(&resolved) {
            // Circular import protection
            if self.analyzing.borrow().contains(&resolved) {
                return false;
            }
            self.analyze_file(&resolved);
        }

        // Check the result from cache
        self.check_export_is_type_only(&resolved, import_name)
    }

    /// Resolve the actual source file path for an import, tracing through barrel exports.
    ///
    /// This is useful for resolving imports like `import { Component } from './index'`
    /// where `./index.ts` contains `export { Component } from './component'`.
    ///
    /// Returns the relative path from `from_file` to the actual source file where
    /// the export is defined.
    ///
    /// # Arguments
    ///
    /// * `import_source` - The import source path (e.g., "./index", "@angular/core")
    /// * `import_name` - The name being imported (e.g., "Component", "User")
    /// * `from_file` - The file containing the import statement
    ///
    /// # Returns
    ///
    /// `Some(path)` if the import can be traced to its source, where `path` is the
    /// relative path from `from_file`'s directory to the source file.
    /// `None` if the import cannot be resolved or is a package import.
    pub fn resolve_import_source_path(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
    ) -> Option<String> {
        // Skip package imports - we cannot resolve these
        if import_source.starts_with('@') || !import_source.starts_with('.') {
            return None;
        }

        // Resolve the initial import path to a file
        let resolved = from_file.parent().and_then(|parent| {
            self.resolver.resolve(parent, import_source).ok().map(|r| r.full_path().to_path_buf())
        })?;

        // Ensure the file is analyzed
        let resolved_str = resolved.to_string_lossy().to_string();
        if !self.cache.borrow().contains_key(&resolved_str) {
            if self.analyzing.borrow().contains(&resolved_str) {
                return None; // Circular import
            }
            self.analyze_file(&resolved_str);
        }

        // Trace through re-exports to find the actual source
        let source_path = self.trace_export_source(&resolved, import_name)?;

        // Calculate relative path from from_file's directory to the source
        let from_dir = from_file.parent()?;
        self.make_relative_path(from_dir, &source_path)
    }

    /// The static value `import_name` is exported with from `import_source`,
    /// as `import_source` is written in `from_file`, following re-export
    /// chains like `is_type_only_import` does. `None` when the import can't
    /// be resolved, the export doesn't exist, or its value isn't statically
    /// analyzable — the caller keeps the opaque import reference either way.
    ///
    /// Files read this way are recorded in [`Self::take_value_dependencies`].
    pub(crate) fn resolve_export_value(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
    ) -> Option<crate::directive::StaticValue> {
        // Pre-compiled packages and files without a parent directory stay
        // opaque, like `is_type_only_import`.
        let parent = from_file.parent()?;
        let resolved = self.resolver.resolve(parent, import_source).ok()?;
        let resolved_path = resolved.full_path();
        if resolved_path.components().any(|c| c.as_os_str() == "node_modules") {
            return None;
        }
        let resolved = resolved_path.to_string_lossy().to_string();
        self.value_dependencies.borrow_mut().insert(resolved.clone());
        if !self.cache.borrow().contains_key(&resolved) {
            if self.analyzing.borrow().contains(&resolved) {
                return None; // Circular import
            }
            self.analyze_file(&resolved);
        }
        self.export_value_at(&resolved, import_name, 0)
    }

    fn standalone_directive_selector(
        &self,
        import_source: &str,
        import_name: &str,
        from_file: &Path,
        implicit_standalone: bool,
    ) -> Option<(ClassUse, (String, String))> {
        let mut file = self.resolve_module_spec(from_file.parent()?, import_source)?;
        let mut name = import_name.to_string();
        for _ in 0..MAX_EXPORT_CHAIN {
            if Path::new(&file).components().any(|c| c.as_os_str() == "node_modules") {
                return None;
            }
            self.value_dependencies.borrow_mut().insert(file.clone());
            if !self.cache.borrow().contains_key(&file) {
                if self.analyzing.borrow().contains(&file) {
                    return None; // Circular import
                }
                self.analyze_file(&file);
            }
            let info = self.cache.borrow().get(&file)?.get(&name).cloned()?;
            match info.re_export_source {
                None => {
                    let local = info.value_local?;
                    let selector =
                        self.class_selector(&file, &name, &local, implicit_standalone)?;
                    return Some((selector, (file, local)));
                }
                Some((source, original)) => {
                    if original == "*" {
                        return None;
                    }
                    file = self.resolve_module_spec(Path::new(&file).parent()?, &source)?;
                    name = original;
                }
            }
        }
        None
    }

    /// [`standalone_class_selector`] memoized per `(file, exported, local)`,
    /// reading the file through [`Self::parsed_file`].
    fn class_selector(
        &self,
        file_path: &str,
        exported: &str,
        local: &str,
        implicit_standalone: bool,
    ) -> Option<ClassUse> {
        let key =
            (file_path.to_string(), exported.to_string(), local.to_string(), implicit_standalone);
        if let Some(cached) = self.selector_memo.borrow().get(&key) {
            return cached.clone();
        }
        let selector = self.parsed_file(file_path).and_then(|file| {
            file.with(|_, program, consts| {
                standalone_class_selector(program, consts, exported, local, implicit_standalone)
            })
        });
        self.selector_memo.borrow_mut().insert(key, selector.clone());
        selector
    }

    /// Every value `import_source` exports, for `{ ...ns }` with
    /// `import * as ns` (ngtsc's `ResolvedModule.getExports()`): each name
    /// with the value it resolves to, or `None` for one that can't be read
    /// statically — ngtsc keeps those as `DynamicValue`s in the object rather
    /// than dropping the key. `None` for the whole map when the module can't
    /// be resolved, like [`Self::resolve_export_value`].
    fn export_values_of(
        &self,
        import_source: &str,
        from_file: &Path,
    ) -> Option<Vec<(String, Option<crate::directive::StaticValue>)>> {
        let parent = from_file.parent()?;
        let resolved = self.resolver.resolve(parent, import_source).ok()?;
        let resolved_path = resolved.full_path();
        if resolved_path.components().any(|c| c.as_os_str() == "node_modules") {
            return None;
        }
        let resolved = resolved_path.to_string_lossy().to_string();
        self.value_dependencies.borrow_mut().insert(resolved.clone());
        if !self.cache.borrow().contains_key(&resolved) {
            if self.analyzing.borrow().contains(&resolved) {
                return None; // Circular import
            }
            self.analyze_file(&resolved);
        }
        let mut names = Vec::new();
        self.export_names_at(&resolved, &mut FxHashSet::default(), &mut names);
        Some(
            names
                .into_iter()
                .map(|name| {
                    let value = self.export_value_at(&resolved, &name, 0);
                    (name, value)
                })
                .collect(),
        )
    }

    /// The names `file_path` exports, direct ones first — the first wins when
    /// an `export *` re-exports a name the file also has (`getExportsOfModule`
    /// answers one symbol per name) — then each star export's. `visited`
    /// guards circular `export *` chains.
    fn export_names_at(
        &self,
        file_path: &str,
        visited: &mut FxHashSet<String>,
        names: &mut Vec<String>,
    ) {
        if !visited.insert(file_path.to_string()) {
            return;
        }
        let Some(exports) = self.cache.borrow().get(file_path).cloned() else {
            return;
        };
        // `export *` sources, stored under their `*:` keys (see
        // `find_in_star_exports_inner`). `export * as ns` carries the same
        // `(source, "*")` marker but exports `ns` as one name — it isn't
        // enumerated flat.
        let star_sources: Vec<String> = exports
            .iter()
            .filter(|(key, _)| key.starts_with("*:"))
            .filter_map(|(_, info)| {
                info.re_export_source.as_ref().map(|(source, _)| source.clone())
            })
            .collect();
        for name in exports.keys() {
            if !name.starts_with("*:") && !names.contains(name) {
                names.push(name.clone());
            }
        }
        let Some(file_parent) = Path::new(file_path).parent() else {
            return;
        };
        for source in star_sources {
            let Some(resolved) = self.resolve_module_spec(file_parent, &source) else {
                continue;
            };
            self.value_dependencies.borrow_mut().insert(resolved.clone());
            if !self.cache.borrow().contains_key(&resolved) {
                if self.analyzing.borrow().contains(&resolved) {
                    continue;
                }
                self.analyze_file(&resolved);
            }
            self.export_names_at(&resolved, visited, names);
        }
    }

    /// The value `export_name` binds to in `file_path`, direct or through
    /// re-exports. Bounded by [`MAX_EXPORT_CHAIN`] for circular chains.
    fn export_value_at(
        &self,
        file_path: &str,
        export_name: &str,
        depth: u16,
    ) -> Option<crate::directive::StaticValue> {
        if depth > MAX_EXPORT_CHAIN {
            return None;
        }
        let export_info = {
            let cache = self.cache.borrow();
            cache.get(file_path)?.get(export_name).cloned()
        };
        let export_info =
            export_info.or_else(|| self.find_in_star_exports(file_path, export_name))?;

        // A re-export (`export { X } from`, `export *`): follow the chain in
        // the file it points at, like `check_export_is_type_only` does.
        if let Some((source_module, original_name)) = export_info.re_export_source {
            // `export * as ns`/`export *` entries can't name one value.
            if original_name == "*" {
                return None;
            }
            let parent = Path::new(file_path).parent()?;
            let resolved = self.resolve_module_spec(parent, &source_module)?;
            self.value_dependencies.borrow_mut().insert(resolved.clone());
            if !self.cache.borrow().contains_key(&resolved) {
                if self.analyzing.borrow().contains(&resolved) {
                    return None;
                }
                self.analyze_file(&resolved);
            }
            return self.export_value_at(&resolved, &original_name, depth + 1);
        }

        // A local export: evaluate its binding lazily. `local` is `"default"`
        // for `export default <expr>`.
        if export_info.never_static {
            return None;
        }
        self.eval_export(file_path, &export_info.value_local?)
    }

    /// The static value `local` is defined with in `file_path`, evaluating
    /// its initializer on demand (memoized per `(file, local)`). Circular
    /// `const` chains end at the [`Self::evaluating_values`] edge instead of
    /// making earlier exports opaque, which is why evaluation happens here
    /// and not eagerly in `analyze_file`.
    fn eval_export(&self, file_path: &str, local: &str) -> Option<crate::directive::StaticValue> {
        let key = (file_path.to_string(), local.to_string());
        if let Some(cached) = self.value_memo.borrow().get(&key) {
            return cached.clone();
        }
        if !self.evaluating_values.borrow_mut().insert(key.clone()) {
            return None; // const cycle back to this export
        }
        let value = self.eval_export_uncached(file_path, local);
        self.evaluating_values.borrow_mut().remove(&key);
        self.value_memo.borrow_mut().insert(key, value.clone());
        value
    }

    /// [`Self::eval_export`] without the memo: evaluate the named binding in
    /// the parsed file.
    fn eval_export_uncached(
        &self,
        file_path: &str,
        local: &str,
    ) -> Option<crate::directive::StaticValue> {
        let file = self.parsed_file(file_path)?;
        file.with(|allocator, program, consts| {
            let evaluator = crate::directive::Evaluator::new(consts);
            if local == "default" {
                return self
                    .find_default_expr(&program.body)
                    .and_then(|expr| evaluator.evaluate(expr).as_static());
            }
            evaluator.evaluate_name(allocator.alloc_str(local)).as_static()
        })
    }

    /// `file_path` parsed, from [`Self::parsed`], or read and parsed now.
    /// `None` when the file can't be read.
    fn parsed_file(&self, file_path: &str) -> Option<Rc<ParsedFile>> {
        if let Some(file) = self.parsed.borrow().get(file_path) {
            return Some(Rc::clone(file));
        }
        let source = std::fs::read_to_string(file_path).ok()?;
        let file = Rc::new(self.parse_file(file_path, &source));
        self.parsed.borrow_mut().insert(file_path.to_string(), Rc::clone(&file));
        Some(file)
    }

    /// Parse `source`, the text of `file_path`, with consts whose imports
    /// resolve from `file_path`.
    fn parse_file(&self, file_path: &str, source: &str) -> ParsedFile {
        let allocator = Box::new(Allocator::default());
        // SAFETY: the boxed allocator outlives the program and consts that
        // borrow it (see `ParsedFile`), and `ParsedFile::with` only lends
        // them for the borrow of the `ParsedFile`.
        let arena: &'static Allocator = unsafe { &*std::ptr::from_ref::<Allocator>(&*allocator) };
        let source = arena.alloc_str(source);
        let source_type = SourceType::from_path(file_path).unwrap_or_default();
        let program = arena.alloc(Parser::new(arena, source, source_type).parse().program);
        let resolver = ParsedFileResolver {
            inner: Weak::clone(&self.this),
            from_file: Path::new(file_path).to_path_buf(),
        };
        let consts = crate::directive::collect_string_consts(arena, program)
            .with_resolver(Rc::new(resolver));
        ParsedFile { consts, program, allocator }
    }

    /// The initializer expression of `export default <expr>`, searching the
    /// same declaration bodies `analyze_statement` walks.
    fn find_default_expr<'a>(&self, body: &'a [Statement<'a>]) -> Option<&'a Expression<'a>> {
        for stmt in body {
            match stmt {
                Statement::ExportDefaultDeclaration(decl) => {
                    return decl.declaration.as_expression();
                }
                Statement::TSExternalModuleDeclaration(module_decl) => {
                    if let Some(block) = &module_decl.body
                        && let Some(expr) = self.find_default_expr(&block.body)
                    {
                        return Some(expr);
                    }
                }
                Statement::TSGlobalDeclaration(global_decl) => {
                    if let Some(expr) = self.find_default_expr(&global_decl.body.body) {
                        return Some(expr);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Resolve `spec` from `parent`'s directory into an absolute path. `spec`
    /// may already be absolute — `find_in_star_exports` reports its hops that
    /// way because the specifier is written relative to the star-exporting
    /// file, not the file the consumer is looking at.
    fn resolve_module_spec(&self, parent: &Path, spec: &str) -> Option<String> {
        if Path::new(spec).is_absolute() {
            return Some(spec.to_string());
        }
        self.resolver
            .resolve(parent, spec)
            .ok()
            .map(|r| r.full_path().to_string_lossy().to_string())
    }

    /// The files [`Self::resolve_export_value`] has read since the last call.
    pub(crate) fn take_value_dependencies(&self) -> FxHashSet<String> {
        std::mem::take(&mut *self.value_dependencies.borrow_mut())
    }

    /// Trace an export through re-export chains to find its original source file.
    ///
    /// Returns the absolute path to the file where the export is actually defined.
    fn trace_export_source(&self, file_path: &Path, export_name: &str) -> Option<PathBuf> {
        let file_str = file_path.to_string_lossy().to_string();

        // Get export info from cache
        let export_info = {
            let cache = self.cache.borrow();
            cache.get(&file_str)?.get(export_name).cloned()
        };

        // Check for star exports if we don't find the export directly
        let export_info = export_info.or_else(|| self.find_in_star_exports(&file_str, export_name));

        // If export not found, return None
        let export_info = export_info?;

        // If no re-export, this file is the source
        let Some((source_module, original_name)) = export_info.re_export_source else {
            // Direct export - this file is the source
            return Some(file_path.to_path_buf());
        };

        // It's a re-export - resolve and follow the chain
        let parent = file_path.parent()?;
        let next_file = PathBuf::from(self.resolve_module_spec(parent, &source_module)?);

        // Analyze the next file if needed
        let next_file_str = next_file.to_string_lossy().to_string();
        if !self.cache.borrow().contains_key(&next_file_str) {
            if self.analyzing.borrow().contains(&next_file_str) {
                return Some(next_file); // Circular - return current file
            }
            self.analyze_file(&next_file_str);
        }

        // Recursively trace the export
        self.trace_export_source(&next_file, &original_name).or(Some(next_file))
    }

    /// Search for an export in star exports (`export * from './other'`).
    ///
    /// When we encounter a file with star exports and don't find the export directly,
    /// we need to check each star export source to find where the export comes from.
    /// `visited` guards circular `export *` chains (`a.ts` <-> `b.ts`) — the
    /// `analyzing` set doesn't cover them because it empties once a file is
    /// cached.
    fn find_in_star_exports(&self, file_path: &str, export_name: &str) -> Option<ExportInfo> {
        let mut visited = FxHashSet::default();
        visited.insert(file_path.to_string());
        self.find_in_star_exports_inner(file_path, export_name, &mut visited)
    }

    fn find_in_star_exports_inner(
        &self,
        file_path: &str,
        export_name: &str,
        visited: &mut FxHashSet<String>,
    ) -> Option<ExportInfo> {
        // Collect star export sources first to avoid borrow issues
        // Star exports are stored with keys like "*:./module"
        let star_sources: Vec<String> = {
            let cache = self.cache.borrow();
            let exports = cache.get(file_path)?;
            exports
                .iter()
                .filter_map(|(key, info)| {
                    // Check for "*:source" keys (plain star exports)
                    if key.starts_with("*:") {
                        return info.re_export_source.as_ref().map(|(source, _)| source.clone());
                    }
                    // Also check for "export * as X" entries
                    info.re_export_source.as_ref().and_then(|(source, name)| {
                        if name == "*" { Some(source.clone()) } else { None }
                    })
                })
                .collect()
        };

        let file_parent = Path::new(file_path).parent()?;

        for source in star_sources {
            // Resolve the star export source
            let resolved = match self.resolver.resolve(file_parent, &source) {
                Ok(r) => r.full_path().to_string_lossy().to_string(),
                Err(_) => continue,
            };
            if !visited.insert(resolved.clone()) {
                continue;
            }
            self.value_dependencies.borrow_mut().insert(resolved.clone());

            // Analyze if needed
            if !self.cache.borrow().contains_key(&resolved) {
                if self.analyzing.borrow().contains(&resolved) {
                    continue;
                }
                self.analyze_file(&resolved);
            }

            // Check if this file exports the name we're looking for
            if let Some(info) = self.cache.borrow().get(&resolved).and_then(|e| e.get(export_name))
            {
                // Found it! Point at the resolved file — `source` is written
                // relative to `file_path`, but consumers resolve
                // `re_export_source` against the outer file's parent.
                return Some(ExportInfo {
                    is_type_only: info.is_type_only,
                    re_export_source: Some((resolved, export_name.to_string())),
                    value_local: None,
                    never_static: false,
                });
            }

            // Recursively check star exports in the resolved file
            if let Some(info) = self.find_in_star_exports_inner(&resolved, export_name, visited) {
                return Some(ExportInfo {
                    is_type_only: info.is_type_only,
                    re_export_source: Some((resolved, export_name.to_string())),
                    value_local: None,
                    never_static: false,
                });
            }
        }

        None
    }

    /// Convert an absolute path to a relative path from the given base directory.
    fn make_relative_path(&self, from_dir: &Path, to_file: &Path) -> Option<String> {
        // Canonicalize paths to resolve symlinks (important for macOS /var -> /private/var)
        let from_canonical = from_dir.canonicalize().ok()?;
        let to_canonical = to_file.canonicalize().ok()?;

        let relative = pathdiff::diff_paths(&to_canonical, &from_canonical)?;
        // The result is used as an ES module specifier — always `/`, even on
        // Windows where `diff_paths` yields `\`.
        let mut path_str = relative.to_string_lossy().replace('\\', "/");

        // Ensure the path starts with "./" for relative imports
        if !path_str.starts_with('.') {
            path_str = format!("./{path_str}");
        }

        // Remove .ts/.tsx extension for TypeScript imports
        if let Some(stripped) = path_str.strip_suffix(".ts") {
            path_str = stripped.to_string();
        } else if let Some(stripped) = path_str.strip_suffix(".tsx") {
            path_str = stripped.to_string();
        }

        Some(path_str)
    }

    /// Check if an export is type-only, following re-export chains.
    fn check_export_is_type_only(&self, file_path: &str, export_name: &str) -> bool {
        // Get export info from cache (clone to avoid borrow issues)
        let export_info = {
            let cache = self.cache.borrow();
            let Some(exports) = cache.get(file_path) else {
                return false;
            };
            exports.get(export_name).cloned()
        };

        // If not found directly, check star exports (export * from './other')
        let export_info = export_info.or_else(|| self.find_in_star_exports(file_path, export_name));

        let Some(export_info) = export_info else {
            return false;
        };

        // If it's a direct export, return its type-only status
        let Some((source_module, original_name)) = export_info.re_export_source else {
            return export_info.is_type_only;
        };

        // It's a re-export - follow the chain
        // Resolve the re-export source relative to the current file
        let current_file = Path::new(file_path);
        let Some(resolved) = current_file
            .parent()
            .and_then(|parent| self.resolve_module_spec(parent, &source_module))
        else {
            return export_info.is_type_only; // Cannot resolve - use direct info
        };

        // Analyze the re-export source if not cached
        if !self.cache.borrow().contains_key(&resolved) {
            if self.analyzing.borrow().contains(&resolved) {
                return export_info.is_type_only; // Circular - use direct info
            }
            self.analyze_file(&resolved);
        }

        // Check the re-export source recursively
        let source_is_type_only = {
            let cache = self.cache.borrow();
            let Some(source_exports) = cache.get(&resolved) else {
                return export_info.is_type_only;
            };
            source_exports.get(&original_name).map(|info| info.is_type_only)
        };

        source_is_type_only.unwrap_or(export_info.is_type_only)
    }

    /// Analyze a file and cache its export information.
    fn analyze_file(&self, file_path: &str) {
        self.analyzing.borrow_mut().insert(file_path.to_string());

        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => {
                self.analyzing.borrow_mut().remove(file_path);
                return;
            }
        };

        // Keep only the export map: the full parse (AST and consts) is built
        // lazily by `parsed_file` for the few files that are evaluated or
        // whose selector is read, so a wide `export *` barrel doesn't hold
        // every analyzed file's AST until the analyzer is dropped.
        let mut exports = FxHashMap::default();
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(file_path).unwrap_or_default();
        let parser_ret = Parser::new(&allocator, &source, source_type).parse();
        self.analyze_program(&parser_ret.program, &mut exports);

        self.cache.borrow_mut().insert(file_path.to_string(), exports);
        self.analyzing.borrow_mut().remove(file_path);
    }

    /// Collect `program`'s exports into `exports`.
    fn analyze_program(&self, program: &Program, exports: &mut FxHashMap<String, ExportInfo>) {
        for stmt in &program.body {
            self.analyze_statement(stmt, exports, true);
        }
        // A top-level variable or enum shadows a class or function of the
        // same name for the evaluator (a duplicate declaration TypeScript
        // rejects): evaluate those exports as before.
        if !exports.values().any(|info| info.never_static) {
            return;
        }
        for stmt in &program.body {
            let decl = match stmt {
                Statement::ExportDeclaration(decl) => &decl.declaration,
                _ => match stmt.as_declaration() {
                    Some(decl) => decl,
                    None => continue,
                },
            };
            let mut shadow = |name: &str| {
                if let Some(info) = exports.get_mut(name) {
                    info.never_static = false;
                }
            };
            match decl {
                Declaration::VariableDeclaration(vars) => {
                    for var in &vars.declarations {
                        for id in var.id.get_binding_identifiers() {
                            shadow(&id.name);
                        }
                    }
                }
                Declaration::TSEnumDeclaration(e) => shadow(&e.id.name),
                _ => {}
            }
        }
    }

    /// Analyze a statement for export information. `top_level` is whether
    /// `stmt` is in the program body rather than a namespace or module block.
    fn analyze_statement(
        &self,
        stmt: &Statement,
        exports: &mut FxHashMap<String, ExportInfo>,
        top_level: bool,
    ) {
        match stmt {
            // export class/function/const/interface/type Foo { ... }
            Statement::ExportDeclaration(decl) => {
                self.analyze_declaration(&decl.declaration, exports, top_level);
            }
            // Local named export: export { X, Y } / export type { X }
            Statement::ExportNamedDeclaration(decl) => {
                if decl.export_kind.is_type() {
                    for spec in &decl.specifiers {
                        let name = spec.exported.name().to_string();
                        exports.insert(
                            name,
                            ExportInfo {
                                is_type_only: true,
                                re_export_source: None,
                                value_local: None,
                                never_static: false,
                            },
                        );
                    }
                    return;
                }

                // Export specifiers without source: export { X, Y }. The
                // export binds the LOCAL name — `export { Y as X }` reads `Y`
                // (`export { A }` of an imported `A` resolves through the
                // evaluator's resolver at `eval_export` time).
                for spec in &decl.specifiers {
                    let name = spec.exported.name().to_string();
                    let is_type_only = spec.export_kind.is_type();
                    let value_local = (!is_type_only).then(|| spec.local.name().to_string());
                    exports.insert(
                        name,
                        ExportInfo {
                            is_type_only,
                            re_export_source: None,
                            value_local,
                            never_static: false,
                        },
                    );
                }
            }
            // Re-export: export { X } from './other' / export type { X } from './other'
            Statement::ExportFromDeclaration(decl) => {
                // Whole-statement `export type { ... } from` — type-only, no re-export chase
                // (matches pre-split behavior where export_kind.is_type() short-circuited
                // before looking at source).
                if decl.export_kind.is_type() {
                    for spec in &decl.specifiers {
                        let name = spec.exported.name().to_string();
                        exports.insert(
                            name,
                            ExportInfo {
                                is_type_only: true,
                                re_export_source: None,
                                value_local: None,
                                never_static: false,
                            },
                        );
                    }
                    return;
                }

                // A source with a lone surrogate (`'\uD800'`) can't be
                // resolved: keep the specifier's own info, unchased.
                let source = decl.source.value.as_str();
                for spec in &decl.specifiers {
                    let exported_name = spec.exported.name().to_string();
                    let local_name = spec.local.name().to_string();
                    exports.insert(
                        exported_name,
                        ExportInfo {
                            is_type_only: spec.export_kind.is_type(),
                            re_export_source: source.map(|source| (source.to_string(), local_name)),
                            value_local: None,
                            never_static: false,
                        },
                    );
                }
            }
            Statement::ExportDefaultDeclaration(decl) => {
                let is_type_only = matches!(
                    &decl.declaration,
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
                );
                // `export default <expr>` is evaluated lazily; the `"default"`
                // local tells `eval_export` to look for the declaration.
                exports.insert(
                    "default".to_string(),
                    ExportInfo {
                        is_type_only,
                        re_export_source: None,
                        value_local: (!is_type_only).then(|| "default".to_string()),
                        never_static: false,
                    },
                );
            }
            Statement::ExportAllDeclaration(decl) => {
                // A source with a lone surrogate (`'\uD800'`) can't be resolved.
                let Some(source_module) = decl.source.value.as_str() else { return };
                let source_module = source_module.to_string();
                if let Some(exported) = &decl.exported {
                    // export * as X from './other'
                    exports.insert(
                        exported.name().to_string(),
                        ExportInfo {
                            is_type_only: false, // Namespace re-export - assume value
                            re_export_source: Some((source_module, "*".to_string())),
                            value_local: None,
                            never_static: false,
                        },
                    );
                } else {
                    // export * from './other' (plain star export)
                    // Use a special key format to track these: "*:./module"
                    let star_key = format!("*:{source_module}");
                    exports.insert(
                        star_key,
                        ExportInfo {
                            is_type_only: false,
                            re_export_source: Some((source_module, "*".to_string())),
                            value_local: None,
                            never_static: false,
                        },
                    );
                }
            }
            // Ambient external module: declare module "foo" { export ... }
            Statement::TSExternalModuleDeclaration(module_decl) => {
                if let Some(block) = &module_decl.body {
                    for inner_stmt in &block.body {
                        self.analyze_statement(inner_stmt, exports, false);
                    }
                }
            }
            // namespace Foo { ... } / module Foo { ... } (and nested Foo.Bar)
            Statement::TSNamespaceDeclaration(ns_decl) => {
                self.analyze_namespace_body(&ns_decl.body, exports);
            }
            // declare global { ... }
            Statement::TSGlobalDeclaration(global_decl) => {
                for inner_stmt in &global_decl.body.body {
                    self.analyze_statement(inner_stmt, exports, false);
                }
            }
            _ => {}
        }
    }

    /// Walk a namespace body, recursing through nested `namespace A.B` forms.
    fn analyze_namespace_body(
        &self,
        body: &TSNamespaceDeclarationBody,
        exports: &mut FxHashMap<String, ExportInfo>,
    ) {
        match body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => {
                for inner_stmt in &block.body {
                    self.analyze_statement(inner_stmt, exports, false);
                }
            }
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(nested) => {
                self.analyze_namespace_body(&nested.body, exports);
            }
        }
    }

    /// Analyze a declaration and add export information. `top_level` is
    /// whether it's declared in the program body (see
    /// [`ExportInfo::never_static`]).
    fn analyze_declaration(
        &self,
        decl: &Declaration,
        exports: &mut FxHashMap<String, ExportInfo>,
        top_level: bool,
    ) {
        // A runtime export binds the declared name; its value is evaluated
        // lazily by `eval_export`. Type declarations have no value local.
        let value_info = |name: &str, is_type_only: bool| ExportInfo {
            is_type_only,
            re_export_source: None,
            value_local: (!is_type_only).then(|| name.to_string()),
            never_static: false,
        };
        // A class or function evaluates to a reference to itself.
        let reference_info =
            |name: &str| ExportInfo { never_static: top_level, ..value_info(name, false) };
        match decl {
            Declaration::TSInterfaceDeclaration(d) => {
                exports.insert(d.id.name.to_string(), value_info(&d.id.name, true));
            }
            Declaration::TSTypeAliasDeclaration(d) => {
                exports.insert(d.id.name.to_string(), value_info(&d.id.name, true));
            }
            Declaration::ClassDeclaration(d) => {
                if let Some(id) = &d.id {
                    exports.insert(id.name.to_string(), reference_info(&id.name));
                }
            }
            Declaration::FunctionDeclaration(d) => {
                if let Some(id) = &d.id {
                    exports.insert(id.name.to_string(), reference_info(&id.name));
                }
            }
            Declaration::VariableDeclaration(d) => {
                // Extract names from variable declarations
                for declarator in &d.declarations {
                    if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                        exports.insert(id.name.to_string(), value_info(&id.name, false));
                    }
                }
            }
            Declaration::TSEnumDeclaration(d) => {
                // Enums have runtime value (unless const enum with isolatedModules)
                exports.insert(d.id.name.to_string(), value_info(&d.id.name, false));
            }
            Declaration::TSNamespaceDeclaration(d) => {
                // Namespaces can have runtime value; `id` is a BindingIdentifier
                exports.insert(d.id.name.to_string(), value_info(&d.id.name, false));
            }
            Declaration::TSExternalModuleDeclaration(_)
            | Declaration::TSImportEqualsDeclaration(_)
            | Declaration::TSGlobalDeclaration(_) => {
                // External modules (string id), import equals, and global — skip
            }
        }
    }

    /// Clear the analysis cache.
    pub fn clear_cache(&self) {
        self.cache.borrow_mut().clear();
        self.parsed.borrow_mut().clear();
        self.value_memo.borrow_mut().clear();
        self.selector_memo.borrow_mut().clear();
    }

    /// Get the number of cached files.
    pub fn cache_size(&self) -> usize {
        self.cache.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_file(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let file_path = dir.join(name);
        // Create parent directories if they don't exist
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&file_path, content).expect("Failed to write test file");
        file_path
    }

    #[test]
    fn test_interface_export_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export interface User { name: string; }");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(analyzer.is_type_only_import("./types", "User", &main_file));
    }

    #[test]
    fn test_type_alias_export_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export type UserId = string;");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(analyzer.is_type_only_import("./types", "UserId", &main_file));
    }

    #[test]
    fn test_class_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export class AuthService {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(!analyzer.is_type_only_import("./service", "AuthService", &main_file));
    }

    #[test]
    fn test_function_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "utils.ts", "export function helper() {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(!analyzer.is_type_only_import("./utils", "helper", &main_file));
    }

    #[test]
    fn test_const_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "constants.ts", "export const TOKEN = 'token';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(!analyzer.is_type_only_import("./constants", "TOKEN", &main_file));
    }

    #[test]
    fn test_enum_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "enums.ts", "export enum Status { Active, Inactive }");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(!analyzer.is_type_only_import("./enums", "Status", &main_file));
    }

    #[test]
    fn test_re_export_interface_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export interface Foo {}");
        create_test_file(dir.path(), "index.ts", "export { Foo } from './types';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(analyzer.is_type_only_import("./index", "Foo", &main_file));
    }

    #[test]
    fn test_re_export_class_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export class MyService {}");
        create_test_file(dir.path(), "index.ts", "export { MyService } from './service';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(!analyzer.is_type_only_import("./index", "MyService", &main_file));
    }

    #[test]
    fn test_export_type_specifier_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export type { Foo } from './foo';");
        create_test_file(dir.path(), "foo.ts", "export class Foo {}"); // Even though Foo is a class
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        // export type { X } is always type-only, regardless of what X is
        assert!(analyzer.is_type_only_import("./types", "Foo", &main_file));
    }

    #[test]
    fn test_mixed_exports() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "mixed.ts",
            r#"
export interface User { name: string; }
export class UserService {}
export type UserId = string;
export const USER_TOKEN = 'token';
"#,
        );
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(analyzer.is_type_only_import("./mixed", "User", &main_file));
        assert!(!analyzer.is_type_only_import("./mixed", "UserService", &main_file));
        assert!(analyzer.is_type_only_import("./mixed", "UserId", &main_file));
        assert!(!analyzer.is_type_only_import("./mixed", "USER_TOKEN", &main_file));
    }

    #[test]
    fn test_package_imports_are_conservative() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        // Package imports should return false (conservative - assume value)
        assert!(!analyzer.is_type_only_import("@angular/core", "Component", &main_file));
        assert!(!analyzer.is_type_only_import("rxjs", "Observable", &main_file));
    }

    #[test]
    fn test_nonexistent_file_is_conservative() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        // Non-existent file should return false (conservative)
        assert!(!analyzer.is_type_only_import("./nonexistent", "Foo", &main_file));
    }

    #[test]
    fn test_caching_works() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export interface User {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);

        // First call - should analyze
        assert!(analyzer.is_type_only_import("./types", "User", &main_file));
        assert_eq!(analyzer.cache_size(), 1);

        // Second call - should use cache
        assert!(analyzer.is_type_only_import("./types", "User", &main_file));
        assert_eq!(analyzer.cache_size(), 1);
    }

    #[test]
    fn test_default_export_interface() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export default interface User {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(analyzer.is_type_only_import("./types", "default", &main_file));
    }

    #[test]
    fn test_default_export_class() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export default class MyService {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(!analyzer.is_type_only_import("./service", "default", &main_file));
    }

    // Tests for resolve_import_source_path

    #[test]
    fn test_resolve_direct_import() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "component.ts", "export class MyComponent {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved =
            analyzer.resolve_import_source_path("./component", "MyComponent", &main_file);
        assert_eq!(resolved, Some("./component".to_string()));
    }

    #[test]
    fn test_resolve_barrel_export() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "component.ts", "export class MyComponent {}");
        create_test_file(dir.path(), "index.ts", "export { MyComponent } from './component';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved = analyzer.resolve_import_source_path("./index", "MyComponent", &main_file);
        assert_eq!(resolved, Some("./component".to_string()));
    }

    #[test]
    fn test_resolve_nested_barrel_exports() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "deep/component.ts", "export class DeepComponent {}");
        create_test_file(
            dir.path(),
            "deep/index.ts",
            "export { DeepComponent } from './component';",
        );
        create_test_file(dir.path(), "index.ts", "export { DeepComponent } from './deep';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved = analyzer.resolve_import_source_path("./index", "DeepComponent", &main_file);
        assert_eq!(resolved, Some("./deep/component".to_string()));
    }

    #[test]
    fn test_resolve_star_export() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export class MyService {}");
        create_test_file(dir.path(), "index.ts", "export * from './service';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved = analyzer.resolve_import_source_path("./index", "MyService", &main_file);
        assert_eq!(resolved, Some("./service".to_string()));
    }

    #[test]
    fn test_resolve_star_export_chain() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "deep/util.ts", "export function helper() {}");
        create_test_file(dir.path(), "deep/index.ts", "export * from './util';");
        create_test_file(dir.path(), "index.ts", "export * from './deep';");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved = analyzer.resolve_import_source_path("./index", "helper", &main_file);
        assert_eq!(resolved, Some("./deep/util".to_string()));
    }

    #[test]
    fn test_resolve_mixed_star_and_named_exports() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export interface Config {}");
        create_test_file(dir.path(), "utils.ts", "export function doSomething() {}");
        create_test_file(
            dir.path(),
            "index.ts",
            r#"
export * from './types';
export { doSomething } from './utils';
"#,
        );
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);

        // Star export
        let resolved_config = analyzer.resolve_import_source_path("./index", "Config", &main_file);
        assert_eq!(resolved_config, Some("./types".to_string()));

        // Named re-export
        let resolved_fn = analyzer.resolve_import_source_path("./index", "doSomething", &main_file);
        assert_eq!(resolved_fn, Some("./utils".to_string()));
    }

    #[test]
    fn test_resolve_package_import_returns_none() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(
            analyzer.resolve_import_source_path("@angular/core", "Component", &main_file).is_none()
        );
        assert!(analyzer.resolve_import_source_path("rxjs", "Observable", &main_file).is_none());
    }

    #[test]
    fn test_resolve_nonexistent_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(analyzer.resolve_import_source_path("./nonexistent", "Foo", &main_file).is_none());
    }

    #[test]
    fn test_resolve_nonexistent_export_returns_none() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "module.ts", "export class Exists {}");
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        assert!(
            analyzer.resolve_import_source_path("./module", "DoesNotExist", &main_file).is_none()
        );
    }

    #[test]
    fn test_resolve_from_subdirectory() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "shared/component.ts", "export class SharedComponent {}");
        create_test_file(
            dir.path(),
            "shared/index.ts",
            "export { SharedComponent } from './component';",
        );
        // Create a placeholder file in app/ so the directory exists
        create_test_file(dir.path(), "app/main.ts", "// placeholder");
        let main_file = dir.path().join("app/main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved =
            analyzer.resolve_import_source_path("../shared/index", "SharedComponent", &main_file);
        assert_eq!(resolved, Some("../shared/component".to_string()));
    }

    #[test]
    fn test_resolve_renamed_export() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "original.ts", "export class OriginalName {}");
        create_test_file(
            dir.path(),
            "index.ts",
            "export { OriginalName as RenamedExport } from './original';",
        );
        let main_file = dir.path().join("main.ts");

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);
        let resolved = analyzer.resolve_import_source_path("./index", "RenamedExport", &main_file);
        assert_eq!(resolved, Some("./original".to_string()));
    }

    /// Regression test for quick-access.component.ts:
    /// `import { WIDGET_CONTROL, WidgetControlService } from '../../widget-control'`
    /// where `widget-control/index.ts` has `export * from './widget-control.service'`
    /// and `widget-control.service.ts` has `export interface WidgetControlService { ... }`
    ///
    /// The interface should be detected as type-only through the barrel star export chain.
    #[test]
    fn test_interface_through_star_export_barrel_is_type_only() {
        let dir = TempDir::new().unwrap();
        // widget-control/widget-control.service.ts — interface (type-only)
        create_test_file(
            dir.path(),
            "widget-control/widget-control.service.ts",
            "export interface WidgetControlService { hideWidget(widgetId: string): void; }",
        );
        // widget-control/widget-control.token.ts — const (runtime value)
        create_test_file(
            dir.path(),
            "widget-control/widget-control.token.ts",
            "import { InjectionToken } from '@angular/core';\nexport const WIDGET_CONTROL = new InjectionToken('WidgetControlService');",
        );
        // widget-control/index.ts — barrel re-export via star
        create_test_file(
            dir.path(),
            "widget-control/index.ts",
            "export * from './widget-control.service';\nexport * from './widget-control.token';",
        );

        let main_file = dir.path().join("quick-access/quick-access/main.ts");
        // Create parent dir so the path is valid
        std::fs::create_dir_all(main_file.parent().unwrap()).unwrap();

        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);

        // WidgetControlService is an interface — should be type-only
        assert!(
            analyzer.is_type_only_import(
                "../../widget-control",
                "WidgetControlService",
                &main_file
            ),
            "WidgetControlService interface should be detected as type-only through star export barrel"
        );

        // WIDGET_CONTROL is a const — should NOT be type-only
        assert!(
            !analyzer.is_type_only_import("../../widget-control", "WIDGET_CONTROL", &main_file),
            "WIDGET_CONTROL const should NOT be type-only"
        );
    }

    // ---- resolve_export_value (imported decorator metadata values) ----

    use crate::directive::StaticValue;

    fn resolve(
        analyzer: &CrossFileAnalyzer,
        from: &Path,
        module: &str,
        name: &str,
    ) -> Option<StaticValue> {
        analyzer.resolve_export_value(module, name, from)
    }

    #[test]
    fn test_value_simple_consts() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "other.ts",
            "export const NAME = 'a';\n\
             export const N = 42;\n\
             export const B = true;\n\
             export const INPUTS = ['a', 'b'];\n\
             export const OPTS = { alias: 'b', required: true };\n\
             export const E = 1 + 2;",
        );
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "NAME"),
            Some(StaticValue::String(s)) if s == "a"
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "N"),
            Some(StaticValue::Number(n)) if n == 42.0
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "B"),
            Some(StaticValue::Bool(true))
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "INPUTS"),
            Some(StaticValue::Array(items)) if matches!(&items[..], [StaticValue::String(a), StaticValue::String(b)] if a == "a" && b == "b")
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "OPTS"),
            Some(StaticValue::Object(props)) if props.iter().any(|(k, _)| k == "alias")
        ));
        // Computed same-file expressions evaluate too.
        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "E"),
            Some(StaticValue::Number(n)) if n == 3.0
        ));
    }

    #[test]
    fn test_value_const_chain_across_files() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "a.ts", "export const A = ['a'];");
        create_test_file(
            dir.path(),
            "b.ts",
            "import { A } from './a';\nexport const B = [...A, 'b'];",
        );
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./b", "B"),
            Some(StaticValue::Array(items)) if items.len() == 2
        ));
    }

    #[test]
    fn test_value_re_export_chain() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "deep.ts", "export const X = 'deep';");
        create_test_file(dir.path(), "mid.ts", "export { X as Y } from './deep';");
        create_test_file(dir.path(), "index.ts", "export { Y } from './mid';");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./index", "Y"),
            Some(StaticValue::String(s)) if s == "deep"
        ));
    }

    #[test]
    fn test_value_star_export() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "impl.ts", "export const SEL = 'sel';");
        create_test_file(dir.path(), "index.ts", "export * from './impl';");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./index", "SEL"),
            Some(StaticValue::String(s)) if s == "sel"
        ));
    }

    #[test]
    fn test_value_export_default_expression() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "d.ts", "export default 'val';");
        create_test_file(dir.path(), "named.ts", "const X = 'x';\nexport default X;");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./d", "default"),
            Some(StaticValue::String(s)) if s == "val"
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./named", "default"),
            Some(StaticValue::String(s)) if s == "x"
        ));
    }

    #[test]
    fn test_value_local_export_of_imported_binding() {
        // `import { A } from './a'; export { A };` — a "local" export that is
        // really a re-export; ngtsc's checker follows it.
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "a.ts", "export const A = 'a';");
        create_test_file(dir.path(), "b.ts", "import { A } from './a';\nexport { A };");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./b", "A"),
            Some(StaticValue::String(s)) if s == "a"
        ));
    }

    #[test]
    fn test_value_enum_member() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "enums.ts",
            "export enum K { A = 'a', B = 'b' }\nexport const E = K;",
        );
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        // The enum evaluates to its member object; `E.A`-shaped access
        // resolves at the member step in the caller's evaluator.
        assert!(matches!(
            resolve(&analyzer, &main_file, "./enums", "E"),
            Some(StaticValue::Object(props)) if props.iter().any(|(k, _)| k == "A")
        ));
    }

    #[test]
    fn test_value_unresolvable_falls_back() {
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "other.ts",
            "export declare const DECLARED: string;\n\
             export const FN = () => 1;\n\
             export function f() {}",
        );
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        // Missing file, missing export, declare-only, function values, and
        // package imports all stay opaque (None → the import-reference error).
        assert!(resolve(&analyzer, &main_file, "./missing", "X").is_none());
        assert!(resolve(&analyzer, &main_file, "./other", "NOPE").is_none());
        assert!(resolve(&analyzer, &main_file, "./other", "DECLARED").is_none());
        assert!(resolve(&analyzer, &main_file, "./other", "FN").is_none());
        assert!(resolve(&analyzer, &main_file, "./other", "f").is_none());
        assert!(resolve(&analyzer, &main_file, "rxjs", "of").is_none());
    }

    #[test]
    fn test_value_circular_files_terminate() {
        // `export const` cycles between files must not hang or panic; each
        // unresolvable link becomes opaque.
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "a.ts",
            "import { B } from './b';\nexport const A = B;\nexport const SELF = 'a';",
        );
        create_test_file(dir.path(), "b.ts", "import { A } from './a';\nexport const B = A;");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        // The circular pair stays opaque; a directly-static export still resolves.
        assert!(matches!(
            resolve(&analyzer, &main_file, "./a", "SELF"),
            Some(StaticValue::String(s)) if s == "a"
        ));
    }

    #[test]
    fn test_value_cycle_through_analyzing_file_resolves() {
        // `a.C` reads `b.B`, which reads `a.A`. Evaluating eagerly while `a`
        // was in `analyzing` cached `b.B` as opaque and made the result depend
        // on which file was discovered first; ngtsc resolves both.
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "a.ts",
            "import { B } from './b';\nexport const A = 'a';\nexport const C = B;",
        );
        create_test_file(dir.path(), "b.ts", "import { A } from './a';\nexport const B = A;");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./a", "C"),
            Some(StaticValue::String(s)) if s == "a"
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./b", "B"),
            Some(StaticValue::String(s)) if s == "a"
        ));
    }

    #[test]
    fn test_value_elision_only_analyzer_resolves_on_demand() {
        // `resolve_export_value` works without `values_enabled`: files are
        // parsed on demand.
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "other.ts", "export const V = 'v';");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, false);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./other", "V"),
            Some(StaticValue::String(s)) if s == "v"
        ));
    }

    #[test]
    fn test_value_dependencies_recorded() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "other.ts", "export const V = 1;");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        resolve(&analyzer, &main_file, "./other", "V");
        let deps = analyzer.take_value_dependencies();
        assert_eq!(deps.len(), 1);
        assert!(deps.iter().next().unwrap().ends_with("other.ts"));
        // Taken deps don't repeat.
        assert!(analyzer.take_value_dependencies().is_empty());
    }

    #[test]
    fn test_value_circular_star_exports_terminate() {
        // `export *` cycles (a -> b -> a) recursed forever in
        // `find_in_star_exports` because the `analyzing` guard empties once
        // a file is cached. The lookup must terminate with `None`.
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "a.ts", "export * from './b';\nexport const A = 'a';");
        create_test_file(dir.path(), "b.ts", "export * from './a';");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        // X doesn't exist anywhere: follow a -> b -> a once, then stop.
        assert!(resolve(&analyzer, &main_file, "./a", "X").is_none());
        // A direct export in the cycle still resolves.
        assert!(matches!(
            resolve(&analyzer, &main_file, "./a", "A"),
            Some(StaticValue::String(s)) if s == "a"
        ));
        // Same for the type-only path, which shares find_in_star_exports.
        assert!(!analyzer.is_type_only_import("./a", "X", &main_file));
    }

    #[test]
    fn test_value_star_export_nested_dirs() {
        // The star specifier is written relative to the barrel's directory;
        // `resolve_export_value` must resolve it there, not relative to the
        // importing file.
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "feat/lib/impl.ts", "export const SEL = 'nested';");
        create_test_file(dir.path(), "feat/index.ts", "export * from './lib/impl';");
        let main_file = dir.path().join("app/main.ts");
        std::fs::create_dir_all(main_file.parent().unwrap()).unwrap();
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "../feat", "SEL"),
            Some(StaticValue::String(s)) if s == "nested"
        ));
        // Every file read along the hop chain lands in dependencies. Paths
        // are OS-native (`\` on Windows) — normalize before suffix checks.
        let deps = analyzer.take_value_dependencies();
        let normalized: Vec<String> = deps.iter().map(|d| d.replace('\\', "/")).collect();
        assert!(
            normalized.iter().any(|d| d.ends_with("feat/index.ts")),
            "{normalized:?} should contain the barrel"
        );
        assert!(
            normalized.iter().any(|d| d.ends_with("feat/lib/impl.ts")),
            "{normalized:?} should contain the star-exported file"
        );
    }

    #[test]
    fn test_standalone_directive_selector_is_memoized() {
        // The selector is read from the file parsed on first use and
        // memoized, not re-read from disk on every lookup, until the caches
        // are cleared. The file is still reported as a dependency each time.
        let dir = TempDir::new().unwrap();
        let component = "import { Component } from '@angular/core';\n@Component({ selector: 'app-a' })\nexport class LazyCmp {}";
        let lazy = create_test_file(dir.path(), "lazy.ts", component);
        let main_file = dir.path().join("main.ts");
        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None, true);
        let selector = |analyzer: &CrossFileAnalyzer| {
            analyzer
                .standalone_directive_selector("./lazy", "LazyCmp", &main_file, true)
                .map(|(selector, _)| selector)
        };

        let app = |selector: &str| Some(ClassUse::Selector(selector.to_string()));
        assert_eq!(selector(&analyzer), app("app-a"));
        assert_eq!(analyzer.take_value_dependencies().len(), 1);

        fs::write(&lazy, component.replace("app-a", "app-b")).unwrap();
        assert_eq!(selector(&analyzer), app("app-a"));
        assert_eq!(analyzer.take_value_dependencies().len(), 1);

        analyzer.clear_cache();
        assert_eq!(selector(&analyzer), app("app-b"));
    }

    #[test]
    fn test_value_file_parsed_once() {
        // The first evaluation's parse serves every later one in the file: an
        // edit made after it isn't seen.
        let dir = TempDir::new().unwrap();
        let meta =
            create_test_file(dir.path(), "meta.ts", "export const A = 'a';\nexport const B = 'b';");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./meta", "A"),
            Some(StaticValue::String(s)) if s == "a"
        ));
        fs::write(&meta, "export const A = 'x';\nexport const B = 'y';").unwrap();
        assert!(matches!(
            resolve(&analyzer, &main_file, "./meta", "B"),
            Some(StaticValue::String(s)) if s == "b"
        ));
        assert_eq!(analyzer.inner.parsed.borrow().len(), 1);
    }

    #[test]
    fn test_value_analysis_does_not_keep_asts_of_barrel_files() {
        // Analyzing a wide `export *` barrel keeps only export maps; just the
        // file whose value is evaluated is parsed and kept.
        let dir = TempDir::new().unwrap();
        let mut barrel = String::new();
        for i in 0..5 {
            create_test_file(
                dir.path(),
                &format!("f{i}.ts"),
                &format!("export const V{i} = 'v{i}';"),
            );
            barrel.push_str(&format!("export * from './f{i}';\n"));
        }
        create_test_file(dir.path(), "barrel.ts", &barrel);
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./barrel", "V4"),
            Some(StaticValue::String(s)) if s == "v4"
        ));
        assert_eq!(analyzer.inner.parsed.borrow().len(), 1);
    }

    #[test]
    fn test_value_class_and_function_exports_are_opaque() {
        // Top-level classes and functions resolve to `None` without being
        // evaluated; the export map still gives their local for selectors.
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "decls.ts",
            "export class C {}\nexport function f() { return 1; }\nexport const V = 'v';",
        );
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(resolve(&analyzer, &main_file, "./decls", "C").is_none());
        assert!(resolve(&analyzer, &main_file, "./decls", "f").is_none());
        assert!(analyzer.inner.value_memo.borrow().is_empty());
        assert!(matches!(
            resolve(&analyzer, &main_file, "./decls", "V"),
            Some(StaticValue::String(s)) if s == "v"
        ));
    }

    #[test]
    fn test_value_namespace_class_export_still_evaluates() {
        // A namespace member's export entry evaluates the top-level name it
        // shares (here a `const`): only top-level declarations skip
        // evaluation, as do ones a top-level variable doesn't shadow.
        let dir = TempDir::new().unwrap();
        create_test_file(
            dir.path(),
            "ns.ts",
            "export const Foo = 'x';\nexport namespace N { export class Foo {} }",
        );
        create_test_file(dir.path(), "dup.ts", "const Bar = 'y';\nexport class Bar {}");
        let main_file = dir.path().join("main.ts");
        let analyzer = CrossFileAnalyzer::new(dir.path(), None, true);

        assert!(matches!(
            resolve(&analyzer, &main_file, "./ns", "Foo"),
            Some(StaticValue::String(s)) if s == "x"
        ));
        assert!(matches!(
            resolve(&analyzer, &main_file, "./dup", "Bar"),
            Some(StaticValue::String(s)) if s == "y"
        ));
    }
}

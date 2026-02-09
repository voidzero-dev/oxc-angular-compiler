//! Cross-file type-only export detection using oxc_resolver, oxc_parser, and oxc_semantic.
//!
//! This module resolves import paths to actual files and analyzes their exports
//! to determine if they are type-only (interfaces, type aliases) or have runtime values.
//!
//! ## Purpose
//!
//! The existing `ImportElisionAnalyzer` uses `oxc_semantic` to detect type-only vs value
//! references via `ReferenceFlags`. However, it cannot look across file boundaries to see
//! if an imported symbol is actually a type-only export from the source file.
//!
//! This module provides cross-file resolution to improve import elision accuracy.
//!
//! ## Scope
//!
//! This is intended for compare test purposes only. In production, bundlers like rolldown
//! handle import elision as part of their tree-shaking process.

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Declaration, ExportDefaultDeclarationKind, Statement, TSModuleDeclarationBody,
};
use oxc_parser::Parser;
use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use oxc_span::SourceType;
use rustc_hash::{FxHashMap, FxHashSet};

/// Result of analyzing a file's exports
#[derive(Debug, Clone)]
pub struct ExportInfo {
    /// Whether this export is type-only (interface, type alias)
    pub is_type_only: bool,
    /// If re-export, the source module and original name
    pub re_export_source: Option<(String, String)>,
}

/// Cross-file analyzer for detecting type-only exports.
///
/// This analyzer resolves import paths to actual files, parses them, and
/// determines if exports are type-only (interfaces, type aliases) or have
/// runtime values (classes, functions, variables).
pub struct CrossFileAnalyzer {
    resolver: Resolver,
    /// Cache of file path -> export analysis
    cache: FxHashMap<String, FxHashMap<String, ExportInfo>>,
    /// Files currently being analyzed (for circular import detection)
    analyzing: FxHashSet<String>,
}

impl CrossFileAnalyzer {
    /// Create a new cross-file analyzer.
    ///
    /// # Arguments
    ///
    /// * `base_dir` - The base directory for module resolution
    /// * `tsconfig_path` - Optional path to tsconfig.json for path aliases
    pub fn new(_base_dir: &Path, tsconfig_path: Option<&Path>) -> Self {
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
            resolver: Resolver::new(options),
            cache: FxHashMap::default(),
            analyzing: FxHashSet::default(),
        }
    }

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
        &mut self,
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
        if !self.cache.contains_key(&resolved) {
            // Circular import protection
            if self.analyzing.contains(&resolved) {
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
        &mut self,
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
        if !self.cache.contains_key(&resolved_str) {
            if self.analyzing.contains(&resolved_str) {
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

    /// Trace an export through re-export chains to find its original source file.
    ///
    /// Returns the absolute path to the file where the export is actually defined.
    fn trace_export_source(&mut self, file_path: &Path, export_name: &str) -> Option<PathBuf> {
        let file_str = file_path.to_string_lossy().to_string();

        // Get export info from cache
        let export_info = self.cache.get(&file_str)?.get(export_name).cloned();

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
        let next_file =
            self.resolver.resolve(parent, &source_module).ok()?.full_path().to_path_buf();

        // Analyze the next file if needed
        let next_file_str = next_file.to_string_lossy().to_string();
        if !self.cache.contains_key(&next_file_str) {
            if self.analyzing.contains(&next_file_str) {
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
    fn find_in_star_exports(&mut self, file_path: &str, export_name: &str) -> Option<ExportInfo> {
        // Collect star export sources first to avoid borrow issues
        // Star exports are stored with keys like "*:./module"
        let star_sources: Vec<String> = {
            let exports = self.cache.get(file_path)?;
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

            // Analyze if needed
            if !self.cache.contains_key(&resolved) {
                if self.analyzing.contains(&resolved) {
                    continue;
                }
                self.analyze_file(&resolved);
            }

            // Check if this file exports the name we're looking for
            if let Some(exports) = self.cache.get(&resolved) {
                if let Some(info) = exports.get(export_name) {
                    // Found it! Return with the source information
                    return Some(ExportInfo {
                        is_type_only: info.is_type_only,
                        re_export_source: Some((source, export_name.to_string())),
                    });
                }
            }

            // Recursively check star exports in the resolved file
            if let Some(info) = self.find_in_star_exports(&resolved, export_name) {
                return Some(ExportInfo {
                    is_type_only: info.is_type_only,
                    re_export_source: Some((source, export_name.to_string())),
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
        let mut path_str = relative.to_string_lossy().to_string();

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
    fn check_export_is_type_only(&mut self, file_path: &str, export_name: &str) -> bool {
        // Get export info from cache (clone to avoid borrow issues)
        let export_info = {
            let Some(exports) = self.cache.get(file_path) else {
                return false;
            };
            exports.get(export_name).cloned()
        };

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
        let resolved = match current_file.parent() {
            Some(parent) => match self.resolver.resolve(parent, &source_module) {
                Ok(resolution) => resolution.full_path().to_string_lossy().to_string(),
                Err(_) => return export_info.is_type_only, // Cannot resolve - use direct info
            },
            None => return export_info.is_type_only,
        };

        // Analyze the re-export source if not cached
        if !self.cache.contains_key(&resolved) {
            if self.analyzing.contains(&resolved) {
                return export_info.is_type_only; // Circular - use direct info
            }
            self.analyze_file(&resolved);
        }

        // Check the re-export source recursively
        let source_is_type_only = {
            let Some(source_exports) = self.cache.get(&resolved) else {
                return export_info.is_type_only;
            };
            source_exports.get(&original_name).map(|info| info.is_type_only)
        };

        source_is_type_only.unwrap_or(export_info.is_type_only)
    }

    /// Analyze a file and cache its export information.
    fn analyze_file(&mut self, file_path: &str) {
        self.analyzing.insert(file_path.to_string());

        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => {
                self.analyzing.remove(file_path);
                return;
            }
        };

        let allocator = Allocator::default();
        let source_type = SourceType::from_path(file_path).unwrap_or_default();
        let parser_ret = Parser::new(&allocator, &source, source_type).parse();

        let mut exports = FxHashMap::default();

        for stmt in &parser_ret.program.body {
            self.analyze_statement(stmt, &mut exports);
        }

        self.cache.insert(file_path.to_string(), exports);
        self.analyzing.remove(file_path);
    }

    /// Analyze a statement for export information.
    fn analyze_statement<'a>(
        &self,
        stmt: &Statement<'a>,
        exports: &mut FxHashMap<String, ExportInfo>,
    ) {
        match stmt {
            Statement::ExportNamedDeclaration(decl) => {
                // Handle declarations first (export interface/type/class/function/etc.)
                // This includes `export interface User {}` which has export_kind.is_type() = true
                // but has a declaration rather than specifiers
                if let Some(declaration) = &decl.declaration {
                    self.analyze_declaration(declaration, exports);
                    return;
                }

                // export type { X } - type-only specifiers (no declaration)
                if decl.export_kind.is_type() {
                    for spec in &decl.specifiers {
                        let name = spec.exported.name().to_string();
                        exports.insert(
                            name,
                            ExportInfo { is_type_only: true, re_export_source: None },
                        );
                    }
                    return;
                }

                // Re-export: export { X } from './other'
                if let Some(source) = &decl.source {
                    for spec in &decl.specifiers {
                        let exported_name = spec.exported.name().to_string();
                        let local_name = spec.local.name().to_string();
                        exports.insert(
                            exported_name,
                            ExportInfo {
                                is_type_only: spec.export_kind.is_type(),
                                re_export_source: Some((source.value.to_string(), local_name)),
                            },
                        );
                    }
                    return;
                }

                // Export specifiers without source: export { X, Y }
                // These re-export local bindings - we need to check if the local
                // binding is type-only. For now, mark as not type-only (conservative).
                if !decl.specifiers.is_empty() && decl.declaration.is_none() {
                    for spec in &decl.specifiers {
                        let name = spec.exported.name().to_string();
                        exports.insert(
                            name,
                            ExportInfo {
                                is_type_only: spec.export_kind.is_type(),
                                re_export_source: None,
                            },
                        );
                    }
                    return;
                }

                // Inline declaration: export class/function/const/interface/type
                if let Some(declaration) = &decl.declaration {
                    self.analyze_declaration(declaration, exports);
                }
            }
            Statement::ExportDefaultDeclaration(decl) => {
                let is_type_only = matches!(
                    &decl.declaration,
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
                );
                exports.insert(
                    "default".to_string(),
                    ExportInfo { is_type_only, re_export_source: None },
                );
            }
            Statement::ExportAllDeclaration(decl) => {
                let source_module = decl.source.value.to_string();
                if let Some(exported) = &decl.exported {
                    // export * as X from './other'
                    exports.insert(
                        exported.name().to_string(),
                        ExportInfo {
                            is_type_only: false, // Namespace re-export - assume value
                            re_export_source: Some((source_module, "*".to_string())),
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
                        },
                    );
                }
            }
            // Handle ambient module declarations: declare module "foo" { export ... }
            Statement::TSModuleDeclaration(module_decl) => {
                if let Some(TSModuleDeclarationBody::TSModuleBlock(block)) = &module_decl.body {
                    for inner_stmt in &block.body {
                        self.analyze_statement(inner_stmt, exports);
                    }
                }
            }
            _ => {}
        }
    }

    /// Analyze a declaration and add export information.
    fn analyze_declaration<'a>(
        &self,
        decl: &Declaration<'a>,
        exports: &mut FxHashMap<String, ExportInfo>,
    ) {
        match decl {
            Declaration::TSInterfaceDeclaration(d) => {
                exports.insert(
                    d.id.name.to_string(),
                    ExportInfo { is_type_only: true, re_export_source: None },
                );
            }
            Declaration::TSTypeAliasDeclaration(d) => {
                exports.insert(
                    d.id.name.to_string(),
                    ExportInfo { is_type_only: true, re_export_source: None },
                );
            }
            Declaration::ClassDeclaration(d) => {
                if let Some(id) = &d.id {
                    exports.insert(
                        id.name.to_string(),
                        ExportInfo { is_type_only: false, re_export_source: None },
                    );
                }
            }
            Declaration::FunctionDeclaration(d) => {
                if let Some(id) = &d.id {
                    exports.insert(
                        id.name.to_string(),
                        ExportInfo { is_type_only: false, re_export_source: None },
                    );
                }
            }
            Declaration::VariableDeclaration(d) => {
                // Extract names from variable declarations
                for declarator in &d.declarations {
                    if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                        exports.insert(
                            id.name.to_string(),
                            ExportInfo { is_type_only: false, re_export_source: None },
                        );
                    }
                }
            }
            Declaration::TSEnumDeclaration(d) => {
                // Enums have runtime value (unless const enum with isolatedModules)
                exports.insert(
                    d.id.name.to_string(),
                    ExportInfo { is_type_only: false, re_export_source: None },
                );
            }
            Declaration::TSModuleDeclaration(d) => {
                // Module declarations (namespaces) can have runtime value
                use oxc_ast::ast::TSModuleDeclarationName;
                if let TSModuleDeclarationName::Identifier(id) = &d.id {
                    exports.insert(
                        id.name.to_string(),
                        ExportInfo { is_type_only: false, re_export_source: None },
                    );
                }
            }
            Declaration::TSImportEqualsDeclaration(_) | Declaration::TSGlobalDeclaration(_) => {
                // Import equals and global declarations - skip
            }
        }
    }

    /// Clear the analysis cache.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get the number of cached files.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(analyzer.is_type_only_import("./types", "User", &main_file));
    }

    #[test]
    fn test_type_alias_export_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export type UserId = string;");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(analyzer.is_type_only_import("./types", "UserId", &main_file));
    }

    #[test]
    fn test_class_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export class AuthService {}");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(!analyzer.is_type_only_import("./service", "AuthService", &main_file));
    }

    #[test]
    fn test_function_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "utils.ts", "export function helper() {}");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(!analyzer.is_type_only_import("./utils", "helper", &main_file));
    }

    #[test]
    fn test_const_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "constants.ts", "export const TOKEN = 'token';");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(!analyzer.is_type_only_import("./constants", "TOKEN", &main_file));
    }

    #[test]
    fn test_enum_export_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "enums.ts", "export enum Status { Active, Inactive }");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(!analyzer.is_type_only_import("./enums", "Status", &main_file));
    }

    #[test]
    fn test_re_export_interface_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export interface Foo {}");
        create_test_file(dir.path(), "index.ts", "export { Foo } from './types';");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(analyzer.is_type_only_import("./index", "Foo", &main_file));
    }

    #[test]
    fn test_re_export_class_is_not_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export class MyService {}");
        create_test_file(dir.path(), "index.ts", "export { MyService } from './service';");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(!analyzer.is_type_only_import("./index", "MyService", &main_file));
    }

    #[test]
    fn test_export_type_specifier_is_type_only() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export type { Foo } from './foo';");
        create_test_file(dir.path(), "foo.ts", "export class Foo {}"); // Even though Foo is a class
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(analyzer.is_type_only_import("./mixed", "User", &main_file));
        assert!(!analyzer.is_type_only_import("./mixed", "UserService", &main_file));
        assert!(analyzer.is_type_only_import("./mixed", "UserId", &main_file));
        assert!(!analyzer.is_type_only_import("./mixed", "USER_TOKEN", &main_file));
    }

    #[test]
    fn test_package_imports_are_conservative() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        // Package imports should return false (conservative - assume value)
        assert!(!analyzer.is_type_only_import("@angular/core", "Component", &main_file));
        assert!(!analyzer.is_type_only_import("rxjs", "Observable", &main_file));
    }

    #[test]
    fn test_nonexistent_file_is_conservative() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        // Non-existent file should return false (conservative)
        assert!(!analyzer.is_type_only_import("./nonexistent", "Foo", &main_file));
    }

    #[test]
    fn test_caching_works() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "types.ts", "export interface User {}");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);

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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(analyzer.is_type_only_import("./types", "default", &main_file));
    }

    #[test]
    fn test_default_export_class() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export default class MyService {}");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(!analyzer.is_type_only_import("./service", "default", &main_file));
    }

    // Tests for resolve_import_source_path

    #[test]
    fn test_resolve_direct_import() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "component.ts", "export class MyComponent {}");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        let resolved = analyzer.resolve_import_source_path("./index", "DeepComponent", &main_file);
        assert_eq!(resolved, Some("./deep/component".to_string()));
    }

    #[test]
    fn test_resolve_star_export() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "service.ts", "export class MyService {}");
        create_test_file(dir.path(), "index.ts", "export * from './service';");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);

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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(
            analyzer.resolve_import_source_path("@angular/core", "Component", &main_file).is_none()
        );
        assert!(analyzer.resolve_import_source_path("rxjs", "Observable", &main_file).is_none());
    }

    #[test]
    fn test_resolve_nonexistent_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        assert!(analyzer.resolve_import_source_path("./nonexistent", "Foo", &main_file).is_none());
    }

    #[test]
    fn test_resolve_nonexistent_export_returns_none() {
        let dir = TempDir::new().unwrap();
        create_test_file(dir.path(), "module.ts", "export class Exists {}");
        let main_file = dir.path().join("main.ts");

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
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

        let mut analyzer = CrossFileAnalyzer::new(dir.path(), None);
        let resolved = analyzer.resolve_import_source_path("./index", "RenamedExport", &main_file);
        assert_eq!(resolved, Some("./original".to_string()));
    }
}

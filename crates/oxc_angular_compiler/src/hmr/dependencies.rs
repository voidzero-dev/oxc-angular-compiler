//! HMR dependency tracking.
//!
//! This module tracks dependencies for components to determine what
//! needs to be reloaded when files change.
//!
//! Ported from Angular's `packages/compiler/src/render3/r3_hmr_compiler.ts`
//! and `packages/compiler-cli/src/ngtsc/hmr/src/extract_dependencies.ts`.

use std::collections::HashSet;

use oxc_allocator::Box;
use oxc_str::Ident;

use crate::output::ast::{
    ExternalExpr, OutputExpression, OutputStatement, ReadVarExpr, RecursiveOutputAstVisitor,
};

// ============================================================================
// HMR Namespace Dependency
// ============================================================================

/// HMR dependency on a namespace import.
///
/// When the compiler generates new imports, they get produced as namespace imports
/// (e.g. `import * as i0 from '@angular/core'`). These namespaces have to be captured
/// and passed along to the update callback.
///
/// See: `packages/compiler/src/render3/r3_hmr_compiler.ts:40-49`
#[derive(Debug, Clone)]
pub struct HmrNamespaceDependency<'a> {
    /// Module name of the import (e.g., `@angular/core`).
    pub module_name: Ident<'a>,

    /// Name under which to refer to the namespace inside HMR-related code.
    /// Must be a valid JS identifier (e.g., `i0`).
    pub assigned_name: Ident<'a>,
}

impl<'a> HmrNamespaceDependency<'a> {
    /// Create a new namespace dependency.
    pub fn new(module_name: Ident<'a>, assigned_name: Ident<'a>) -> Self {
        Self { module_name, assigned_name }
    }
}

// ============================================================================
// HMR Local Dependency
// ============================================================================

/// Local dependency for HMR update functions.
///
/// HMR update functions cannot contain imports so any locals the generated code
/// depends on (e.g. references to imports within the same file or imported symbols)
/// have to be passed in as function parameters.
///
/// See: `packages/compiler/src/render3/r3_hmr_compiler.ts:31-36`
#[derive(Debug)]
pub struct HmrLocalDependency<'a> {
    /// Name of the local symbol.
    pub name: Ident<'a>,

    /// Runtime representation of the local (the expression to pass as argument).
    pub runtime_representation: OutputExpression<'a>,
}

impl<'a> HmrLocalDependency<'a> {
    /// Create a new local dependency.
    pub fn new(name: Ident<'a>, runtime_representation: OutputExpression<'a>) -> Self {
        Self { name, runtime_representation }
    }
}

// ============================================================================
// HMR Metadata
// ============================================================================

/// Metadata necessary to compile HMR-related code.
///
/// See: `packages/compiler/src/render3/r3_hmr_compiler.ts:14-37`
#[derive(Debug)]
pub struct HmrMetadata<'a> {
    /// Component class for which HMR is being enabled.
    pub component_type: OutputExpression<'a>,

    /// Name of the component class.
    pub class_name: Ident<'a>,

    /// File path of the component class.
    pub file_path: Ident<'a>,

    /// Namespace dependencies (e.g., `import * as i0 from '@angular/core'`).
    ///
    /// When the compiler generates new imports, they get produced as namespace imports.
    /// These namespaces have to be captured and passed along to the update callback.
    pub namespace_dependencies: Vec<HmrNamespaceDependency<'a>>,

    /// Local dependencies that need to be passed to the update function.
    ///
    /// HMR update functions cannot contain imports so any locals the generated code
    /// depends on have to be passed in as function parameters.
    pub local_dependencies: Vec<HmrLocalDependency<'a>>,
}

impl<'a> HmrMetadata<'a> {
    /// Create new HMR metadata.
    pub fn new(
        component_type: OutputExpression<'a>,
        class_name: Ident<'a>,
        file_path: Ident<'a>,
    ) -> Self {
        Self {
            component_type,
            class_name,
            file_path,
            namespace_dependencies: Vec::new(),
            local_dependencies: Vec::new(),
        }
    }

    /// Add a namespace dependency.
    pub fn add_namespace_dependency(&mut self, module_name: Ident<'a>, assigned_name: Ident<'a>) {
        self.namespace_dependencies.push(HmrNamespaceDependency::new(module_name, assigned_name));
    }

    /// Add a local dependency.
    pub fn add_local_dependency(
        &mut self,
        name: Ident<'a>,
        runtime_representation: OutputExpression<'a>,
    ) {
        self.local_dependencies.push(HmrLocalDependency::new(name, runtime_representation));
    }
}

// ============================================================================
// HMR Dependency Collector Visitor
// ============================================================================

/// Visitor that collects potential top-level variable reads from compiled output.
///
/// This visitor traverses compiled output AST expressions and statements to find:
/// - Namespace reads from ExternalExpr (e.g., `@angular/core`)
/// - Local variable reads from ReadVarExpr
///
/// Ported from Angular's `PotentialTopLevelReadsVisitor` in `extract_dependencies.ts`.
pub struct HmrDependencyCollector<'a> {
    /// All variable reads found during traversal.
    pub local_reads: HashSet<&'a str>,

    /// All namespace (module) reads found during traversal.
    pub namespace_reads: Vec<&'a str>,

    /// Set to track unique namespace names (for deduplication).
    namespace_set: HashSet<&'a str>,
}

impl<'a> HmrDependencyCollector<'a> {
    /// Create a new dependency collector.
    pub fn new() -> Self {
        Self {
            local_reads: HashSet::new(),
            namespace_reads: Vec::new(),
            namespace_set: HashSet::new(),
        }
    }

    /// Get extracted namespace dependencies with assigned names.
    ///
    /// Returns a vector of (module_name, assigned_name) pairs.
    /// Assigned names follow the pattern `ɵhmr0`, `ɵhmr1`, etc.
    pub fn get_namespace_dependencies(&self) -> Vec<(&'a str, String)> {
        self.namespace_reads
            .iter()
            .enumerate()
            .map(|(index, &module_name)| (module_name, format!("ɵhmr{index}")))
            .collect()
    }

    /// Get local dependencies filtered by top-level symbols.
    ///
    /// # Arguments
    /// * `top_level_symbols` - Set of symbols defined at file scope
    /// * `class_name` - Name of the component class (to exclude)
    ///
    /// Returns variable names that are both referenced and defined at top-level.
    pub fn get_local_dependencies(
        &self,
        top_level_symbols: &HashSet<&str>,
        class_name: Option<&str>,
    ) -> Vec<&'a str> {
        self.local_reads
            .iter()
            .filter(|&&name| {
                // Exclude the class name since it's always present
                if let Some(cn) = class_name {
                    if name == cn {
                        return false;
                    }
                }
                // Only include if it's a top-level symbol
                top_level_symbols.contains(name)
            })
            .copied()
            .collect()
    }
}

impl Default for HmrDependencyCollector<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> RecursiveOutputAstVisitor<'a> for HmrDependencyCollector<'a> {
    fn visit_external(&mut self, expr: &ExternalExpr<'a>) {
        // Collect module names from external references
        if let Some(ref module_name) = expr.value.module_name {
            let name = module_name.as_str();
            if !self.namespace_set.contains(name) {
                self.namespace_set.insert(name);
                self.namespace_reads.push(name);
            }
        }
    }

    fn visit_read_var(&mut self, expr: &ReadVarExpr<'a>) {
        // Collect all variable reads
        self.local_reads.insert(expr.name.as_str());
    }
}

/// Result of extracting HMR dependencies from compiled expressions.
#[derive(Debug)]
pub struct ExtractedHmrDependencies<'a> {
    /// Local dependencies that need to be passed to the update function.
    pub local: Vec<LocalDependency<'a>>,

    /// Namespace dependencies (external module imports).
    pub external: Vec<HmrNamespaceDependency<'a>>,
}

/// A local dependency with its runtime representation.
#[derive(Debug)]
pub struct LocalDependency<'a> {
    /// Name of the local symbol.
    pub name: Ident<'a>,

    /// Runtime representation (the expression to pass as argument).
    pub runtime_representation: OutputExpression<'a>,
}

/// Extract HMR dependencies from compiled component expressions.
///
/// This function analyzes all compiled expressions (definition, factory, metadata)
/// to determine what dependencies the HMR update function needs.
///
/// # Arguments
/// * `allocator` - Allocator for creating output expressions
/// * `definition_expr` - Compiled component definition expression
/// * `definition_stmts` - Compiled component definition statements
/// * `factory_initializer` - Compiled factory initializer expression (optional)
/// * `factory_stmts` - Compiled factory statements
/// * `class_metadata` - Compiled setClassMetadata statement (optional)
/// * `debug_info` - Compiled setClassDebugInfo statement (optional)
/// * `top_level_symbols` - Set of symbols defined at file scope
/// * `class_name` - Name of the component class
///
/// # Returns
/// Extracted dependencies or None if extraction fails.
///
/// Ported from Angular's `extractHmrDependencies` in `extract_dependencies.ts`.
pub fn extract_compiled_dependencies<'a>(
    allocator: &'a oxc_allocator::Allocator,
    definition_expr: &OutputExpression<'a>,
    definition_stmts: &[OutputStatement<'a>],
    factory_initializer: Option<&OutputExpression<'a>>,
    factory_stmts: &[OutputStatement<'a>],
    class_metadata: Option<&OutputStatement<'a>>,
    debug_info: Option<&OutputStatement<'a>>,
    top_level_symbols: &HashSet<&str>,
    class_name: Option<&str>,
) -> ExtractedHmrDependencies<'a> {
    let mut collector = HmrDependencyCollector::new();

    // Visit all compiled expressions to collect dependencies
    collector.visit_expression(definition_expr);

    for stmt in definition_stmts {
        collector.visit_statement(stmt);
    }

    if let Some(init) = factory_initializer {
        collector.visit_expression(init);
    }

    for stmt in factory_stmts {
        collector.visit_statement(stmt);
    }

    if let Some(metadata) = class_metadata {
        collector.visit_statement(metadata);
    }

    if let Some(info) = debug_info {
        collector.visit_statement(info);
    }

    // Build namespace dependencies
    let external: Vec<HmrNamespaceDependency<'a>> = collector
        .get_namespace_dependencies()
        .into_iter()
        .map(|(module_name, assigned_name)| HmrNamespaceDependency {
            module_name: Ident::from(allocator.alloc_str(module_name)),
            assigned_name: Ident::from(allocator.alloc_str(&assigned_name)),
        })
        .collect();

    // Build local dependencies
    let local_names = collector.get_local_dependencies(top_level_symbols, class_name);
    let local: Vec<LocalDependency<'a>> = local_names
        .into_iter()
        .map(|name| {
            // Create variable reference as runtime representation
            let runtime_representation = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from(allocator.alloc_str(name)), source_span: None },
                allocator,
            ));
            LocalDependency { name: Ident::from(allocator.alloc_str(name)), runtime_representation }
        })
        .collect();

    ExtractedHmrDependencies { local, external }
}

// ============================================================================
// Legacy HMR Dependencies (for file-level tracking)
// ============================================================================

/// Dependencies tracked for HMR.
#[derive(Debug, Default)]
pub struct HmrDependencies {
    /// Template URLs that this component depends on.
    pub template_urls: HashSet<String>,

    /// Style URLs that this component depends on.
    pub style_urls: HashSet<String>,

    /// Other files this component depends on (e.g., imported modules).
    pub other_files: HashSet<String>,

    /// Local symbols referenced in the template.
    pub local_symbols: HashSet<String>,

    /// External namespace imports (e.g., `* as core from '@angular/core'`).
    pub namespace_imports: HashSet<String>,
}

impl HmrDependencies {
    /// Create new empty dependencies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a template URL dependency.
    pub fn add_template_url(&mut self, url: String) {
        self.template_urls.insert(url);
    }

    /// Add a style URL dependency.
    pub fn add_style_url(&mut self, url: String) {
        self.style_urls.insert(url);
    }

    /// Add a file dependency.
    pub fn add_file(&mut self, path: String) {
        self.other_files.insert(path);
    }

    /// Add a local symbol dependency.
    pub fn add_local_symbol(&mut self, symbol: String) {
        self.local_symbols.insert(symbol);
    }

    /// Add a namespace import dependency.
    pub fn add_namespace_import(&mut self, namespace: String) {
        self.namespace_imports.insert(namespace);
    }

    /// Get all file dependencies.
    pub fn all_files(&self) -> impl Iterator<Item = &String> {
        self.template_urls.iter().chain(self.style_urls.iter()).chain(self.other_files.iter())
    }

    /// Check if this component has any external dependencies.
    pub fn has_external_dependencies(&self) -> bool {
        !self.template_urls.is_empty() || !self.style_urls.is_empty()
    }
}

/// Extract HMR dependencies from component metadata.
///
/// This function analyzes a component's metadata to determine what
/// files and symbols it depends on for HMR.
///
/// # Arguments
///
/// * `template_url` - Optional template URL
/// * `style_urls` - Optional style URLs
/// * `file_path` - Path to the component file
///
/// # Returns
///
/// HMR dependencies for the component.
pub fn extract_hmr_dependencies(
    template_url: Option<&str>,
    style_urls: Option<&[String]>,
    file_path: &str,
) -> HmrDependencies {
    let mut deps = HmrDependencies::new();

    // Add template URL if present
    if let Some(url) = template_url {
        let resolved = resolve_relative_url(url, file_path);
        deps.add_template_url(resolved);
    }

    // Add style URLs if present
    if let Some(urls) = style_urls {
        for url in urls {
            let resolved = resolve_relative_url(url, file_path);
            deps.add_style_url(resolved);
        }
    }

    deps
}

/// Resolve a relative URL to an absolute path.
fn resolve_relative_url(url: &str, base_path: &str) -> String {
    if url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }

    // Get the directory of the base path
    let base_dir = if let Some(pos) = base_path.rfind('/') { &base_path[..pos] } else { "." };

    // Simple resolution - a real implementation would handle ./ and ../
    if url.starts_with("./") {
        format!("{}/{}", base_dir, &url[2..])
    } else if url.starts_with("../") {
        // Go up one directory
        let parent = if let Some(pos) = base_dir.rfind('/') { &base_dir[..pos] } else { "." };
        format!("{}/{}", parent, &url[3..])
    } else {
        format!("{}/{}", base_dir, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::ExternalReference;

    #[test]
    fn test_resolve_relative_url() {
        assert_eq!(
            resolve_relative_url("./app.component.html", "src/app/app.component.ts"),
            "src/app/app.component.html"
        );

        assert_eq!(
            resolve_relative_url("../shared/styles.css", "src/app/app.component.ts"),
            "src/shared/styles.css"
        );

        assert_eq!(
            resolve_relative_url("/absolute/path.html", "src/app/app.component.ts"),
            "/absolute/path.html"
        );
    }

    #[test]
    fn test_extract_dependencies() {
        let deps = extract_hmr_dependencies(
            Some("./app.component.html"),
            Some(&["./app.component.css".to_string()]),
            "src/app/app.component.ts",
        );

        assert!(deps.template_urls.contains("src/app/app.component.html"));
        assert!(deps.style_urls.contains("src/app/app.component.css"));
    }

    #[test]
    fn test_dependency_collector_local_reads() {
        let allocator = oxc_allocator::Allocator::default();
        let mut collector = HmrDependencyCollector::new();

        // Create a variable read expression
        let var_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("myVar"), source_span: None },
            &allocator,
        ));

        collector.visit_expression(&var_expr);

        assert!(collector.local_reads.contains("myVar"));
    }

    #[test]
    fn test_dependency_collector_namespace_reads() {
        let allocator = oxc_allocator::Allocator::default();
        let mut collector = HmrDependencyCollector::new();

        // Create an external expression
        let external_expr = OutputExpression::External(Box::new_in(
            ExternalExpr {
                value: ExternalReference {
                    module_name: Some(Ident::from("@angular/core")),
                    name: Some(Ident::from("Component")),
                },
                source_span: None,
            },
            &allocator,
        ));

        collector.visit_expression(&external_expr);

        assert!(collector.namespace_set.contains("@angular/core"));
        assert_eq!(collector.namespace_reads.len(), 1);
        assert_eq!(collector.namespace_reads[0], "@angular/core");
    }

    #[test]
    fn test_dependency_collector_namespace_deduplication() {
        let allocator = oxc_allocator::Allocator::default();
        let mut collector = HmrDependencyCollector::new();

        // Create two external expressions from the same module
        let expr1 = OutputExpression::External(Box::new_in(
            ExternalExpr {
                value: ExternalReference {
                    module_name: Some(Ident::from("@angular/core")),
                    name: Some(Ident::from("Component")),
                },
                source_span: None,
            },
            &allocator,
        ));

        let expr2 = OutputExpression::External(Box::new_in(
            ExternalExpr {
                value: ExternalReference {
                    module_name: Some(Ident::from("@angular/core")),
                    name: Some(Ident::from("Injectable")),
                },
                source_span: None,
            },
            &allocator,
        ));

        collector.visit_expression(&expr1);
        collector.visit_expression(&expr2);

        // Should only have one namespace entry despite two expressions
        assert_eq!(collector.namespace_reads.len(), 1);
    }

    #[test]
    fn test_dependency_collector_get_namespace_dependencies() {
        let allocator = oxc_allocator::Allocator::default();
        let mut collector = HmrDependencyCollector::new();

        // Add multiple namespaces
        let expr1 = OutputExpression::External(Box::new_in(
            ExternalExpr {
                value: ExternalReference {
                    module_name: Some(Ident::from("@angular/core")),
                    name: Some(Ident::from("Component")),
                },
                source_span: None,
            },
            &allocator,
        ));

        let expr2 = OutputExpression::External(Box::new_in(
            ExternalExpr {
                value: ExternalReference {
                    module_name: Some(Ident::from("./my-dep")),
                    name: Some(Ident::from("MyDep")),
                },
                source_span: None,
            },
            &allocator,
        ));

        collector.visit_expression(&expr1);
        collector.visit_expression(&expr2);

        let deps = collector.get_namespace_dependencies();

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0], ("@angular/core", "ɵhmr0".to_string()));
        assert_eq!(deps[1], ("./my-dep", "ɵhmr1".to_string()));
    }

    #[test]
    fn test_dependency_collector_filter_by_top_level() {
        let allocator = oxc_allocator::Allocator::default();
        let mut collector = HmrDependencyCollector::new();

        // Add some variable reads
        for name in ["MyService", "localVar", "AppComponent", "helperFn"] {
            let expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from(name), source_span: None },
                &allocator,
            ));
            collector.visit_expression(&expr);
        }

        // Define top-level symbols (excluding localVar which is not top-level)
        let mut top_level = HashSet::new();
        top_level.insert("MyService");
        top_level.insert("AppComponent");
        top_level.insert("helperFn");

        // Filter with class name excluded
        let deps = collector.get_local_dependencies(&top_level, Some("AppComponent"));

        // Should only include MyService and helperFn (not localVar or AppComponent)
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"MyService"));
        assert!(deps.contains(&"helperFn"));
        assert!(!deps.contains(&"localVar"));
        assert!(!deps.contains(&"AppComponent"));
    }

    #[test]
    fn test_extract_compiled_dependencies() {
        let allocator = oxc_allocator::Allocator::default();

        // Create a simple definition expression with external and local refs
        let def_expr = OutputExpression::External(Box::new_in(
            ExternalExpr {
                value: ExternalReference {
                    module_name: Some(Ident::from("@angular/core")),
                    name: Some(Ident::from("ɵɵdefineComponent")),
                },
                source_span: None,
            },
            &allocator,
        ));

        // Top-level symbols
        let mut top_level = HashSet::new();
        top_level.insert("MyService");

        let result = extract_compiled_dependencies(
            &allocator,
            &def_expr,
            &[],
            None,
            &[],
            None,
            None,
            &top_level,
            Some("AppComponent"),
        );

        // Should have extracted the @angular/core namespace
        assert_eq!(result.external.len(), 1);
        assert_eq!(result.external[0].module_name.as_str(), "@angular/core");
        assert_eq!(result.external[0].assigned_name.as_str(), "ɵhmr0");
    }
}

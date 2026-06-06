//! Angular file transformation.
//!
//! This module provides the main entry point for transforming TypeScript files
//! containing Angular components into compiled JavaScript.

use std::collections::HashMap;

use std::path::Path;

use oxc_allocator::{Allocator, Vec as OxcVec};
use oxc_ast::ast::{
    Argument, Declaration, ExportDefaultDeclarationKind, Expression, ImportDeclarationSpecifier,
    ImportOrExportKind, ModuleExportName, ObjectPropertyKind, PropertyKey, Statement,
};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_str::Ident;
use rustc_hash::FxHashMap;

use crate::optimizer::{Edit, apply_edits, apply_edits_with_sourcemap};

#[cfg(feature = "cross_file_elision")]
use super::cross_file_elision::CrossFileAnalyzer;
use super::decorator::{
    collect_constructor_decorator_spans, collect_member_decorator_spans,
    extract_component_metadata, find_component_decorator, find_component_decorator_span,
};
use super::definition::{const_value_to_expression, generate_component_definitions};
use super::hoist::{collect_hoist_edits, program_has_angular_decorated_class};
use super::import_elision::{ImportElisionAnalyzer, import_elision_edits};
use super::metadata::{AngularVersion, ComponentMetadata, HostMetadata};
use super::namespace_registry::NamespaceRegistry;
use crate::ast::expression::{BindingType, ParsedEventType};
use crate::ast::r3::{R3BoundAttribute, R3BoundEvent, SecurityContext};
use crate::class_metadata::{
    R3ClassMetadata, R3DeferPerComponentDependency, build_ctor_params_metadata,
    build_decorator_metadata_array, build_prop_decorators_metadata, compile_class_metadata,
    compile_component_class_metadata,
};
use crate::directive::collect_string_consts;
use crate::directive::{
    R3QueryMetadata, create_content_queries_function, create_view_queries_function,
    extract_content_queries, extract_directive_metadata, extract_view_queries,
    find_directive_decorator, find_directive_decorator_span, generate_directive_definitions,
};
use crate::dts;
use crate::injectable::{
    extract_injectable_metadata, find_injectable_decorator, find_injectable_decorator_span,
    generate_injectable_definition_from_decorator,
};
use crate::ng_module::{
    extract_ng_module_metadata, find_ng_module_decorator, find_ng_module_decorator_span,
    generate_full_ng_module_definition,
};
use crate::output::ast::{
    DeclareFunctionStmt, FunctionExpr, OutputExpression, OutputStatement, ReadPropExpr,
    ReadVarExpr, StmtModifier,
};
use crate::output::emitter::JsEmitter;
use crate::parser::ParseTemplateOptions;
use crate::parser::expression::BindingParser;
use crate::parser::html::{HtmlParser, remove_whitespaces};
use crate::pipe::{
    extract_pipe_metadata, find_pipe_decorator, find_pipe_decorator_span,
    generate_full_pipe_definition_from_decorator,
};
use crate::pipeline::compilation::{DeferBlockDepsEmitMode, TemplateCompilationMode};
use crate::pipeline::emit::{
    HostBindingCompilationResult, compile_host_bindings, compile_template,
};
use crate::pipeline::ingest::{
    HostBindingInput, IngestOptions, ingest_component, ingest_component_with_options,
    ingest_host_binding_with_version,
};
use crate::service::{extract_service_metadata, generate_service_definition_from_decorator};
use crate::transform::HtmlToR3Transform;
use crate::transform::html_to_r3::TransformOptions as R3TransformOptions;

/// Options for Angular file transformation.
#[derive(Debug, Clone)]
pub struct TransformOptions {
    /// Generate source maps.
    pub sourcemap: bool,

    /// Enable JIT (Just-In-Time) compilation mode.
    /// When true, generates code compatible with JIT compilation.
    pub jit: bool,

    /// Enable HMR (Hot Module Replacement) support.
    /// When true, generates HMR initialization and update code.
    pub hmr: bool,

    /// Enable advanced optimizations.
    /// When true, applies additional optimizations like constant folding.
    pub advanced_optimizations: bool,

    /// i18n message ID strategy.
    ///
    /// When true (default), uses external message IDs for Closure Compiler
    /// variable naming (MSG_EXTERNAL_abc123$$SUFFIX).
    /// When false, uses file-based naming (MSG_SUFFIX_0).
    pub i18n_use_external_ids: bool,

    /// Angular core version for version-conditional behavior.
    ///
    /// When set, used to determine defaults like:
    /// - `standalone`: defaults to `false` for v18 and earlier, `true` for v19+
    ///
    /// When `None`, assumes latest Angular version (v19+ behavior).
    pub angular_version: Option<AngularVersion>,

    /// Override for the `legacyOptionalChaining` Angular compiler option.
    ///
    /// Controls how the safe-navigation operator (`?.`) in template expressions is
    /// emitted. When `Some(true)`, always uses the legacy `== null ? null` ternary;
    /// when `Some(false)`, always emits native optional chaining (yielding
    /// `undefined`). When `None`, the default is derived from `angular_version`
    /// (legacy for < v22, modern for >= v22, legacy when the version is unknown).
    ///
    /// See `angular/angular@2896c93cc1`.
    pub legacy_optional_chaining: Option<bool>,

    // Component metadata overrides for template-only compilation.
    // These allow the build tool to pass component metadata when compiling
    // templates in isolation (e.g., for testing or compare tool).
    /// Override the CSS selector for the component.
    pub selector: Option<String>,

    /// Override the standalone flag for the component.
    pub standalone: Option<bool>,

    /// Override the view encapsulation mode.
    pub encapsulation: Option<super::metadata::ViewEncapsulation>,

    /// Override the change detection strategy.
    pub change_detection: Option<super::metadata::ChangeDetectionStrategy>,

    /// Override the preserve whitespaces setting.
    pub preserve_whitespaces: Option<bool>,

    /// Host bindings metadata for the component.
    /// Contains property bindings, attribute bindings, and event listeners.
    pub host: Option<HostMetadataInput>,

    /// Enable cross-file import elision analysis.
    ///
    /// When true, resolves imports to source files to check if exports are type-only.
    /// This improves import elision accuracy by detecting interfaces and type aliases
    /// in external files.
    ///
    /// **Note**: This is intended for compare tests only. In production, bundlers
    /// like rolldown handle import elision as part of their tree-shaking process.
    #[cfg(feature = "cross_file_elision")]
    pub cross_file_elision: bool,

    /// Base directory for module resolution.
    ///
    /// Used when `cross_file_elision` is enabled to resolve relative imports.
    /// Defaults to the directory containing the source file if not specified.
    #[cfg(feature = "cross_file_elision")]
    pub base_dir: Option<std::path::PathBuf>,

    /// Path to tsconfig.json for path aliases.
    ///
    /// Used when `cross_file_elision` is enabled to resolve path aliases
    /// defined in tsconfig.json (e.g., `@app/*` -> `src/app/*`).
    #[cfg(feature = "cross_file_elision")]
    pub tsconfig_path: Option<std::path::PathBuf>,

    /// Resolved import paths for host directives and other imports.
    ///
    /// Maps local identifier name (e.g., "AriaDisableDirective") to the resolved
    /// module path (e.g., "../a11y/aria-disable.directive").
    ///
    /// This is used to override barrel export paths with actual file paths.
    /// When an identifier is found in this map, the resolved path is used
    /// instead of the original import declaration's source.
    ///
    /// Example:
    /// ```text
    /// // Original import uses barrel export:
    /// import { AriaDisableDirective } from '../a11y';
    ///
    /// // resolved_imports maps to actual file:
    /// { "AriaDisableDirective": "../a11y/aria-disable.directive" }
    /// ```
    pub resolved_imports: Option<HashMap<String, String>>,

    /// Emit setClassMetadata() calls for TestBed support.
    ///
    /// When true, generates `ɵɵsetClassMetadata()` calls wrapped in
    /// `(typeof ngDevMode === "undefined" || ngDevMode) && …`. Production bundles
    /// tree-shake the guarded call. Preserves original decorator information for
    /// TestBed's recompilation APIs.
    ///
    /// Default: true — matches `ngc`, which always emits class metadata.
    pub emit_class_metadata: bool,

    /// Minify final component styles before emitting them into `styles: [...]`.
    ///
    /// This runs after Angular style encapsulation, so it applies to the same
    /// final CSS strings that are embedded in component definitions.
    pub minify_component_styles: bool,

    /// Selects between full Ivy emit (`ɵɵdefine*`) and partial-declaration
    /// emit (`ɵɵngDeclare*`).
    ///
    /// `Full` (default) targets applications; `Partial` targets library
    /// builds, where consumers run the linker to expand declarations.
    pub compilation_mode: crate::CompilationMode,
}

/// Input for host metadata when passed via TransformOptions.
/// Uses owned String types for easier NAPI interop.
#[derive(Debug, Clone, Default)]
pub struct HostMetadataInput {
    /// Host property bindings: `{ "[class.active]": "isActive" }`
    pub properties: Vec<(String, String)>,

    /// Host attribute bindings: `{ "role": "button" }`
    pub attributes: Vec<(String, String)>,

    /// Host event listeners: `{ "(click)": "onClick()" }`
    pub listeners: Vec<(String, String)>,

    /// Special attribute for static class binding.
    pub class_attr: Option<String>,

    /// Special attribute for static style binding.
    pub style_attr: Option<String>,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            sourcemap: false,
            jit: false,
            hmr: false,
            advanced_optimizations: false,
            i18n_use_external_ids: true,    // Angular's JIT default
            angular_version: None,          // None means assume latest (v19+ behavior)
            legacy_optional_chaining: None, // None: derive default from angular_version
            // Metadata overrides default to None (use extracted/default values)
            selector: None,
            standalone: None,
            encapsulation: None,
            change_detection: None,
            preserve_whitespaces: None,
            host: None,
            // Cross-file elision options (feature-gated)
            #[cfg(feature = "cross_file_elision")]
            cross_file_elision: false,
            #[cfg(feature = "cross_file_elision")]
            base_dir: None,
            #[cfg(feature = "cross_file_elision")]
            tsconfig_path: None,
            // Resolved imports for host directives
            resolved_imports: None,
            // Class metadata for TestBed support — matches ngc, which always emits
            // it; production bundles strip the guarded call via tree-shaking.
            emit_class_metadata: true,
            minify_component_styles: false,
            compilation_mode: crate::CompilationMode::Full,
        }
    }
}

impl TransformOptions {
    /// Compute the implicit standalone value based on the Angular version.
    ///
    /// - Returns `true` for Angular v19+ or when version is unknown (None)
    /// - Returns `false` for Angular v18 and earlier
    pub fn implicit_standalone(&self) -> bool {
        self.angular_version.map(|v| v.supports_implicit_standalone()).unwrap_or(true) // Default to true when version is unknown
    }
}

/// Pre-resolved external resources for component transformation.
///
/// The build tool (e.g., Vite) resolves `templateUrl` and `styleUrls` before
/// calling the Rust compiler, since file I/O and preprocessing (SCSS, etc.)
/// needs to happen in JavaScript.
#[derive(Debug, Default)]
pub struct ResolvedResources {
    /// Map from templateUrl path to resolved template content.
    pub templates: HashMap<String, String>,

    /// Map from styleUrl path to resolved (preprocessed) style content.
    pub styles: HashMap<String, Vec<String>>,
}

/// Result of transforming an Angular file.
#[derive(Debug, Default)]
pub struct TransformResult {
    /// The transformed code.
    pub code: String,

    /// Source map (if sourcemap option was enabled).
    pub map: Option<String>,

    /// Files this file depends on (for watch mode).
    /// Includes template URLs and style URLs.
    pub dependencies: Vec<String>,

    /// Template updates for HMR.
    /// Maps component ID (path@ClassName) to compiled template function.
    pub template_updates: HashMap<String, String>,

    /// Style updates for HMR.
    /// Maps component ID to list of styles.
    pub style_updates: HashMap<String, Vec<String>>,

    /// Compilation diagnostics (errors and warnings).
    pub diagnostics: Vec<OxcDiagnostic>,

    /// Number of components found in the file.
    pub component_count: usize,

    /// `.d.ts` type declarations for Angular classes.
    ///
    /// Each entry contains the class name and the static member declarations
    /// that should be injected into the corresponding `.d.ts` class body.
    /// This enables library builds to include proper Ivy type declarations
    /// for template type-checking by consumers.
    ///
    /// The declarations use `i0` as the namespace alias for `@angular/core`.
    /// Consumers must ensure their `.d.ts` files include:
    /// ```typescript
    /// import * as i0 from "@angular/core";
    /// ```
    pub dts_declarations: Vec<crate::dts::DtsDeclaration>,
}

impl TransformResult {
    /// Create a new empty transform result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any errors.
    pub fn has_errors(&self) -> bool {
        use miette::Diagnostic;
        use oxc_diagnostics::Severity;
        self.diagnostics
            .iter()
            .any(|d| d.severity() == Some(Severity::Error) || d.severity().is_none())
    }

    /// Check if there are any warnings.
    pub fn has_warnings(&self) -> bool {
        use miette::Diagnostic;
        use oxc_diagnostics::Severity;
        self.diagnostics.iter().any(|d| d.severity() == Some(Severity::Warning))
    }
}

/// Result of compiling a template to JavaScript.
#[derive(Debug, Default)]
pub struct TemplateCompileOutput {
    /// The compiled template function as JavaScript code.
    pub code: String,

    /// Source map (if sourcemap option was enabled).
    pub map: Option<oxc_sourcemap::OwnedSourceMap>,
}

impl TemplateCompileOutput {
    /// Create a new template compile output with just code.
    pub fn new(code: String) -> Self {
        Self { code, map: None }
    }

    /// Create a new template compile output with code and source map.
    pub fn with_source_map(code: String, map: Option<oxc_sourcemap::OwnedSourceMap>) -> Self {
        Self { code, map }
    }
}

/// Output from compiling a template for HMR.
/// Includes the template function, constant declarations, extracted styles, and consts array.
#[derive(Debug)]
pub struct HmrTemplateCompileOutput {
    /// The compiled template function as JavaScript code.
    pub template_js: String,

    /// Constant declarations (child view functions, pooled constants) as JavaScript code.
    /// These need to be included in the HMR update module before the component definition.
    pub declarations_js: String,

    /// Styles extracted from `<style>` tags in the template.
    /// These must be included in the HMR update module to avoid constant pool mismatches.
    pub styles: std::vec::Vec<String>,

    /// The consts array as JavaScript code.
    /// This must be included in the HMR update module to match the template function's
    /// constant references. Without this, the template may reference indices that don't
    /// exist in the old component definition's consts array.
    pub consts_js: Option<String>,
}

/// Compiled component information.
#[derive(Debug)]
pub struct CompiledComponent<'a> {
    /// Component metadata.
    pub metadata: ComponentMetadata<'a>,

    /// Compiled template function.
    pub template_fn: Option<FunctionExpr<'a>>,

    /// Compiled template as JavaScript string.
    pub template_js: Option<String>,
}

/// Map from imported identifier name to its source module path.
///
/// This is used to track where constructor dependency tokens come from,
/// enabling proper namespace aliasing in the compiled output.
///
/// Example:
/// ```typescript
/// import { AuthService } from "@bitwarden/common/auth/abstractions/auth.service";
/// import { ServiceA, ServiceB } from "./services";
/// ```
/// Results in:
/// ```text
/// {
///   "AuthService" -> "@bitwarden/common/auth/abstractions/auth.service",
///   "ServiceA" -> "./services",
///   "ServiceB" -> "./services"
/// }
/// ```
/// Information about an imported symbol.
#[derive(Debug, Clone)]
pub struct ImportInfo<'a> {
    /// The source module path (e.g., "@angular/core", "./services").
    pub source_module: Ident<'a>,
    /// Whether this is a named import that can be reused with bare name.
    /// True for: `import { AuthService } from "module"`
    /// False for: `import * as core from "module"` (namespace imports)
    pub is_named_import: bool,
    /// Whether this is a type-only import (`import type { X }` or `import { type X }`).
    /// Type-only imports are erased at runtime and should not generate namespace
    /// imports for `setClassMetadata()` type references.
    pub is_type_only: bool,
    /// Original exported name on the source module, when it differs from the
    /// local binding name. Used to build correct `import('./mod').then(m => m.X)`
    /// chains for `@defer` dependency resolvers.
    ///
    /// - `import { Foo }` → `None` (export name equals local name)
    /// - `import { Foo as Bar }` → `Some("Foo")` (use original exported name)
    /// - `import Foo from "./mod"` → `Some("default")` (default export)
    /// - `import * as ns from "./mod"` → `None` (namespace, not deferrable)
    pub imported_name: Option<Ident<'a>>,
}

/// Map from local identifier name to its import information.
///
/// Used to look up where a constructor dependency token was imported from
/// and whether it can be reused with a bare name or requires namespace prefix.
pub type ImportMap<'a> = FxHashMap<Ident<'a>, ImportInfo<'a>>;

/// Build an import map from the program's import declarations.
///
/// Extracts all imports and maps the local identifier name to the
/// source module path and import type. This enables:
/// - Looking up where a constructor dependency token was imported from
/// - Determining if the import can be reused with bare name (named import)
///   or requires namespace prefix (namespace import)
///
/// # Arguments
///
/// * `allocator` - Memory allocator for creating new Atoms
/// * `program_body` - The program's statement list
/// * `resolved_imports` - Optional map of identifier names to resolved module paths.
///   When provided, these paths override the source module from the import declaration.
///   This is used to resolve barrel exports to actual file paths.
///
/// Handles:
/// - Named imports: `import { AuthService } from "@angular/core"`
///   -> `is_named_import: true` (can use bare `AuthService`)
/// - Named imports with alias: `import { AuthService as Auth } from "@angular/core"`
///   -> `is_named_import: true` (can use bare `Auth`)
/// - Default imports: `import DefaultService from "@bitwarden/common"`
///   -> `is_named_import: true` (can use bare `DefaultService`)
/// - Namespace imports: `import * as core from "@angular/core"`
///   -> `is_named_import: false` (need namespace prefix)
pub fn build_import_map<'a>(
    allocator: &'a Allocator,
    program_body: &[Statement<'a>],
    resolved_imports: Option<&HashMap<String, String>>,
) -> ImportMap<'a> {
    let mut import_map = ImportMap::default();

    for stmt in program_body {
        let Statement::ImportDeclaration(import_decl) = stmt else {
            continue;
        };

        let default_source_module = import_decl.source.value.clone();

        // `import type { ... }` makes all specifiers type-only
        let decl_is_type_only = import_decl.import_kind == ImportOrExportKind::Type;

        // Process all specifiers
        let Some(specifiers) = &import_decl.specifiers else {
            // Side-effect import: `import 'foo'` - no identifiers to map
            continue;
        };

        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(spec) => {
                    // Named import: `import { AuthService } from "module"`
                    // or aliased: `import { AuthService as Auth } from "module"`
                    // We use the local name as the key
                    // Named imports CAN be reused with bare name
                    let local_name: Ident<'a> = spec.local.name.clone().into();

                    // Type-only if the declaration is `import type { ... }` or the specifier
                    // is `import { type X }` (inline type specifier)
                    let is_type_only =
                        decl_is_type_only || spec.import_kind == ImportOrExportKind::Type;

                    // Check if we have a resolved path for this identifier
                    let source_module = resolved_imports
                        .and_then(|m| m.get(local_name.as_str()))
                        .map(|resolved| Ident::from(allocator.alloc_str(resolved)))
                        .unwrap_or_else(|| default_source_module.clone().into());

                    // Capture the original exported name when it differs from the
                    // local binding (i.e., `import { Foo as Bar }`). This is used
                    // when building `@defer` dependency resolvers so the dynamic
                    // import chain references the original export, not the alias.
                    let imported_export_name = module_export_name_to_str(&spec.imported);
                    let imported_name = imported_export_name
                        .filter(|exported| *exported != local_name.as_str())
                        .map(|exported| Ident::from(allocator.alloc_str(exported)));

                    import_map.insert(
                        local_name,
                        ImportInfo {
                            source_module,
                            is_named_import: true,
                            is_type_only,
                            imported_name,
                        },
                    );
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) => {
                    // Default import: `import DefaultService from "module"`
                    // Default imports CAN be reused with bare name
                    let local_name: Ident<'a> = spec.local.name.clone().into();

                    // Check if we have a resolved path for this identifier
                    let source_module = resolved_imports
                        .and_then(|m| m.get(local_name.as_str()))
                        .map(|resolved| Ident::from(allocator.alloc_str(resolved)))
                        .unwrap_or_else(|| default_source_module.clone().into());

                    import_map.insert(
                        local_name,
                        ImportInfo {
                            source_module,
                            is_named_import: true,
                            is_type_only: decl_is_type_only,
                            // Default imports always resolve to `m.default` in
                            // dynamic-import chains, regardless of the local
                            // binding name.
                            imported_name: Some(Ident::from("default")),
                        },
                    );
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
                    // Namespace import: `import * as core from "module"`
                    // Namespace imports CANNOT be reused with bare name for individual symbols
                    let local_name: Ident<'a> = spec.local.name.clone().into();

                    // Check if we have a resolved path for this identifier
                    let source_module = resolved_imports
                        .and_then(|m| m.get(local_name.as_str()))
                        .map(|resolved| Ident::from(allocator.alloc_str(resolved)))
                        .unwrap_or_else(|| default_source_module.clone().into());

                    import_map.insert(
                        local_name,
                        ImportInfo {
                            source_module,
                            is_named_import: false,
                            is_type_only: decl_is_type_only,
                            imported_name: None,
                        },
                    );
                }
            }
        }
    }

    import_map
}

/// Extract a `ModuleExportName` as a plain string slice when it's a textual
/// name. Quoted-string export names (e.g., `import { "string-name" as foo }`)
/// are not supported for deferrable resolution and return `None`.
fn module_export_name_to_str<'a>(name: &ModuleExportName<'a>) -> Option<&'a str> {
    match name {
        ModuleExportName::IdentifierName(id) => Some(id.name.as_str()),
        ModuleExportName::IdentifierReference(id) => Some(id.name.as_str()),
        ModuleExportName::StringLiteral(_) => None,
    }
}

/// Find the byte position (in the source) just after the last import statement.
///
/// Resolve namespace imports for factory dependency tokens.
///
/// The import elision phase removes type-only imports (e.g., `import { Store } from '@ngrx/store'`)
/// because constructor parameter types are considered type-only. However, the factory function
/// needs to reference these types at runtime (e.g., `i0.ɵɵinject(Store)`).
///
/// This function replaces bare `ReadVar` tokens with namespace-prefixed `ReadProp` references
/// (e.g., `Store` → `i1.Store`) for any token that has a corresponding import in the import map.
/// This ensures the factory works correctly even after import elision.
fn resolve_factory_dep_namespaces<'a>(
    allocator: &'a Allocator,
    deps: &mut oxc_allocator::Vec<'a, crate::factory::R3DependencyMetadata<'a>>,
    import_map: &ImportMap<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) {
    for dep in deps.iter_mut() {
        let Some(ref token) = dep.token else { continue };
        // Only process bare variable references (ReadVar)
        let OutputExpression::ReadVar(var) = token else { continue };
        let name = &var.name;
        // Look up this identifier in the import map
        let Some(import_info) = import_map.get(name) else { continue };
        if import_info.is_type_only {
            // Type-only imports (`import type { X }` / `import { type X }`) are erased
            // at runtime, so they cannot be used as DI tokens. Flag the dep so the
            // factory collapses to `ɵɵinvalidFactory()` and avoid registering a
            // namespace that would otherwise add a runtime import for the module.
            // See issue #288.
            dep.token = None;
            dep.type_only_invalid = true;
            continue;
        }
        // Replace with namespace-prefixed reference: i1.Store instead of Store
        let namespace = namespace_registry.get_or_assign(&import_info.source_module);
        dep.token = Some(OutputExpression::ReadProp(oxc_allocator::Box::new_in(
            ReadPropExpr {
                receiver: oxc_allocator::Box::new_in(
                    OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                        ReadVarExpr { name: namespace, source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: name.clone(),
                optional: false,
                source_span: None,
            },
            allocator,
        )));
    }
}

/// Resolves namespace imports for host directive references in `R3HostDirectiveMetadata`.
///
/// Replaces bare `ReadVar("X")` references with namespace-prefixed `ReadProp(ReadVar("i1"), "X")`
/// for any host directive that has a corresponding import in the import map.
/// This ensures the compiled output works correctly even after import elision.
fn resolve_host_directive_namespaces<'a>(
    allocator: &'a Allocator,
    host_directives: &mut oxc_allocator::Vec<'a, crate::R3HostDirectiveMetadata<'a>>,
    import_map: &ImportMap<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) {
    for hd in host_directives.iter_mut() {
        // Only process bare variable references (ReadVar)
        let OutputExpression::ReadVar(ref var) = hd.directive else { continue };
        let name = &var.name;
        // Look up this identifier in the import map
        let Some(import_info) = import_map.get(name) else { continue };
        // Replace with namespace-prefixed reference: i1.BrnTooltipTrigger instead of BrnTooltipTrigger
        let namespace = namespace_registry.get_or_assign(&import_info.source_module);
        hd.directive = OutputExpression::ReadProp(oxc_allocator::Box::new_in(
            ReadPropExpr {
                receiver: oxc_allocator::Box::new_in(
                    OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                        ReadVarExpr { name: namespace, source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: name.clone(),
                optional: false,
                source_span: None,
            },
            allocator,
        ));
    }
}

/// Compute the effective start position for a class statement in the original source.
///
/// This accounts for non-Angular decorators that remain on the class after Angular
/// decorator removal. The effective start is the minimum of `stmt_start` and the
/// earliest remaining (non-removed) decorator's start position.
fn compute_effective_start(
    class: &oxc_ast::ast::Class,
    decorator_spans_to_remove: &[Span],
    stmt_start: u32,
) -> u32 {
    class
        .decorators
        .iter()
        .filter(|d| !decorator_spans_to_remove.contains(&d.span))
        .map(|d| d.span.start)
        .min()
        .map_or(stmt_start, |dec_start| dec_start.min(stmt_start))
}

/// This is used to determine where to insert namespace imports so they appear
/// AFTER existing imports but BEFORE other code (like class declarations).
///
/// Returns `Some(position)` if imports were found, or `None` if no imports exist.
fn find_last_import_end(program_body: &[Statement<'_>]) -> Option<usize> {
    let mut last_import_end: Option<u32> = None;

    for stmt in program_body {
        // Check for import declarations and import equals declarations
        // These are the import statement types that Angular's TypeScript transform considers
        // See: packages/compiler-cli/src/ngtsc/translator/src/import_manager/import_typescript_transform.ts
        let span_end = match stmt {
            Statement::ImportDeclaration(import_decl) => Some(import_decl.span.end),
            // Note: TypeScript import equals (import x = require()) are not common in Angular
            // but we handle them for completeness
            _ => None,
        };

        if let Some(end) = span_end {
            last_import_end = Some(last_import_end.map_or(end, |prev| prev.max(end)));
        }
    }

    last_import_end.map(|pos| pos as usize)
}

/// Build the `ɵsetClassMetadata(...)` declaration string for a non-`@Component`
/// decorated class (`@Directive`/`@Pipe`/`@Injectable`/`@NgModule`).
///
/// Mirrors the `@Component` metadata block (without template/style inlining):
/// emits the decorator metadata, `ctorParameters` (reflected from the class, with
/// imported token types namespace-prefixed via the import map), and prop decorators
/// (real `@Input`/`@Output`/query plus synthesized initializer-API). Returns an
/// empty string when metadata emission is disabled. Matches ngc, which emits
/// `setClassMetadata` for all decorated classes (needed for TestBed overrides).
#[allow(clippy::too_many_arguments)]
fn build_set_class_metadata_decls<'a>(
    allocator: &'a Allocator,
    class: &oxc_ast::ast::Class<'a>,
    class_name: &str,
    decorator: &oxc_ast::ast::Decorator<'a>,
    options: &TransformOptions,
    source: &'a str,
    string_consts: &crate::directive::StringConsts<'a>,
    import_map: &ImportMap<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> String {
    if !options.emit_class_metadata || options.advanced_optimizations {
        return String::new();
    }

    let type_expr = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
        ReadVarExpr { name: Ident::from(class_name), source_span: None },
        allocator,
    ));
    let class_metadata = R3ClassMetadata {
        r#type: type_expr,
        decorators: build_decorator_metadata_array(
            allocator,
            &[decorator],
            Some(source),
            None,
            None,
            Some(string_consts),
        ),
        ctor_parameters: build_ctor_params_metadata(
            allocator,
            class,
            None,
            namespace_registry,
            import_map,
            Some(source),
        ),
        prop_decorators: build_prop_decorators_metadata(
            allocator,
            class,
            Some(source),
            namespace_registry,
        ),
    };
    let metadata_expr = match options.compilation_mode {
        crate::CompilationMode::Full => compile_class_metadata(allocator, &class_metadata),
        crate::CompilationMode::Partial => {
            crate::partial::compile_declare_class_metadata(allocator, &class_metadata)
        }
    };
    let emitter = JsEmitter::new();
    format!("{};", emitter.emit_expression(&metadata_expr))
}

// ============================================================================
// JIT Compilation Transform
// ============================================================================

/// Identifies which Angular decorator type a class has.
#[derive(Debug, Clone, Copy)]
enum AngularDecoratorKind {
    Component,
    Directive,
    Pipe,
    Injectable,
    Service,
    NgModule,
}

/// Information about an Angular-decorated class for JIT transformation.
struct JitClassInfo {
    /// The class name.
    class_name: String,
    /// Spans of ALL class-level decorators (including @) to be removed.
    all_class_decorator_spans: std::vec::Vec<Span>,
    /// Start of the statement (includes export keyword if present).
    stmt_start: u32,
    /// Start of the class keyword.
    class_start: u32,
    /// End of the class body (the closing `}`).
    class_body_end: u32,
    /// Whether the class is exported (not default).
    is_exported: bool,
    /// Whether the class is export default.
    is_default_export: bool,
    /// Whether the class is abstract.
    is_abstract: bool,
    /// Constructor parameter info for ctorParameters.
    ctor_params: std::vec::Vec<JitCtorParam>,
    /// Member decorator info for propDecorators (Angular decorators like @Input, @Output).
    member_decorators: std::vec::Vec<JitMemberDecorator>,
    /// All class-level decorator expression texts for __decorate call, in source order.
    all_class_decorator_texts: std::vec::Vec<String>,
    /// Non-Angular member decorators that need __decorate() calls.
    non_angular_member_decorators: std::vec::Vec<JitNonAngularMemberDecorator>,
}

/// Constructor parameter info for JIT ctorParameters generation.
struct JitCtorParam {
    /// The type name (if resolvable to a runtime value).
    type_name: Option<String>,
    /// Angular decorators on the parameter, as source text spans.
    decorators: std::vec::Vec<JitParamDecorator>,
}

/// A single decorator on a constructor parameter.
struct JitParamDecorator {
    /// The decorator name (e.g., "Optional", "Inject").
    name: String,
    /// The decorator arguments as source text (e.g., "TOKEN" for @Inject(TOKEN)).
    args: Option<String>,
}

/// A member (property/method) with its Angular decorators for propDecorators.
struct JitMemberDecorator {
    /// The property/member name.
    member_name: String,
    /// The Angular decorators on this member.
    decorators: std::vec::Vec<JitParamDecorator>,
}

/// A non-Angular member decorator that needs to be lowered via __decorate().
struct JitNonAngularMemberDecorator {
    /// The member name.
    member_name: String,
    /// Whether the member is static.
    is_static: bool,
    /// Whether this is a property (field) vs a method/accessor.
    /// TypeScript uses `void 0` for properties and `null` for methods/accessors.
    is_property: bool,
    /// The decorator expression texts (e.g., "Selector()", "Action(AddTodo)").
    decorator_texts: std::vec::Vec<String>,
}

/// Whether the class declares its own constructor with at least one parameter.
///
/// Used by the `@Service` handler to catch the common-but-broken pattern of
/// declaring constructor-based DI on a service: upstream service.ts:278-309
/// surfaces a diagnostic because `@Service` ɵfac is generated with empty
/// deps, so those parameters would silently become `undefined` at runtime.
///
/// Unlike upstream, we don't walk to base classes — that requires cross-file
/// resolution that oxc doesn't perform. Upstream's LOCAL compilation mode
/// also skips that walk, and our single-file transform is closer to LOCAL
/// mode than to a full reflector.
fn class_has_own_constructor_params(class: &oxc_ast::ast::Class<'_>) -> bool {
    use oxc_ast::ast::{ClassElement, MethodDefinitionKind};
    class.body.body.iter().any(|element| {
        if let ClassElement::MethodDefinition(method) = element {
            method.kind == MethodDefinitionKind::Constructor
                && !method.value.params.items.is_empty()
        } else {
            false
        }
    })
}

/// Return the `@Service` decorator on a class iff the `Service` identifier
/// resolves to `@angular/core` via the import map. Returns `None` for a bare
/// `@Service()` from a third-party library.
///
/// `Service` is a common export name in DI containers and web frameworks, so
/// matching by name alone would misclassify unrelated decorators. Mirrors
/// the gate in [`find_angular_decorator`].
fn find_angular_service_decorator<'a>(
    class: &'a oxc_ast::ast::Class<'a>,
    import_map: &ImportMap<'a>,
) -> Option<&'a oxc_ast::ast::Decorator<'a>> {
    for decorator in &class.decorators {
        let Expression::CallExpression(call) = &decorator.expression else { continue };
        let from_angular_core = match &call.callee {
            Expression::Identifier(id) => {
                is_angular_core_export(import_map, id.name.as_str(), "Service")
            }
            // Namespace form `@ns.Service()`: accept when `ns` is a
            // namespace import from `@angular/core`. Without this, AOT
            // would silently skip namespaced services that the JIT path
            // already classifies correctly.
            Expression::StaticMemberExpression(member) => {
                member.property.name.as_str() == "Service"
                    && match &member.object {
                        Expression::Identifier(ns) => {
                            is_angular_core_namespace(import_map, ns.name.as_str())
                        }
                        _ => false,
                    }
            }
            _ => false,
        };
        if from_angular_core {
            return Some(decorator);
        }
    }
    None
}

/// Whether `local_name` resolves to the named `@angular/core` export.
///
/// Matches when the import is from `@angular/core` AND the original exported
/// name equals `exported_name` — so `import { Injectable as Service }` does
/// not pass `is_angular_core_export(.., "Service", "Service")` even though
/// the local binding is `Service`. Bare `import { Service }` (no alias)
/// passes because `imported_name` is `None`, meaning the local name and the
/// exported name agree.
fn is_angular_core_export(
    import_map: &ImportMap<'_>,
    local_name: &str,
    exported_name: &str,
) -> bool {
    let Some(info) = import_map.get(&Ident::from(local_name)) else { return false };
    if info.source_module.as_str() != "@angular/core" {
        return false;
    }
    match &info.imported_name {
        Some(imported) => imported.as_str() == exported_name,
        None => local_name == exported_name,
    }
}

/// Whether `local_name` is a namespace import (`import * as ns from ...`)
/// from `@angular/core`. Used to validate namespace-style decorator calls
/// like `@ns.Service()` — without this check, any third-party namespace
/// `Service` decorator would classify as the Angular v22 decorator.
fn is_angular_core_namespace(import_map: &ImportMap<'_>, local_name: &str) -> bool {
    import_map
        .get(&Ident::from(local_name))
        .map(|info| info.source_module.as_str() == "@angular/core" && !info.is_named_import)
        .unwrap_or(false)
}

/// Return the name of the first non-`Service` `@angular/core` decorator on
/// the class, if any. Used to enforce upstream's collision rule (see
/// `service.ts:101-116`): `@Service` cannot coexist with another Angular
/// decorator on the same class.
fn find_conflicting_angular_decorator<'a>(
    class: &'a oxc_ast::ast::Class<'a>,
    import_map: &ImportMap<'a>,
) -> Option<&'a str> {
    const ANGULAR_DECORATORS: &[&str] =
        &["Component", "Directive", "Pipe", "Injectable", "NgModule"];

    for decorator in &class.decorators {
        let name = match &decorator.expression {
            Expression::CallExpression(call) => match &call.callee {
                Expression::Identifier(id) => id.name.as_str(),
                Expression::StaticMemberExpression(member) => member.property.name.as_str(),
                _ => continue,
            },
            _ => continue,
        };
        if !ANGULAR_DECORATORS.contains(&name) {
            continue;
        }
        // Verify the import resolves to @angular/core for both identifier
        // and namespace callees. Without the namespace lookup, an unrelated
        // `@thirdParty.Component()` on an @Service class would falsely
        // trigger the collision preflight and block valid services.
        let is_angular = if let Expression::CallExpression(call) = &decorator.expression {
            match &call.callee {
                Expression::Identifier(id) => import_map
                    .get(&Ident::from(id.name.as_str()))
                    .map(|info| info.source_module.as_str() == "@angular/core")
                    .unwrap_or(false),
                Expression::StaticMemberExpression(member) => match &member.object {
                    Expression::Identifier(ns) => {
                        is_angular_core_namespace(import_map, ns.name.as_str())
                    }
                    _ => false,
                },
                _ => false,
            }
        } else {
            false
        };
        if is_angular {
            return Some(name);
        }
    }
    None
}

/// Find any Angular decorator on a class and return its kind and the decorator reference.
///
/// For the `Service` identifier specifically, the import map is consulted so a
/// bare `@Service()` from a non-Angular library doesn't shadow a real Angular
/// decorator that follows it on the same class. `Service` is common enough as a
/// library export name (DI containers, web frameworks) that name-only matching
/// would cause the JIT pipeline to either misclassify the class or, on
/// pre-v22 targets, emit a misleading diagnostic and skip the sibling
/// `@Component`/`@Injectable`/etc. Other Angular decorator names are unique
/// enough in practice that the same check isn't applied to them — and doing so
/// would regress namespace-style usage like `@core.Component()` where the
/// identifier isn't directly in the import map.
fn find_angular_decorator<'a>(
    class: &'a oxc_ast::ast::Class<'a>,
    import_map: &ImportMap<'a>,
) -> Option<(AngularDecoratorKind, &'a oxc_ast::ast::Decorator<'a>)> {
    for decorator in &class.decorators {
        if let Expression::CallExpression(call) = &decorator.expression {
            let name = match &call.callee {
                Expression::Identifier(id) => Some(id.name.as_str()),
                Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
                _ => None,
            };
            let kind = match name {
                Some("Component") => Some(AngularDecoratorKind::Component),
                Some("Directive") => Some(AngularDecoratorKind::Directive),
                Some("Pipe") => Some(AngularDecoratorKind::Pipe),
                Some("Injectable") => Some(AngularDecoratorKind::Injectable),
                Some("Service") => Some(AngularDecoratorKind::Service),
                Some("NgModule") => Some(AngularDecoratorKind::NgModule),
                _ => None,
            };

            if matches!(kind, Some(AngularDecoratorKind::Service)) {
                let from_angular_core = match &call.callee {
                    Expression::Identifier(id) => {
                        is_angular_core_export(import_map, id.name.as_str(), "Service")
                    }
                    // Namespace form `@ns.Service()`: verify `ns` is a
                    // namespace import from `@angular/core`. Without this,
                    // any `@third.Service()` from a third-party namespace
                    // import would classify as the v22 decorator.
                    Expression::StaticMemberExpression(member) => match &member.object {
                        Expression::Identifier(ns) => {
                            is_angular_core_namespace(import_map, ns.name.as_str())
                        }
                        _ => false,
                    },
                    _ => false,
                };
                if !from_angular_core {
                    continue;
                }
            }

            if let Some(k) = kind {
                return Some((k, decorator));
            }
        }
    }
    None
}

/// Extract constructor parameter info for JIT ctorParameters generation.
fn extract_jit_ctor_params(
    source: &str,
    class: &oxc_ast::ast::Class<'_>,
) -> std::vec::Vec<JitCtorParam> {
    use oxc_ast::ast::{ClassElement, MethodDefinitionKind};

    let constructor = class.body.body.iter().find_map(|element| {
        if let ClassElement::MethodDefinition(method) = element {
            if method.kind == MethodDefinitionKind::Constructor {
                return Some(method);
            }
        }
        None
    });

    let Some(ctor) = constructor else {
        return std::vec::Vec::new();
    };

    let mut params = std::vec::Vec::new();
    for param in &ctor.value.params.items {
        // Extract type name from type annotation (directly on FormalParameter)
        let type_name = param
            .type_annotation
            .as_ref()
            .and_then(|ann| extract_type_name_from_annotation(&ann.type_annotation));

        // Extract Angular decorators
        let mut decorators = std::vec::Vec::new();
        for decorator in &param.decorators {
            if let Expression::CallExpression(call) = &decorator.expression {
                let dec_name = match &call.callee {
                    Expression::Identifier(id) => Some(id.name.to_string()),
                    _ => None,
                };
                if let Some(name) = dec_name {
                    match name.as_str() {
                        "Inject" | "Optional" | "SkipSelf" | "Self" | "Host" | "Attribute" => {
                            let args = if call.arguments.is_empty() {
                                None
                            } else {
                                // Extract args from source
                                let args_start = call.arguments.first().unwrap().span().start;
                                let args_end = call.arguments.last().unwrap().span().end;
                                Some(source[args_start as usize..args_end as usize].to_string())
                            };
                            decorators.push(JitParamDecorator { name, args });
                        }
                        _ => {}
                    }
                }
            } else if let Expression::Identifier(id) = &decorator.expression {
                let name = id.name.to_string();
                match name.as_str() {
                    "Optional" | "SkipSelf" | "Self" | "Host" => {
                        decorators.push(JitParamDecorator { name, args: None });
                    }
                    _ => {}
                }
            }
        }

        params.push(JitCtorParam { type_name, decorators });
    }

    params
}

/// Angular field decorators that go into `static propDecorators`.
/// Matches Angular's official `FIELD_DECORATORS` constant from `@angular/compiler-cli`.
const ANGULAR_FIELD_DECORATORS: &[&str] = &[
    "Input",
    "Output",
    "HostBinding",
    "HostListener",
    "ViewChild",
    "ViewChildren",
    "ContentChild",
    "ContentChildren",
];

/// All Angular decorator names from `@angular/core`.
/// Any decorator with one of these names is treated as Angular and excluded from
/// non-Angular `__decorate()` lowering. Angular identifies decorators by import source;
/// we use names since they're unique to `@angular/core`.
const ANGULAR_DECORATOR_NAMES: &[&str] = &[
    // Field decorators (→ propDecorators)
    "Input",
    "Output",
    "HostBinding",
    "HostListener",
    "ViewChild",
    "ViewChildren",
    "ContentChild",
    "ContentChildren",
    // Parameter decorators (→ ctorParameters)
    "Inject",
    "Optional",
    "Self",
    "SkipSelf",
    "Host",
    "Attribute",
    // Class decorators (→ class __decorate)
    "Component",
    "Directive",
    "Pipe",
    "Injectable",
    "Service",
    "NgModule",
];

/// Extract all member decorators for JIT transformation in a single pass.
///
/// Returns two collections:
/// - Angular field decorators → emitted as `static propDecorators = { ... }`
/// - Non-Angular decorators → emitted as `__decorate([...], target, "name", desc)` calls
fn extract_all_jit_member_decorators(
    source: &str,
    class: &oxc_ast::ast::Class<'_>,
) -> (std::vec::Vec<JitMemberDecorator>, std::vec::Vec<JitNonAngularMemberDecorator>) {
    use oxc_ast::ast::{ClassElement, MethodDefinitionKind, PropertyKey};

    let mut angular_members: std::vec::Vec<JitMemberDecorator> = std::vec::Vec::new();
    let mut non_angular_members: std::vec::Vec<JitNonAngularMemberDecorator> = std::vec::Vec::new();

    for element in &class.body.body {
        let (member_name, is_static, is_property, decorators, initializer) = match element {
            ClassElement::PropertyDefinition(prop) => {
                let name = match &prop.key {
                    PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                    PropertyKey::StringLiteral(s) => s.value.to_string(),
                    _ => continue,
                };
                (name, prop.r#static, true, &prop.decorators, prop.value.as_ref())
            }
            ClassElement::MethodDefinition(method) => {
                if method.kind == MethodDefinitionKind::Constructor {
                    continue;
                }
                let name = match &method.key {
                    PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                    PropertyKey::StringLiteral(s) => s.value.to_string(),
                    _ => continue,
                };
                (name, method.r#static, false, &method.decorators, None)
            }
            ClassElement::AccessorProperty(accessor) => {
                let name = match &accessor.key {
                    PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                    PropertyKey::StringLiteral(s) => s.value.to_string(),
                    _ => continue,
                };
                (name, accessor.r#static, false, &accessor.decorators, None)
            }
            _ => continue,
        };

        let mut angular_decs: std::vec::Vec<JitParamDecorator> = std::vec::Vec::new();
        let mut non_angular_texts: std::vec::Vec<String> = std::vec::Vec::new();
        let mut explicit_field_decorators: rustc_hash::FxHashSet<String> =
            rustc_hash::FxHashSet::default();

        for decorator in decorators {
            let (dec_name, call_args) = match &decorator.expression {
                Expression::CallExpression(call) => {
                    let name = match &call.callee {
                        Expression::Identifier(id) => id.name.to_string(),
                        Expression::StaticMemberExpression(m) => m.property.name.to_string(),
                        _ => continue,
                    };
                    let args = if call.arguments.is_empty() {
                        None
                    } else {
                        let start = call.arguments.first().unwrap().span().start;
                        let end = call.arguments.last().unwrap().span().end;
                        Some(source[start as usize..end as usize].to_string())
                    };
                    (name, args)
                }
                Expression::Identifier(id) => (id.name.to_string(), None),
                _ => continue,
            };

            if ANGULAR_FIELD_DECORATORS.contains(&dec_name.as_str()) {
                // Angular field decorator → goes into propDecorators
                explicit_field_decorators.insert(dec_name.clone());
                angular_decs.push(JitParamDecorator { name: dec_name, args: call_args });
            } else if !ANGULAR_DECORATOR_NAMES.contains(&dec_name.as_str()) {
                // Non-Angular decorator → goes into __decorate() call
                let expr_start = decorator.expression.span().start;
                let expr_end = decorator.expression.span().end;
                non_angular_texts.push(source[expr_start as usize..expr_end as usize].to_string());
            }
            // Angular non-field decorators (e.g. @Inject on a member) are silently dropped
            // since they have no meaningful effect on members.
        }

        // Signal initializer-API lowering: synthesize @Input / @Output / @ViewChild / etc.
        // decorators from `input()`, `output()`, `model()`, `viewChild*()`, `contentChild*()`
        // initializers so the runtime JIT facade can discover them via `propDecorators`.
        // Mirrors `packages/compiler-cli/src/ngtsc/transform/jit/src/initializer_api_transforms/`.
        // When an explicit matching decorator already exists, the explicit one wins.
        if let Some(init) = initializer {
            let synthesized = synthesize_signal_api_decorators(
                source,
                init,
                &member_name,
                &explicit_field_decorators,
            );
            angular_decs.extend(synthesized);
        }

        if !angular_decs.is_empty() {
            angular_members.push(JitMemberDecorator {
                member_name: member_name.clone(),
                decorators: angular_decs,
            });
        }

        if !non_angular_texts.is_empty() {
            non_angular_members.push(JitNonAngularMemberDecorator {
                member_name,
                is_static,
                is_property,
                decorator_texts: non_angular_texts,
            });
        }
    }

    (angular_members, non_angular_members)
}

/// Strip `as`/`satisfies`/parenthesized wrappers around an initializer expression,
/// mirroring ngc's `tryParseInitializerApi` (e.g. `x = input(0) as any`, `x = (input(0))`).
fn unwrap_jit_initializer<'a, 'b>(expr: &'b Expression<'a>) -> &'b Expression<'a> {
    match expr {
        Expression::TSAsExpression(e) => unwrap_jit_initializer(&e.expression),
        Expression::TSSatisfiesExpression(e) => unwrap_jit_initializer(&e.expression),
        Expression::ParenthesizedExpression(e) => unwrap_jit_initializer(&e.expression),
        _ => expr,
    }
}

/// Recognized initializer API kinds. Used to dispatch synthesis.
#[derive(Debug, Clone, Copy)]
enum InitializerApiKind {
    Input,
    InputRequired,
    Output,
    OutputFromObservable,
    Model,
    ModelRequired,
    ViewChild,
    ViewChildRequired,
    ViewChildren,
    ContentChild,
    ContentChildRequired,
    ContentChildren,
}

/// Identify which initializer API a call expression represents.
///
/// Handles three call shapes:
/// - bare identifier: `input(...)`, `output(...)`
/// - `.required` member: `input.required(...)`, `model.required(...)`
/// - namespaced: `core.input(...)`, `core.viewChild.required(...)`
fn classify_initializer_api(callee: &Expression<'_>) -> Option<InitializerApiKind> {
    fn match_name(name: &str) -> Option<InitializerApiKind> {
        match name {
            "input" => Some(InitializerApiKind::Input),
            "output" => Some(InitializerApiKind::Output),
            "outputFromObservable" => Some(InitializerApiKind::OutputFromObservable),
            "model" => Some(InitializerApiKind::Model),
            "viewChild" => Some(InitializerApiKind::ViewChild),
            "viewChildren" => Some(InitializerApiKind::ViewChildren),
            "contentChild" => Some(InitializerApiKind::ContentChild),
            "contentChildren" => Some(InitializerApiKind::ContentChildren),
            _ => None,
        }
    }
    fn required_variant(base: InitializerApiKind) -> Option<InitializerApiKind> {
        match base {
            InitializerApiKind::Input => Some(InitializerApiKind::InputRequired),
            InitializerApiKind::Model => Some(InitializerApiKind::ModelRequired),
            InitializerApiKind::ViewChild => Some(InitializerApiKind::ViewChildRequired),
            InitializerApiKind::ContentChild => Some(InitializerApiKind::ContentChildRequired),
            _ => None,
        }
    }

    match callee {
        Expression::Identifier(id) => match_name(id.name.as_str()),
        Expression::StaticMemberExpression(member) => {
            // `<base>.required` — find the underlying base API and promote it.
            if member.property.name == "required" {
                let base = match &member.object {
                    Expression::Identifier(id) => match_name(id.name.as_str())?,
                    Expression::StaticMemberExpression(inner) => {
                        // Namespaced: `core.input.required`
                        match_name(inner.property.name.as_str())?
                    }
                    _ => return None,
                };
                required_variant(base)
            } else {
                // Namespaced: `core.input(...)`. The outer property *is* the function name.
                match &member.object {
                    Expression::Identifier(_) => match_name(member.property.name.as_str()),
                    _ => None,
                }
            }
        }
        _ => None,
    }
}

/// Namespace alias under which `@angular/core` is imported when JIT synthesis needs
/// to reference decorator names that the user didn't import (e.g. `Input` for a
/// `@Component` that uses `input()` but didn't import the capital-letter decorator).
///
/// Matches ngc's `createSyntheticAngularCoreDecoratorAccess`, which emits
/// `i0.Input` / `i0.Output` / etc. backed by `import * as i0 from "@angular/core"`.
/// Without this prefixing, the synthesized `static propDecorators` would reference
/// an undefined identifier and throw `ReferenceError` at module-evaluation time.
pub(crate) const JIT_ANGULAR_CORE_NS: &str = "i0";

/// Inspect a property initializer; if it matches a recognized signal initializer API,
/// return the synthesized `propDecorators` entries that JIT runtime needs.
///
/// `existing` contains the names of explicit field decorators already present on the
/// property (e.g. `Input`, `Output`); we skip synthesis when the user-authored decorator
/// already covers the binding (matches upstream behavior — explicit decorator wins).
///
/// Synthesized decorator names are namespace-prefixed (e.g. `i0.Input`) — see
/// [`JIT_ANGULAR_CORE_NS`]. The caller is responsible for emitting the matching
/// `import * as i0 from "@angular/core"` when any synthesis occurred.
fn synthesize_signal_api_decorators(
    source: &str,
    initializer: &Expression<'_>,
    field_name: &str,
    existing: &rustc_hash::FxHashSet<String>,
) -> std::vec::Vec<JitParamDecorator> {
    let unwrapped = unwrap_jit_initializer(initializer);
    let Expression::CallExpression(call) = unwrapped else { return std::vec::Vec::new() };
    let Some(kind) = classify_initializer_api(&call.callee) else { return std::vec::Vec::new() };

    match kind {
        InitializerApiKind::Input | InitializerApiKind::InputRequired => {
            if existing.contains("Input") {
                return std::vec::Vec::new();
            }
            let required = matches!(kind, InitializerApiKind::InputRequired);
            // input(initial, options?) → options is args[1]; input.required(options?) → args[0].
            let options_index = if required { 0 } else { 1 };
            let alias = extract_string_option(call, options_index, "alias")
                .unwrap_or_else(|| field_name.to_string());
            let args = format!(
                "{{ isSignal: true, alias: \"{alias}\", required: {required}, transform: undefined }}",
                alias = escape_js_string(&alias),
                required = required,
            );
            std::vec::Vec::from([JitParamDecorator {
                name: format!("{JIT_ANGULAR_CORE_NS}.Input"),
                args: Some(args),
            }])
        }
        InitializerApiKind::Output | InitializerApiKind::OutputFromObservable => {
            if existing.contains("Output") {
                return std::vec::Vec::new();
            }
            // output(options?) → args[0]; outputFromObservable(source, options?) → args[1].
            let options_index =
                if matches!(kind, InitializerApiKind::OutputFromObservable) { 1 } else { 0 };
            let alias = extract_string_option(call, options_index, "alias")
                .unwrap_or_else(|| field_name.to_string());
            let args = format!("\"{}\"", escape_js_string(&alias));
            std::vec::Vec::from([JitParamDecorator {
                name: format!("{JIT_ANGULAR_CORE_NS}.Output"),
                args: Some(args),
            }])
        }
        InitializerApiKind::Model | InitializerApiKind::ModelRequired => {
            // model() is two bindings — block if either @Input or @Output is already present.
            if existing.contains("Input") || existing.contains("Output") {
                return std::vec::Vec::new();
            }
            let required = matches!(kind, InitializerApiKind::ModelRequired);
            // Same arg layout as input.
            let options_index = if required { 0 } else { 1 };
            let alias = extract_string_option(call, options_index, "alias")
                .unwrap_or_else(|| field_name.to_string());
            let input_args = format!(
                "{{ isSignal: true, alias: \"{alias}\", required: {required}, transform: undefined }}",
                alias = escape_js_string(&alias),
                required = required,
            );
            let output_args = format!("\"{}Change\"", escape_js_string(&alias));
            std::vec::Vec::from([
                JitParamDecorator {
                    name: format!("{JIT_ANGULAR_CORE_NS}.Input"),
                    args: Some(input_args),
                },
                JitParamDecorator {
                    name: format!("{JIT_ANGULAR_CORE_NS}.Output"),
                    args: Some(output_args),
                },
            ])
        }
        InitializerApiKind::ViewChild
        | InitializerApiKind::ViewChildRequired
        | InitializerApiKind::ViewChildren
        | InitializerApiKind::ContentChild
        | InitializerApiKind::ContentChildRequired
        | InitializerApiKind::ContentChildren => {
            // Any existing query decorator blocks all query synthesis on this field.
            const QUERY_DECS: &[&str] =
                &["ViewChild", "ViewChildren", "ContentChild", "ContentChildren"];
            if QUERY_DECS.iter().any(|d| existing.contains(*d)) {
                return std::vec::Vec::new();
            }
            let decorator_name = match kind {
                InitializerApiKind::ViewChild | InitializerApiKind::ViewChildRequired => {
                    "ViewChild"
                }
                InitializerApiKind::ViewChildren => "ViewChildren",
                InitializerApiKind::ContentChild | InitializerApiKind::ContentChildRequired => {
                    "ContentChild"
                }
                InitializerApiKind::ContentChildren => "ContentChildren",
                _ => unreachable!(),
            };
            // Mirror ngc query lowering: positional args carry over; isSignal is folded into
            // the options object (spreading the existing options if present).
            let Some(locator_arg) = call.arguments.first() else { return std::vec::Vec::new() };
            let locator_text = source
                [locator_arg.span().start as usize..locator_arg.span().end as usize]
                .to_string();
            let options_text = if let Some(opts_arg) = call.arguments.get(1) {
                let opts_src =
                    &source[opts_arg.span().start as usize..opts_arg.span().end as usize];
                format!("{{ ...{}, isSignal: true }}", opts_src)
            } else {
                "{ isSignal: true }".to_string()
            };
            let args = format!("{locator_text}, {options_text}");
            std::vec::Vec::from([JitParamDecorator {
                name: format!("{JIT_ANGULAR_CORE_NS}.{decorator_name}"),
                args: Some(args),
            }])
        }
    }
}

/// Returns `true` when any field of any JIT class has a synthesized decorator
/// (signal API lowering) that references the `@angular/core` namespace. Used to
/// gate the emission of `import * as i0 from "@angular/core"`.
fn jit_classes_need_angular_core_namespace(jit_classes: &[JitClassInfo]) -> bool {
    let prefix = format!("{JIT_ANGULAR_CORE_NS}.");
    jit_classes.iter().any(|info| {
        info.member_decorators
            .iter()
            .any(|m| m.decorators.iter().any(|d| d.name.starts_with(&prefix)))
    })
}

/// Pull a string option (e.g. `alias`) from the object literal at `args[options_index]`.
/// Returns `None` if the argument isn't an object literal or the option is missing/non-string.
fn extract_string_option(
    call: &oxc_ast::ast::CallExpression<'_>,
    options_index: usize,
    option_name: &str,
) -> Option<String> {
    let arg = call.arguments.get(options_index)?;
    let Argument::ObjectExpression(obj) = arg else { return None };
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(prop) = prop else { continue };
        let key_matches = match &prop.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str() == option_name,
            PropertyKey::StringLiteral(s) => s.value.as_str() == option_name,
            _ => false,
        };
        if !key_matches {
            continue;
        }
        if let Expression::StringLiteral(s) = &prop.value {
            return Some(s.value.to_string());
        }
    }
    None
}

/// Minimal JS-string escape for values embedded in synthesized propDecorator text.
/// Property names are TS identifiers or string literals, so only `\` and `"` matter.
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the propDecorators static property text for JIT member decorator metadata.
fn build_prop_decorators_text(members: &[JitMemberDecorator]) -> Option<String> {
    if members.is_empty() {
        return None;
    }

    let mut entries: std::vec::Vec<String> = std::vec::Vec::new();
    for member in members {
        let dec_strs: std::vec::Vec<String> = member
            .decorators
            .iter()
            .map(|d| {
                if let Some(ref args) = d.args {
                    format!("{{ type: {}, args: [{}] }}", d.name, args)
                } else {
                    format!("{{ type: {} }}", d.name)
                }
            })
            .collect();
        entries.push(format!("    {}: [{}]", member.member_name, dec_strs.join(", ")));
    }

    Some(format!("static propDecorators = {{\n{}\n}}", entries.join(",\n")))
}

/// Extract a type name from a TypeScript type annotation for JIT ctorParameters.
fn extract_type_name_from_annotation(type_annotation: &oxc_ast::ast::TSType<'_>) -> Option<String> {
    match type_annotation {
        oxc_ast::ast::TSType::TSTypeReference(type_ref) => {
            // Simple type reference: `SomeClass`
            match &type_ref.type_name {
                oxc_ast::ast::TSTypeName::IdentifierReference(id) => Some(id.name.to_string()),
                oxc_ast::ast::TSTypeName::QualifiedName(qn) => {
                    // Qualified name: `ns.SomeClass`
                    Some(format!("{}.{}", extract_ts_type_name_left(&qn.left), qn.right.name))
                }
                _ => None,
            }
        }
        oxc_ast::ast::TSType::TSUnionType(union) => {
            // Match Angular's typeReferenceToExpression behavior:
            // filter out only `null` literal types, and if exactly one type remains,
            // resolve that type. Otherwise, return None (unresolvable).
            // See: angular/packages/compiler-cli/src/ngtsc/transform/jit/src/downlevel_decorators_transform.ts
            let non_null: std::vec::Vec<_> = union
                .types
                .iter()
                .filter(|t| !matches!(t, oxc_ast::ast::TSType::TSNullKeyword(_)))
                .collect();
            if non_null.len() == 1 { extract_type_name_from_annotation(non_null[0]) } else { None }
        }
        _ => None,
    }
}

/// Helper to extract the string from a TSTypeName (left side of qualified name).
fn extract_ts_type_name_left(name: &oxc_ast::ast::TSTypeName<'_>) -> String {
    match name {
        oxc_ast::ast::TSTypeName::IdentifierReference(id) => id.name.to_string(),
        oxc_ast::ast::TSTypeName::QualifiedName(qn) => {
            format!("{}.{}", extract_ts_type_name_left(&qn.left), qn.right.name)
        }
        _ => String::new(),
    }
}

/// Build the ctorParameters static property text.
fn build_ctor_parameters_text(params: &[JitCtorParam]) -> Option<String> {
    if params.is_empty() {
        return None;
    }

    let mut entries = std::vec::Vec::new();
    for param in params {
        let mut parts = std::vec::Vec::new();

        // type
        if let Some(ref type_name) = param.type_name {
            parts.push(format!("type: {}", type_name));
        } else {
            parts.push("type: undefined".to_string());
        }

        // decorators
        if !param.decorators.is_empty() {
            let dec_strs: std::vec::Vec<String> = param
                .decorators
                .iter()
                .map(|d| {
                    if let Some(ref args) = d.args {
                        format!("{{ type: {}, args: [{}] }}", d.name, args)
                    } else {
                        format!("{{ type: {} }}", d.name)
                    }
                })
                .collect();
            parts.push(format!("decorators: [{}]", dec_strs.join(", ")));
        }

        entries.push(format!("{{ {} }}", parts.join(", ")));
    }

    Some(format!("static ctorParameters = () => [\n    {}\n]", entries.join(",\n    ")))
}

/// Build the modified decorator expression text for JIT __decorate call.
///
/// For @Component decorators, replaces:
/// - `templateUrl: './path'` → `template: __NG_CLI_RESOURCE__N`
/// - `styleUrl: './path'` → `styles: [__NG_CLI_RESOURCE__N]`
/// - `styleUrls: ['./a', './b']` → `styles: [__NG_CLI_RESOURCE__N, __NG_CLI_RESOURCE__M]`
///
/// `consts` and `allocator` enable folding the same identifier and template-
/// literal interpolation shapes as the AOT metadata extraction path (e.g.
/// `` templateUrl: `${DIR}/x.html` `` with a same-file `const DIR = '...'`).
fn build_jit_decorator_text<'a>(
    allocator: &'a Allocator,
    source: &str,
    decorator: &oxc_ast::ast::Decorator<'a>,
    decorator_kind: AngularDecoratorKind,
    resource_counter: &mut u32,
    resource_imports: &mut std::vec::Vec<(String, String)>, // (import_name, specifier)
    consts: &crate::directive::StringConsts<'a>,
) -> String {
    let expr_start = decorator.expression.span().start as usize;
    let expr_end = decorator.expression.span().end as usize;
    let expr_text = &source[expr_start..expr_end];

    // For non-Component decorators, just return the expression text as-is
    if !matches!(decorator_kind, AngularDecoratorKind::Component) {
        return expr_text.to_string();
    }

    // For Component decorators, check for resource properties to replace
    let Expression::CallExpression(call) = &decorator.expression else {
        return expr_text.to_string();
    };

    let Some(config_arg) = call.arguments.first() else {
        return expr_text.to_string();
    };

    let Argument::ObjectExpression(config_obj) = config_arg else {
        return expr_text.to_string();
    };

    // Collect edits within the expression text
    let mut edits: std::vec::Vec<(usize, usize, String)> = std::vec::Vec::new();

    for prop in &config_obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            let key_name = match &prop.key {
                PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                PropertyKey::StringLiteral(s) => Some(s.value.as_str()),
                _ => None,
            };

            match key_name {
                Some("templateUrl") => {
                    // Resolve the URL through the same const/template-literal
                    // folder used by AOT extraction so JIT and AOT accept the
                    // same metadata shapes.
                    if let Some(url) =
                        crate::directive::extract_string_value(allocator, &prop.value, consts)
                    {
                        let import_name = format!("__NG_CLI_RESOURCE__{}", *resource_counter);
                        let specifier = format!("angular:jit:template:file;{}", url.as_str());
                        resource_imports.push((import_name.clone(), specifier));

                        // Replace the entire property: `templateUrl: './app.html'` → `template: __NG_CLI_RESOURCE__0`
                        let prop_start = prop.span.start as usize - expr_start;
                        let prop_end = prop.span.end as usize - expr_start;
                        edits.push((prop_start, prop_end, format!("template: {}", import_name)));

                        *resource_counter += 1;
                    }
                }
                Some("styleUrl") => {
                    // Single style URL
                    if let Some(url) =
                        crate::directive::extract_string_value(allocator, &prop.value, consts)
                    {
                        let import_name = format!("__NG_CLI_RESOURCE__{}", *resource_counter);
                        let specifier = format!("angular:jit:style:file;{}", url.as_str());
                        resource_imports.push((import_name.clone(), specifier));

                        let prop_start = prop.span.start as usize - expr_start;
                        let prop_end = prop.span.end as usize - expr_start;
                        edits.push((prop_start, prop_end, format!("styles: [{}]", import_name)));

                        *resource_counter += 1;
                    }
                }
                Some("styleUrls") => {
                    // Array of style URLs — fold each element through the same
                    // resolver so `${DIR}/a.css`-style entries are accepted.
                    if let Expression::ArrayExpression(arr) = &prop.value {
                        let mut style_refs = std::vec::Vec::new();
                        for elem in &arr.elements {
                            // ArrayExpressionElement variants that can hold a
                            // value expression all carry one — funnel them
                            // through `as_expression()` and let the resolver
                            // decide which shapes fold.
                            let Some(elem_expr) = elem.as_expression() else { continue };
                            if let Some(url) =
                                crate::directive::extract_string_value(allocator, elem_expr, consts)
                            {
                                let import_name =
                                    format!("__NG_CLI_RESOURCE__{}", *resource_counter);
                                let specifier = format!("angular:jit:style:file;{}", url.as_str());
                                resource_imports.push((import_name.clone(), specifier));
                                style_refs.push(import_name);
                                *resource_counter += 1;
                            }
                        }

                        let prop_start = prop.span.start as usize - expr_start;
                        let prop_end = prop.span.end as usize - expr_start;
                        edits.push((
                            prop_start,
                            prop_end,
                            format!("styles: [{}]", style_refs.join(", ")),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    if edits.is_empty() {
        return expr_text.to_string();
    }

    // Apply edits in reverse order to preserve positions
    let mut result = expr_text.to_string();
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    for (start, end, replacement) in edits {
        result.replace_range(start..end, &replacement);
    }

    result
}

/// Collect names of constructor parameters that have modifiers (parameter properties).
/// These will generate field declarations in oxc 0.129.0 which we need to remove.
fn collect_parameter_property_names(
    program: &oxc_ast::ast::Program,
) -> rustc_hash::FxHashSet<String> {
    use oxc_ast::ast::{ClassElement, Statement};
    use rustc_hash::FxHashSet;

    let mut names = FxHashSet::default();

    fn visit_class(class: &oxc_ast::ast::Class, names: &mut FxHashSet<String>) {
        for element in &class.body.body {
            if let ClassElement::MethodDefinition(method) = element {
                if method.kind.is_constructor() {
                    for param in &method.value.params.items {
                        if param.accessibility.is_some() || param.readonly || param.r#override {
                            if let Some(ident) = param.pattern.get_identifier_name() {
                                names.insert(ident.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    for stmt in &program.body {
        match stmt {
            Statement::ClassDeclaration(class_decl) => {
                visit_class(class_decl, &mut names);
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(oxc_ast::ast::Declaration::ClassDeclaration(class_decl)) =
                    &export.declaration
                {
                    visit_class(class_decl, &mut names);
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class_decl) =
                    &export.declaration
                {
                    visit_class(class_decl, &mut names);
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                // Handle: let AppComponent = class AppComponent { ... }
                for declarator in &var_decl.declarations {
                    if let Some(oxc_ast::ast::Expression::ClassExpression(class_expr)) =
                        &declarator.init
                    {
                        visit_class(class_expr, &mut names);
                    }
                }
            }
            _ => {}
        }
    }

    names
}

/// Remove field declarations that were generated from parameter properties.
/// oxc 0.129.0 generates field declarations for parameter properties, but we don't want them in JIT mode.
fn remove_parameter_property_fields(
    program: &mut oxc_ast::ast::Program,
    param_names: &rustc_hash::FxHashSet<String>,
) {
    use oxc_ast::ast::{ClassElement, Statement};

    fn visit_class(class: &mut oxc_ast::ast::Class, param_names: &rustc_hash::FxHashSet<String>) {
        class.body.body.retain(|element| {
            if let ClassElement::PropertyDefinition(prop) = element {
                // Remove field if it:
                // 1. Has no initializer
                // 2. Has no decorators
                // 3. Matches a parameter property name
                if prop.value.is_none() && prop.decorators.is_empty() {
                    if let oxc_ast::ast::PropertyKey::StaticIdentifier(ident) = &prop.key {
                        if param_names.contains(ident.name.as_str()) {
                            return false; // Remove this field
                        }
                    }
                }
            }
            true // Keep this element
        });
    }

    for stmt in &mut program.body {
        match stmt {
            Statement::ClassDeclaration(class_decl) => {
                visit_class(class_decl, param_names);
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(oxc_ast::ast::Declaration::ClassDeclaration(class_decl)) =
                    &mut export.declaration
                {
                    visit_class(class_decl, param_names);
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class_decl) =
                    &mut export.declaration
                {
                    visit_class(class_decl, param_names);
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                // Handle: let AppComponent = class AppComponent { ... }
                for declarator in &mut var_decl.declarations {
                    if let Some(oxc_ast::ast::Expression::ClassExpression(class_expr)) =
                        &mut declarator.init
                    {
                        visit_class(class_expr, param_names);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Transform an Angular TypeScript file in JIT (Just-In-Time) compilation mode.
///
/// Strip TypeScript syntax from JIT output using oxc_transformer.
///
/// This runs as a post-pass after JIT text-edits, converting TypeScript → JavaScript.
/// It handles abstract members, type annotations, parameter properties, etc.
fn strip_typescript(allocator: &Allocator, path: &str, code: &str) -> String {
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let parser_ret = Parser::new(allocator, code, source_type).parse();
    if parser_ret.panicked {
        return code.to_string();
    }

    let mut program = parser_ret.program;

    // Collect parameter property names before oxc transforms them.
    // In oxc 0.129.0, parameter properties generate field declarations which we need to remove.
    let param_property_names = collect_parameter_property_names(&program);

    let semantic_ret =
        oxc_semantic::SemanticBuilder::new().with_excess_capacity(2.0).build(&program);

    let ts_options =
        oxc_transformer::TypeScriptOptions { only_remove_type_imports: true, ..Default::default() };

    let transform_options =
        oxc_transformer::TransformOptions { typescript: ts_options, ..Default::default() };

    let transformer =
        oxc_transformer::Transformer::new(allocator, Path::new(path), &transform_options);
    transformer.build_with_scoping(semantic_ret.semantic.into_scoping(), &mut program);

    // Remove field declarations that were generated from parameter properties.
    // oxc 0.129.0 adds field declarations for parameter properties, but we don't want them in JIT mode.
    remove_parameter_property_fields(&mut program, &param_property_names);

    let codegen_ret = oxc_codegen::Codegen::new().with_source_text(code).build(&program);

    codegen_ret.code
}

/// JIT mode produces output compatible with Angular's JIT runtime compiler:
/// - Decorators are downleveled using `__decorate` from tslib
/// - `templateUrl` is replaced with `angular:jit:template:file;` imports
/// - `styleUrl`/`styleUrls` are replaced with `angular:jit:style:file;` imports
/// - Constructor parameters are emitted as `ctorParameters` static property
/// - Templates are NOT compiled (the runtime JIT compiler handles that)
fn transform_angular_file_jit(
    allocator: &Allocator,
    path: &str,
    source: &str,
    options: &TransformOptions,
) -> TransformResult {
    let mut result = TransformResult::new();

    // 1. Parse the TypeScript file
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let parser_ret = Parser::new(allocator, source, source_type).parse();

    if !parser_ret.errors.is_empty() {
        for error in parser_ret.errors {
            result.diagnostics.push(OxcDiagnostic::error(error.to_string()));
        }
    }

    // 2. Import elision is DISABLED in JIT mode.
    // JIT mode needs all imports preserved because constructor parameter types
    // are referenced at runtime in ctorParameters. Angular's TS JIT transform
    // patches TypeScript's import elision for the same reason.

    // Collect file-scope string consts so the JIT decorator rewriter can fold
    // identifier and template-literal references in `templateUrl` / `styleUrl`
    // / `styleUrls`, matching the AOT metadata extraction path.
    let string_consts = collect_string_consts(allocator, &parser_ret.program);

    // Build an import map so `find_angular_decorator` can verify that a bare
    // `@Service()` is actually Angular's, not a same-named decorator from
    // another library.
    let import_map =
        build_import_map(allocator, &parser_ret.program.body, options.resolved_imports.as_ref());

    // 3. Walk AST to find Angular-decorated classes
    let mut jit_classes: std::vec::Vec<JitClassInfo> = std::vec::Vec::new();
    let mut resource_counter: u32 = 0;
    let mut resource_imports: std::vec::Vec<(String, String)> = std::vec::Vec::new();

    for stmt in &parser_ret.program.body {
        let (class, stmt_start, is_exported, is_default_export) = match stmt {
            Statement::ClassDeclaration(class) => {
                (Some(class.as_ref()), class.span.start, false, false)
            }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    (Some(class.as_ref()), export.span.start, false, true)
                }
                _ => (None, 0, false, false),
            },
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => {
                    (Some(class.as_ref()), export.span.start, true, false)
                }
                _ => (None, 0, false, false),
            },
            _ => (None, 0, false, false),
        };

        let Some(class) = class else { continue };
        let Some(class_name) = class.id.as_ref().map(|id| id.name.to_string()) else {
            continue;
        };

        let Some((decorator_kind, angular_decorator)) = find_angular_decorator(class, &import_map)
        else {
            continue;
        };

        // Version gating: the `@Service` decorator requires Angular v22+, where the
        // runtime JIT facade gained `compileService`/`ɵɵdefineService`. When targeting an
        // older version, surface the gap and leave the decorator unchanged (pass-through)
        // so the JIT downlevel pipeline doesn't emit metadata the runtime can't consume.
        // When the version is unknown, assume support (matches the `map_or(true, …)` pattern).
        if matches!(decorator_kind, AngularDecoratorKind::Service)
            && !options.angular_version.map_or(true, |v| v.supports_service_decorator())
        {
            result.diagnostics.push(OxcDiagnostic::error(format!(
                "The @Service decorator on '{}' requires Angular v22 or later.",
                class_name
            )));
            continue;
        }

        // Collect ALL class-level decorator spans and texts (in source order)
        let mut all_class_decorator_spans: std::vec::Vec<Span> = std::vec::Vec::new();
        let mut all_class_decorator_texts: std::vec::Vec<String> = std::vec::Vec::new();

        for dec in &class.decorators {
            all_class_decorator_spans.push(dec.span);

            // Check if this is the Angular decorator that needs special text transformation
            if dec.span == angular_decorator.span {
                let text = build_jit_decorator_text(
                    allocator,
                    source,
                    dec,
                    decorator_kind,
                    &mut resource_counter,
                    &mut resource_imports,
                    &string_consts,
                );
                all_class_decorator_texts.push(text);
            } else {
                // Non-Angular decorator: extract expression text from source (without @)
                let expr_start = dec.expression.span().start;
                let expr_end = dec.expression.span().end;
                all_class_decorator_texts
                    .push(source[expr_start as usize..expr_end as usize].to_string());
            }
        }

        // Extract constructor parameters for ctorParameters
        let ctor_params = extract_jit_ctor_params(source, class);

        // Extract Angular and non-Angular member decorators
        let (member_decorators, non_angular_member_decorators) =
            extract_all_jit_member_decorators(source, class);

        jit_classes.push(JitClassInfo {
            class_name,
            all_class_decorator_spans,
            stmt_start,
            class_start: class.span.start,
            class_body_end: class.body.span.end,
            is_exported,
            is_default_export,
            is_abstract: class.r#abstract,
            ctor_params,
            member_decorators,
            all_class_decorator_texts,
            non_angular_member_decorators,
        });

        result.component_count +=
            if matches!(decorator_kind, AngularDecoratorKind::Component) { 1 } else { 0 };
    }

    if jit_classes.is_empty() {
        // No Angular classes found, return source as-is
        if options.sourcemap {
            let (code, map) = apply_edits_with_sourcemap(source, vec![], path);
            result.code = code;
            result.map = map;
        } else {
            result.code = source.to_string();
        }
        return result;
    }

    // 4. Build edits
    let mut edits: std::vec::Vec<Edit> = std::vec::Vec::new();

    // Build the additional imports text (tslib + resource imports + Angular namespace
    // when signal-API lowering synthesized decorators that need to resolve to
    // `i0.Input`/`i0.Output`/etc. at runtime).
    let mut additional_imports = String::new();
    additional_imports.push_str("import { __decorate } from \"tslib\";\n");
    if jit_classes_need_angular_core_namespace(&jit_classes) {
        additional_imports
            .push_str(&format!("import * as {JIT_ANGULAR_CORE_NS} from \"@angular/core\";\n"));
    }
    for (import_name, specifier) in &resource_imports {
        additional_imports.push_str(&format!("import {} from \"{}\";\n", import_name, specifier));
    }

    // Insert additional imports after the last existing import
    let ns_insert_pos = find_last_import_end(&parser_ret.program.body);
    if let Some(insert_pos) = ns_insert_pos {
        let bytes = source.as_bytes();
        let mut actual_pos = insert_pos;
        while actual_pos < bytes.len() {
            let c = bytes[actual_pos];
            if c == b'\n' {
                actual_pos += 1;
                break;
            } else if c == b' ' || c == b'\t' || c == b'\r' {
                actual_pos += 1;
            } else {
                break;
            }
        }
        // Ensure insert position doesn't fall inside an import elision edit
        for edit in &edits {
            if (edit.start as usize) < actual_pos && (edit.end as usize) > actual_pos {
                actual_pos = edit.end as usize;
            }
        }
        edits.push(Edit::insert(actual_pos as u32, additional_imports).with_priority(10));
    } else {
        edits.push(Edit::insert(0, additional_imports).with_priority(10));
    }

    // Process each Angular class - generate edits for class restructuring
    // Also need to collect member/constructor decorator spans from the AST
    // Build a lookup of class positions to match against JitClassInfo
    for stmt in parser_ret.program.body.iter() {
        let class = match stmt {
            Statement::ClassDeclaration(class) => Some(class.as_ref()),
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class.as_ref()),
                _ => None,
            },
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
                _ => None,
            },
            _ => None,
        };

        let Some(class) = class else { continue };

        // Find the matching JitClassInfo by class start position
        let Some(jit_info) = jit_classes.iter().find(|info| info.class_start == class.span.start)
        else {
            continue;
        };

        // 4a. Remove ALL class-level decorators (including @ and trailing whitespace)
        for decorator_span in &jit_info.all_class_decorator_spans {
            let mut end = decorator_span.end as usize;
            let bytes = source.as_bytes();
            while end < bytes.len() {
                let c = bytes[end];
                if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                    end += 1;
                } else {
                    break;
                }
            }
            edits.push(Edit::delete(decorator_span.start, end as u32));
        }

        // 4b. Remove ALL member decorators and constructor param decorators
        {
            let mut decorator_spans: std::vec::Vec<Span> = std::vec::Vec::new();
            super::decorator::collect_constructor_decorator_spans(class, &mut decorator_spans);
            super::decorator::collect_all_member_decorator_spans(class, &mut decorator_spans);
            for span in &decorator_spans {
                let mut end = span.end as usize;
                let bytes = source.as_bytes();
                while end < bytes.len() {
                    let c = bytes[end];
                    if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                        end += 1;
                    } else {
                        break;
                    }
                }
                edits.push(Edit::delete(span.start, end as u32));
            }
        }

        // 4c. Class restructuring: `export class X` → `let X = class X`
        // For abstract classes, also strip the `abstract` keyword since class expressions can't be abstract.
        let class_keyword_start = if jit_info.is_abstract {
            let rest = &source[jit_info.class_start as usize..];
            let offset = rest.find("class").unwrap_or(0);
            jit_info.class_start + offset as u32
        } else {
            jit_info.class_start
        };

        if jit_info.is_exported || jit_info.is_default_export {
            edits.push(Edit::replace(
                jit_info.stmt_start,
                class_keyword_start,
                format!("let {} = ", jit_info.class_name),
            ));
        } else {
            edits.push(Edit::replace(
                jit_info.class_start,
                class_keyword_start,
                format!("let {} = ", jit_info.class_name),
            ));
        }

        // 4d. Add ctorParameters and propDecorators inside class body (before closing `}`)
        {
            let mut class_statics = String::new();
            if let Some(ctor_text) = build_ctor_parameters_text(&jit_info.ctor_params) {
                class_statics.push_str(&format!("\n{};", ctor_text));
            }
            if let Some(prop_text) = build_prop_decorators_text(&jit_info.member_decorators) {
                class_statics.push_str(&format!("\n{};", prop_text));
            }
            if !class_statics.is_empty() {
                class_statics.push('\n');
                edits.push(Edit::insert(jit_info.class_body_end - 1, class_statics));
            }
        }

        // 4e. After class body, add member __decorate calls, then class __decorate call, then export
        let mut after_class = String::from(";\n");

        // Emit __decorate() for non-Angular member decorators (before class __decorate).
        // Match TypeScript's ordering: instance (prototype) members first, then static members.
        // Within each group, preserve source declaration order.
        for member_dec in jit_info
            .non_angular_member_decorators
            .iter()
            .filter(|m| !m.is_static)
            .chain(jit_info.non_angular_member_decorators.iter().filter(|m| m.is_static))
        {
            let target = if member_dec.is_static {
                jit_info.class_name.clone()
            } else {
                format!("{}.prototype", jit_info.class_name)
            };
            // TypeScript uses `null` for methods/accessors (reads existing descriptor)
            // and `void 0` for properties (no existing descriptor).
            let desc = if member_dec.is_property { "void 0" } else { "null" };
            after_class.push_str(&format!(
                "__decorate([{}], {}, \"{}\", {});\n",
                member_dec.decorator_texts.join(", "),
                target,
                member_dec.member_name,
                desc
            ));
        }

        // Emit class-level __decorate() with ALL class decorators
        let all_decorator_text = jit_info.all_class_decorator_texts.join(",\n    ");
        after_class.push_str(&format!(
            "{} = __decorate([\n    {}\n], {});\n",
            jit_info.class_name, all_decorator_text, jit_info.class_name
        ));

        if jit_info.is_exported {
            after_class.push_str(&format!("export {{ {} }};\n", jit_info.class_name));
        } else if jit_info.is_default_export {
            after_class.push_str(&format!("export default {};\n", jit_info.class_name));
        }

        edits.push(Edit::insert(jit_info.class_body_end, after_class));
    }

    // Apply all edits
    if options.sourcemap {
        let (code, map) = apply_edits_with_sourcemap(source, edits, path);
        result.code = code;
        result.map = map;
    } else {
        result.code = apply_edits(source, edits);
    }

    // 5. Strip TypeScript syntax from JIT output
    result.code = strip_typescript(allocator, path, &result.code);

    result
}

/// Transform an Angular TypeScript file.
///
/// This function:
/// 1. Parses the TypeScript file using oxc_parser
/// 2. Finds all @Component decorated classes
/// 3. Extracts and compiles templates
/// 4. Generates HMR code if enabled
/// 5. Returns the transformed code
///
/// # Arguments
///
/// * `allocator` - Memory allocator for AST nodes
/// * `path` - File path (used for error messages and HMR IDs)
/// * `source` - Source code content
/// * `options` - Transformation options
/// * `resolved_resources` - Pre-resolved external templates and styles
///
/// # Returns
///
/// A `TransformResult` containing the transformed code and metadata.
pub fn transform_angular_file(
    allocator: &Allocator,
    path: &str,
    source: &str,
    options: Option<&TransformOptions>,
    resolved_resources: Option<&ResolvedResources>,
) -> TransformResult {
    let default_options = TransformOptions::default();
    let options = options.unwrap_or_else(|| &default_options);
    // JIT mode uses a completely different code path
    if options.jit {
        return transform_angular_file_jit(allocator, path, source, options);
    }

    let mut result = TransformResult::new();

    // 1. Parse the TypeScript file
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let parser_ret = Parser::new(allocator, source, source_type).parse();

    // Collect parse errors
    if !parser_ret.errors.is_empty() {
        for error in parser_ret.errors {
            result.diagnostics.push(OxcDiagnostic::error(error.to_string()));
        }
        // Still continue to try to generate output for partial results
    }

    // Run import elision analysis on the original program.
    // This identifies type-only imports that can be removed.
    // Must run BEFORE transformation to capture correct type vs value references.
    let import_elision = ImportElisionAnalyzer::analyze(&parser_ret.program);

    // Collect class definitions by class name.
    // Each entry is (class_name, static_definitions_to_insert, external_declarations)
    // The external_declarations are things like _c0 constants and child view functions that
    // go outside the class.
    // We track by name because positions shift after import filtering.
    // (property_assignments, decls_before_class, decls_after_class)
    let mut class_definitions: HashMap<String, (String, String, String)> = HashMap::new();
    let mut decorator_spans_to_remove: Vec<Span> = Vec::new();
    // Collect class positions from original AST for edit-based output.
    // (class_name, effective_start, class_body_end)
    let mut class_positions: Vec<(String, u32, u32)> = Vec::new();

    // File-level namespace registry to collect all module imports
    let mut file_namespace_registry = NamespaceRegistry::new(allocator);

    // Shared constant pool index across all components in this file.
    // This ensures constant names (_c0, _c1, etc.) don't conflict when
    // multiple components are compiled in the same file.
    // TypeScript Angular uses ONE file-level constant pool; we simulate this
    // by tracking the next index and passing it to each component.
    let mut shared_pool_index: u32 = 0;

    // When cross_file_elision is enabled, collect type-only information for each import
    // by checking if the exported symbol is an interface or type alias. This is separate
    // from barrel resolution to avoid changing namespace import paths.
    #[cfg(feature = "cross_file_elision")]
    let cross_file_type_only: FxHashMap<String, bool> = if options.cross_file_elision {
        let file_path = std::path::Path::new(path);
        let base_dir = options.base_dir.as_deref().or_else(|| file_path.parent());

        if let Some(base) = base_dir {
            let mut analyzer = CrossFileAnalyzer::new(base, options.tsconfig_path.as_deref());
            let mut type_only: FxHashMap<String, bool> = FxHashMap::default();

            for stmt in &parser_ret.program.body {
                let Statement::ImportDeclaration(import_decl) = stmt else {
                    continue;
                };

                let source = import_decl.source.value.as_str();
                let Some(specifiers) = &import_decl.specifiers else {
                    continue;
                };

                for specifier in specifiers {
                    if let ImportDeclarationSpecifier::ImportSpecifier(spec) = specifier {
                        let local_name = spec.local.name.as_str();
                        let imported_name = spec.imported.name().as_str();

                        // Check if this import is type-only using the original import path.
                        // Resolves the file and checks if the exported symbol is an interface
                        // or type alias. Unresolvable imports return false (conservative).
                        if analyzer.is_type_only_import(source, imported_name, file_path) {
                            type_only.insert(local_name.to_string(), true);
                        }
                    }
                }
            }

            type_only
        } else {
            FxHashMap::default()
        }
    } else {
        FxHashMap::default()
    };

    // Build import map from import declarations using ORIGINAL import paths.
    // Only externally-provided resolved_imports (e.g., for host directives) override paths.
    // Barrel-resolved paths are NOT used here to avoid changing namespace import paths
    // from what Angular's compiler would produce.
    #[cfg(not(feature = "cross_file_elision"))]
    let import_map =
        build_import_map(allocator, &parser_ret.program.body, options.resolved_imports.as_ref());

    // Collect file-scope string consts so decorator metadata can resolve identifier
    // references (e.g. `host: { [ATTR_NAME]: '' }`) the same way the official
    // Angular compiler does.
    let string_consts = collect_string_consts(allocator, &parser_ret.program);

    #[cfg(feature = "cross_file_elision")]
    let mut import_map =
        build_import_map(allocator, &parser_ret.program.body, options.resolved_imports.as_ref());

    // Apply cross-file type-only information to the import_map.
    // This marks imports that resolve to interfaces or type aliases as type-only,
    // even when they don't use `import type` syntax. This is needed because many
    // codebases import interfaces with regular `import { X }` syntax, and without
    // a TypeScript type checker we cannot otherwise distinguish interfaces from classes.
    #[cfg(feature = "cross_file_elision")]
    for (name, is_type_only) in &cross_file_type_only {
        if let Some(info) = import_map.get_mut(&Ident::from(name.as_str())) {
            if *is_type_only {
                info.is_type_only = true;
            }
        }
    }

    // 2. Walk AST to find @Component decorated classes and extract metadata
    for stmt in &parser_ret.program.body {
        let (class, stmt_start) = match stmt {
            Statement::ClassDeclaration(class) => (Some(class.as_ref()), class.span.start),
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    (Some(class.as_ref()), export.span.start)
                }
                _ => (None, 0),
            },
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => {
                    (Some(class.as_ref()), export.span.start)
                }
                _ => (None, 0),
            },
            _ => (None, 0),
        };

        if let Some(class) = class {
            // Pre-flight: catch @Service co-located with another Angular
            // decorator before the primary-decorator branches dispatch.
            // Upstream service.ts:101-116 rejects this combination; without
            // this early check, @Component / @Directive / @Pipe / @NgModule /
            // @Injectable would win the branch race and compile the class
            // (leaving the @Service decorator removed inconsistently) instead
            // of producing the intended diagnostic.
            if find_angular_service_decorator(class, &import_map).is_some() {
                if let Some(conflict_name) = find_conflicting_angular_decorator(class, &import_map)
                {
                    let class_name_for_diag =
                        class.id.as_ref().map_or(String::new(), |id| id.name.to_string());
                    result.diagnostics.push(OxcDiagnostic::error(format!(
                        "Cannot apply more than one Angular decorator on an @Service class. \
                         '{}' is also decorated with @{}.",
                        class_name_for_diag, conflict_name
                    )));
                    continue;
                }
            }

            // Compute implicit_standalone based on Angular version
            let implicit_standalone = options.implicit_standalone();

            if let Some(mut metadata) = extract_component_metadata(
                allocator,
                class,
                implicit_standalone,
                &import_map,
                Some(source),
                &string_consts,
            ) {
                // Track external resource dependencies before resolution so build tools
                // still learn which sibling files to watch when the resource isn't in
                // `resolved_resources` or compilation fails downstream.
                if let Some(template_url) = &metadata.template_url {
                    result.dependencies.push(template_url.to_string());
                }
                for style_url in &metadata.style_urls {
                    result.dependencies.push(style_url.to_string());
                }

                // 3. Resolve external styles and merge into metadata
                let missing_style_urls =
                    resolve_styles(allocator, &mut metadata, resolved_resources);

                // 4. Resolve template from inline or external source
                let (template_source, missing_template_url) =
                    resolve_template(&metadata, resolved_resources);
                let class_name = metadata.class_name.to_string();

                // Resources were provided but a styleUrl was not among them:
                // fail loudly like ngc (COMPONENT_RESOURCE_NOT_FOUND) instead
                // of silently dropping the styles (#314).
                for style_url in &missing_style_urls {
                    result.diagnostics.push(OxcDiagnostic::error(format!(
                        "Component '{}': style URL '{}' could not be resolved \
                         (COMPONENT_RESOURCE_NOT_FOUND)",
                        class_name, style_url
                    )));
                }
                if let Some(template_url) = &missing_template_url {
                    result.diagnostics.push(OxcDiagnostic::error(format!(
                        "Component '{}': template URL '{}' could not be resolved \
                         (COMPONENT_RESOURCE_NOT_FOUND)",
                        class_name, template_url
                    )));
                }

                if let Some(template_string) = template_source {
                    // Allocate template in arena so it has the allocator's lifetime.
                    // This is needed because namespace_registry outlives the template_source.
                    let template = allocator.alloc_str(&template_string);
                    // 4.5 Extract view queries from the class (for @ViewChild/@ViewChildren)
                    // These need to be passed to compile_component_full so predicates can be pooled
                    let view_queries = extract_view_queries(allocator, class, Some(source));

                    // 4.6 Extract content queries from the class (for @ContentChild/@ContentChildren)
                    // Signal-based queries (contentChild(), contentChildren()) are also detected here
                    let content_queries = extract_content_queries(allocator, class, Some(source));

                    // Collect content query property names for .d.ts generation
                    // (before content_queries is moved into compile_component_full)
                    let content_query_names: Vec<String> =
                        content_queries.iter().map(|q| q.property_name.to_string()).collect();

                    // 4. Compile the template and generate ɵcmp/ɵfac
                    // Pass the shared pool index to ensure unique constant names
                    // Pass the file-level namespace registry to ensure consistent namespace assignments
                    match compile_component_full(
                        allocator,
                        template,
                        &mut metadata,
                        path,
                        source,
                        options,
                        view_queries,
                        content_queries,
                        shared_pool_index,
                        &mut file_namespace_registry,
                        &import_map,
                    ) {
                        Ok(compilation_result) => {
                            // Update the shared pool index for the next component
                            shared_pool_index = compilation_result.next_pool_index;

                            let component_id = format!("{}@{}", path, class_name);

                            // Store for HMR if enabled
                            if options.hmr {
                                result.template_updates.insert(
                                    component_id.clone(),
                                    compilation_result.template_js.clone(),
                                );
                            }

                            // Track the decorator span to remove
                            if let Some(span) = find_component_decorator_span(class) {
                                decorator_spans_to_remove.push(span);
                            }
                            // Collect constructor parameter decorators (@Optional, @Inject, etc.)
                            collect_constructor_decorator_spans(
                                class,
                                &mut decorator_spans_to_remove,
                            );
                            // Collect member decorators (@Input, @Output, @HostBinding, etc.)
                            collect_member_decorator_spans(class, &mut decorator_spans_to_remove);

                            // Store the ɵfac/ɵcmp definitions.
                            // Order: ɵfac BEFORE ɵcmp (Angular convention).
                            // External declarations (child view functions, constants) go BEFORE the class.
                            // Note: The /*@__PURE__*/ annotation is already included in cmp_js by the emitter.
                            // ES2022 style: static fields INSIDE the class body
                            let mut property_assignments = format!(
                                "static ɵfac = {};\nstatic ɵcmp = {};",
                                compilation_result.fac_js, compilation_result.cmp_js
                            );

                            // Check if the class also has an @Injectable decorator.
                            // @Injectable is SHARED precedence and can coexist with @Component.
                            let has_injectable =
                                extract_injectable_metadata(allocator, class, Some(source));
                            if let Some(injectable_metadata) = &has_injectable {
                                if let Some(span) = find_injectable_decorator_span(class) {
                                    decorator_spans_to_remove.push(span);
                                }
                                if let Some(inj_def) = generate_injectable_definition_from_decorator(
                                    allocator,
                                    injectable_metadata,
                                    options.compilation_mode,
                                ) {
                                    let emitter = JsEmitter::new();
                                    property_assignments.push_str(&format!(
                                        "\nstatic ɵprov = {};",
                                        emitter.emit_expression(&inj_def.prov_definition)
                                    ));
                                }
                            }
                            let has_injectable = has_injectable.is_some();

                            // Split declarations into two groups:
                            // 1. decls_before_class: child view functions, constants (needed BEFORE class)
                            // 2. decls_after_class: debug info, metadata, HMR code (references class, needs AFTER)
                            let decls_before_class = compilation_result.declarations_js.clone();
                            let mut decls_after_class = String::new();

                            // Add class debug info (before class metadata, following Angular's order)
                            if let Some(debug_info_js) = &compilation_result.class_debug_info_js {
                                if !decls_after_class.is_empty() {
                                    decls_after_class.push('\n');
                                }
                                decls_after_class.push_str(debug_info_js);
                                decls_after_class.push(';');
                            }

                            // Add class metadata for TestBed support (after debug info, before HMR)
                            // Only emit when enabled and not in advanced optimizations mode
                            if options.emit_class_metadata && !options.advanced_optimizations {
                                if let Some(decorator) = find_component_decorator(&class.decorators)
                                {
                                    let emitter = JsEmitter::new();

                                    // Build the type expression: reference to the class
                                    let type_expr = crate::output::ast::OutputExpression::ReadVar(
                                        oxc_allocator::Box::new_in(
                                            ReadVarExpr {
                                                name: Ident::from(class_name.as_str()),
                                                source_span: None,
                                            },
                                            allocator,
                                        ),
                                    );

                                    // Build metadata from the class AST
                                    // Pass constructor deps and namespace registry so that
                                    // imported types get namespace-prefixed references
                                    // (e.g., i1.SomeService instead of bare SomeService)
                                    let ctor_deps_slice =
                                        metadata.constructor_deps.as_ref().map(|v| v.as_slice());
                                    let class_metadata = R3ClassMetadata {
                                        r#type: type_expr,
                                        decorators: build_decorator_metadata_array(
                                            allocator,
                                            &[decorator],
                                            Some(source),
                                            Some(template),
                                            Some(metadata.styles.as_slice()),
                                            Some(&string_consts),
                                        ),
                                        ctor_parameters: build_ctor_params_metadata(
                                            allocator,
                                            class,
                                            ctor_deps_slice,
                                            &mut file_namespace_registry,
                                            &import_map,
                                            Some(source),
                                        ),
                                        prop_decorators: build_prop_decorators_metadata(
                                            allocator,
                                            class,
                                            Some(source),
                                            &mut file_namespace_registry,
                                        ),
                                    };

                                    // If the component opted into lazy loading via
                                    // `@Component.deferredImports` AND actually uses
                                    // `@defer` in its template, emit
                                    // `ɵsetClassMetadataAsync` so the TestBed-facing
                                    // metadata is built lazily inside a callback that
                                    // receives the dynamically-imported classes as
                                    // parameters. The decorator object literal still
                                    // references symbols like `LazyCmp` by name, but
                                    // those names resolve to the callback's parameters
                                    // — there's no static reference to the import in
                                    // the emitted code, so bundlers can tree-shake the
                                    // `import { LazyCmp } from './lazy'` declaration in
                                    // production builds (the sync `setClassMetadata`
                                    // call is itself dev-only and gets dropped too).
                                    //
                                    // When the template has no `@defer` block we fall
                                    // back to plain `setClassMetadata`: the resolver
                                    // and async wrapper would never run, and emitting
                                    // them would leave a stray `import(...)` in the
                                    // output that defeats tree-shaking.
                                    let deferred_deps: oxc_allocator::Vec<
                                        R3DeferPerComponentDependency<'_>,
                                    > = if compilation_result.has_defer_block {
                                        super::defer_resolver::build_defer_per_component_deps(
                                            allocator,
                                            &metadata.deferred_imports,
                                            &import_map,
                                        )
                                    } else {
                                        oxc_allocator::Vec::new_in(allocator)
                                    };
                                    let metadata_expr = match options.compilation_mode {
                                        crate::CompilationMode::Partial => {
                                            // Partial mode mirrors upstream's
                                            // compileComponentDeclareClassMetadata
                                            // (class_metadata.ts:50): empty deferred
                                            // deps → sync ɵɵngDeclareClassMetadata; one
                                            // or more → async ɵɵngDeclareClassMetadataAsync.
                                            crate::partial::compile_component_declare_class_metadata(
                                                allocator,
                                                &class_metadata,
                                                deferred_deps.as_slice(),
                                            )
                                        }
                                        crate::CompilationMode::Full => {
                                            if deferred_deps.is_empty() {
                                                compile_class_metadata(allocator, &class_metadata)
                                            } else {
                                                compile_component_class_metadata(
                                                    allocator,
                                                    &class_metadata,
                                                    Some(deferred_deps.as_slice()),
                                                )
                                            }
                                        }
                                    };
                                    let metadata_js = emitter.emit_expression(&metadata_expr);

                                    if !decls_after_class.is_empty() {
                                        decls_after_class.push('\n');
                                    }
                                    decls_after_class.push_str(&metadata_js);
                                    decls_after_class.push(';');
                                }
                            }

                            // Add HMR initializer last
                            if let Some(hmr_js) = &compilation_result.hmr_initializer_js {
                                if !decls_after_class.is_empty() {
                                    decls_after_class.push('\n');
                                }
                                decls_after_class.push_str(hmr_js);
                                decls_after_class.push(';');
                            }

                            class_definitions.insert(
                                class_name.clone(),
                                (property_assignments, decls_before_class, decls_after_class),
                            );
                            class_positions.push((
                                class_name,
                                compute_effective_start(
                                    class,
                                    &decorator_spans_to_remove,
                                    stmt_start,
                                ),
                                class.body.span.end,
                            ));

                            // Generate .d.ts type declaration for this component
                            let type_argument_count = class
                                .type_parameters
                                .as_ref()
                                .map_or(0, |tp| tp.params.len() as u32);
                            result.dts_declarations.push(dts::generate_component_dts(
                                &metadata,
                                type_argument_count,
                                &content_query_names,
                                has_injectable,
                                &compilation_result.ng_content_selectors,
                            ));

                            result.component_count += 1;
                        }
                        Err(diags) => {
                            result.diagnostics.extend(diags);
                        }
                    }
                } else if missing_template_url.is_none()
                    && let Some(template_url) = &metadata.template_url
                {
                    // External template not resolved: the class is emitted
                    // without its compiled definition, so fail loudly like
                    // ngc's COMPONENT_RESOURCE_NOT_FOUND instead of letting
                    // the build ship a broken component (#314).
                    result.diagnostics.push(OxcDiagnostic::error(format!(
                        "Component '{}': template URL '{}' could not be resolved \
                         (COMPONENT_RESOURCE_NOT_FOUND); the component was emitted \
                         without its compiled definition",
                        class_name, template_url
                    )));
                }
            } else {
                // Not a @Component - check if it's a @Directive
                // We need to compile @Directive classes properly to generate ɵdir/ɵfac
                // definitions. This prevents Angular's JIT runtime from processing
                // the directive and creating conflicting property definitions (like
                // ɵfac getters) that interfere with the AOT-compiled assignments.
                if let Some(mut directive_metadata) = extract_directive_metadata(
                    allocator,
                    class,
                    implicit_standalone,
                    Some(source),
                    &string_consts,
                ) {
                    // Track decorator span for removal
                    if let Some(span) = find_directive_decorator_span(class) {
                        decorator_spans_to_remove.push(span);
                    }
                    // Collect constructor parameter decorators (@Optional, @Inject, etc.)
                    collect_constructor_decorator_spans(class, &mut decorator_spans_to_remove);
                    // Collect member decorators (@Input, @Output, @HostBinding, etc.)
                    collect_member_decorator_spans(class, &mut decorator_spans_to_remove);

                    // Resolve namespace imports for directive constructor deps.
                    // Directives can inject services from other modules (e.g., Store from @ngrx/store),
                    // so factory deps must use namespace-prefixed references (e.g., i1.Store).
                    if let Some(ref mut deps) = directive_metadata.deps {
                        resolve_factory_dep_namespaces(
                            allocator,
                            deps,
                            &import_map,
                            &mut file_namespace_registry,
                        );
                    }

                    // Resolve namespace imports for hostDirectives references.
                    // Host directive references (e.g., BrnTooltipTrigger from '@spartan-ng/brain/tooltip')
                    // must use namespace-prefixed references (e.g., i1.BrnTooltipTrigger) because the
                    // original named import may be elided and replaced by a namespace import.
                    resolve_host_directive_namespaces(
                        allocator,
                        &mut directive_metadata.host_directives,
                        &import_map,
                        &mut file_namespace_registry,
                    );

                    // Compile directive and generate definitions
                    // Pass shared_pool_index to ensure unique constant names across the file
                    let definitions = generate_directive_definitions(
                        allocator,
                        &directive_metadata,
                        shared_pool_index,
                        options.compilation_mode,
                        options.angular_version,
                    );

                    // Update shared_pool_index for the next compilation
                    shared_pool_index = definitions.next_pool_index;

                    // Use JsEmitter to emit the expressions
                    let emitter = JsEmitter::new();
                    let class_name = directive_metadata.name.to_string();
                    // Order: ɵfac BEFORE ɵdir (Angular convention)
                    // ES2022 style: static fields INSIDE the class body
                    let mut property_assignments = format!(
                        "static ɵfac = {};\nstatic ɵdir = {};",
                        emitter.emit_expression(&definitions.fac_definition),
                        emitter.emit_expression(&definitions.dir_definition)
                    );

                    // Check if the class also has an @Injectable decorator.
                    // @Injectable is SHARED precedence and can coexist with @Directive.
                    let has_injectable =
                        extract_injectable_metadata(allocator, class, Some(source));
                    if let Some(injectable_metadata) = &has_injectable {
                        if let Some(span) = find_injectable_decorator_span(class) {
                            decorator_spans_to_remove.push(span);
                        }
                        if let Some(inj_def) = generate_injectable_definition_from_decorator(
                            allocator,
                            injectable_metadata,
                            options.compilation_mode,
                        ) {
                            property_assignments.push_str(&format!(
                                "\nstatic ɵprov = {};",
                                emitter.emit_expression(&inj_def.prov_definition)
                            ));
                        }
                    }
                    let has_injectable = has_injectable.is_some();

                    // Generate .d.ts type declaration for this directive
                    let type_argument_count =
                        class.type_parameters.as_ref().map_or(0, |tp| tp.params.len() as u32);
                    directive_metadata.type_argument_count = type_argument_count;
                    result
                        .dts_declarations
                        .push(dts::generate_directive_dts(&directive_metadata, has_injectable));

                    // Emit setClassMetadata for TestBed support (overrideDirective +
                    // signal members), mirroring the @Component path.
                    let decls_after_class = find_directive_decorator(&class.decorators)
                        .map(|decorator| {
                            build_set_class_metadata_decls(
                                allocator,
                                class,
                                &class_name,
                                decorator,
                                options,
                                source,
                                &string_consts,
                                &import_map,
                                &mut file_namespace_registry,
                            )
                        })
                        .unwrap_or_default();

                    class_positions.push((
                        class_name.clone(),
                        compute_effective_start(class, &decorator_spans_to_remove, stmt_start),
                        class.body.span.end,
                    ));
                    class_definitions.insert(
                        class_name,
                        (property_assignments, String::new(), decls_after_class),
                    );
                } else if let Some(mut pipe_metadata) =
                    extract_pipe_metadata(allocator, class, implicit_standalone, Some(source))
                {
                    // Not a @Component or @Directive - check if it's a @Pipe (PRIMARY)
                    // We need to compile @Pipe classes to generate ɵpipe and ɵfac definitions.
                    // - ɵpipe: Pipe definition for Angular's pipe system
                    // - ɵfac: Factory function for dependency injection (when pipe has constructor deps)

                    // Track decorator span for removal
                    if let Some(span) = find_pipe_decorator_span(class) {
                        decorator_spans_to_remove.push(span);
                    }
                    // Collect constructor parameter decorators (@Optional, @Inject, etc.)
                    collect_constructor_decorator_spans(class, &mut decorator_spans_to_remove);

                    // Resolve namespace imports for pipe constructor deps
                    if let Some(ref mut deps) = pipe_metadata.deps {
                        resolve_factory_dep_namespaces(
                            allocator,
                            deps,
                            &import_map,
                            &mut file_namespace_registry,
                        );
                    }

                    // Compile pipe and generate both ɵfac and ɵpipe definitions as external property assignments
                    if let Some(definition) = generate_full_pipe_definition_from_decorator(
                        allocator,
                        &pipe_metadata,
                        options.compilation_mode,
                    ) {
                        // Use JsEmitter to emit both expressions
                        let emitter = JsEmitter::new();
                        let class_name = pipe_metadata.class_name.to_string();
                        // Order: ɵfac BEFORE ɵpipe (Angular convention)
                        // ES2022 style: static fields INSIDE the class body
                        let mut property_assignments = format!(
                            "static ɵfac = {};\nstatic ɵpipe = {};",
                            emitter.emit_expression(&definition.fac_definition),
                            emitter.emit_expression(&definition.pipe_definition)
                        );

                        // Check if the class also has an @Injectable decorator (issue #65).
                        // @Injectable is SHARED precedence and can coexist with @Pipe.
                        let has_injectable =
                            extract_injectable_metadata(allocator, class, Some(source));
                        if let Some(injectable_metadata) = &has_injectable {
                            if let Some(span) = find_injectable_decorator_span(class) {
                                decorator_spans_to_remove.push(span);
                            }
                            if let Some(inj_def) = generate_injectable_definition_from_decorator(
                                allocator,
                                injectable_metadata,
                                options.compilation_mode,
                            ) {
                                property_assignments.push_str(&format!(
                                    "\nstatic ɵprov = {};",
                                    emitter.emit_expression(&inj_def.prov_definition)
                                ));
                            }
                        }
                        let has_injectable = has_injectable.is_some();

                        // Generate .d.ts type declaration for this pipe
                        let type_argument_count =
                            class.type_parameters.as_ref().map_or(0, |tp| tp.params.len() as u32);
                        result.dts_declarations.push(dts::generate_pipe_dts(
                            &pipe_metadata,
                            type_argument_count,
                            has_injectable,
                        ));

                        // Emit setClassMetadata for TestBed support (overridePipe).
                        let decls_after_class = find_pipe_decorator(&class.decorators)
                            .map(|decorator| {
                                build_set_class_metadata_decls(
                                    allocator,
                                    class,
                                    &class_name,
                                    decorator,
                                    options,
                                    source,
                                    &string_consts,
                                    &import_map,
                                    &mut file_namespace_registry,
                                )
                            })
                            .unwrap_or_default();

                        class_positions.push((
                            class_name.clone(),
                            compute_effective_start(class, &decorator_spans_to_remove, stmt_start),
                            class.body.span.end,
                        ));
                        class_definitions.insert(
                            class_name,
                            (property_assignments, String::new(), decls_after_class),
                        );
                    }
                } else if let Some(mut ng_module_metadata) =
                    extract_ng_module_metadata(allocator, class, Some(source))
                {
                    // Not a @Component, @Directive, @Injectable, or @Pipe - check if it's an @NgModule
                    // We need to compile @NgModule classes to generate ɵmod, ɵfac, and ɵinj definitions.
                    // - ɵmod: NgModule definition
                    // - ɵfac: Factory function for instantiation (with constructor dependencies)
                    // - ɵinj: Injector definition with providers and imports

                    // Track decorator span for removal
                    if let Some(span) = find_ng_module_decorator_span(class) {
                        decorator_spans_to_remove.push(span);
                    }
                    // Collect constructor parameter decorators (@Optional, @Inject, etc.)
                    collect_constructor_decorator_spans(class, &mut decorator_spans_to_remove);

                    // Resolve namespace imports for NgModule constructor deps
                    if let Some(ref mut deps) = ng_module_metadata.deps {
                        resolve_factory_dep_namespaces(
                            allocator,
                            deps,
                            &import_map,
                            &mut file_namespace_registry,
                        );
                    }

                    // Compile NgModule and generate all definitions as external property assignments
                    if let Some(definition) = generate_full_ng_module_definition(
                        allocator,
                        &ng_module_metadata,
                        options.compilation_mode,
                    ) {
                        let emitter = JsEmitter::new();
                        let class_name = ng_module_metadata.class_name.to_string();

                        // Generate static field definitions
                        // Order: ɵfac BEFORE ɵmod BEFORE ɵinj (Angular convention)
                        // ES2022 style: static fields INSIDE the class body
                        let mut property_assignments = format!(
                            "static ɵfac = {};\nstatic ɵmod = {};\nstatic ɵinj = {};",
                            emitter.emit_expression(&definition.fac_definition),
                            emitter.emit_expression(&definition.mod_definition),
                            emitter.emit_expression(&definition.inj_definition)
                        );

                        // Check if the class also has an @Injectable decorator.
                        // @Injectable is SHARED precedence and can coexist with @NgModule.
                        let has_injectable =
                            extract_injectable_metadata(allocator, class, Some(source));
                        if let Some(injectable_metadata) = &has_injectable {
                            if let Some(span) = find_injectable_decorator_span(class) {
                                decorator_spans_to_remove.push(span);
                            }
                            if let Some(inj_def) = generate_injectable_definition_from_decorator(
                                allocator,
                                injectable_metadata,
                                options.compilation_mode,
                            ) {
                                property_assignments.push_str(&format!(
                                    "\nstatic ɵprov = {};",
                                    emitter.emit_expression(&inj_def.prov_definition)
                                ));
                            }
                        }
                        let has_injectable = has_injectable.is_some();

                        // Collect any side-effect statements as external declarations
                        let mut external_decls = String::new();
                        for stmt in &definition.statements {
                            if !external_decls.is_empty() {
                                external_decls.push('\n');
                            }
                            external_decls.push_str(&emitter.emit_statement(stmt));
                        }

                        // Generate .d.ts type declaration for this NgModule
                        let type_argument_count =
                            class.type_parameters.as_ref().map_or(0, |tp| tp.params.len() as u32);
                        result.dts_declarations.push(dts::generate_ng_module_dts(
                            &ng_module_metadata,
                            type_argument_count,
                            has_injectable,
                        ));

                        // Emit setClassMetadata for TestBed support (overrideModule),
                        // appended after the NgModule's external declarations.
                        if let Some(decorator) = find_ng_module_decorator(&class.decorators) {
                            let metadata = build_set_class_metadata_decls(
                                allocator,
                                class,
                                &class_name,
                                decorator,
                                options,
                                source,
                                &string_consts,
                                &import_map,
                                &mut file_namespace_registry,
                            );
                            if !metadata.is_empty() {
                                if !external_decls.is_empty() {
                                    external_decls.push('\n');
                                }
                                external_decls.push_str(&metadata);
                            }
                        }

                        // NgModule: external_decls go AFTER the class (they reference the class name)
                        class_positions.push((
                            class_name.clone(),
                            compute_effective_start(class, &decorator_spans_to_remove, stmt_start),
                            class.body.span.end,
                        ));
                        class_definitions.insert(
                            class_name,
                            (property_assignments, String::new(), external_decls),
                        );
                    }
                } else if let Some(service_decorator) =
                    find_angular_service_decorator(class, &import_map)
                {
                    // Standalone @Service (Angular v22+). Mirrors the standalone-@Injectable
                    // branch below but emits ɵɵdefineService and a deps-less ɵfac.
                    let class_name_for_diag =
                        class.id.as_ref().map_or(String::new(), |id| id.name.to_string());

                    // Version gate: v22+ runtime introduced ɵɵdefineService. Unknown
                    // version defaults to "supports" (matches the JIT-side gate).
                    if !options.angular_version.map_or(true, |v| v.supports_service_decorator()) {
                        result.diagnostics.push(OxcDiagnostic::error(format!(
                            "The @Service decorator on '{}' requires Angular v22 or later.",
                            class_name_for_diag
                        )));
                        continue;
                    }

                    // Constructor DI diagnostic: @Service ɵfac is generated with
                    // empty deps, so any constructor parameter would silently
                    // become `undefined` at runtime. Surface upstream's error
                    // (service.ts:312-318) instead of emitting broken code.
                    if class_has_own_constructor_params(class) {
                        result.diagnostics.push(OxcDiagnostic::error(
                            "@Service class cannot use constructor dependency injection. \
                             Use the `inject` function instead."
                                .to_string(),
                        ));
                        continue;
                    }

                    if let Some(service_metadata) =
                        extract_service_metadata(allocator, class, service_decorator, Some(source))
                    {
                        // Track decorator span for removal. Use the resolved
                        // decorator directly so aliased imports
                        // (`import { Service as NgService }`) still get
                        // stripped — re-searching by literal name would miss
                        // them.
                        decorator_spans_to_remove.push(service_decorator.span);
                        // Even though @Service ɵfac doesn't inject ctor params, the
                        // user's constructor may still carry @Inject/@Optional/etc.
                        // decorators that need to be stripped from the output (the
                        // class metadata IIFE will pick them up).
                        collect_constructor_decorator_spans(class, &mut decorator_spans_to_remove);

                        let type_argument_count =
                            class.type_parameters.as_ref().map_or(0, |tp| tp.params.len() as u32);
                        let definition = generate_service_definition_from_decorator(
                            allocator,
                            &service_metadata,
                            type_argument_count,
                        );

                        let emitter = JsEmitter::new();
                        let class_name = service_metadata.class_name.to_string();

                        let property_assignments = format!(
                            "static ɵfac = {};\nstatic ɵprov = {};",
                            emitter.emit_expression(&definition.fac_definition),
                            emitter.emit_expression(&definition.prov_definition)
                        );

                        result.dts_declarations.push(dts::generate_service_dts(
                            &service_metadata,
                            type_argument_count,
                        ));

                        let decls_after_class = build_set_class_metadata_decls(
                            allocator,
                            class,
                            &class_name,
                            service_decorator,
                            options,
                            source,
                            &string_consts,
                            &import_map,
                            &mut file_namespace_registry,
                        );

                        class_positions.push((
                            class_name.clone(),
                            compute_effective_start(class, &decorator_spans_to_remove, stmt_start),
                            class.body.span.end,
                        ));
                        class_definitions.insert(
                            class_name,
                            (property_assignments, String::new(), decls_after_class),
                        );
                    }
                } else if let Some(mut injectable_metadata) =
                    extract_injectable_metadata(allocator, class, Some(source))
                {
                    // Standalone @Injectable (no PRIMARY decorator on the class)
                    // We need to compile @Injectable classes to generate ɵprov and ɵfac definitions.
                    // - ɵprov: Provider metadata for Angular's DI system
                    // - ɵfac: Factory function to instantiate the class

                    // Track decorator span for removal
                    if let Some(span) = find_injectable_decorator_span(class) {
                        decorator_spans_to_remove.push(span);
                    }
                    // Collect constructor parameter decorators (@Optional, @Inject, etc.)
                    collect_constructor_decorator_spans(class, &mut decorator_spans_to_remove);

                    // Resolve namespace imports for constructor deps.
                    if let Some(ref mut deps) = injectable_metadata.deps {
                        resolve_factory_dep_namespaces(
                            allocator,
                            deps,
                            &import_map,
                            &mut file_namespace_registry,
                        );
                    }

                    // Compile injectable and generate definitions
                    if let Some(definition) = generate_injectable_definition_from_decorator(
                        allocator,
                        &injectable_metadata,
                        options.compilation_mode,
                    ) {
                        let emitter = JsEmitter::new();
                        let class_name = injectable_metadata.class_name.to_string();

                        // ES2022 style: static fields INSIDE the class body
                        let property_assignments = format!(
                            "static ɵfac = {};\nstatic ɵprov = {};",
                            emitter.emit_expression(&definition.fac_definition),
                            emitter.emit_expression(&definition.prov_definition)
                        );

                        // Generate .d.ts type declaration for this injectable
                        let type_argument_count =
                            class.type_parameters.as_ref().map_or(0, |tp| tp.params.len() as u32);
                        result.dts_declarations.push(dts::generate_injectable_dts(
                            &injectable_metadata,
                            type_argument_count,
                        ));

                        // Emit setClassMetadata for TestBed support.
                        let decls_after_class = find_injectable_decorator(&class.decorators)
                            .map(|decorator| {
                                build_set_class_metadata_decls(
                                    allocator,
                                    class,
                                    &class_name,
                                    decorator,
                                    options,
                                    source,
                                    &string_consts,
                                    &import_map,
                                    &mut file_namespace_registry,
                                )
                            })
                            .unwrap_or_default();

                        class_positions.push((
                            class_name.clone(),
                            compute_effective_start(class, &decorator_spans_to_remove, stmt_start),
                            class.body.span.end,
                        ));
                        class_definitions.insert(
                            class_name,
                            (property_assignments, String::new(), decls_after_class),
                        );
                    }
                }
            }
        }
    }

    // 5. Generate output code using span-based edits from the original AST.
    // All edits reference positions in the original source and are applied in one pass.

    // 5a. Import elision edits (collected first for namespace insert position check)
    let elision_edits = import_elision_edits(source, &parser_ret.program, &import_elision);

    // 5b. Namespace import insertion
    // Must be computed before merging other edits, since we need to check if the
    // insert position falls inside an import elision edit span.
    let namespace_imports = file_namespace_registry.generate_import_statements();
    let ns_edit = if !namespace_imports.is_empty() && !class_definitions.is_empty() {
        let ns_insert_pos = find_last_import_end(&parser_ret.program.body);
        if let Some(insert_pos) = ns_insert_pos {
            let bytes = source.as_bytes();
            let mut actual_pos = insert_pos;
            // Skip past trailing whitespace/newline after the last import semicolon
            while actual_pos < bytes.len() {
                let c = bytes[actual_pos];
                if c == b'\n' {
                    actual_pos += 1;
                    break;
                } else if c == b' ' || c == b'\t' || c == b'\r' {
                    actual_pos += 1;
                } else {
                    break;
                }
            }
            // Ensure the insert position doesn't fall inside any import elision
            // edit span. Import elision edits extend past trailing newlines, which
            // may cover the position we computed above.
            // Note: elision edits are in ascending source order and non-overlapping
            // (one per import declaration), so a single forward pass is sufficient —
            // adjusting actual_pos forward can never land inside an earlier edit.
            for edit in &elision_edits {
                if (edit.start as usize) < actual_pos && (edit.end as usize) > actual_pos {
                    actual_pos = edit.end as usize;
                }
            }
            Some(Edit::insert(actual_pos as u32, namespace_imports).with_priority(10))
        } else {
            Some(Edit::insert(0, namespace_imports).with_priority(10))
        }
    } else {
        None
    };

    // 5c. Merge all edits
    let mut edits: Vec<Edit> = elision_edits;

    // Decorator removal edits
    for span in &decorator_spans_to_remove {
        let mut end = span.end as usize;
        let bytes = source.as_bytes();
        while end < bytes.len() {
            let c = bytes[end];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                end += 1;
            } else {
                break;
            }
        }
        edits.push(Edit::delete(span.start, end as u32));
    }

    if let Some(edit) = ns_edit {
        edits.push(edit);
    }

    // 5d. Class definition insertion edits
    for (class_name, effective_start, class_body_end) in &class_positions {
        if let Some((property_assignments, decls_before_class, decls_after_class)) =
            class_definitions.get(class_name)
        {
            // Static fields inside class body (before closing `}`)
            if !property_assignments.is_empty() {
                edits.push(Edit::insert(
                    class_body_end - 1,
                    format!("\n{}\n", property_assignments),
                ));
            }

            // Constants/child views before the class
            if !decls_before_class.is_empty() {
                edits.push(Edit::insert(*effective_start, format!("{}\n", decls_before_class)));
            }

            // Debug info/class metadata/HMR after the class
            if !decls_after_class.is_empty() {
                edits.push(Edit::insert(*class_body_end, format!("\n{}", decls_after_class)));
            }
        }
    }

    // 5e. TDZ-safe hoisting of top-level bindings referenced by decorator
    // metadata but declared after the decorated class. Without this, the
    // emitted `ɵcmp` static field's `ɵɵProvidersFeature` would evaluate the
    // reference at class-definition time and throw `ReferenceError`. See
    // issue #287.
    //
    // The hoister resolves identifier references through `oxc_semantic` so
    // a nested-scope shadow of a top-level name can't be mistaken for the
    // top-level binding itself.
    //
    // Gate the Semantic build behind a cheap top-level scan: a real Angular
    // codebase contains plenty of plain `.ts` helpers, type-only modules, and
    // services without `@Injectable` that we route through this function. For
    // those, building a full symbol table just to discover there's nothing to
    // hoist is pure overhead.
    if program_has_angular_decorated_class(&parser_ret.program) {
        // Semantic builder errors (redeclarations, etc.) are intentionally
        // dropped: the parser already captured syntax errors into
        // `result.diagnostics` upstream, and Semantic-level diagnostics here
        // aren't actionable for the hoist pass — we treat the input as
        // best-effort and rely on the host build to surface genuine errors.
        // The JIT path (see ~line 1380) follows the same convention.
        let hoist_semantic =
            oxc_semantic::SemanticBuilder::new().build(&parser_ret.program).semantic;
        edits.extend(collect_hoist_edits(&parser_ret.program, source, &hoist_semantic));
    }

    // Apply all edits in one pass
    if options.sourcemap {
        let (code, map) = apply_edits_with_sourcemap(source, edits, path);
        result.code = code;
        result.map = map;
    } else {
        result.code = apply_edits(source, edits);
    }

    result
}

/// Result of full component compilation including ɵcmp/ɵfac.
struct FullCompilationResult {
    /// Compiled template function as JavaScript.
    template_js: String,

    /// ɵcmp definition as JavaScript.
    cmp_js: String,

    /// ɵfac factory function as JavaScript.
    fac_js: String,

    /// Additional declarations (child view functions, constants).
    declarations_js: String,

    /// HMR initialization code (if HMR is enabled).
    hmr_initializer_js: Option<String>,

    /// Class debug info statement (for dev mode runtime errors).
    /// Wrapped in ngDevMode guard: `(() => { (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassDebugInfo(...); })();`
    class_debug_info_js: Option<String>,

    /// The next constant pool index to use for the next component.
    /// This is used to share pool state across multiple components in the same file.
    next_pool_index: u32,

    /// The ng-content selectors found in the template (e.g., `["*", ".header"]`).
    ng_content_selectors: Vec<String>,

    /// `true` when the parsed template contains at least one `@defer` block.
    ///
    /// Used by the caller to gate `setClassMetadataAsync` emission: a component
    /// can declare `deferredImports: [...]` without actually using `@defer` in
    /// its template (e.g., during development before the block is wired up).
    /// In that case Angular's runtime needs neither the lazy resolver nor the
    /// async metadata callback, and emitting them would leave dead
    /// `import(...)` calls in the output.
    has_defer_block: bool,
}

/// Compile a component template and generate ɵcmp/ɵfac definitions.
///
/// The `pool_starting_index` parameter is used to ensure constant names don't conflict
/// when compiling multiple components in the same file. Each component continues from
/// where the previous component's pool left off.
///
/// The `namespace_registry` parameter is used to track and assign namespace aliases
/// for imported modules. It is shared across all components in the file to ensure
/// consistent namespace assignments for factory generation and import statements.
/// Partial-mode component compile: bypass the entire template/IR pipeline
/// and emit `ɵɵngDeclareComponent` + `ɵɵngDeclareFactory` directly from the
/// metadata. The linker re-parses the verbatim template at consumer build
/// time. See `crate::partial::component` for the shape.
fn compile_component_partial<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    metadata: &ComponentMetadata<'a>,
    pool_starting_index: u32,
) -> FullCompilationResult {
    let inputs =
        crate::partial::PartialComponentInputs { template, is_inline: metadata.template.is_some() };
    let cmp_expr =
        crate::partial::compile_declare_component_from_metadata(allocator, metadata, &inputs);
    let fac_expr =
        crate::partial::component::compile_declare_factory_for_component(allocator, metadata);

    // Detect `@defer` block presence by a cheap string scan — partial
    // mode skips the template pipeline, but the caller's class-metadata
    // dispatch uses this flag to decide whether to build deferred-deps
    // and pick `ɵɵngDeclareClassMetadataAsync` over the sync form. A
    // false positive (e.g. `@defer` inside a string literal in the
    // template) is harmless: `deferred_deps` will be built from
    // `metadata.deferred_imports`, which is empty when the user hasn't
    // declared deferrable imports, and the dispatch falls back to sync.
    // A false NEGATIVE silently strips the async lazy-loading metadata —
    // so err on the side of detection.
    let has_defer_block = template.contains("@defer");

    let emitter = JsEmitter::new();
    FullCompilationResult {
        template_js: String::new(),
        cmp_js: emitter.emit_expression(&cmp_expr),
        fac_js: emitter.emit_expression(&fac_expr),
        declarations_js: String::new(),
        hmr_initializer_js: None,
        class_debug_info_js: None,
        next_pool_index: pool_starting_index,
        ng_content_selectors: Vec::new(),
        has_defer_block,
    }
}

fn compile_component_full<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    metadata: &mut ComponentMetadata<'a>,
    file_path: &str,
    source: &str,
    options: &TransformOptions,
    view_queries: OxcVec<'a, R3QueryMetadata<'a>>,
    content_queries: OxcVec<'a, R3QueryMetadata<'a>>,
    pool_starting_index: u32,
    namespace_registry: &mut NamespaceRegistry<'a>,
    import_map: &ImportMap<'a>,
) -> Result<FullCompilationResult, Vec<OxcDiagnostic>> {
    use oxc_allocator::FromIn;

    // Partial-mode early branch: skip the entire template pipeline.
    // Partial declarations carry the template as a verbatim string and
    // let the linker re-parse at consumer build time.
    if matches!(options.compilation_mode, crate::CompilationMode::Partial) {
        return Ok(compile_component_partial(allocator, template, metadata, pool_starting_index));
    }

    let mut diagnostics = Vec::new();

    // Stage 1: Parse HTML
    // Build parse options from component metadata
    // Angular always forces tokenizeExpansionForms: true in parseTemplate()
    let parse_options = ParseTemplateOptions {
        preserve_whitespaces: metadata.preserve_whitespaces,
        // Enable modern syntax features (block syntax, let declarations)
        enable_block_syntax: true,
        enable_let_syntax: true,
        // Always enable ICU expansion forms - Angular forces this in template.ts:152
        tokenize_expansion_forms: true,
        ..Default::default()
    };
    let parser = HtmlParser::with_options(allocator, template, file_path, &parse_options);
    let html_result = parser.parse();

    if !html_result.errors.is_empty() {
        for error in &html_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 1.5: Remove whitespace if not preserving
    // This must happen before R3 transform, matching Angular's pipeline
    let nodes = if parse_options.preserve_whitespaces {
        &html_result.nodes
    } else {
        // Apply whitespace removal with preserveSignificantWhitespace=true
        // (Angular uses true for template compilation, false for message extraction)
        let processed = remove_whitespaces(allocator, &html_result.nodes, true);
        // We need to leak this to get a reference with the right lifetime
        // since we're in a function that returns Result
        allocator.alloc(processed) as &_
    };

    // Stage 2: Transform HTML to R3 AST
    let r3_transform_options =
        R3TransformOptions { collect_comment_nodes: parse_options.collect_comment_nodes };
    let transformer = HtmlToR3Transform::new(allocator, template, r3_transform_options);
    let r3_result = transformer.transform(nodes);

    if !r3_result.errors.is_empty() {
        for error in &r3_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Capture ng-content selectors from the R3 AST for .d.ts generation
    let ng_content_selectors: Vec<String> =
        r3_result.ng_content_selectors.iter().map(|s| s.to_string()).collect();

    // Merge inline template styles into component metadata
    // These are styles from <style> tags directly in the template HTML
    for style in r3_result.styles.iter() {
        metadata.styles.push(style.clone());
    }

    // Stage 3-5: Ingest and compile
    // Build ingest options from metadata and transform options
    let component_name_atom = Ident::from_in(metadata.class_name.as_str(), allocator);

    // OXC is a single-file compiler, equivalent to Angular's local compilation mode.
    // In local compilation mode, Angular ALWAYS sets hasDirectiveDependencies=true,
    // so DomOnly mode is never used for component templates.
    // See: angular/packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts:1257
    //
    // Note: DomOnly mode is still used for host bindings (separate code path).
    let mode = TemplateCompilationMode::Full;

    // Reject overlap between `imports: [X]` and `deferredImports: [X]` —
    // a symbol must be either eager OR deferrable, never both. Mirrors
    // Angular's `validateNoImportOverlap` (handler.ts:2558+).
    if !metadata.deferred_imports.is_empty() && !metadata.imports.is_empty() {
        let eager: rustc_hash::FxHashSet<&str> =
            metadata.imports.iter().map(|i| i.as_str()).collect();
        for deferred in &metadata.deferred_imports {
            if eager.contains(deferred.as_str()) {
                diagnostics.push(OxcDiagnostic::error(format!(
                    "`{}` is imported via both `@Component.imports` and `@Component.deferredImports`. \
                     To fix this, make sure that dependencies are imported only once.",
                    deferred.as_str()
                )));
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
    }

    // Build the deferrable-dependencies resolver from
    // `@Component.deferredImports`.
    //
    // OXC is a single-file (local) compiler, so we use Angular's PerComponent
    // emit mode for defer dependencies: a single shared resolver function is
    // generated from the component's `deferredImports: [...]` array and
    // wired into every `ɵɵdefer(...)` call. The runtime invokes it lazily,
    // the first time any defer block triggers.
    //
    // Issue: voidzero-dev/oxc-angular-compiler#289 — previously the resolver
    // was omitted entirely, leaving `ɵɵdefer(...)` with no dependency
    // argument and no `import(...)` calls in the output, so `@defer` did no
    // code-splitting.
    //
    // Note: Angular's full compilation mode also derives deferrable symbols
    // from `imports: [...]` by matching template selectors against imported
    // directive classes (`resolveAllDeferredDependencies`). That needs
    // cross-file metadata OXC doesn't have, so we follow Angular's local
    // path and require explicit `deferredImports`.
    // Computed once and reused below: the resolver decision and the eventual
    // `setClassMetadataAsync` gate both depend on whether the template uses
    // `@defer`.
    let has_defer_block = super::defer_resolver::template_has_defer_block(&r3_result.nodes);

    let defer_resolver_fn = if metadata.deferred_imports.is_empty() {
        None
    } else if has_defer_block {
        super::defer_resolver::build_defer_resolver_expression(allocator, metadata, import_map)
    } else {
        // `deferredImports` declared but no `@defer` block — the resolver
        // would be unused dead code. Leave it off.
        None
    };

    // When we have a resolver, switch to PerComponent emit mode so the
    // ingest stage threads `all_deferrable_deps_fn` into each `DeferOp`.
    // Without a resolver, the existing PerBlock-vs-JIT behavior stays in
    // place (and `all_deferrable_deps_fn = None`).
    let defer_block_deps_emit_mode = if defer_resolver_fn.is_some() {
        DeferBlockDepsEmitMode::PerComponent
    } else if options.jit {
        DeferBlockDepsEmitMode::PerComponent
    } else {
        DeferBlockDepsEmitMode::PerBlock
    };

    // Enable debug locations when NOT using advanced optimizations
    // (advanced optimizations strip debug info for smaller output)
    let enable_debug_locations = !options.advanced_optimizations;

    // Build relative paths for debug locations
    let relative_template_path =
        if enable_debug_locations { Some(Ident::from_in(file_path, allocator)) } else { None };

    let relative_context_file_path = Some(Ident::from_in(file_path, allocator));

    let ingest_options = IngestOptions {
        mode,
        relative_context_file_path,
        i18n_use_external_ids: options.i18n_use_external_ids,
        defer_block_deps_emit_mode,
        relative_template_path,
        enable_debug_locations,
        template_source: if enable_debug_locations { Some(template) } else { None },
        // PerComponent mode: pass the shared resolver expression so the
        // ingest stage attaches it to every defer block's create op.
        all_deferrable_deps_fn: defer_resolver_fn,
        // Use the shared pool starting index to avoid duplicate constant names
        // when compiling multiple components in the same file
        pool_starting_index,
        // Pass Angular version for feature-gated instruction selection
        angular_version: options.angular_version,
        legacy_optional_chaining: options.legacy_optional_chaining,
    };

    let mut job = ingest_component_with_options(
        allocator,
        component_name_atom,
        r3_result.nodes,
        ingest_options,
    );

    // BEFORE template compilation: Pool attrs const if first selector has attributes.
    // This matches TypeScript Angular which adds attrs to the pool BEFORE template ingestion.
    // By pooling attrs first, it gets _c0 (or the first available index).
    // See: packages/compiler/src/render3/view/compiler.ts lines 192-212
    let attrs_ref = pool_selector_attrs(allocator, &mut job, metadata);

    // BEFORE template compilation: Pool query predicates so they
    // appear in the constant table as `attrs -> query predicates ->
    // pure functions`.
    //
    // Order matters: upstream ngtsc pools **content queries first, then
    // view queries** (compiler/src/render3/view/compiler.ts emits
    // `contentQueries:` before `viewQuery:` in the metadata object, and
    // pools predicates in the same order). Reversing this swaps the
    // `_c<N>` indices visible in the emitted query calls — the runtime
    // still works either way but the emit diverges from upstream and
    // breaks goldens / source-anchored tooling.
    let content_queries_fn = if !content_queries.is_empty() {
        let fn_name = Some(metadata.class_name.as_str());
        Some(create_content_queries_function(
            allocator,
            content_queries.as_slice(),
            fn_name,
            Some(&mut job.pool),
            options.angular_version,
        ))
    } else {
        None
    };

    let view_query_fn = if !view_queries.is_empty() {
        let fn_name = Some(metadata.class_name.as_str());
        Some(create_view_queries_function(
            allocator,
            view_queries.as_slice(),
            fn_name,
            Some(&mut job.pool),
            options.angular_version,
        ))
    } else {
        None
    };

    let compiled = compile_template(&mut job);

    // Stage 6: Compile host bindings (if any)
    // Pass the template pool's current index to ensure host binding constants
    // continue from where template compilation left off (avoiding duplicate names)
    let template_pool_index = job.pool.next_name_index();
    let host_binding_output = compile_component_host_bindings(
        allocator,
        metadata,
        template_pool_index,
        options.angular_version,
        options.legacy_optional_chaining,
    );

    // Extract the result and update pool index if host bindings were compiled
    let (host_binding_result, host_binding_next_pool_index, host_binding_declarations) =
        match host_binding_output {
            Some(output) => {
                let declarations = output.result.declarations;
                let result = HostBindingCompilationResult {
                    host_binding_fn: output.result.host_binding_fn,
                    host_attrs: output.result.host_attrs,
                    host_vars: output.result.host_vars,
                    declarations: OxcVec::new_in(allocator),
                };
                (Some(result), Some(output.next_pool_index), declarations)
            }
            None => (None, None, OxcVec::new_in(allocator)),
        };

    // Stage 7: Generate ɵcmp/ɵfac definitions
    // The namespace registry is shared across all components in the file to ensure
    // consistent namespace assignments between factory generation and import statements.
    // Pass the pre-pooled attrs_ref so the attrs entry uses the correct constant index
    let definitions = generate_component_definitions(
        allocator,
        metadata,
        options,
        &mut job,
        compiled.template_fn,
        host_binding_result,
        attrs_ref,
        view_query_fn,
        content_queries_fn,
        namespace_registry,
    );

    // Emit JavaScript
    let emitter = JsEmitter::new();

    // Emit declarations (child view functions, constants)
    let mut declarations_js = String::new();
    for decl in compiled.declarations.iter() {
        declarations_js.push_str(&emitter.emit_statement(decl));
        declarations_js.push('\n');
    }
    // Emit host binding declarations (pooled constants like pure functions)
    for decl in host_binding_declarations.iter() {
        declarations_js.push_str(&emitter.emit_statement(decl));
        declarations_js.push('\n');
    }

    // For HMR, we emit the template separately using compile_template_to_js
    // The ɵcmp already contains the template function inline
    let template_js =
        compile_template_to_js(allocator, template, metadata.class_name.as_str(), file_path)
            .map_err(|diags| {
                let mut all_diags = diagnostics.clone();
                all_diags.extend(diags);
                all_diags
            })?;

    let cmp_js = emitter.emit_expression(&definitions.cmp_definition);
    let fac_js = emitter.emit_expression(&definitions.fac_definition);

    // Generate HMR initialization code if enabled
    let hmr_initializer_js = if options.hmr {
        use crate::hmr::{HmrMetadata, compile_hmr_initializer};
        use crate::output::ast::{OutputExpression, ReadVarExpr};

        // Create component type expression (reference to the class)
        let component_type = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
            ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
            allocator,
        ));

        // Build HMR metadata
        let mut hmr_meta = HmrMetadata::new(
            component_type,
            metadata.class_name.clone(),
            Ident::from_in(file_path, allocator),
        );

        // Add the @angular/core namespace dependency (i0)
        hmr_meta.add_namespace_dependency(Ident::from("@angular/core"), Ident::from("i0"));

        // Generate the HMR initializer expression
        let hmr_expr = compile_hmr_initializer(allocator, &hmr_meta);

        // Emit to JavaScript
        Some(emitter.emit_expression(&hmr_expr))
    } else {
        None
    };

    // Generate class debug info for dev-mode runtime errors
    // This provides class name, file path, and line number for better error messages
    let class_debug_info_js = if !options.advanced_optimizations {
        use crate::class_debug_info::{R3ClassDebugInfo, compile_class_debug_info};
        use crate::output::ast::{OutputExpression, ReadVarExpr};

        // Create component type expression (reference to the class)
        let component_type = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
            ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
            allocator,
        ));

        // Compute the 1-indexed line number of the class declaration from its byte offset
        let class_line_number = {
            let offset = metadata.class_span.start as usize;
            let mut line = 1u32;
            for &byte in &source.as_bytes()[..offset.min(source.len())] {
                if byte == b'\n' {
                    line += 1;
                }
            }
            line
        };

        // Build the debug info with the actual class declaration line number
        let debug_info = R3ClassDebugInfo::new(component_type, metadata.class_name.clone())
            .with_file_path(Ident::from_in(file_path, allocator))
            .with_line_number(class_line_number);

        // Compile to IIFE-wrapped expression
        let debug_info_expr = compile_class_debug_info(allocator, &debug_info);

        Some(emitter.emit_expression(&debug_info_expr))
    } else {
        None
    };

    // Get the next pool index for the next component in this file
    // If host bindings were compiled, use the host binding pool's next index
    // (since it continues from the template pool)
    // Otherwise, use the template pool's next index
    let next_pool_index =
        host_binding_next_pool_index.unwrap_or_else(|| job.pool.next_name_index());

    // Collect any diagnostics from the compilation job
    // (Done after using job to avoid borrow issues)
    diagnostics.extend(job.diagnostics);

    Ok(FullCompilationResult {
        template_js,
        cmp_js,
        fac_js,
        declarations_js,
        hmr_initializer_js,
        class_debug_info_js,
        next_pool_index,
        ng_content_selectors,
        has_defer_block,
    })
}

/// Resolve template content from inline or external source.
///
/// Precedence matches Angular's AOT compiler (`parseTemplateDeclaration` in
/// `compiler-cli/src/ngtsc/annotations/component/src/resources.ts`): when both
/// `templateUrl` and inline `template` are present, **`templateUrl` wins** and
/// the inline `template` is silently ignored. Angular's reference checks
/// `component.has('templateUrl')` first and returns immediately, so the inline
/// branch is never reached. (ngc's JIT runtime diverges — it prefers inline via
/// `componentNeedsResolution` — but OXC is AOT-equivalent.)
fn resolve_template(
    metadata: &ComponentMetadata<'_>,
    resources: Option<&ResolvedResources>,
) -> (Option<String>, Option<String>) {
    // ngc AOT precedence: templateUrl first. When resources were supplied, a
    // missing entry is reported to the caller instead of falling back to inline.
    if let Some(template_url) = &metadata.template_url {
        if let Some(resources) = resources {
            if let Some(template) = resources.templates.get(template_url.as_str()) {
                return (Some(template.clone()), None);
            }
            return (None, Some(template_url.to_string()));
        }
    }

    if let Some(template) = &metadata.template {
        return (Some(template.to_string()), None);
    }

    (None, None)
}

/// Resolve external styles and merge into component metadata.
///
/// Returns the styleUrls that were NOT found in `resources` (only when
/// resources were provided), so the caller can surface a
/// COMPONENT_RESOURCE_NOT_FOUND diagnostic per missing resource.
fn resolve_styles<'a>(
    allocator: &'a Allocator,
    metadata: &mut ComponentMetadata<'a>,
    resources: Option<&ResolvedResources>,
) -> Vec<String> {
    use oxc_allocator::FromIn;

    let mut missing = Vec::new();

    if let Some(resources) = resources {
        // Resolve each styleUrl from the resources
        for style_url in &metadata.style_urls {
            if let Some(style_contents) = resources.styles.get(style_url.as_str()) {
                // Add all resolved style contents to the metadata styles
                for style in style_contents {
                    metadata.styles.push(Ident::from_in(style.as_str(), allocator));
                }
            } else {
                missing.push(style_url.to_string());
            }
        }
    }

    missing
}

/// Compile a component template to JavaScript.
///
/// This is the core template compilation API that can be used
/// when you already have the template source and component name.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `template` - Template HTML source
/// * `component_name` - Name of the component class
/// * `file_path` - Path to the component file (for error messages)
///
/// # Returns
///
/// A tuple of (template_function, diagnostics) or an error.
pub fn compile_component_template<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    component_name: &str,
    file_path: &str,
) -> Result<(FunctionExpr<'a>, Vec<OxcDiagnostic>), Vec<OxcDiagnostic>> {
    let mut diagnostics = Vec::new();

    // Stage 1: Parse HTML with default options
    // Angular always forces tokenizeExpansionForms: true in parseTemplate()
    let parse_options = ParseTemplateOptions {
        // Enable modern syntax features (block syntax, let declarations)
        enable_block_syntax: true,
        enable_let_syntax: true,
        // Always enable ICU expansion forms - Angular forces this in template.ts:152
        tokenize_expansion_forms: true,
        // preserve_whitespaces defaults to false, matching Angular's default
        ..Default::default()
    };
    let parser = HtmlParser::with_options(allocator, template, file_path, &parse_options);
    let html_result = parser.parse();

    if !html_result.errors.is_empty() {
        for error in &html_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 1.5: Remove whitespace if not preserving (default behavior)
    let nodes = if parse_options.preserve_whitespaces {
        &html_result.nodes
    } else {
        let processed = remove_whitespaces(allocator, &html_result.nodes, true);
        allocator.alloc(processed) as &_
    };

    // Stage 2: Transform HTML to R3 AST
    let r3_transform_options =
        R3TransformOptions { collect_comment_nodes: parse_options.collect_comment_nodes };
    let transformer = HtmlToR3Transform::new(allocator, template, r3_transform_options);
    let r3_result = transformer.transform(nodes);

    if !r3_result.errors.is_empty() {
        for error in &r3_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 3-5: Ingest and compile
    use oxc_allocator::FromIn;
    let component_name_atom = Ident::from_in(component_name, allocator);
    let mut job = ingest_component(allocator, component_name_atom, r3_result.nodes);

    let compiled = compile_template(&mut job);

    // Collect any diagnostics from the compilation job
    diagnostics.extend(job.diagnostics.into_iter());

    Ok((compiled.template_fn, diagnostics))
}

/// Compile a template and return JavaScript code.
///
/// This is a convenience function that compiles a template and
/// returns the JavaScript code as a string. Uses default options.
pub fn compile_template_to_js<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    component_name: &str,
    file_path: &str,
) -> Result<String, Vec<OxcDiagnostic>> {
    compile_template_to_js_with_options(
        allocator,
        template,
        component_name,
        file_path,
        &TransformOptions::default(),
    )
    .map(|output| output.code)
}

/// Compile a template and return JavaScript code with custom options.
///
/// This is a convenience function that compiles a template and
/// returns the JavaScript code, using the provided options.
///
/// When `options.sourcemap` is true, also generates a source map.
pub fn compile_template_to_js_with_options<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    component_name: &str,
    file_path: &str,
    options: &TransformOptions,
) -> Result<TemplateCompileOutput, Vec<OxcDiagnostic>> {
    use std::sync::Arc;

    use crate::pipeline::ingest::{IngestOptions, ingest_component_with_options};
    use crate::util::ParseSourceFile;
    use oxc_allocator::FromIn;

    let mut diagnostics = Vec::new();

    // Stage 1: Parse HTML with options
    // Angular always forces tokenizeExpansionForms: true in parseTemplate()
    let parse_options = ParseTemplateOptions {
        // Use preserve_whitespaces from options, defaulting to false (Angular's default)
        preserve_whitespaces: options.preserve_whitespaces.unwrap_or(false),
        // Enable modern syntax features (block syntax, let declarations)
        enable_block_syntax: true,
        enable_let_syntax: true,
        // Always enable ICU expansion forms - Angular forces this in template.ts:152
        tokenize_expansion_forms: true,
        ..Default::default()
    };
    let parser = HtmlParser::with_options(allocator, template, file_path, &parse_options);
    let html_result = parser.parse();

    if !html_result.errors.is_empty() {
        for error in &html_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 1.5: Remove whitespace if not preserving (default behavior)
    let nodes = if parse_options.preserve_whitespaces {
        &html_result.nodes
    } else {
        let processed = remove_whitespaces(allocator, &html_result.nodes, true);
        allocator.alloc(processed) as &_
    };

    // Stage 2: Transform HTML to R3 AST
    let r3_transform_options =
        R3TransformOptions { collect_comment_nodes: parse_options.collect_comment_nodes };
    let transformer = HtmlToR3Transform::new(allocator, template, r3_transform_options);
    let r3_result = transformer.transform(nodes);

    if !r3_result.errors.is_empty() {
        for error in &r3_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Build IngestOptions from TransformOptions
    // OXC is a single-file compiler (local compilation mode): always use Full mode.
    let mode = TemplateCompilationMode::Full;

    let defer_block_deps_emit_mode = if options.jit {
        DeferBlockDepsEmitMode::PerComponent
    } else {
        DeferBlockDepsEmitMode::PerBlock
    };

    let enable_debug_locations = !options.advanced_optimizations;

    let ingest_options = IngestOptions {
        mode,
        relative_context_file_path: None,
        i18n_use_external_ids: options.i18n_use_external_ids,
        defer_block_deps_emit_mode,
        relative_template_path: None,
        enable_debug_locations,
        template_source: Some(template),
        all_deferrable_deps_fn: None,
        pool_starting_index: 0, // Standalone template compilation starts from 0
        angular_version: options.angular_version,
        legacy_optional_chaining: options.legacy_optional_chaining,
    };

    // Stage 3-5: Ingest and compile
    let component_name_atom = Ident::from_in(component_name, allocator);
    let mut job = ingest_component_with_options(
        allocator,
        component_name_atom,
        r3_result.nodes,
        ingest_options,
    );

    let compiled = compile_template(&mut job);

    // Collect any diagnostics from the compilation job
    diagnostics.extend(job.diagnostics.into_iter());

    let emitter = JsEmitter::new();

    // Build a list of all statements to emit:
    // 1. Declarations (child view functions, pooled constants)
    // 2. Main template function as a declaration
    let mut all_statements: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);

    // Add all declarations first (child view functions come before main template)
    for decl in compiled.declarations {
        all_statements.push(decl);
    }

    // Convert the main template function expression to a function declaration
    // The template_fn already has a name set (e.g., "AppComponent_Template")
    if let Some(fn_name) = compiled.template_fn.name.clone() {
        let main_fn_stmt = OutputStatement::DeclareFunction(oxc_allocator::Box::new_in(
            DeclareFunctionStmt {
                name: fn_name,
                params: compiled.template_fn.params,
                statements: compiled.template_fn.statements,
                modifiers: StmtModifier::NONE,
                source_span: compiled.template_fn.source_span,
            },
            allocator,
        ));
        all_statements.push(main_fn_stmt);
    }

    // Stage 6: Compile host bindings if provided via options
    let host_pool_starting_index = job.pool.next_name_index();
    if let Some(ref host_input) = options.host {
        if let Some(host_result) = compile_host_bindings_from_input(
            allocator,
            host_input,
            component_name,
            options.selector.as_deref(),
            host_pool_starting_index,
            options.angular_version,
            options.legacy_optional_chaining,
        ) {
            // Add host binding pool declarations (pure functions, etc.)
            for decl in host_result.declarations {
                all_statements.push(decl);
            }

            // Add the host bindings function as a declaration if present
            if let Some(host_fn) = host_result.host_binding_fn {
                if let Some(fn_name) = host_fn.name.clone() {
                    let host_fn_stmt =
                        OutputStatement::DeclareFunction(oxc_allocator::Box::new_in(
                            DeclareFunctionStmt {
                                name: fn_name,
                                params: host_fn.params,
                                statements: host_fn.statements,
                                modifiers: StmtModifier::NONE,
                                source_span: host_fn.source_span,
                            },
                            allocator,
                        ));
                    all_statements.push(host_fn_stmt);
                }
            }
        }
    }

    // Generate code with optional source map
    if options.sourcemap {
        let source_file = Arc::new(ParseSourceFile::new(template, file_path));
        let (code, map) =
            emitter.emit_statements_with_source_map(&all_statements, source_file, None);
        Ok(TemplateCompileOutput::with_source_map(code, map))
    } else {
        let code = emitter.emit_statements(&all_statements);
        Ok(TemplateCompileOutput::new(code))
    }
}

/// Compile a template for HMR, returning both the template function and declarations.
///
/// This is similar to `compile_template_to_js_with_options` but also returns
/// the constant declarations (child view functions, pooled constants) that
/// need to be included in the HMR update module.
pub fn compile_template_for_hmr<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    component_name: &str,
    file_path: &str,
    options: &TransformOptions,
) -> Result<HmrTemplateCompileOutput, Vec<OxcDiagnostic>> {
    use crate::output::ast::OutputExpression;
    use crate::pipeline::ingest::{IngestOptions, ingest_component_with_options};
    use oxc_allocator::{Box, FromIn};

    let mut diagnostics = Vec::new();

    // Stage 1: Parse HTML with options
    let parse_options = ParseTemplateOptions {
        // Use preserve_whitespaces from options, defaulting to false (Angular's default)
        preserve_whitespaces: options.preserve_whitespaces.unwrap_or(false),
        enable_block_syntax: true,
        enable_let_syntax: true,
        tokenize_expansion_forms: true,
        ..Default::default()
    };
    let parser = HtmlParser::with_options(allocator, template, file_path, &parse_options);
    let html_result = parser.parse();

    if !html_result.errors.is_empty() {
        for error in &html_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 1.5: Remove whitespace if not preserving
    let nodes = if parse_options.preserve_whitespaces {
        &html_result.nodes
    } else {
        let processed = remove_whitespaces(allocator, &html_result.nodes, true);
        allocator.alloc(processed) as &_
    };

    // Stage 2: Transform HTML to R3 AST
    let r3_transform_options =
        R3TransformOptions { collect_comment_nodes: parse_options.collect_comment_nodes };
    let transformer = HtmlToR3Transform::new(allocator, template, r3_transform_options);
    let r3_result = transformer.transform(nodes);

    if !r3_result.errors.is_empty() {
        for error in &r3_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Build IngestOptions from TransformOptions
    // OXC is a single-file compiler (local compilation mode): always use Full mode.
    let mode = TemplateCompilationMode::Full;

    let defer_block_deps_emit_mode = if options.jit {
        DeferBlockDepsEmitMode::PerComponent
    } else {
        DeferBlockDepsEmitMode::PerBlock
    };

    let enable_debug_locations = !options.advanced_optimizations;

    let ingest_options = IngestOptions {
        mode,
        relative_context_file_path: None,
        i18n_use_external_ids: options.i18n_use_external_ids,
        defer_block_deps_emit_mode,
        relative_template_path: None,
        enable_debug_locations,
        template_source: Some(template),
        all_deferrable_deps_fn: None,
        pool_starting_index: 0, // HMR template compilation starts from 0
        angular_version: options.angular_version,
        legacy_optional_chaining: options.legacy_optional_chaining,
    };

    // Stage 3-5: Ingest and compile
    let component_name_atom = Ident::from_in(component_name, allocator);
    let mut job = ingest_component_with_options(
        allocator,
        component_name_atom,
        r3_result.nodes,
        ingest_options,
    );

    let compiled = compile_template(&mut job);

    // Collect any diagnostics from the compilation job
    diagnostics.extend(job.diagnostics.into_iter());

    let emitter = JsEmitter::new();

    // Emit the template function
    let fn_expr = OutputExpression::Function(Box::new_in(compiled.template_fn, allocator));
    let template_js = emitter.emit_expression(&fn_expr);

    // Emit the declarations (child view functions, pooled constants)
    let mut declarations_js = String::new();
    for decl in &compiled.declarations {
        let decl_code = emitter.emit_statement(decl);
        declarations_js.push_str(&decl_code);
        declarations_js.push('\n');
    }

    // Extract styles from <style> tags in the template
    let styles: std::vec::Vec<String> = r3_result.styles.iter().map(|s| s.to_string()).collect();

    // Emit the consts array if present
    // The consts array must be included in HMR updates to ensure the template function's
    // constant references match. Without this, the HMR module would spread the old ɵcmp
    // which has a different consts array, causing index out of bounds errors.
    let consts_js = if !job.consts.is_empty() {
        use crate::output::ast::{
            FunctionExpr, LiteralArrayExpr, OutputExpression, OutputStatement, ReturnStatement,
        };

        let mut const_entries: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);
        for const_value in &job.consts {
            const_entries.push(const_value_to_expression(allocator, const_value));
        }

        let consts_expr = if !job.consts_initializers.is_empty() {
            // When there are initializers (e.g., i18n variable declarations), wrap consts
            // in a function that runs initializers first and returns the array.
            // This matches what definition.rs does for the initial component definition.
            let mut fn_stmts: OxcVec<'a, OutputStatement<'a>> =
                OxcVec::with_capacity_in(job.consts_initializers.len() + 1, allocator);

            for stmt in job.consts_initializers.drain(..) {
                fn_stmts.push(stmt);
            }

            fn_stmts.push(OutputStatement::Return(oxc_allocator::Box::new_in(
                ReturnStatement {
                    value: OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
                        LiteralArrayExpr { entries: const_entries, source_span: None },
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            )));

            OutputExpression::Function(oxc_allocator::Box::new_in(
                FunctionExpr {
                    name: None,
                    params: OxcVec::new_in(allocator),
                    statements: fn_stmts,
                    source_span: None,
                },
                allocator,
            ))
        } else {
            OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
                LiteralArrayExpr { entries: const_entries, source_span: None },
                allocator,
            ))
        };

        Some(emitter.emit_expression(&consts_expr))
    } else {
        None
    };

    Ok(HmrTemplateCompileOutput { template_js, declarations_js, styles, consts_js })
}

/// Generate component compilation output for HMR.
///
/// Returns the compiled template and metadata needed for HMR updates.
pub fn compile_for_hmr<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    file_path: &str,
) -> Result<(String, String), Vec<OxcDiagnostic>> {
    // Get the template source
    let template = match &metadata.template {
        Some(t) => t.as_str(),
        None => {
            return Err(vec![OxcDiagnostic::error(
                "Component must have an inline template for HMR",
            )]);
        }
    };

    // Compile the template
    let js = compile_template_to_js(allocator, template, &metadata.class_name, file_path)?;

    // Generate the component ID
    let component_id = metadata.component_id(file_path);

    Ok((component_id, js))
}

/// Result of host binding compilation including the next pool index.
struct HostBindingCompilationOutput<'a> {
    /// The compiled host binding result (function, hostAttrs, hostVars).
    result: HostBindingCompilationResult<'a>,
    /// The next constant pool index after host binding compilation.
    /// Used to continue the shared pool state for subsequent compilations.
    next_pool_index: u32,
}

/// Compile host bindings for a component.
///
/// This function converts the raw host binding metadata (strings from the decorator)
/// into a compiled host binding result (function, hostAttrs, hostVars).
///
/// The `pool_starting_index` parameter is used to ensure constant names don't conflict
/// with those already used by template compilation. In Angular TypeScript, both template
/// and host binding compilation share the same ConstantPool instance. In our implementation,
/// we pass the template pool's next index to achieve the same effect.
///
/// Returns None if the component has no host bindings.
fn compile_component_host_bindings<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    pool_starting_index: u32,
    angular_version: Option<AngularVersion>,
    legacy_optional_chaining: Option<bool>,
) -> Option<HostBindingCompilationOutput<'a>> {
    let host = metadata.host.as_ref()?;

    // Check if there are any host bindings at all
    // Include class_attr and style_attr as they become static attributes
    if host.properties.is_empty()
        && host.attributes.is_empty()
        && host.listeners.is_empty()
        && host.class_attr.is_none()
        && host.style_attr.is_none()
    {
        return None;
    }

    // Get component name and selector
    let component_name = metadata.class_name.clone();
    let component_selector = metadata.selector.clone().unwrap_or_else(|| Ident::from(""));

    // Convert HostMetadata to HostBindingInput
    let input = convert_host_metadata_to_input(allocator, host, component_name, component_selector);

    // Ingest and compile the host bindings with the pool starting index
    // This ensures constant names continue from where template compilation left off
    let mut job = ingest_host_binding_with_version(
        allocator,
        input,
        pool_starting_index,
        angular_version,
        legacy_optional_chaining,
    );
    let result = compile_host_bindings(&mut job);

    // Get the next pool index after host binding compilation
    let next_pool_index = job.pool.next_name_index();

    Some(HostBindingCompilationOutput { result, next_pool_index })
}

/// Convert HostMetadata (raw strings) to HostBindingInput (parsed structures).
///
/// This function parses the host binding expressions from the decorator
/// and creates the properly typed structures needed for ingestion.
fn convert_host_metadata_to_input<'a>(
    allocator: &'a Allocator,
    host: &HostMetadata<'a>,
    component_name: Ident<'a>,
    component_selector: Ident<'a>,
) -> HostBindingInput<'a> {
    use oxc_allocator::FromIn;

    let binding_parser = BindingParser::new(allocator);
    let empty_span = Span::empty(0);

    // Convert property bindings: "[class.active]" -> R3BoundAttribute
    let mut properties: OxcVec<'a, R3BoundAttribute<'a>> = OxcVec::new_in(allocator);

    for (key, value) in host.properties.iter() {
        // Strip the brackets from the key: "[prop]" -> "prop"
        let key_str = key.as_str();
        let prop_name = if key_str.starts_with('[') && key_str.ends_with(']') {
            &key_str[1..key_str.len() - 1]
        } else {
            key_str
        };

        // Determine binding type based on property name prefix
        let (binding_type, final_name, unit) = parse_host_property_name(prop_name);

        // Parse the value expression
        let value_str = allocator.alloc_str(value.as_str());
        let parse_result = binding_parser.parse_binding(value_str, empty_span);

        properties.push(R3BoundAttribute {
            name: Ident::from_in(final_name, allocator),
            binding_type,
            security_context: SecurityContext::None,
            value: parse_result.ast,
            unit: unit.map(|u| Ident::from_in(u, allocator)),
            source_span: empty_span,
            key_span: empty_span,
            value_span: Some(empty_span),
            i18n: None,
        });
    }

    // Convert event listeners: "(click)" -> R3BoundEvent
    let mut events: OxcVec<'a, R3BoundEvent<'a>> = OxcVec::new_in(allocator);

    for (key, value) in host.listeners.iter() {
        // Strip the parentheses from the key: "(click)" -> "click"
        let key_str = key.as_str();
        let event_name = if key_str.starts_with('(') && key_str.ends_with(')') {
            &key_str[1..key_str.len() - 1]
        } else {
            key_str
        };

        // Check for target prefix (window:, document:, body:)
        let (final_event_name, target) = parse_event_target(event_name);

        let (effective_name, event_type, phase) =
            parse_legacy_animation_event(final_event_name, allocator);

        // Parse the handler expression
        let value_str = allocator.alloc_str(value.as_str());
        let parse_result = binding_parser.parse_event(value_str, empty_span);

        events.push(R3BoundEvent {
            name: Ident::from_in(effective_name, allocator),
            event_type,
            handler: parse_result.ast,
            target: target.map(|t| Ident::from_in(t, allocator)),
            phase,
            source_span: empty_span,
            handler_span: empty_span,
            key_span: empty_span,
        });
    }

    // Convert static attributes: "role" -> OutputExpression::Literal
    // This matches TypeScript which uses `o.literal(value)` for host attributes
    let mut attributes: FxHashMap<Ident<'a>, crate::output::ast::OutputExpression<'a>> =
        FxHashMap::default();

    for (key, value) in host.attributes.iter() {
        // Create a literal string expression for static attributes
        let expr = crate::output::ast::OutputExpression::Literal(oxc_allocator::Box::new_in(
            crate::output::ast::LiteralExpr {
                value: crate::output::ast::LiteralValue::String(value.clone()),
                source_span: None,
            },
            allocator,
        ));
        attributes.insert(key.clone(), expr);
    }

    // Add special attributes (class_attr, style_attr) to the attributes map.
    // This matches Angular's compiler.ts lines 501-510 which adds these to attributes
    // before passing to the ingestion phase.
    if let Some(ref style_attr) = host.style_attr {
        let expr = crate::output::ast::OutputExpression::Literal(oxc_allocator::Box::new_in(
            crate::output::ast::LiteralExpr {
                value: crate::output::ast::LiteralValue::String(style_attr.clone()),
                source_span: None,
            },
            allocator,
        ));
        attributes.insert(Ident::from("style"), expr);
    }

    if let Some(ref class_attr) = host.class_attr {
        let expr = crate::output::ast::OutputExpression::Literal(oxc_allocator::Box::new_in(
            crate::output::ast::LiteralExpr {
                value: crate::output::ast::LiteralValue::String(class_attr.clone()),
                source_span: None,
            },
            allocator,
        ));
        attributes.insert(Ident::from("class"), expr);
    }

    HostBindingInput { component_name, component_selector, properties, attributes, events }
}

/// Parse a host property name to determine binding type and extract the final name.
///
/// Examples:
/// - "class.active" -> (BindingType::Class, "active", None)
/// - "style.color" -> (BindingType::Style, "color", None)
/// - "style.width.px" -> (BindingType::Style, "width", Some("px"))
/// - "attr.role" -> (BindingType::Attribute, "role", None)
/// - "disabled" -> (BindingType::Property, "disabled", None)
fn parse_host_property_name(name: &str) -> (BindingType, &str, Option<&str>) {
    if let Some(rest) = name.strip_prefix("class.") {
        (BindingType::Class, rest, None)
    } else if let Some(rest) = name.strip_prefix("style.") {
        // Check for unit suffix: style.width.px
        if let Some(dot_pos) = rest.find('.') {
            let prop = &rest[..dot_pos];
            let unit = &rest[dot_pos + 1..];
            (BindingType::Style, prop, Some(unit))
        } else {
            (BindingType::Style, rest, None)
        }
    } else if let Some(rest) = name.strip_prefix("attr.") {
        (BindingType::Attribute, rest, None)
    } else if name.starts_with('@') {
        // Animation binding like @triggerName
        (BindingType::Animation, name, None)
    } else {
        (BindingType::Property, name, None)
    }
}

/// Classify a host event name as a legacy animation event.
///
/// Mirrors Angular's `parseLegacyAnimationEventName` (binding_parser.ts) +
/// `splitAtPeriod` (util.ts): `@` is stripped, the name is split on the first `.`,
/// **both halves are trimmed**, and the **phase is lowercased** via `.toLowerCase()`.
///
/// - `@trigger.phase`  → (`"trigger"`, `LegacyAnimation`, `Some("phase")`)
/// - `@anim.START`     → (`"anim"`,    `LegacyAnimation`, `Some("start")`)
/// - `@anim. start `   → (`"anim"`,    `LegacyAnimation`, `Some("start")`)
/// - `@anim.foo`       → (`"anim"`,    `LegacyAnimation`, `Some("foo")`)  Angular reports
///                       an error for invalid phases; we drop the diagnostic to match
///                       this codebase's convention for host metadata (see
///                       `binding_parser.parse_event` callers below — `parse_result.errors`
///                       is also discarded). Code output still matches Angular byte-for-byte.
/// - `@trigger`        → (`"trigger"`, `LegacyAnimation`, `None`)
/// - `click`           → (`"click"`,   `Regular`,         `None`)
///
/// Keep in sync with the identical helper in `directive/compiler.rs`.
fn parse_legacy_animation_event<'a>(
    event_name: &'a str,
    allocator: &'a Allocator,
) -> (&'a str, ParsedEventType, Option<Ident<'a>>) {
    use oxc_allocator::FromIn;
    let Some(without_at) = event_name.strip_prefix('@') else {
        return (event_name, ParsedEventType::Regular, None);
    };
    let (trigger_raw, phase_raw) = match without_at.find('.') {
        Some(dot) => (&without_at[..dot], Some(&without_at[dot + 1..])),
        None => (without_at, None),
    };
    let trigger_trimmed = trigger_raw.trim();
    let trigger: &'a str = if trigger_trimmed.len() == trigger_raw.len() {
        trigger_trimmed
    } else {
        allocator.alloc_str(trigger_trimmed)
    };
    let phase = phase_raw.map(|p| {
        let normalized = p.trim().to_lowercase();
        Ident::from_in(normalized.as_str(), allocator)
    });
    (trigger, ParsedEventType::LegacyAnimation, phase)
}

/// Parse event name to extract target (window:, document:, body:).
///
/// Examples:
/// - "click" -> ("click", None)
/// - "window:resize" -> ("resize", Some("window"))
/// - "document:keydown" -> ("keydown", Some("document"))
fn parse_event_target(event_name: &str) -> (&str, Option<&str>) {
    if let Some(rest) = event_name.strip_prefix("window:") {
        (rest, Some("window"))
    } else if let Some(rest) = event_name.strip_prefix("document:") {
        (rest, Some("document"))
    } else if let Some(rest) = event_name.strip_prefix("body:") {
        (rest, Some("body"))
    } else {
        (event_name, None)
    }
}

/// Convert HostMetadataInput (owned strings) to HostMetadata<'a> (with Atom types).
///
/// This function is used when compiling templates in isolation (e.g., for the compare tool)
/// where the host metadata comes from NAPI options rather than from parsing a decorator.
fn convert_host_metadata_input_to_host_metadata<'a>(
    allocator: &'a Allocator,
    input: &HostMetadataInput,
) -> HostMetadata<'a> {
    use oxc_allocator::FromIn;

    let mut properties: OxcVec<'a, (Ident<'a>, Ident<'a>)> = OxcVec::new_in(allocator);
    for (k, v) in &input.properties {
        properties
            .push((Ident::from_in(k.as_str(), allocator), Ident::from_in(v.as_str(), allocator)));
    }

    let mut attributes: OxcVec<'a, (Ident<'a>, Ident<'a>)> = OxcVec::new_in(allocator);
    for (k, v) in &input.attributes {
        attributes
            .push((Ident::from_in(k.as_str(), allocator), Ident::from_in(v.as_str(), allocator)));
    }

    let mut listeners: OxcVec<'a, (Ident<'a>, Ident<'a>)> = OxcVec::new_in(allocator);
    for (k, v) in &input.listeners {
        listeners
            .push((Ident::from_in(k.as_str(), allocator), Ident::from_in(v.as_str(), allocator)));
    }

    HostMetadata {
        properties,
        attributes,
        listeners,
        class_attr: input.class_attr.as_ref().map(|s| Ident::from_in(s.as_str(), allocator)),
        style_attr: input.style_attr.as_ref().map(|s| Ident::from_in(s.as_str(), allocator)),
    }
}

/// Pool selector attrs constant BEFORE template compilation.
///
/// This matches TypeScript Angular's behavior where the attrs constant is added to
/// the constant pool BEFORE template ingestion and compilation. By pooling it first,
/// attrs gets the first available constant index (_c0), ensuring the correct constant
/// ordering in the output.
///
/// See: packages/compiler/src/render3/view/compiler.ts lines 192-212
fn pool_selector_attrs<'a>(
    allocator: &'a Allocator,
    job: &mut crate::pipeline::compilation::ComponentCompilationJob<'a>,
    metadata: &super::metadata::ComponentMetadata<'a>,
) -> Option<crate::output::ast::OutputExpression<'a>> {
    use crate::output::ast::{LiteralArrayExpr, LiteralExpr, LiteralValue, OutputExpression};
    use crate::pipeline::selector::CssSelector;
    use oxc_allocator::FromIn;

    let selector = metadata.selector.as_ref()?;
    let parsed_selectors = CssSelector::parse(selector);
    let first_selector = parsed_selectors.first()?;
    let selector_attrs = first_selector.get_attrs();

    if selector_attrs.is_empty() {
        return None;
    }

    // Build the attrs array: ["attrName", "attrValue", ...]
    let mut attr_entries: OxcVec<'a, OutputExpression<'a>> =
        OxcVec::with_capacity_in(selector_attrs.len(), allocator);
    for attr in selector_attrs {
        attr_entries.push(OutputExpression::Literal(oxc_allocator::Box::new_in(
            LiteralExpr {
                value: LiteralValue::String(Ident::from_in(attr.as_str(), allocator)),
                source_span: None,
            },
            allocator,
        )));
    }

    // Create the attrs array literal
    let attrs_array = OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
        LiteralArrayExpr { entries: attr_entries, source_span: None },
        allocator,
    ));

    // Pool the attrs array using constantPool.getConstLiteral() with forceShared=true
    // This extracts it to a const like: const _c0 = ["bitBadge", ""];
    Some(job.pool.get_const_literal(attrs_array, true))
}

/// Compile host bindings from HostMetadataInput (owned strings).
///
/// This is used by `compile_template_to_js_with_options` when host metadata is provided
/// via TransformOptions for isolated template compilation.
fn compile_host_bindings_from_input<'a>(
    allocator: &'a Allocator,
    host_input: &HostMetadataInput,
    component_name: &str,
    selector: Option<&str>,
    pool_starting_index: u32,
    angular_version: Option<crate::AngularVersion>,
    legacy_optional_chaining: Option<bool>,
) -> Option<HostBindingCompilationResult<'a>> {
    use oxc_allocator::FromIn;

    // Check if there are any host bindings at all
    // Include class_attr and style_attr as they become static attributes
    if host_input.properties.is_empty()
        && host_input.attributes.is_empty()
        && host_input.listeners.is_empty()
        && host_input.class_attr.is_none()
        && host_input.style_attr.is_none()
    {
        return None;
    }

    // Convert to HostMetadata
    let host = convert_host_metadata_input_to_host_metadata(allocator, host_input);

    // Get component name and selector as atoms
    let component_name_atom = Ident::from_in(component_name, allocator);
    let component_selector =
        selector.map(|s| Ident::from_in(s, allocator)).unwrap_or_else(|| Ident::from(""));

    // Convert to HostBindingInput and compile
    let input =
        convert_host_metadata_to_input(allocator, &host, component_name_atom, component_selector);
    let mut job = ingest_host_binding_with_version(
        allocator,
        input,
        pool_starting_index,
        angular_version,
        legacy_optional_chaining,
    );
    let result = compile_host_bindings(&mut job);

    Some(result)
}

/// Result of compiling host bindings for the linker.
pub struct LinkerHostBindingOutput {
    /// The host binding function as JS.
    pub fn_js: String,
    /// Number of host variables.
    pub host_vars: u32,
    /// Pool constant declarations (pure functions, etc.) as JS.
    pub declarations_js: String,
}

/// Compile host bindings for the linker, returning the emitted JS function + hostVars count.
///
/// This takes host property/listener data extracted from a partial declaration and compiles
/// it through the full Angular expression parser and host binding pipeline, producing
/// correctly compiled output (unlike raw string interpolation which would fail for complex
/// Angular template expressions).
pub fn compile_host_bindings_for_linker(
    host_input: &HostMetadataInput,
    component_name: &str,
    selector: Option<&str>,
    pool_starting_index: u32,
) -> Option<LinkerHostBindingOutput> {
    let allocator = Allocator::default();
    let result = compile_host_bindings_from_input(
        &allocator,
        host_input,
        component_name,
        selector,
        pool_starting_index,
        None, // Linker always targets latest Angular version
        None, // legacyOptionalChaining: derive from (absent) version
    )?;

    let emitter = JsEmitter::new();

    let host_vars = result.host_vars.unwrap_or(0);

    // Emit host binding pool declarations (pure functions, etc.)
    let mut declarations_js = String::new();
    for decl in result.declarations.iter() {
        declarations_js.push_str(&emitter.emit_statement(decl));
        declarations_js.push('\n');
    }

    let fn_js = result.host_binding_fn.map(|f| {
        let expr = OutputExpression::Function(oxc_allocator::Box::new_in(f, &allocator));
        emitter.emit_expression(&expr)
    })?;

    Some(LinkerHostBindingOutput { fn_js, host_vars, declarations_js })
}

/// Output from compiling a template for the linker.
///
/// Used by the partial declaration linker to generate `ɵɵdefineComponent` calls
/// from `ɵɵngDeclareComponent` partial declarations.
#[derive(Debug)]
pub struct LinkerTemplateOutput {
    /// All declarations (child view functions, pooled constants, main template function)
    /// as JavaScript code. These need to be emitted before the `defineComponent` call.
    pub declarations_js: String,

    /// The name of the main template function (e.g., "ComponentName_Template").
    pub template_fn_name: String,

    /// Number of element/text/container declarations in the root view.
    pub decls: u32,

    /// Number of variable binding slots in the root view.
    pub vars: u32,

    /// The consts array as a JavaScript expression string, if any.
    pub consts_js: Option<String>,

    /// The ngContentSelectors array as a JavaScript expression string, if any.
    pub ng_content_selectors_js: Option<String>,

    /// The next available pool index after template compilation.
    /// Used to continue constant numbering in host binding compilation.
    pub next_pool_index: u32,
}

/// Compile a template for the linker, returning all data needed to build a `defineComponent` call.
///
/// This is similar to `compile_template_to_js_with_options` but returns a richer result
/// that includes numeric metadata (decls, vars) and the consts/ngContentSelectors as strings,
/// which the linker needs to assemble the `defineComponent({...})` replacement.
pub fn compile_template_for_linker<'a>(
    allocator: &'a Allocator,
    template: &'a str,
    component_name: &str,
    file_path: &str,
    preserve_whitespaces: bool,
) -> Result<LinkerTemplateOutput, std::vec::Vec<OxcDiagnostic>> {
    use crate::pipeline::ingest::{IngestOptions, ingest_component_with_options};
    use oxc_allocator::FromIn;

    let mut diagnostics = std::vec::Vec::new();

    // Stage 1: Parse HTML
    let parse_options = ParseTemplateOptions {
        preserve_whitespaces,
        enable_block_syntax: true,
        enable_let_syntax: true,
        tokenize_expansion_forms: true,
        ..Default::default()
    };
    let parser = HtmlParser::with_options(allocator, template, file_path, &parse_options);
    let html_result = parser.parse();

    if !html_result.errors.is_empty() {
        for error in &html_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 1.5: Remove whitespace if not preserving
    let nodes = if parse_options.preserve_whitespaces {
        &html_result.nodes
    } else {
        let processed = remove_whitespaces(allocator, &html_result.nodes, true);
        allocator.alloc(processed) as &_
    };

    // Stage 2: Transform HTML to R3 AST
    let r3_transform_options =
        R3TransformOptions { collect_comment_nodes: parse_options.collect_comment_nodes };
    let transformer = HtmlToR3Transform::new(allocator, template, r3_transform_options);
    let r3_result = transformer.transform(nodes);

    if !r3_result.errors.is_empty() {
        for error in &r3_result.errors {
            diagnostics.push(OxcDiagnostic::error(error.msg.clone()));
        }
        return Err(diagnostics);
    }

    // Stage 3-5: Ingest and compile
    let ingest_options = IngestOptions {
        mode: TemplateCompilationMode::Full,
        relative_context_file_path: None,
        i18n_use_external_ids: true,
        defer_block_deps_emit_mode: DeferBlockDepsEmitMode::PerBlock,
        relative_template_path: None,
        enable_debug_locations: false,
        template_source: Some(template),
        all_deferrable_deps_fn: None,
        pool_starting_index: 0,
        angular_version: None,
        legacy_optional_chaining: None,
    };

    let component_name_atom = Ident::from_in(component_name, allocator);
    let mut job = ingest_component_with_options(
        allocator,
        component_name_atom,
        r3_result.nodes,
        ingest_options,
    );

    let compiled = compile_template(&mut job);

    // Collect diagnostics
    diagnostics.extend(job.diagnostics.into_iter());

    // Extract numeric metadata from the compilation job
    let decls = job.root.decl_count.unwrap_or(0);
    let vars = job.root.vars.unwrap_or(0);

    let emitter = JsEmitter::new();

    // Emit consts array as JS expression
    let consts_js = if !job.consts.is_empty() {
        let mut const_entries: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);
        for const_value in &job.consts {
            const_entries.push(const_value_to_expression(allocator, const_value));
        }

        let consts_expr = if !job.consts_initializers.is_empty() {
            // Wrap in function with initializers
            let mut fn_stmts: OxcVec<'a, OutputStatement<'a>> =
                OxcVec::with_capacity_in(job.consts_initializers.len() + 1, allocator);
            for stmt in job.consts_initializers.drain(..) {
                fn_stmts.push(stmt);
            }
            fn_stmts.push(OutputStatement::Return(oxc_allocator::Box::new_in(
                crate::output::ast::ReturnStatement {
                    value: OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
                        crate::output::ast::LiteralArrayExpr {
                            entries: const_entries,
                            source_span: None,
                        },
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            )));
            OutputExpression::Function(oxc_allocator::Box::new_in(
                FunctionExpr {
                    name: None,
                    params: OxcVec::new_in(allocator),
                    statements: fn_stmts,
                    source_span: None,
                },
                allocator,
            ))
        } else {
            OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
                crate::output::ast::LiteralArrayExpr { entries: const_entries, source_span: None },
                allocator,
            ))
        };
        Some(emitter.emit_expression(&consts_expr))
    } else {
        None
    };

    // Emit ngContentSelectors as JS expression
    let ng_content_selectors_js =
        job.content_selectors.take().map(|expr| emitter.emit_expression(&expr));

    // Get template function name
    let template_fn_name = compiled
        .template_fn
        .name
        .as_ref()
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("{component_name}_Template"));

    // Emit all declarations + main template function as JS code
    let mut all_statements: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);

    for decl in compiled.declarations {
        all_statements.push(decl);
    }

    if let Some(fn_name) = compiled.template_fn.name.clone() {
        let main_fn_stmt = OutputStatement::DeclareFunction(oxc_allocator::Box::new_in(
            DeclareFunctionStmt {
                name: fn_name,
                params: compiled.template_fn.params,
                statements: compiled.template_fn.statements,
                modifiers: StmtModifier::NONE,
                source_span: compiled.template_fn.source_span,
            },
            allocator,
        ));
        all_statements.push(main_fn_stmt);
    }

    let declarations_js = emitter.emit_statements(&all_statements);
    let next_pool_index = job.pool.next_name_index();

    Ok(LinkerTemplateOutput {
        declarations_js,
        template_fn_name,
        decls,
        vars,
        consts_js,
        ng_content_selectors_js,
        next_pool_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    #[test]
    fn test_transform_angular_file_with_component() {
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-hello',
    template: '<h1>Hello {{name}}</h1>',
    standalone: true
})
export class HelloComponent {
    name = 'World';
}
"#;

        let result = transform_angular_file(&allocator, "hello.component.ts", source, None, None);

        assert_eq!(result.component_count, 1);
        assert!(!result.has_errors());

        // Check that ɵfac and ɵcmp are injected as external property assignments
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain .ɵfac = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵcmp ="),
            "Code should contain .ɵcmp =, but got:\n{}",
            result.code
        );
        assert!(result.code.contains("ɵɵdefineComponent"), "Code should contain ɵɵdefineComponent");
    }

    #[test]
    fn test_output_format_matches_angular_convention() {
        // Verify the output format uses static class fields inside the class:
        // 1. Constants/child view functions BEFORE the class
        // 2. Class definition with static ɵfac and ɵcmp fields inside
        // 3. ɵfac comes BEFORE ɵcmp (Angular convention)
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '<div>Test</div>'
})
export class TestComponent {}
"#;

        let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

        assert!(!result.has_errors());

        // Verify static fields are inside the class using static class field syntax
        assert!(
            result.code.contains("static ɵfac ="),
            "Should use static class field syntax for ɵfac, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵcmp ="),
            "Should use static class field syntax for ɵcmp, but got:\n{}",
            result.code
        );

        // Verify ɵfac comes before ɵcmp (Angular convention)
        let fac_pos = result.code.find("static ɵfac =").expect("ɵfac should exist");
        let cmp_pos = result.code.find("static ɵcmp =").expect("ɵcmp should exist");
        assert!(
            fac_pos < cmp_pos,
            "ɵfac should come BEFORE ɵcmp (Angular convention). ɵfac at {}, ɵcmp at {}",
            fac_pos,
            cmp_pos
        );

        // Verify it's NOT using external property assignment syntax
        assert!(
            !result.code.contains("TestComponent.ɵfac ="),
            "Should NOT use external property assignment syntax for ɵfac"
        );
        assert!(
            !result.code.contains("TestComponent.ɵcmp ="),
            "Should NOT use external property assignment syntax for ɵcmp"
        );
    }

    #[test]
    fn test_external_declarations_before_class() {
        // Verify that child view functions and constants are placed BEFORE the class
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-test',
    template: '@for (item of items; track item) { <div>{{item}}</div> }'
})
export class TestComponent {
    items = ['a', 'b'];
}
"#;

        let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

        assert!(!result.has_errors());

        // The @for block should generate a child view function
        let class_pos = result.code.find("class TestComponent").expect("class should exist");

        // Check if there's a child view function (will be named TestComponent_For_N_Template)
        if let Some(child_fn_pos) = result.code.find("function TestComponent_For") {
            assert!(
                child_fn_pos < class_pos,
                "Child view function should come BEFORE the class. Function at {}, class at {}",
                child_fn_pos,
                class_pos
            );
        }
    }

    #[test]
    fn test_transform_file_without_component() {
        let allocator = Allocator::default();
        let source = r#"
export class RegularClass {
    value = 42;
}
"#;

        let result = transform_angular_file(&allocator, "regular.ts", source, None, None);

        assert_eq!(result.component_count, 0);
        assert!(!result.has_errors());
        // Should not have any Angular definitions
        assert!(!result.code.contains("ɵcmp"));
        assert!(!result.code.contains("ɵfac"));
    }

    #[test]
    fn test_transform_with_multiple_components() {
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-first',
    template: '<div>First</div>'
})
export class FirstComponent {}

@Component({
    selector: 'app-second',
    template: '<div>Second</div>'
})
export class SecondComponent {}
"#;

        let result = transform_angular_file(&allocator, "multi.component.ts", source, None, None);

        assert_eq!(result.component_count, 2);
        assert!(!result.has_errors());
        // With multiple components, each should have ɵcmp property assignment after its class
        // Check that both classes have ɵcmp definitions
        assert!(
            result.code.contains("class FirstComponent") && result.code.contains("static ɵcmp ="),
            "Code should contain FirstComponent with ɵcmp property assignment"
        );
        assert!(
            result.code.contains("class SecondComponent"),
            "Code should contain SecondComponent"
        );
    }

    #[test]
    fn test_transform_with_host_bindings() {
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-button',
    template: '<ng-content></ng-content>',
    host: {
        '[class.active]': 'isActive',
        '[disabled]': 'isDisabled',
        '(click)': 'onClick()',
        'role': 'button'
    }
})
export class ButtonComponent {
    isActive = false;
    isDisabled = false;
    onClick() {}
}
"#;

        let result = transform_angular_file(&allocator, "button.component.ts", source, None, None);

        assert_eq!(result.component_count, 1);
        assert!(!result.has_errors());
        assert!(
            result.code.contains("static ɵcmp ="),
            "Code should contain .ɵcmp = , but got:\n{}",
            result.code
        );
        // Verify hostBindings is generated (for [class.active], [disabled], (click))
        assert!(result.code.contains("hostBindings"), "Code should contain hostBindings property");
        // Verify hostAttrs is generated for the static 'role' attribute
        assert!(
            result.code.contains("hostAttrs"),
            "Code should contain hostAttrs property for static host attributes, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_transform_with_static_host_attributes_only() {
        // Test that static host attributes (without bindings) generate hostAttrs
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'bit-error-summary',
    template: '<ng-content></ng-content>',
    host: {
        'aria-live': 'assertive',
        'class': 'tw-block tw-text-danger tw-mt-2'
    }
})
export class BitErrorSummaryComponent {}
"#;

        let result =
            transform_angular_file(&allocator, "error-summary.component.ts", source, None, None);

        assert_eq!(result.component_count, 1);
        assert!(!result.has_errors());
        // Verify hostAttrs is generated for static attributes
        assert!(
            result.code.contains("hostAttrs"),
            "Code should contain hostAttrs property for static host attributes, but got:\n{}",
            result.code
        );
        // Verify the specific attributes are in the output
        assert!(
            result.code.contains("aria-live") || result.code.contains("\"aria-live\""),
            "Code should contain aria-live attribute, but got:\n{}",
            result.code
        );
        // Verify class values are properly split with AttributeMarker.Classes (value 1)
        // Angular expects: hostAttrs: ["aria-live", "assertive", 1, "tw-block", "tw-text-danger", "tw-mt-2"]
        assert!(
            result.code.contains("tw-block") || result.code.contains("\"tw-block\""),
            "Code should contain class name 'tw-block', but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_host_class_attr_generates_classes_marker() {
        // Verify that host: { class: '...' } generates individual class names
        // with the AttributeMarker.Classes marker (value 1)
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';

@Component({
    selector: 'app-root',
    template: '<div>Hello</div>',
    host: { class: 'tw-block tw-text-danger' }
})
export class AppComponent {}
"#;

        let result = transform_angular_file(&allocator, "app.component.ts", source, None, None);

        assert_eq!(result.component_count, 1);
        assert!(!result.has_errors());

        // Verify hostAttrs contains the Classes marker (1) followed by individual class names
        // Expected format: hostAttrs: [1, "tw-block", "tw-text-danger"]
        assert!(
            result.code.contains("hostAttrs"),
            "Code should contain hostAttrs, but got:\n{}",
            result.code
        );
        // Check for class names in the output
        assert!(
            result.code.contains("tw-block"),
            "Code should contain class name 'tw-block', but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("tw-text-danger"),
            "Code should contain class name 'tw-text-danger', but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_directive_is_compiled() {
        // This tests the fix for the runtime error:
        // "TypeError: Cannot set property ɵfac of class NavBaseComponent which has only a getter"
        //
        // The issue was that @Directive decorators were only being removed but not compiled,
        // so Angular's JIT runtime would process them and create getter-only ɵfac properties
        // that conflicted with OXC's AOT-compiled ɵfac assignments.
        //
        // The fix is to compile directives properly, generating ɵdir and ɵfac definitions.
        let allocator = Allocator::default();
        let source = r#"
import { Directive } from '@angular/core';

@Directive({
    selector: '[myDirective]'
})
export class MyDirective {}
"#;

        let result = transform_angular_file(&allocator, "my.directive.ts", source, None, None);

        // The @Directive decorator should be removed from the output
        assert!(
            !result.code.contains("@Directive"),
            "Code should NOT contain @Directive decorator, but got:\n{}",
            result.code
        );
        // The class itself should still be present
        assert!(
            result.code.contains("class MyDirective"),
            "Code should still contain the class declaration"
        );
        // ɵdir and ɵfac should be generated
        assert!(
            result.code.contains("static ɵdir = "),
            "Code should contain .ɵdir = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain .ɵfac = , but got:\n{}",
            result.code
        );
        assert!(result.code.contains("ɵɵdefineDirective"), "Code should contain ɵɵdefineDirective");
    }

    #[test]
    fn test_directive_and_component_both_compiled() {
        // Test that both @Component and @Directive classes are properly compiled
        // when they exist in the same file (common inheritance pattern).
        let allocator = Allocator::default();
        let source = r#"
import { Component, Directive } from '@angular/core';

@Directive({
    selector: '[baseNav]'
})
export class NavBaseDirective {}

@Component({
    selector: 'app-nav',
    template: '<div>Nav</div>'
})
export class NavComponent extends NavBaseDirective {}
"#;

        let result = transform_angular_file(&allocator, "nav.component.ts", source, None, None);

        // Both decorators should be removed
        assert!(
            !result.code.contains("@Directive"),
            "Code should NOT contain @Directive decorator"
        );
        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator"
        );
        // Both classes should still be present
        assert!(result.code.contains("class NavBaseDirective"));
        assert!(result.code.contains("class NavComponent"));
        // Component should have ɵcmp and ɵfac
        assert!(result.code.contains("static ɵcmp ="));
        assert!(result.code.contains("static ɵfac = "));
        // Directive should have ɵdir and ɵfac
        assert!(
            result.code.contains("static ɵdir = "),
            "Code should contain .ɵdir = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain .ɵfac = , but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_abstract_directive_with_empty_parens() {
        // Test @Directive() with empty parentheses (no config object).
        // This is a common pattern for abstract base directive classes.
        // This was the exact case causing the Bitwarden runtime error.
        let allocator = Allocator::default();
        let source = r#"
import { Directive, Output, EventEmitter } from '@angular/core';

@Directive()
export abstract class NavBaseComponent {
    @Output() mainContentClicked: EventEmitter<MouseEvent> = new EventEmitter();
}
"#;

        let result =
            transform_angular_file(&allocator, "nav-base.component.ts", source, None, None);

        // The @Directive() decorator should be removed
        assert!(
            !result.code.contains("@Directive"),
            "Code should NOT contain @Directive decorator, but got:\n{}",
            result.code
        );
        // ɵdir and ɵfac should be generated
        assert!(
            result.code.contains("static ɵdir = "),
            "Code should contain .ɵdir = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain .ɵfac = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineDirective call
        assert!(result.code.contains("ɵɵdefineDirective"), "Code should contain ɵɵdefineDirective");
        // Should extract @Output from class members
        assert!(
            result.code.contains("mainContentClicked"),
            "Code should contain mainContentClicked output"
        );
    }

    #[test]
    fn test_injectable_is_compiled() {
        // Test that @Injectable decorated classes are properly compiled
        // with ɵprov definitions generated.
        let allocator = Allocator::default();
        let source = r#"
import { Injectable } from '@angular/core';

@Injectable({
    providedIn: 'root'
})
export class MyService {
    getValue() { return 42; }
}
"#;

        let result = transform_angular_file(&allocator, "my.service.ts", source, None, None);

        // The @Injectable decorator should be removed from the output
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        // The class itself should still be present
        assert!(
            result.code.contains("class MyService"),
            "Code should still contain the class declaration"
        );
        // ɵprov should be generated
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineInjectable call
        assert!(
            result.code.contains("ɵɵdefineInjectable"),
            "Code should contain ɵɵdefineInjectable"
        );
        // Should have providedIn: 'root'
        assert!(result.code.contains("root"), "Code should contain providedIn: 'root'");
    }

    #[test]
    fn test_injectable_with_empty_parens() {
        // Test @Injectable() with empty parentheses - should default to providedIn: 'root'.
        let allocator = Allocator::default();
        let source = r#"
import { Injectable } from '@angular/core';

@Injectable()
export class LocalService {}
"#;

        let result = transform_angular_file(&allocator, "local.service.ts", source, None, None);

        // The @Injectable() decorator should be removed
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        // ɵprov should be generated
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineInjectable call
        assert!(
            result.code.contains("ɵɵdefineInjectable"),
            "Code should contain ɵɵdefineInjectable"
        );
        // Should NOT have providedIn when not explicitly specified
        assert!(
            !result.code.contains("providedIn"),
            "Code should NOT contain providedIn when not specified, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_injectable_with_use_factory() {
        // Test @Injectable with useFactory provider.
        let allocator = Allocator::default();
        let source = r#"
import { Injectable } from '@angular/core';

@Injectable({
    providedIn: 'root',
    useFactory: () => new MyService('custom')
})
export class MyService {
    constructor(private value: string) {}
}
"#;

        let result = transform_angular_file(&allocator, "factory.service.ts", source, None, None);

        // The @Injectable decorator should be removed
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        // ɵprov should be generated
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineInjectable call
        assert!(
            result.code.contains("ɵɵdefineInjectable"),
            "Code should contain ɵɵdefineInjectable"
        );
    }

    #[test]
    fn test_injectable_with_use_value() {
        // Test @Injectable with useValue provider.
        let allocator = Allocator::default();
        let source = r#"
import { Injectable } from '@angular/core';

@Injectable({
    providedIn: 'root',
    useValue: { apiUrl: 'https://api.example.com' }
})
export class Config {}
"#;

        let result = transform_angular_file(&allocator, "config.service.ts", source, None, None);

        // The @Injectable decorator should be removed
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        // ɵprov should be generated
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_injectable_and_component_in_same_file() {
        // Test that both @Injectable and @Component are properly compiled
        // when they exist in the same file.
        let allocator = Allocator::default();
        let source = r#"
import { Injectable, Component } from '@angular/core';

@Injectable({
    providedIn: 'root'
})
export class DataService {
    getData() { return [1, 2, 3]; }
}

@Component({
    selector: 'app-data',
    template: '<div>Data</div>'
})
export class DataComponent {
    constructor(private dataService: DataService) {}
}
"#;

        let result = transform_angular_file(&allocator, "data.component.ts", source, None, None);

        // Both decorators should be removed
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator"
        );
        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator"
        );
        // Injectable should have ɵprov
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        // Component should have ɵcmp and ɵfac
        assert!(result.code.contains("static ɵcmp ="));
        assert!(result.code.contains("static ɵfac = "));
        // Should have both ɵɵdefineInjectable and ɵɵdefineComponent
        assert!(result.code.contains("ɵɵdefineInjectable"));
        assert!(result.code.contains("ɵɵdefineComponent"));
    }

    #[test]
    fn test_injectable_with_platform_provided_in() {
        // Test @Injectable with providedIn: 'platform'.
        let allocator = Allocator::default();
        let source = r#"
import { Injectable } from '@angular/core';

@Injectable({
    providedIn: 'platform'
})
export class PlatformService {}
"#;

        let result = transform_angular_file(&allocator, "platform.service.ts", source, None, None);

        // ɵprov should be generated
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        // Should have providedIn: 'platform'
        assert!(result.code.contains("platform"), "Code should contain providedIn: 'platform'");
    }

    #[test]
    fn test_pipe_is_compiled() {
        // Test that @Pipe decorated classes are properly compiled
        // with ɵpipe definitions generated.
        let allocator = Allocator::default();
        let source = r#"
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({
    name: 'uppercase'
})
export class UppercasePipe implements PipeTransform {
    transform(value: string): string {
        return value.toUpperCase();
    }
}
"#;

        let result = transform_angular_file(&allocator, "uppercase.pipe.ts", source, None, None);

        // The @Pipe decorator should be removed from the output
        assert!(
            !result.code.contains("@Pipe"),
            "Code should NOT contain @Pipe decorator, but got:\n{}",
            result.code
        );
        // The class itself should still be present
        assert!(
            result.code.contains("class UppercasePipe"),
            "Code should still contain the class declaration"
        );
        // ɵpipe should be generated
        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        // ɵfac should be generated for dependency injection
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain static {{ this.ɵfac = for DI, but got:\n{}",
            result.code
        );
        // Should have ɵɵdefinePipe call
        assert!(result.code.contains("ɵɵdefinePipe"), "Code should contain ɵɵdefinePipe");
        // Should contain the pipe name
        assert!(result.code.contains("uppercase"), "Code should contain pipe name 'uppercase'");
    }

    #[test]
    fn test_pipe_pure_true() {
        // Test @Pipe with pure: true (default behavior).
        let allocator = Allocator::default();
        let source = r#"
import { Pipe } from '@angular/core';

@Pipe({
    name: 'myPure',
    pure: true
})
export class MyPurePipe {}
"#;

        let result = transform_angular_file(&allocator, "my-pure.pipe.ts", source, None, None);

        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        // pure: true should be in output
        assert!(result.code.contains("pure"), "Code should contain 'pure' property");
    }

    #[test]
    fn test_pipe_pure_false() {
        // Test @Pipe with pure: false (impure pipe).
        let allocator = Allocator::default();
        let source = r#"
import { Pipe } from '@angular/core';

@Pipe({
    name: 'impure',
    pure: false
})
export class ImpurePipe {}
"#;

        let result = transform_angular_file(&allocator, "impure.pipe.ts", source, None, None);

        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        // pure: false should be in output
        assert!(
            result.code.contains("false"),
            "Code should contain 'false' for pure: false, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_pipe_standalone_false() {
        // Test @Pipe with standalone: false (non-standalone pipe for v18 compatibility).
        let allocator = Allocator::default();
        let source = r#"
import { Pipe } from '@angular/core';

@Pipe({
    name: 'legacy',
    standalone: false
})
export class LegacyPipe {}
"#;

        let result = transform_angular_file(&allocator, "legacy.pipe.ts", source, None, None);

        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        // standalone: false should be in output
        assert!(
            result.code.contains("standalone"),
            "Code should contain 'standalone' property when false, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_pipe_and_component_in_same_file() {
        // Test that both @Pipe and @Component are properly compiled
        // when they exist in the same file.
        let allocator = Allocator::default();
        let source = r#"
import { Pipe, Component } from '@angular/core';

@Pipe({
    name: 'format'
})
export class FormatPipe {}

@Component({
    selector: 'app-data',
    template: '<div>{{ value | format }}</div>'
})
export class DataComponent {
    value = 'test';
}
"#;

        let result = transform_angular_file(&allocator, "data.component.ts", source, None, None);

        // Both decorators should be removed
        assert!(!result.code.contains("@Pipe"), "Code should NOT contain @Pipe decorator");
        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator"
        );
        // Pipe should have ɵpipe and ɵfac
        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain static {{ this.ɵfac = for DI, but got:\n{}",
            result.code
        );
        // Component should have ɵcmp and ɵfac
        assert!(result.code.contains("static ɵcmp ="));
        assert!(result.code.contains("static ɵfac = "));
        // Should have both ɵɵdefinePipe and ɵɵdefineComponent
        assert!(result.code.contains("ɵɵdefinePipe"));
        assert!(result.code.contains("ɵɵdefineComponent"));
    }

    #[test]
    fn test_all_decorators_in_same_file() {
        // Test that @Component, @Directive, @Injectable, and @Pipe are all
        // properly compiled when they exist in the same file.
        let allocator = Allocator::default();
        let source = r#"
import { Component, Directive, Injectable, Pipe } from '@angular/core';

@Pipe({
    name: 'myPipe'
})
export class MyPipe {}

@Injectable({
    providedIn: 'root'
})
export class MyService {}

@Directive({
    selector: '[myDirective]'
})
export class MyDirective {}

@Component({
    selector: 'app-root',
    template: '<div myDirective>{{ value | myPipe }}</div>'
})
export class AppComponent {
    value = 'test';
}
"#;

        let result = transform_angular_file(&allocator, "app.component.ts", source, None, None);

        // All decorators should be removed
        assert!(!result.code.contains("@Pipe"), "Code should NOT contain @Pipe decorator");
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator"
        );
        assert!(
            !result.code.contains("@Directive"),
            "Code should NOT contain @Directive decorator"
        );
        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator"
        );

        // All definitions should be generated
        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵdir = "),
            "Code should contain .ɵdir = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain .ɵfac = , but got:\n{}",
            result.code
        );
        assert!(result.code.contains("static ɵcmp ="));
        assert!(result.code.contains("static ɵfac = "));

        // Should have all define functions
        assert!(result.code.contains("ɵɵdefinePipe"));
        assert!(result.code.contains("ɵɵdefineInjectable"));
        assert!(result.code.contains("ɵɵdefineDirective"));
        assert!(result.code.contains("ɵɵdefineComponent"));
    }

    #[test]
    fn test_ng_module_is_compiled() {
        // Test that @NgModule decorated classes are properly compiled
        // with ɵmod definitions generated.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule } from '@angular/core';

@NgModule({
    declarations: [AppComponent],
    imports: [CommonModule],
    exports: [AppComponent],
    bootstrap: [AppComponent]
})
export class AppModule {}
"#;

        let result = transform_angular_file(&allocator, "app.module.ts", source, None, None);

        // The @NgModule decorator should be removed from the output
        assert!(
            !result.code.contains("@NgModule"),
            "Code should NOT contain @NgModule decorator, but got:\n{}",
            result.code
        );
        // The class itself should still be present
        assert!(
            result.code.contains("class AppModule"),
            "Code should still contain the class declaration"
        );
        // ɵmod should be generated
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineNgModule call
        assert!(result.code.contains("ɵɵdefineNgModule"), "Code should contain ɵɵdefineNgModule");
        // Should have declarations, imports, exports, bootstrap
        assert!(result.code.contains("declarations"), "Code should contain declarations");
        assert!(result.code.contains("imports"), "Code should contain imports");
        assert!(result.code.contains("exports"), "Code should contain exports");
        assert!(result.code.contains("bootstrap"), "Code should contain bootstrap");
    }

    #[test]
    fn test_ng_module_empty_decorator() {
        // Test @NgModule({}) with empty config object.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule } from '@angular/core';

@NgModule({})
export class EmptyModule {}
"#;

        let result = transform_angular_file(&allocator, "empty.module.ts", source, None, None);

        // The @NgModule() decorator should be removed
        assert!(
            !result.code.contains("@NgModule"),
            "Code should NOT contain @NgModule decorator, but got:\n{}",
            result.code
        );
        // ɵmod should be generated
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineNgModule call
        assert!(result.code.contains("ɵɵdefineNgModule"), "Code should contain ɵɵdefineNgModule");
    }

    #[test]
    fn test_ng_module_with_schemas() {
        // Test @NgModule with schemas for custom elements.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@NgModule({
    schemas: [CUSTOM_ELEMENTS_SCHEMA]
})
export class CustomElementsModule {}
"#;

        let result =
            transform_angular_file(&allocator, "custom-elements.module.ts", source, None, None);

        // ɵmod should be generated with schemas
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );
        assert!(result.code.contains("schemas"), "Code should contain schemas property");
    }

    #[test]
    fn test_ng_module_with_id() {
        // Test @NgModule with module ID for registration.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule } from '@angular/core';

@NgModule({
    id: 'unique-module-id'
})
export class IdentifiedModule {}
"#;

        let result = transform_angular_file(&allocator, "identified.module.ts", source, None, None);

        // ɵmod should be generated
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );
        // Should contain the module id
        assert!(
            result.code.contains("unique-module-id"),
            "Code should contain the module id, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_ng_module_and_component_in_same_file() {
        // Test that both @NgModule and @Component are properly compiled
        // when they exist in the same file.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule, Component } from '@angular/core';

@Component({
    selector: 'app-root',
    template: '<div>Hello</div>'
})
export class AppComponent {}

@NgModule({
    declarations: [AppComponent],
    bootstrap: [AppComponent]
})
export class AppModule {}
"#;

        let result = transform_angular_file(&allocator, "app.module.ts", source, None, None);

        // Both decorators should be removed
        assert!(!result.code.contains("@NgModule"), "Code should NOT contain @NgModule decorator");
        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator"
        );
        // NgModule should have ɵmod
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );
        // Component should have ɵcmp and ɵfac
        assert!(result.code.contains("static ɵcmp ="));
        assert!(result.code.contains("static ɵfac = "));
        // Should have both ɵɵdefineNgModule and ɵɵdefineComponent
        assert!(result.code.contains("ɵɵdefineNgModule"));
        assert!(result.code.contains("ɵɵdefineComponent"));
    }

    #[test]
    fn test_all_decorators_including_ng_module() {
        // Test that @Component, @Directive, @Injectable, @Pipe, and @NgModule
        // are all properly compiled when they exist in the same file.
        let allocator = Allocator::default();
        let source = r#"
import { Component, Directive, Injectable, Pipe, NgModule } from '@angular/core';

@Pipe({ name: 'myPipe' })
export class MyPipe {}

@Injectable({ providedIn: 'root' })
export class MyService {}

@Directive({ selector: '[myDirective]' })
export class MyDirective {}

@Component({
    selector: 'app-root',
    template: '<div myDirective>{{ value | myPipe }}</div>'
})
export class AppComponent {
    value = 'test';
}

@NgModule({
    declarations: [AppComponent, MyDirective, MyPipe],
    providers: [MyService],
    bootstrap: [AppComponent]
})
export class AppModule {}
"#;

        let result = transform_angular_file(&allocator, "app.module.ts", source, None, None);

        // All decorators should be removed
        assert!(!result.code.contains("@Pipe"), "Code should NOT contain @Pipe decorator");
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator"
        );
        assert!(
            !result.code.contains("@Directive"),
            "Code should NOT contain @Directive decorator"
        );
        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator"
        );
        assert!(!result.code.contains("@NgModule"), "Code should NOT contain @NgModule decorator");

        // All definitions should be generated
        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain .ɵpipe = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain .ɵprov = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵdir = "),
            "Code should contain .ɵdir = , but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain .ɵfac = , but got:\n{}",
            result.code
        );
        assert!(result.code.contains("static ɵcmp ="));
        assert!(result.code.contains("static ɵfac = "));
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );

        // Should have all define functions
        assert!(result.code.contains("ɵɵdefinePipe"));
        assert!(result.code.contains("ɵɵdefineInjectable"));
        assert!(result.code.contains("ɵɵdefineDirective"));
        assert!(result.code.contains("ɵɵdefineComponent"));
        assert!(result.code.contains("ɵɵdefineNgModule"));
    }

    #[test]
    fn test_ng_module_with_forward_refs() {
        // Test @NgModule with forwardRef for circular dependencies.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule, forwardRef } from '@angular/core';

@NgModule({
    declarations: [forwardRef(() => MyComponent)]
})
export class AppModule {}
"#;

        let result =
            transform_angular_file(&allocator, "forward-ref.module.ts", source, None, None);

        // ɵmod should be generated
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain .ɵmod = , but got:\n{}",
            result.code
        );
        // Should have ɵɵdefineNgModule call
        assert!(result.code.contains("ɵɵdefineNgModule"), "Code should contain ɵɵdefineNgModule");
        // When forward refs are detected, arrays may be wrapped in functions
        assert!(result.code.contains("declarations"), "Code should contain declarations");
    }

    #[test]
    fn test_build_import_map_with_resolved_imports() {
        // Test that resolved_imports overrides barrel export paths
        let allocator = Allocator::default();
        let source_type = SourceType::tsx();
        let code = r#"
            import { AriaDisableDirective } from '../a11y';
            import { OtherDirective } from './other';
        "#;
        let parser_ret = Parser::new(&allocator, code, source_type).parse();

        // Without resolved imports, source modules come from the import declaration
        let import_map_without_resolved =
            build_import_map(&allocator, &parser_ret.program.body, None);
        assert_eq!(
            import_map_without_resolved
                .get(&Ident::from("AriaDisableDirective"))
                .map(|i| i.source_module.as_str()),
            Some("../a11y")
        );

        // With resolved imports, source modules are overridden
        let mut resolved_imports = HashMap::new();
        resolved_imports.insert(
            "AriaDisableDirective".to_string(),
            "../a11y/aria-disable.directive".to_string(),
        );

        let import_map_with_resolved =
            build_import_map(&allocator, &parser_ret.program.body, Some(&resolved_imports));

        // AriaDisableDirective should have the resolved path
        assert_eq!(
            import_map_with_resolved
                .get(&Ident::from("AriaDisableDirective"))
                .map(|i| i.source_module.as_str()),
            Some("../a11y/aria-disable.directive"),
            "AriaDisableDirective should use resolved path"
        );

        // OtherDirective should still use the original import path (not in resolved_imports)
        assert_eq!(
            import_map_with_resolved
                .get(&Ident::from("OtherDirective"))
                .map(|i| i.source_module.as_str()),
            Some("./other"),
            "OtherDirective should use original import path"
        );
    }

    #[test]
    fn test_resolved_imports_in_transform_options() {
        // Test that resolved_imports in TransformOptions is used for host directive resolution
        let allocator = Allocator::default();
        let source = r#"
import { Component } from '@angular/core';
import { AriaDisableDirective } from '../a11y';

@Component({
    selector: 'app-test',
    template: '<div>Test</div>',
    hostDirectives: [AriaDisableDirective]
})
export class TestComponent {}
"#;

        let mut resolved_imports = HashMap::new();
        resolved_imports.insert(
            "AriaDisableDirective".to_string(),
            "../a11y/aria-disable.directive".to_string(),
        );

        let mut options = TransformOptions::default();
        options.resolved_imports = Some(resolved_imports);

        let result =
            transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

        assert!(!result.has_errors());
        // The output should contain the host directive feature
        assert!(
            result.code.contains("HostDirectivesFeature"),
            "Code should contain HostDirectivesFeature, but got:\n{}",
            result.code
        );

        // The output should contain an import statement for the resolved path
        // (../a11y/aria-disable.directive), NOT the barrel path (../a11y)
        assert!(
            result.code.contains("'../a11y/aria-disable.directive'"),
            "Code should import from resolved path '../a11y/aria-disable.directive', but got:\n{}",
            result.code
        );

        // The output should NOT contain an import for just the barrel path
        // (unless it's also used for another purpose)
        // Note: The barrel path might still appear in the original import at the top,
        // but the namespace import should use the resolved path
        assert!(
            !result.code.contains("import * as i1 from '../a11y'"),
            "Code should NOT have namespace import from barrel path '../a11y', but got:\n{}",
            result.code
        );
    }

    #[cfg(feature = "cross_file_elision")]
    #[test]
    fn test_cross_file_elision_resolves_host_directive_barrel() {
        use std::fs;
        use tempfile::TempDir;

        // Create a temp directory structure similar to bitwarden:
        // a11y/
        //   aria-disable.directive.ts
        //   index.ts  (re-exports AriaDisableDirective)
        // button/
        //   button.component.ts (imports AriaDisableDirective from '../a11y')

        let dir = TempDir::new().unwrap();

        // Create a11y/aria-disable.directive.ts
        fs::create_dir_all(dir.path().join("a11y")).unwrap();
        fs::write(
            dir.path().join("a11y/aria-disable.directive.ts"),
            r#"
import { Directive } from '@angular/core';

@Directive({
    selector: '[ariaDisable]'
})
export class AriaDisableDirective {}
"#,
        )
        .unwrap();

        // Create a11y/index.ts (barrel export)
        fs::write(
            dir.path().join("a11y/index.ts"),
            "export { AriaDisableDirective } from './aria-disable.directive';\n",
        )
        .unwrap();

        // Create button/button.component.ts
        fs::create_dir_all(dir.path().join("button")).unwrap();
        let component_source = r#"
import { Component } from '@angular/core';
import { AriaDisableDirective } from '../a11y';

@Component({
    selector: 'app-button',
    template: '<button>Click me</button>',
    hostDirectives: [AriaDisableDirective]
})
export class ButtonComponent {}
"#;
        let component_path = dir.path().join("button/button.component.ts");
        fs::write(&component_path, component_source).unwrap();

        let allocator = Allocator::default();
        let mut options = TransformOptions::default();
        options.cross_file_elision = true;
        options.base_dir = Some(dir.path().to_path_buf());

        let result = transform_angular_file(
            &allocator,
            component_path.to_str().unwrap(),
            component_source,
            Some(&options),
            None,
        );

        assert!(!result.has_errors(), "Transform should not have errors: {:?}", result.diagnostics);

        // The output should contain the host directive feature
        assert!(
            result.code.contains("HostDirectivesFeature"),
            "Code should contain HostDirectivesFeature, but got:\n{}",
            result.code
        );

        // The output should use the ORIGINAL import path (barrel path), matching Angular's behavior.
        // Angular's compiler uses original import paths, not barrel-resolved paths.
        assert!(
            result.code.contains("'../a11y'"),
            "Code should import from original barrel path '../a11y', but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_inject_decorator_with_different_type_module_generates_namespace_import() {
        // When @Inject(TOKEN) is used and the type annotation comes from a different module
        // than the token, setClassMetadata should emit a namespace-prefixed type reference
        // and generate the namespace import for the type's module.
        //
        // Example: @Inject(DARK_THEME) darkTheme$: Observable<boolean>
        //   - Token: DARK_THEME from '@app/theme'
        //   - Type: Observable from 'rxjs'
        //   - Expected: { type: i2.Observable, decorators: [{ type: Inject, args: [DARK_THEME] }] }
        //   - Expected import: import * as i2 from "rxjs"
        let allocator = Allocator::default();
        let source = r#"
import { Component, Inject } from '@angular/core';
import { Observable } from 'rxjs';
import { DARK_THEME } from '@app/theme';

@Component({
    selector: 'app-test',
    template: '<div></div>'
})
export class TestComponent {
    constructor(
        @Inject(DARK_THEME) protected readonly darkTheme$: Observable<boolean>,
    ) {}
}
"#;

        let mut options = TransformOptions::default();
        options.emit_class_metadata = true;

        let result =
            transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

        // @Inject(DARK_THEME) now uses namespace imports (i1 for @app/theme),
        // so rxjs gets i2 for Observable.
        assert!(
            result.code.contains("import * as i1 from '@app/theme'"),
            "Should generate namespace import for @app/theme, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("import * as i2 from 'rxjs'"),
            "Should generate namespace import for rxjs, but got:\n{}",
            result.code
        );

        // The factory should use namespace-prefixed DARK_THEME
        assert!(
            result.code.contains("i1.DARK_THEME"),
            "Factory should use namespace-prefixed DARK_THEME, but got:\n{}",
            result.code
        );

        // The setClassMetadata ctor params should reference i2.Observable
        assert!(
            result.code.contains("i2.Observable"),
            "setClassMetadata should use namespace-prefixed Observable, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_inject_decorator_with_service_type_from_different_module() {
        // Similar to the Observable case but with a service type:
        // @Inject(TOKEN) service: AbstractService
        // where TOKEN is from one module and AbstractService is from another.
        let allocator = Allocator::default();
        let source = r#"
import { Component, Inject } from '@angular/core';
import { AbstractService, SERVICE_TOKEN } from './service';
import { Store } from '@ngrx/store';

@Component({
    selector: 'app-test',
    template: '<div></div>'
})
export class TestComponent {
    constructor(
        @Inject(SERVICE_TOKEN) private readonly service: AbstractService,
        private readonly store: Store,
    ) {}
}
"#;

        let mut options = TransformOptions::default();
        options.emit_class_metadata = true;

        let result =
            transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

        // The type 'AbstractService' is imported from './service' and should get a namespace import
        // even though the @Inject token 'SERVICE_TOKEN' is also from './service'.
        // The factory generates i1 for @ngrx/store (Store), so ./service should get i2
        // for the metadata type reference.
        assert!(
            result.code.contains("i1.Store") || result.code.contains("i2.Store"),
            "Should generate namespace-prefixed Store reference, but got:\n{}",
            result.code
        );

        // AbstractService should get a namespace-prefixed reference in metadata
        // (the exact index depends on registration order, but it must be namespace-prefixed)
        assert!(
            result.code.contains("i1.AbstractService")
                || result.code.contains("i2.AbstractService"),
            "setClassMetadata should use namespace-prefixed AbstractService, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_inject_decorator_with_type_only_import_skips_namespace() {
        // When the type annotation is from a type-only import (`import type { X }`),
        // setClassMetadata should NOT generate a namespace import for it because
        // type-only imports are erased at runtime and don't resolve to values.
        // Angular's compiler uses typeToValue() which returns null for interfaces/types.
        let allocator = Allocator::default();
        let source = r#"
import { Component, Inject, InjectionToken } from '@angular/core';
import type { SomeInterface } from './some.interface';

const MY_TOKEN = new InjectionToken<SomeInterface>('my-token');

@Component({
    selector: 'app-test',
    template: '<div></div>'
})
export class TestComponent {
    constructor(
        @Inject(MY_TOKEN) private readonly data: SomeInterface,
    ) {}
}
"#;

        let mut options = TransformOptions::default();
        options.emit_class_metadata = true;

        let result =
            transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

        // Should NOT generate a namespace import for ./some.interface
        // (the original `import type` statement may still appear, but no `import * as iN`)
        assert!(
            !result.code.contains("import * as i1 from './some.interface'"),
            "Should NOT generate namespace import for type-only import, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_inject_decorator_with_inline_type_specifier_skips_namespace() {
        // Same as above but with inline type specifier: `import { type X } from '...'`
        let allocator = Allocator::default();
        let source = r#"
import { Component, Inject, InjectionToken } from '@angular/core';
import { type SomeInterface } from './some.interface';

const MY_TOKEN = new InjectionToken<SomeInterface>('my-token');

@Component({
    selector: 'app-test',
    template: '<div></div>'
})
export class TestComponent {
    constructor(
        @Inject(MY_TOKEN) private readonly data: SomeInterface,
    ) {}
}
"#;

        let mut options = TransformOptions::default();
        options.emit_class_metadata = true;

        let result =
            transform_angular_file(&allocator, "test.component.ts", source, Some(&options), None);

        // Should NOT generate a namespace import for ./some.interface
        assert!(
            !result.code.contains("import * as i1 from './some.interface'"),
            "Should NOT generate namespace import for inline type-only import, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_directive_factory_deps_get_correct_namespace_resolution() {
        // Regression test for bug where resolve_factory_dep_namespaces() was NOT called
        // for @Directive constructor deps. This caused bare ReadVar references (e.g., Store)
        // to remain unresolved, resulting in incorrect namespace prefixes at runtime
        // (e.g., i0.Store instead of the correct i1.Store).
        //
        // The fix: Added resolve_factory_dep_namespaces() call for directive deps in
        // the directive processing path of transform_angular_file().
        let allocator = Allocator::default();
        let source = r#"
import { Directive } from '@angular/core';
import { Store } from '@ngrx/store';
import { SomeService } from '@app/services';

@Directive({ selector: '[myDir]' })
export class MyDirective {
    constructor(private store: Store, private svc: SomeService) {}
}
"#;

        let result = transform_angular_file(&allocator, "my.directive.ts", source, None, None);

        assert!(!result.has_errors(), "Transform should not have errors: {:?}", result.diagnostics);

        // Verify namespace imports are generated for the external modules
        assert!(
            result.code.contains("import * as i1 from '@ngrx/store'"),
            "Should generate namespace import for @ngrx/store, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("import * as i2 from '@app/services'"),
            "Should generate namespace import for @app/services, but got:\n{}",
            result.code
        );

        // Verify the factory uses the correct namespace prefixes for deps
        // Store should be i1.Store (from @ngrx/store), NOT i0.Store
        assert!(
            result.code.contains("i1.Store"),
            "Factory should reference Store as i1.Store (from @ngrx/store), but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("i0.Store"),
            "Factory should NOT reference Store as i0.Store (that's @angular/core), but got:\n{}",
            result.code
        );

        // SomeService should be i2.SomeService (from @app/services), NOT i0.SomeService
        assert!(
            result.code.contains("i2.SomeService"),
            "Factory should reference SomeService as i2.SomeService (from @app/services), but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("i0.SomeService"),
            "Factory should NOT reference SomeService as i0.SomeService (that's @angular/core), but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_pipe_and_injectable_on_same_class() {
        // Test that @Pipe + @Injectable on the same class both get compiled.
        // Issue: https://github.com/voidzero-dev/oxc-angular-compiler/issues/65
        let allocator = Allocator::default();
        let source = r#"
import { Pipe, Injectable, PipeTransform } from '@angular/core';

@Pipe({ name: 'osTypeIcon' })
@Injectable({ providedIn: 'root' })
export class OSTypeIconPipe implements PipeTransform {
  transform(os: string): string {
    return os;
  }
}
"#;

        let result = transform_angular_file(&allocator, "os-type-icon.pipe.ts", source, None, None);

        // Both decorators should be removed
        assert!(
            !result.code.contains("@Pipe"),
            "Code should NOT contain @Pipe decorator, but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );

        // Both definitions should be generated
        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain ɵpipe definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain ɵprov definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain ɵfac definition, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_injectable_and_pipe_reversed_order() {
        // Test that @Injectable + @Pipe (reversed order) on the same class both get compiled.
        let allocator = Allocator::default();
        let source = r#"
import { Pipe, Injectable, PipeTransform } from '@angular/core';

@Injectable({ providedIn: 'root' })
@Pipe({ name: 'osTypeIcon' })
export class OSTypeIconPipe implements PipeTransform {
  transform(os: string): string {
    return os;
  }
}
"#;

        let result = transform_angular_file(&allocator, "os-type-icon.pipe.ts", source, None, None);

        // Both decorators should be removed
        assert!(
            !result.code.contains("@Pipe"),
            "Code should NOT contain @Pipe decorator, but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );

        // Both definitions should be generated
        assert!(
            result.code.contains("static ɵpipe = "),
            "Code should contain ɵpipe definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain ɵprov definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain ɵfac definition, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_component_and_injectable_on_same_class() {
        // Test that @Component + @Injectable on the same class both get compiled.
        // Angular allows this: @Component is PRIMARY, @Injectable is SHARED.
        let allocator = Allocator::default();
        let source = r#"
import { Component, Injectable } from '@angular/core';

@Component({
  selector: 'test-cmp',
  template: '<div>test</div>'
})
@Injectable()
export class TestCmp {}
"#;

        let result = transform_angular_file(&allocator, "test.component.ts", source, None, None);

        assert!(
            !result.code.contains("@Component"),
            "Code should NOT contain @Component decorator, but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵcmp = "),
            "Code should contain ɵcmp definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain ɵprov definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain ɵfac definition, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_directive_and_injectable_on_same_class() {
        // Test that @Directive + @Injectable on the same class both get compiled.
        // Angular allows this: @Directive is PRIMARY, @Injectable is SHARED.
        let allocator = Allocator::default();
        let source = r#"
import { Directive, Injectable } from '@angular/core';

@Directive({
  selector: '[testDir]'
})
@Injectable()
export class TestDir {}
"#;

        let result = transform_angular_file(&allocator, "test.directive.ts", source, None, None);

        assert!(
            !result.code.contains("@Directive"),
            "Code should NOT contain @Directive decorator, but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵdir = "),
            "Code should contain ɵdir definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain ɵprov definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain ɵfac definition, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_ng_module_and_injectable_on_same_class() {
        // Test that @NgModule + @Injectable on the same class both get compiled.
        // Angular allows this: @NgModule is PRIMARY, @Injectable is SHARED.
        let allocator = Allocator::default();
        let source = r#"
import { NgModule, Injectable } from '@angular/core';

@NgModule({})
@Injectable()
export class TestNgModule {}
"#;

        let result = transform_angular_file(&allocator, "test.module.ts", source, None, None);

        assert!(
            !result.code.contains("@NgModule"),
            "Code should NOT contain @NgModule decorator, but got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("@Injectable"),
            "Code should NOT contain @Injectable decorator, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵmod = "),
            "Code should contain ɵmod definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵprov = "),
            "Code should contain ɵprov definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵfac = "),
            "Code should contain ɵfac definition, but got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("static ɵinj = "),
            "Code should contain ɵinj definition, but got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_directive_host_directives_get_namespace_resolution() {
        // Regression test for https://github.com/voidzero-dev/oxc-angular-compiler/issues/68
        // hostDirectives references must use namespace-prefixed references (e.g., i1.BrnTooltipTrigger)
        // instead of bare variable references (e.g., BrnTooltipTrigger), because the original
        // named import may be elided and replaced by a namespace import.
        let allocator = Allocator::default();
        let source = r#"
import { Directive } from '@angular/core';
import { BrnTooltipTrigger } from '@spartan-ng/brain/tooltip';

@Directive({
    selector: '[uTooltip]',
    hostDirectives: [{ directive: BrnTooltipTrigger }]
})
export class UnityTooltipTrigger {}
"#;

        let result = transform_angular_file(&allocator, "tooltip.directive.ts", source, None, None);

        assert!(!result.has_errors(), "Transform should not have errors: {:?}", result.diagnostics);

        // Verify namespace import is generated for the external module
        assert!(
            result.code.contains("import * as i1 from '@spartan-ng/brain/tooltip'"),
            "Should generate namespace import for @spartan-ng/brain/tooltip, but got:\n{}",
            result.code
        );

        // Verify the host directive uses the namespace-prefixed reference
        assert!(
            result.code.contains("i1.BrnTooltipTrigger"),
            "Host directive should reference BrnTooltipTrigger as i1.BrnTooltipTrigger, but got:\n{}",
            result.code
        );

        // Verify there's no bare BrnTooltipTrigger reference in the features array
        // (it should only appear in the import statement and as i1.BrnTooltipTrigger)
        let features_section = result.code.split("features:").nth(1);
        if let Some(features) = features_section {
            assert!(
                !features.contains("BrnTooltipTrigger")
                    || features.contains("i1.BrnTooltipTrigger"),
                "Features should NOT contain bare BrnTooltipTrigger reference, but got:\n{}",
                result.code
            );
        }
    }
}
